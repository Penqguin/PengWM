use std::collections::HashMap;

use pengwm_core::layout::{HidePlacement, Rect, WriteOutcome};
use pengwm_core::tree::WindowId;

/// Typed per-window classification returned across the seam by
/// `OsAdapter::window_kind`. Replaces the old binary manageable/dropped
/// gate: discovery classifies instead of dropping, so tree routing, popup
/// handling, and the background sweep share one answer. `Standard` tiles;
/// `Dialog` / `SystemDialog` / `Floating` become workspace-bound popups;
/// `Sheet` and `Unknown` are dropped at the gate exactly as the old
/// `AXStandardWindow`-only check did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowClass {
    /// `AXStandardWindow` — tiled like everything else.
    Standard,
    /// App dialogs and modal panels (`AXDialog`).
    Dialog,
    /// System dialogs (`AXSystemDialog`).
    SystemDialog,
    /// Floating windows: PiP, launcher panels, Chromium popups
    /// (`AXFloatingWindow`).
    Floating,
    /// Sheets attached to a parent window — left alone, as before.
    Sheet,
    /// Anything else, including unreadable or missing attributes.
    Unknown,
}

impl WindowClass {
    /// True when the window becomes a workspace-bound popup. Restricted
    /// apps additionally force their `Standard` windows into this fate —
    /// that check lives with routing, which knows the config.
    pub fn is_popup(self) -> bool {
        matches!(
            self,
            WindowClass::Dialog | WindowClass::SystemDialog | WindowClass::Floating
        )
    }

    /// True when discovery should cache + observe + event the window.
    /// Sheets and unknowns are dropped, exactly as the old gate did;
    /// everything else is now seen.
    pub fn is_manageable(self) -> bool {
        !matches!(self, WindowClass::Sheet | WindowClass::Unknown)
    }

    /// Pure role/subrole → `WindowClass` mapping. No FFI: the FFI caller
    /// (`ax_element::classify`) reads the attributes and hands the strings
    /// over; unit tests drive this directly.
    pub fn classify_window(role: Option<&str>, subrole: Option<&str>) -> WindowClass {
        if role != Some(AX_WINDOW_ROLE) {
            return WindowClass::Unknown;
        }
        match subrole {
            Some(AX_STANDARD_WINDOW_SUBROLE) => WindowClass::Standard,
            Some(AX_DIALOG_SUBROLE) => WindowClass::Dialog,
            Some(AX_SYSTEM_DIALOG_SUBROLE) => WindowClass::SystemDialog,
            Some(AX_FLOATING_WINDOW_SUBROLE) => WindowClass::Floating,
            Some(AX_SHEET_SUBROLE) => WindowClass::Sheet,
            _ => WindowClass::Unknown,
        }
    }
}

/// AX role/subrole strings. Mirrors the `accessibility_sys` constants,
/// which are `CFStringRef` statics and therefore not plain `&'static str` —
/// keeping the values here is what makes the mapping pure and unit-testable.
const AX_WINDOW_ROLE: &str = "AXWindow";
const AX_STANDARD_WINDOW_SUBROLE: &str = "AXStandardWindow";
const AX_DIALOG_SUBROLE: &str = "AXDialog";
const AX_SYSTEM_DIALOG_SUBROLE: &str = "AXSystemDialog";
const AX_FLOATING_WINDOW_SUBROLE: &str = "AXFloatingWindow";
const AX_SHEET_SUBROLE: &str = "AXSheet";

pub trait ObserverRegistry {
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
}

pub trait OsAdapter: ObserverRegistry {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&self, pid: i32) -> Vec<WindowId>;
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> WriteOutcome;
    /// Read back the current OS rect. Lets verify-and-retry policy and tests
    /// observe whether a write landed without reaching through FFI.
    fn window_rect(&self, window_id: WindowId) -> Option<Rect>;
    /// Typed classification of a tracked window. `None` when the window is
    /// not in the element cache (never seen or evicted). Routing asks this
    /// once at creation: `Standard` tiles, popup classes overlay, and the
    /// rest never reached the daemon anyway.
    fn window_kind(&self, window_id: WindowId) -> Option<WindowClass>;
    /// Bring the window to the front of the stacking order (`AXRaise`).
    /// Popups re-assert it on switch-back reveal so overlays return on top.
    fn raise_window(&self, window_id: WindowId);
    fn focus_window(&self, window_id: WindowId);
    fn close_window(&self, window_id: WindowId);
    /// Hide windows at precomputed placements. `placements` maps each
    /// `WindowId` to a `HidePlacement` variant so the adapter can match on
    /// intent instead of guessing from raw coordinates (no magic threshold).
    /// `StateManager` computes placements so the adapter stays display-agnostic
    /// and `pengwm-core` stays pure.
    fn hide_windows(&self, placements: &HashMap<WindowId, HidePlacement>);
    /// True when the window is minimized or hidden (per-window `AXHidden`,
    /// `AXMinimized`, or its app is hidden). Used by the periodic reconcile so
    /// hidden windows stop being tiled even when AX notifications are missed.
    fn window_is_hidden(&self, window_id: WindowId) -> bool;
    fn app_bundle_id(&self, pid: i32) -> Option<String>;
    /// Human-readable display name for the app owning `pid` (e.g. "Safari"),
    /// distinct from its bundle id ("com.apple.Safari"). Drives the menubar's
    /// per-window app labels.
    fn app_name(&self, pid: i32) -> Option<String>;
}

#[derive(Clone)]
pub struct DisplayInfo {
    pub id: u32,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_windows_classify_standard() {
        assert_eq!(
            WindowClass::classify_window(Some("AXWindow"), Some("AXStandardWindow")),
            WindowClass::Standard
        );
    }

    #[test]
    fn popup_subroles_classify_popup() {
        for subrole in ["AXDialog", "AXSystemDialog", "AXFloatingWindow"] {
            let class = WindowClass::classify_window(Some("AXWindow"), Some(subrole));
            assert!(class.is_popup(), "{subrole} is popup");
            assert!(class.is_manageable(), "{subrole} is seen");
        }
    }

    #[test]
    fn sheets_and_unknowns_drop_as_before() {
        let sheet = WindowClass::classify_window(Some("AXWindow"), Some("AXSheet"));
        assert!(!sheet.is_popup() && !sheet.is_manageable());
        let unlabeled = WindowClass::classify_window(Some("AXWindow"), Some("AXUnlabeled"));
        assert!(!unlabeled.is_popup() && !unlabeled.is_manageable());
        let missing = WindowClass::classify_window(Some("AXWindow"), None);
        assert_eq!(missing, WindowClass::Unknown);
        let non_window_role =
            WindowClass::classify_window(Some("AXButton"), Some("AXStandardWindow"));
        assert_eq!(non_window_role, WindowClass::Unknown);
    }
}
