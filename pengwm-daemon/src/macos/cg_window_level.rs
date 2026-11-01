//! CG window-level lookup — the structural overlay signal (ADR-0002).
//!
//! "Shows on top on every workspace" is a non-normal `NSWindow.Level`
//! (0 = normal, 3 = floating, 8 = modal panel, ...). AX does not expose
//! levels, but `CGWindowListCopyWindowInfo` reports `kCGWindowLayer` per
//! on-screen window, and PengWM's window ids *are* CG window numbers
//! (`_AXUIElementGetWindow`). One snapshot answers for the whole screen,
//! so the poll path matches its windows against one map; the
//! notification path does a single-window query.
//!
//! This is how yabai (`window_level_is_standard`) and AeroSpace
//! (`windowLevelCache`) decide the same thing; see
//! `docs/research/untracked-windows.md`.

use std::collections::HashMap;

use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::{CFNumber, CFNumberRef};
use core_foundation::string::CFString;
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowListOptionExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly,
};

use pengwm_core::tree::WindowId;

use super::cf_intern::intern;

const LAYER_NORMAL: i32 = 0;

pub fn is_overlay_level(level: i32) -> bool {
    level != LAYER_NORMAL
}

/// The dictionary value for `key` as an i64 via `CFNumber` (get-rule
/// borrow — the value belongs to the snapshot array).
fn number_value(dict: CFDictionaryRef, key: &'static str) -> Option<i64> {
    unsafe {
        let typed: CFDictionary<CFString, CFType> = CFDictionary::wrap_under_get_rule(dict);
        let item = typed.find(intern(key))?;
        let raw = TCFType::as_concrete_TypeRef(&*item);
        CFNumber::wrap_under_get_rule(raw as CFNumberRef).to_i64()
    }
}

/// On-screen window id → CG level, from one `CGWindowListCopyWindowInfo`
/// snapshot. Windows that dropped out between the AX poll and this call
/// simply have no entry; callers treat absence as "normal level"
/// (conservative: the subrole classification stands).
pub fn window_levels() -> HashMap<WindowId, i32> {
    let mut levels = HashMap::new();
    let Some(list) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListOptionExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return levels;
    };
    unsafe {
        let array_ref = list.as_concrete_TypeRef();
        for i in 0..CFArrayGetCount(array_ref) {
            let dict_ref = CFArrayGetValueAtIndex(array_ref, i) as CFDictionaryRef;
            if dict_ref.is_null() {
                continue;
            }
            let (Some(number), Some(layer)) = (
                number_value(dict_ref, "kCGWindowNumber"),
                number_value(dict_ref, "kCGWindowLayer"),
            ) else {
                continue;
            };
            levels.insert(number as WindowId, layer as i32);
        }
    }
    levels
}

/// Frame of the window with `number`, from a fresh all-windows snapshot
/// — including windows that are not on screen (parked/hides), which an
/// on-screen-only query silently omits. The WindowServer's own verdict
/// is the ground truth the write-verify path (`cg_ground_outcome`)
/// leans on: it cannot phantom-succeed the way an app's AX readback
/// can. `None` when the id is not in the snapshot (CG answered nothing,
/// or the window closed between the write and this probe).
pub fn window_cg_frame(window_id: WindowId) -> Option<pengwm_core::layout::Rect> {
    let list = copy_window_info(
        kCGWindowListOptionExcludeDesktopElements, // 0 = all windows
        kCGNullWindowID,
    )?;
    unsafe {
        let array_ref = list.as_concrete_TypeRef();
        for i in 0..CFArrayGetCount(array_ref) {
            let dict_ref = CFArrayGetValueAtIndex(array_ref, i) as CFDictionaryRef;
            if dict_ref.is_null() {
                continue;
            }
            let Some(number) = number_value(dict_ref, "kCGWindowNumber") else {
                continue;
            };
            if number as WindowId != window_id {
                continue;
            }
            let Some(bounds_ref) = dict_value(dict_ref, "kCGWindowBounds") else {
                continue;
            };
            let (Some(x), Some(y), Some(w), Some(h)) = (
                number_value(bounds_ref, "X"),
                number_value(bounds_ref, "Y"),
                number_value(bounds_ref, "Width"),
                number_value(bounds_ref, "Height"),
            ) else {
                continue;
            };
            return Some(pengwm_core::layout::Rect::new(
                x as f64,
                y as f64,
                w as f64,
                h as f64,
            ));
        }
    }
    None
}

/// The dictionary value for `key` as a `CFDictionaryRef` (get-rule
/// borrow — the value belongs to the snapshot array). The bounds
/// sub-dictionary lives under `kCGWindowBounds`.
fn dict_value(dict: CFDictionaryRef, key: &'static str) -> Option<CFDictionaryRef> {
    unsafe {
        let typed: CFDictionary<CFString, CFType> = CFDictionary::wrap_under_get_rule(dict);
        let item = typed.find(intern(key))?;
        let raw = TCFType::as_concrete_TypeRef(&*item);
        Some(raw as CFDictionaryRef)
    }
}

/// Level of one window, from a fresh on-screen snapshot. `None` when the
/// window is not on screen (or CG answered nothing) — the caller keeps
/// the subrole classification.
///
/// No snapshot cache, deliberately: the only per-call caller is the
/// `WindowCreated` path classifying a window **for the first time**, and
/// first-classification-wins (ADR-0002) makes a wrong class sticky. A
/// TTL-cached snapshot taken before the new window existed would answer
/// `None` for it and hand a level≠0 overlay (a PiP) to the subrole
/// answer — `AXFloatingWindow` → popup, exactly the misrouting the
/// level gate exists to prevent. A cache hit requires the window to have
/// been on screen at snapshot time, but a first-time classification is
/// by definition for a window that may not have been; absence-from-
/// cache would then have to force a fresh snapshot anyway, which
/// collapses the cache back to one snapshot per created window. The
/// interned dict keys are the win that was actually available here.
pub fn window_level(window_id: WindowId) -> Option<i32> {
    window_levels().get(&window_id).copied()
}
