//! Read-only macOS native-Space query — the detector behind the WM's
//! Space gate (ADR-0007).
//!
//! macOS Spaces are invisible to AX and to public CG: `kCGWindowWorkspace`
//! reads 0 on modern macOS. The WindowServer's own per-display space list
//! lives in SkyLight's `SLSCopyManagedDisplaySpaces`, the same read-only
//! SPI yabai/AeroSpace read. PengWM only *reads* here — `dlopen` + `dlsym`,
//! no injection, no SIP changes.
//!
//! Gate semantics: a display is on its first desktop iff its current space
//! is the first user desktop (`type 0`) in that display's ordered space
//! list. Fullscreen (`type 4`) and system (`type 2`) spaces are exempt —
//! the existing fullscreen parking machinery owns those transitions. A
//! display missing from the snapshot is treated as allowed by the state
//! layer (fail open).

use std::ffi::{c_void, CString};
use std::sync::OnceLock;

use core_foundation::array::{CFArray, CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::{CFNumber, CFNumberRef};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::uuid::{CFUUIDCreateFromString, CFUUIDRef, CFUUID};

use super::cf_intern::intern;

const SKYLIGHT_PATH: &str =
    "/System/Library/PrivateFrameworks/SkyLight.framework/Versions/A/SkyLight";

/// SkyLight's space type for a normal user desktop.
pub const SPACE_TYPE_USER: u32 = 0;

/// One display's native-space state, as answered by the WindowServer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySpaceState {
    pub display_id: u32,
    pub current_space: u64,
    pub current_type: u32,
    /// First `type 0` space in the display's ordered list — the one
    /// desktop PengWM is allowed to manage on that display.
    pub first_user_space: Option<u64>,
}

impl DisplaySpaceState {
    /// The gate decision: true when the WM may act on this display.
    pub fn on_first_desktop(&self) -> bool {
        if self.current_type != SPACE_TYPE_USER {
            return true;
        }
        self.first_user_space
            .map(|first| first == self.current_space)
            .unwrap_or(true)
    }
}

/// One resolved SkyLight entry point. The dlopen handle is intentionally
/// never closed: these live for the process lifetime.
struct Symbols {
    main_connection: unsafe extern "C" fn() -> u32,
    copy_managed_display_spaces: unsafe extern "C" fn(u32) -> CFArrayRef,
    space_get_type: unsafe extern "C" fn(u32, u64) -> u32,
    display_id_from_uuid: unsafe extern "C" fn(CFUUIDRef) -> u32,
}

/// The per-display native-space snapshot. `None` when SkyLight (or one of
/// the symbols) is unavailable — the state layer then leaves every display
/// fully managed.
pub fn snapshot() -> Option<Vec<DisplaySpaceState>> {
    let syms = symbols()?;
    unsafe {
        let cid = (syms.main_connection)();
        let array_ref = (syms.copy_managed_display_spaces)(cid);
        if array_ref.is_null() {
            return None;
        }
        // Create rule: the CFArray owns the +1 and releases on drop.
        let array: CFArray<CFType> = CFArray::wrap_under_create_rule(array_ref);
        let raw_array = array.as_concrete_TypeRef();
        let count = CFArrayGetCount(raw_array);
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let display_ref = CFArrayGetValueAtIndex(raw_array, i) as CFDictionaryRef;
            if display_ref.is_null() {
                continue;
            }
            let Some(uuid) = string_value(display_ref, "Display Identifier") else {
                continue;
            };
            let Some(display_id) = display_id_for_uuid(&uuid, syms) else {
                continue;
            };
            let Some(current_ref) = value(display_ref, "Current Space") else {
                continue;
            };
            let Some(current_space) = number_value(current_ref as CFDictionaryRef, "id64") else {
                continue;
            };
            let current_space = current_space as u64;
            let current_type = (syms.space_get_type)(cid, current_space);
            out.push(DisplaySpaceState {
                display_id,
                current_space,
                current_type,
                first_user_space: first_user_space(cid, display_ref, syms),
            });
        }
        Some(out)
    }
}

fn symbols() -> Option<&'static Symbols> {
    static SYMBOLS: OnceLock<Option<Symbols>> = OnceLock::new();
    SYMBOLS.get_or_init(load).as_ref()
}

