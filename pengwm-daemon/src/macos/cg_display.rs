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
pub fn learn_top_band_from_window_rect(actual: &pengwm_core::layout::Rect) {
    let ids = match CGDisplay::active_displays() {
        Ok(ids) => ids,
        Err(_) => return,
    };
    let cx = actual.x + actual.width / 2.0;
    let cy = actual.y + actual.height / 2.0;
    for &id in &ids {
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
            "learned top band for display {}: {}px (actual y {}, frame top {})",
            id,
            band,
            actual.y,
            frame.origin.y
        );
        return;
    }
}

pub fn learned_top_bands() -> std::collections::HashMap<u32, i32> {
    LEARNED_TOP_BANDS
        .get_or_init(|| Mutex::new(Default::default()))
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default()
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
    fn learns_top_band_from_renormalized_readback() {
        clear_learned_top_bands_for_tests();
        let id = primary_display_id();
        let frame = CGDisplay::new(id).bounds();
        // A window the OS renormalized to frame.top + 30: its readback
        // carries the enforced band for its display.
        let renormalized = pengwm_core::layout::Rect {
            x: frame.origin.x + 10.0,
            y: frame.origin.y + 30.0,
            width: 800.0,
            height: 600.0,
        };
        learn_top_band_from_window_rect(&renormalized);
        assert_eq!(learned_top_bands().get(&id), Some(&30));
        // Only ever upward, and sane-range rejections learn nothing.
        let park_clamp = pengwm_core::layout::Rect {
            x: frame.origin.x + 10.0,
            y: frame.origin.y + 500.0,
            width: 800.0,
            height: 600.0,
        };
        learn_top_band_from_window_rect(&park_clamp);
        assert_eq!(
            learned_top_bands().get(&id),
            Some(&30),
            "hundreds-of-px rejections are hide clamps, not top bands — band unchanged"
        );
        clear_learned_top_bands_for_tests();
        assert!(learned_top_bands().is_empty(), "cleared for neighbors");
    }
}
