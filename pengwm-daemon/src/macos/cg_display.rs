use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use core_graphics::display::{CGDirectDisplayID, CGDisplay};
use tokio::sync::mpsc;

use crate::adapter::DisplayInfo;
use crate::event_loop::DaemonEvent;
use crate::macos::ns_workspace;

/// Displays whose placement carries a top band WindowServer enforces over
/// and above what AppKit's `visibleFrame` reports. AppKit cannot tell the
/// truth about auto-hidden menu bars: an AirPlay Sidecar panel reports a
/// full visible frame while *every* AX geometry write that requests a top
/// edge inside the band gets renormalized below it. The band is learned
/// empirically from renormalizations (`learn_top_band_from_window_rect` —
/// the OS is the only authority that knows) and merged on top of the
/// AppKit inset, so tile targets are born convergent instead of pinned
/// and renormalized forever.
static LEARNED_TOP_BANDS: OnceLock<Mutex<std::collections::HashMap<u32, i32>>> =
    OnceLock::new();

/// Record the enforced top band for the display containing `actual`'s
/// center. Called on Pinned/Drift write outcomes, where the readback is
/// window-reported ground truth. Bands are clamped to a sane range and
/// only ever move upward.
///
/// A band is *learned* only when the top edge actually renormalized —
/// `actual.y` differs from `target.y`. A write that pinned on width or
/// size while its top edge landed exactly where requested carries no
/// band information: reading `actual.y - frame.top` there would
/// misattribute whatever offset led to the target (the inset the
/// target was already built with, a stray half-pixel snap) into the
/// band, at exactly the "window added → everything shifted" moment.
/// Real bands re-teach within one write anyway, so skipping these
/// costs nothing; misattribution is sticky (the map only ratchets
/// upward), which is the expensive direction to get wrong.
pub fn learn_top_band_from_write(
    display_ids: &[u32],
    target: &pengwm_core::layout::Rect,
    actual: &pengwm_core::layout::Rect,
) {
    if actual.y == target.y {
        return; // top edge never moved — not a band story.
    }
    let cx = actual.x + actual.width / 2.0;
    let cy = actual.y + actual.height / 2.0;
    for &id in display_ids {
        let frame = CGDisplay::new(id).bounds();
        let contains_center = cx >= frame.origin.x
            && cx <= frame.origin.x + frame.size.width
            && cy >= frame.origin.y
            && cy <= frame.origin.y + frame.size.height;
        if !contains_center {
            continue;
        }
        let band = (actual.y - frame.origin.y).round() as i32;
        if !(0..=200).contains(&band) {
            // Not a top-band story: hides clamp whole windows (hundreds
            // of px), cross-display drops report the old frame — out of
            // scope. The band is for *renormalized top edges*.
            return;
        }
        if band <= 0 {
            return;
        }
        let map = LEARNED_TOP_BANDS.get_or_init(|| Mutex::new(Default::default()));
        if let Ok(mut map) = map.lock() {
            let entry = map.entry(id).or_insert(0);
            *entry = (*entry).max(band);
        }
        log::debug!(
            "learned top band for display {}: {}px (target y {}, actual y {}, frame top {})",
            id,
            band,
            target.y,
            actual.y,
            frame.origin.y
        );
        return;
    }
}

/// The display set of `.active_displays()` is not re-enumerated inside
/// the lock; the executor hands it in.
///
/// Prune learned bands for displays no longer in `keep`: the map only
/// ratchets upward, so a stale entry would outlive its display forever
/// — a reconnected monitor must re-teach from live renormalizations,
/// not inherit its previous session's band (the plug-out/plug-in of the
/// changed-value band showed the cost: strip sizes that vary per session
/// would never correct downward).
pub fn prune_learned_top_bands(keep: &[u32]) {
    let map = LEARNED_TOP_BANDS.get_or_init(|| Mutex::new(Default::default()));
    if let Ok(mut map) = map.lock() {
        let before = map.len();
        map.retain(|id, _| keep.contains(id));
        if map.len() != before {
            log::debug!(
                "pruned {} stale learned top band(s); keep {:?}",
                before - map.len(),
                keep
            );
        }
    }
}

/// Record the enforced top band for the display containing `actual`'s
/// center. Legacy wrapper retained for the existing readback-centered
/// probe callers.
#[allow(dead_code)]
pub fn learn_top_band_from_window_rect(actual: &pengwm_core::layout::Rect) {
    let ids = match CGDisplay::active_displays() {
        Ok(ids) => ids,
        Err(_) => return,
    };
    learn_top_band_from_write(&ids, actual, actual);
}

