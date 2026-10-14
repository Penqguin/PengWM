use std::collections::{HashMap, HashSet};
use std::time::Duration;

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
    /// Cross-workspace untracked overlay (see *UntrackedWindow* in
    /// CONTEXT.md): never tiled, never parked, never hidden with its
    /// workspace. Set at discovery by the CG window-level gate, or at
    /// routing by the `float_apps` bundle list.
    Untracked,
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

    /// The CG window-level gate: a window whose level is not the normal
    /// level (0) declared itself an overlay — it floats above every
    /// level-0 window and (for system overlays) across Spaces, so it is
    /// untracked no matter what its subrole says (level beats popup
    /// classes; a PiP window at level 3 stops being workspace-bound).
    /// `None` (unqueried — unreadable CG snapshot) keeps the subrole
    /// answer; first-classification wins, same as popups.
    pub fn apply_level_gate(class: WindowClass, level: Option<i32>) -> WindowClass {
        if class.is_manageable() && level.is_some_and(|l| l != 0) {
            return WindowClass::Untracked;
        }
        class
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
    /// Pids with an attached observer — the dead-pid sweep's suspect set:
    /// an observer whose app died without a termination notification is a
    /// stranded `AXObserverRef` + app element + runloop-source retain.
    fn observed_pids(&self) -> Vec<i32>;
    /// Replace observers attached longer than `older_than`. AX accumulates
    /// per-registration records for an observer's lifetime (no
    /// per-notification removal), so the tick re-attaches on an age bound —
    /// the same replace-on-attach path the wake resync uses. The adapter owns
    /// the mechanics; the state owns the policy (the interval), passed here
    /// so configuration can tune it from one place later.
    fn refresh_aged_observers(&mut self, older_than: Duration);
}

/// An app's answer to the window poll. The distinction the poll seam once
/// threw away: the app **answered** (possibly with zero windows — the
/// closed-only-window signature) versus AX **could not answer** (the
/// post-wake blackout signature). Both used to collapse into an empty
/// `Vec`, forcing every judgment sweep to treat an empty answer as
/// inadmissible — which stranded a closed-only-window app in the tree.
pub enum WindowListing {
    /// The app's AX element answered the `AXWindows` query successfully;
    /// the contents (possibly empty) are the app's own verdict and
    /// admissible death evidence for its tracked windows.
    Answered(Vec<WindowId>),
    /// AX errored / the app element would not answer. The blackout
    /// signature: no structural judgment may run on this listing.
    NoAnswer,
}

pub trait OsAdapter: ObserverRegistry {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&self, pid: i32) -> WindowListing;
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
    /// True when the window is in native fullscreen (the `AXFullScreen`
    /// attribute). Drives full-screen parking: a fullscreened tree window
    /// leaves the tree and reveals back through the normal funnel on exit.
    /// No dedicated AX notification exists, so this is probed on focused /
    /// pinned-write events and per tick for the parked set.
    fn window_is_fullscreen(&self, window_id: WindowId) -> bool;
    /// Release every OS-side resource cached for `window_id` (element
    /// caches, per-window notification records). `StateManager` calls this
    /// from the one untrack funnel (`on_window_destroyed`) so both the
    /// notification-driven and the missed-notification paths evict the
    /// same way. Idempotent — evicting an already-gone window is a no-op.
    fn forget_window(&self, window_id: WindowId);
    /// Release every OS-side resource cached for `pid`, including cached
    /// elements for windows the store never tracked — an excluded app's
    /// windows created after bootstrap (their routing returned before
    /// `register`), or any other cached-but-untracked member of the pid.
    /// `StateManager` calls this from `on_app_terminated` after the
    /// per-window untrack loop, so nothing the observer cached outlives
    /// the app. Idempotent; returns the number of elements evicted.
    fn forget_pid(&self, pid: i32) -> usize;
    /// Evict every cached element whose window is not in `keep`. The tick
    /// sweep's cache-vs-store diff: entries the store doesn't track have
    /// no eviction path of their own (excluded apps, missed terminations,
    /// windows that closed during sleep between the resync and the
    /// untrack), so the tick prunes them here. Returns the eviction count.
    fn prune_untracked_elements(&self, keep: &HashSet<WindowId>) -> usize;
    /// Every pid NSWorkspace reports as running, regardless of activation
    /// policy. `running_app_pids` filters to Regular apps because routing
    /// cares about real apps; the dead-pid sweep must not — accessory-policy
    /// apps own windows too, and reading their absence as death would
    /// untrack them every tick. One authoritative liveness list per call.
    fn running_pids_any_policy(&self) -> Vec<i32>;
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

    #[test]
    fn non_zero_window_level_means_untracked_regardless_of_subrole() {
        // Level beats the subrole: a PiP window (`AXFloatingWindow`, level 3)
        // and a deck window (`AXStandardWindow`, level 3) are cross-workspace
        // overlays, not workspace-bound popups and not tiles.
        for class in [
            WindowClass::Standard,
            WindowClass::Dialog,
            WindowClass::SystemDialog,
            WindowClass::Floating,
        ] {
            assert_eq!(
                WindowClass::apply_level_gate(class, Some(3)),
                WindowClass::Untracked,
                "{:?} at a non-normal level is untracked",
                class
            );
        }
    }

    #[test]
    fn normal_and_unqueried_levels_keep_the_subrole_answer() {
        assert_eq!(
            WindowClass::apply_level_gate(WindowClass::Standard, Some(0)),
            WindowClass::Standard
        );
        assert_eq!(
            WindowClass::apply_level_gate(WindowClass::Floating, Some(0)),
            WindowClass::Floating
        );
        assert_eq!(
            WindowClass::apply_level_gate(WindowClass::Standard, None),
            WindowClass::Standard
        );
        // Negative levels are desktop/decals — still not normal, still
        // not ours.
        assert_eq!(
            WindowClass::apply_level_gate(WindowClass::Standard, Some(-20)),
            WindowClass::Untracked
        );
    }

    #[test]
    fn dropped_classes_never_become_untracked_via_level() {
        assert_eq!(
            WindowClass::apply_level_gate(WindowClass::Sheet, Some(3)),
            WindowClass::Sheet
        );
        assert_eq!(
            WindowClass::apply_level_gate(WindowClass::Unknown, Some(3)),
            WindowClass::Unknown
        );
    }
}
