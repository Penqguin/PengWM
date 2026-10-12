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

const LAYER_NORMAL: i32 = 0;

pub fn is_overlay_level(level: i32) -> bool {
    level != LAYER_NORMAL
}

/// The dictionary value for `key` as an i64 via `CFNumber` (get-rule
/// borrow — the value belongs to the snapshot array).
fn number_value(dict: CFDictionaryRef, key: &str) -> Option<i64> {
    unsafe {
        let typed: CFDictionary<CFString, CFType> = CFDictionary::wrap_under_get_rule(dict);
        let item = typed.find(CFString::new(key))?;
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

/// Level of one window, from a fresh on-screen snapshot. `None` when the
/// window is not on screen (or CG answered nothing) — the caller keeps
/// the subrole classification.
pub fn window_level(window_id: WindowId) -> Option<i32> {
    window_levels().get(&window_id).copied()
}