pub fn learned_top_bands() -> std::collections::HashMap<u32, i32> {
    LEARNED_TOP_BANDS
        .get_or_init(|| Mutex::new(Default::default()))
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default()
}

/// Raw CG display ids, for callers that probe bounds themselves (the
/// band-learning path — inset-clamped `DisplayInfo` bounds would defeat
/// the `actual.y - frame.top` read).
pub fn raw_active_display_ids() -> Vec<u32> {
    CGDisplay::active_displays()
        .map(|ids| ids.into_iter().map(|id| id as u32).collect())
        .unwrap_or_default()
}

/// The main display's raw CG height — the y-up/y-down pivot in the
/// AX→AppKit coordinate conversion. Raw, not inset-clamped: the pivot
/// must be the primary's actual bounds, whatever the WM tiles inside
/// them. Verified live against `NSScreen.frame` (1180×820 display at CG
/// y=153 next to a 1169-tall primary: frame origin.y = 1169 − 153 − 820
/// = 196, exactly AppKit's answer).
pub fn raw_main_height() -> f64 {
    let main = unsafe { core_graphics::display::CGMainDisplayID() };
    CGDisplay::new(main).bounds().size.height
}

#[doc(hidden)]
pub fn clear_learned_top_bands_for_tests() {
    if let Some(m) = LEARNED_TOP_BANDS.get() {
        if let Ok(mut m) = m.lock() {
            m.clear();
        }
    }
}

pub fn active_displays() -> Vec<DisplayInfo> {
    let ids = match CGDisplay::active_displays() {
        Ok(ids) => ids,
        Err(_) => return Vec::new(),
    };

    // Per-display menu-bar insets: every screen can carry its own menu bar,
    // and ignoring a secondary's bar makes tile targets renormalize (the
    // pin/jitter loop on Sidecar displays). Insets read each screen's own
    // `visibleFrame`; the main display is just the one CG calls primary.
    let base = ns_workspace::screen_top_insets();
    let learned = learned_top_bands();

    ids.iter()
        .map(|&id| {
            let display = CGDisplay::new(id);
            let bounds = display.bounds();
            let mut origin = (bounds.origin.x as i32, bounds.origin.y as i32);
            let mut size = (bounds.size.width as u32, bounds.size.height as u32);

            // Account for the macOS menu bar on *any* display that has one —
            // the inset is read from that screen's own `visibleFrame` — and
            // for anything the OS enforces *beyond* what AppKit tells us
            // (auto-hidden bars, learned from renormalizations).
            let appkit = base
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| *v)
                .unwrap_or(0.0);
            let band = learned.get(&id).copied().unwrap_or(0) as f64;
            let inset = appkit.max(band);
            if inset > 0.0 {
                origin.1 += inset as i32;
                size.1 = size.1.saturating_sub(inset as u32);
            }

            DisplayInfo { id, origin, size }
        })
        .collect()
}

pub fn primary_display_id() -> u32 {
    unsafe { core_graphics::display::CGMainDisplayID() }
}

type CGDisplayChangeFlags = u32;

type CGDisplayReconfigurationCallBack = unsafe extern "C" fn(
    display: CGDirectDisplayID,
    flags: CGDisplayChangeFlags,
    refcon: *mut c_void,
);

#[allow(non_upper_case_globals)]
const kCGDisplayAddFlag: CGDisplayChangeFlags = 1 << 0;
#[allow(non_upper_case_globals)]
const kCGDisplayRemoveFlag: CGDisplayChangeFlags = 1 << 1;
#[allow(non_upper_case_globals)]
const kCGDisplaySetModeFlag: CGDisplayChangeFlags = 1 << 3;