fn load() -> Option<Symbols> {
    // SAFETY: fixed framework path, symbol names are NUL-terminated
    // CStrings that outlive the calls, and every pointer is checked.
    unsafe {
        let path = CString::new(SKYLIGHT_PATH).ok()?;
        let handle = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            log::warn!(
                "native-space gate: SkyLight unavailable — every display stays fully managed"
            );
            return None;
        }
        let main_connection = resolve(handle, "CGSMainConnectionID")?;
        let copy_managed_display_spaces = resolve(handle, "SLSCopyManagedDisplaySpaces")?;
        let space_get_type = resolve(handle, "SLSSpaceGetType")?;
        let display_id_from_uuid = resolve(handle, "CGDisplayGetDisplayIDFromUUID")?;
        let main_connection: unsafe extern "C" fn() -> u32 = std::mem::transmute(main_connection);
        let copy_managed_display_spaces: unsafe extern "C" fn(u32) -> CFArrayRef =
            std::mem::transmute(copy_managed_display_spaces);
        let space_get_type: unsafe extern "C" fn(u32, u64) -> u32 =
            std::mem::transmute(space_get_type);
        let display_id_from_uuid: unsafe extern "C" fn(CFUUIDRef) -> u32 =
            std::mem::transmute(display_id_from_uuid);
        Some(Symbols {
            main_connection,
            copy_managed_display_spaces,
            space_get_type,
            display_id_from_uuid,
        })
    }
}

unsafe fn resolve(handle: *mut c_void, name: &str) -> Option<*mut c_void> {
    let name = CString::new(name).ok()?;
    let symbol = libc::dlsym(handle, name.as_ptr());
    if symbol.is_null() {
        log::warn!(
            "native-space gate: SkyLight symbol {} unavailable — every display stays fully managed",
            name.to_string_lossy()
        );
        return None;
    }
    Some(symbol)
}

/// The dictionary value for `key`, borrowed from the enclosing snapshot.
unsafe fn value(dict: CFDictionaryRef, key: &'static str) -> Option<CFTypeRef> {
    let typed: CFDictionary<CFString, CFType> = CFDictionary::wrap_under_get_rule(dict);
    let item = typed.find(intern(key))?;
    Some(TCFType::as_concrete_TypeRef(&*item))
}

unsafe fn number_value(dict: CFDictionaryRef, key: &'static str) -> Option<i64> {
    let raw = value(dict, key)?;
    CFNumber::wrap_under_get_rule(raw as CFNumberRef).to_i64()
}

unsafe fn string_value(dict: CFDictionaryRef, key: &'static str) -> Option<String> {
    let raw = value(dict, key)?;
    Some(CFString::wrap_under_get_rule(raw as CFStringRef).to_string())
}

unsafe fn display_id_for_uuid(uuid: &str, syms: &Symbols) -> Option<u32> {
    let text = CFString::new(uuid);
    let uuid_ref = CFUUIDCreateFromString(std::ptr::null(), text.as_concrete_TypeRef());
    if uuid_ref.is_null() {
        return None;
    }
    let uuid = CFUUID::wrap_under_create_rule(uuid_ref);
    let id = (syms.display_id_from_uuid)(uuid.as_concrete_TypeRef());
    (id != 0).then_some(id)
}

unsafe fn first_user_space(cid: u32, display: CFDictionaryRef, syms: &Symbols) -> Option<u64> {
    let spaces_ref = value(display, "Spaces")? as CFArrayRef;
    if spaces_ref.is_null() {
        return None;
    }
    for i in 0..CFArrayGetCount(spaces_ref) {
        let space_ref = CFArrayGetValueAtIndex(spaces_ref, i) as CFDictionaryRef;
        if space_ref.is_null() {
            continue;
        }
        let Some(id) = number_value(space_ref, "id64") else {
            continue;
        };
        let id = id as u64;
        if (syms.space_get_type)(cid, id) == SPACE_TYPE_USER {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(
        current_type: u32,
        current_space: u64,
        first_user_space: Option<u64>,
    ) -> DisplaySpaceState {
        DisplaySpaceState {
            display_id: 1,
            current_space,
            current_type,
            first_user_space,
        }
    }

    #[test]
    fn first_user_desktop_is_allowed() {
        assert!(state(SPACE_TYPE_USER, 10, Some(10)).on_first_desktop());
    }

    #[test]
    fn later_user_desktop_is_gated() {
        assert!(!state(SPACE_TYPE_USER, 11, Some(10)).on_first_desktop());
    }

    #[test]
    fn fullscreen_space_is_exempt() {
        assert!(state(4, 12, Some(10)).on_first_desktop());
    }

    #[test]
    fn system_space_is_exempt() {
        assert!(state(2, 13, Some(10)).on_first_desktop());
    }

    #[test]
    fn a_display_with_no_user_desktop_is_allowed() {
        assert!(state(SPACE_TYPE_USER, 14, None).on_first_desktop());
    }
}