pub fn register_hotplug_callback(event_tx: mpsc::Sender<DaemonEvent>) {
    // Program-lifetime ownership, made explicit: the refcon must outlive
    // every display event, i.e. the daemon's whole run. The Box is handed
    // to the callback and intentionally never reclaimed — there is no
    // second registration and no teardown path that outlives the process.
    // (A previous `unregister_hotplug_callback` teardown existed with zero
    // callers; removed rather than keeping dead teardown code.)
    let ctx = Box::into_raw(Box::new(event_tx)) as *mut c_void;

    unsafe {
        CGDisplayRegisterReconfigurationCallback(Some(hotplug_callback), ctx);
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGDisplayRegisterReconfigurationCallback(
        callback: Option<CGDisplayReconfigurationCallBack>,
        refcon: *mut c_void,
    );
}

unsafe extern "C" fn hotplug_callback(
    display: CGDirectDisplayID,
    flags: CGDisplayChangeFlags,
    refcon: *mut c_void,
) {
    let tx = &*(refcon as *const mpsc::Sender<DaemonEvent>);

    if flags & kCGDisplayAddFlag != 0 {
        log::info!("Display added: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorAdded(display));
    }
    if flags & kCGDisplayRemoveFlag != 0 {
        log::info!("Display removed: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorRemoved(display));
    }
    if flags & kCGDisplaySetModeFlag != 0 {
        log::info!("Display resized: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorResized(display));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_info_struct_size() {
        assert_eq!(std::mem::size_of::<DisplayInfo>(), 20); // u32 + (i32,i32) + (u32,u32)
    }

    #[test]
    fn display_flag_constants() {
        assert_eq!(kCGDisplayAddFlag, 1);
        assert_eq!(kCGDisplayRemoveFlag, 2);
        assert_eq!(kCGDisplaySetModeFlag, 8);
    }

    #[test]
    fn primary_display_id_function_exists() {
        fn _type_check(_f: fn() -> u32) {}
        _type_check(primary_display_id);
    }

    #[test]
    fn active_displays_function_exists() {
        fn _type_check(_f: fn() -> Vec<DisplayInfo>) {}
        _type_check(active_displays);
    }

    #[test]
    fn learned_top_band_lifecycle() {
        // One serial test, deliberately: all three stages share the
        // module-global band map, and parallel test threads clearing it
        // under each other would flake.
        clear_learned_top_bands_for_tests();
        let id = primary_display_id();
        let frame = CGDisplay::new(id).bounds();

        // --- learns from a real renormalization -------------------
        // A window the OS renormalized to frame.top + 30: its readback
        // carries the enforced band for its display. Target and actual
        // differ on the top edge — a real y renormalization.
        let target = pengwm_core::layout::Rect {
            x: frame.origin.x + 10.0,
            y: frame.origin.y,
            width: 800.0,
            height: 600.0,
        };
        let renormalized = pengwm_core::layout::Rect {
            x: frame.origin.x + 10.0,
            y: frame.origin.y + 30.0,
            width: 800.0,
            height: 600.0,
        };
        learn_top_band_from_write(&[id], &target, &renormalized);
        assert_eq!(learned_top_bands().get(&id), Some(&30));
        // Only ever upward, and sane-range rejections learn nothing.
        let park_clamp = pengwm_core::layout::Rect {
            x: frame.origin.x + 10.0,
            y: frame.origin.y + 500.0,
            width: 800.0,
            height: 600.0,
        };
        learn_top_band_from_write(&[id], &target, &park_clamp);
        assert_eq!(
            learned_top_bands().get(&id),
            Some(&30),
            "hundreds-of-px rejections are hide clamps, not top bands — band unchanged"
        );

        // --- no band from a write whose top edge never moved -------
        // The observed misattribution: a write pins on *width*
        // (WindowServer keeps the window's existing width) while its
        // top edge lands exactly where the target put it — sampled
        // delta was target y == actual y == 238, frame top == 153. The
        // old code read actual.y - frame.top as a fresh 85px band even
        // though nothing renormalized, and bands only ratchet upward —
        // the sticky "vertically shrink" the report showed.
        let pinned_target = pengwm_core::layout::Rect {
            x: frame.origin.x + 10.0,
            y: frame.origin.y + 85.0,
            width: 582.5,
            height: 730.0,
        };
        // The pinned readback: x snapped half a pixel, width unchanged
        // at 599, y exactly the target.
        let pinned = pengwm_core::layout::Rect {
            x: frame.origin.x + 9.5,
            y: frame.origin.y + 85.0,
            width: 599.0,
            height: 730.0,
        };
        learn_top_band_from_write(&[id], &pinned_target, &pinned);
        assert_eq!(
            learned_top_bands().get(&id),
            Some(&30),
            "a width pin that left y alone must not teach anything"
        );

        // --- topology prune --------------------------------------
        // A display that left the live CG set loses its band, so a
        // re-plug re-teaches from live renorms instead of inheriting
        // its previous session's value.
        prune_learned_top_bands(&[]);
        assert!(learned_top_bands().is_empty());
        learn_top_band_from_write(&[id], &target, &renormalized);
        assert_eq!(learned_top_bands().get(&id), Some(&30));
        prune_learned_top_bands(&[id]);
        assert_eq!(learned_top_bands().get(&id), Some(&30));
        clear_learned_top_bands_for_tests();
        assert!(learned_top_bands().is_empty(), "cleared for neighbors");
    }
}
