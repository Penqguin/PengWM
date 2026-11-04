use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use core_graphics::display::{CGDirectDisplayID, CGDisplay};
use tokio::sync::mpsc;

use crate::adapter::DisplayInfo;
use crate::event_loop::DaemonEvent;
use crate::macos::ns_workspace;

/// Per-display extra inset confirmed by top-row tiled-window writes.
static LEARNED_TOP_BANDS: OnceLock<Mutex<std::collections::HashMap<u32, i32>>> = OnceLock::new();
const MIN_BAND_CORRECTION: f64 = 8.0;
const MAX_BAND_CORRECTION: f64 = 200.0;
const CONSISTENT_BAND_EPSILON: i32 = 4;
const REQUIRED_BAND_OBSERVATIONS: u8 = 2;

#[derive(Clone, Copy)]
struct PendingBandEvidence {
    candidate: i32,
    observations: u8,
}

static PENDING_BAND_EVIDENCE: OnceLock<Mutex<std::collections::HashMap<u32, PendingBandEvidence>>> =
    OnceLock::new();

#[derive(Clone, Copy)]
struct PhysicalDisplayGeometry {
    raw_origin: (i32, i32),
    raw_size: (u32, u32),
    appkit_inset: f64,
}

static LAST_PHYSICAL_GEOMETRY: OnceLock<
    Mutex<std::collections::HashMap<u32, PhysicalDisplayGeometry>>,
> = OnceLock::new();
#[derive(Clone, Copy, PartialEq)]
struct DisplayGeometryInputs {
    raw_origin: (i32, i32),
    raw_size: (u32, u32),
    appkit_inset: f64,
    learned_band: i32,
}

static LAST_DISPLAY_GEOMETRY: OnceLock<
    Mutex<std::collections::HashMap<u32, DisplayGeometryInputs>>,
> = OnceLock::new();

/// Learn an extra display inset only from a top-row tiled window and
/// repeated, WindowServer-verified downward clamps. `target` already
/// includes the current AppKit/learned inset and outer gap, so add only
/// the measured correction to the current inset.
pub fn learn_top_band_from_write(
    display_ids: &[u32],
    target: &pengwm_core::layout::Rect,
    actual: &pengwm_core::layout::Rect,
) {
    let insets = ns_workspace::screen_top_insets();
    let correction = actual.y - target.y;
    if !(MIN_BAND_CORRECTION..=MAX_BAND_CORRECTION).contains(&correction) {
        clear_band_evidence_at(display_ids, target);
        return;
    }
    let target_center = (
        target.x + target.width / 2.0,
        target.y + target.height / 2.0,
    );
    let Some(id) = display_ids.iter().copied().find(|&id| {
        let frame = CGDisplay::new(id).bounds();
        target_center.0 >= frame.origin.x
            && target_center.0 <= frame.origin.x + frame.size.width
            && target_center.1 >= frame.origin.y
            && target_center.1 <= frame.origin.y + frame.size.height
    }) else {
        return;
    };
    let frame = CGDisplay::new(id).bounds();
    let actual_center = (
        actual.x + actual.width / 2.0,
        actual.y + actual.height / 2.0,
    );
    if actual_center.0 < frame.origin.x
        || actual_center.0 > frame.origin.x + frame.size.width
        || actual_center.1 < frame.origin.y
        || actual_center.1 > frame.origin.y + frame.size.height
    {
        clear_band_evidence_at(display_ids, target);
        return;
    }

    let base_inset = insets
        .iter()
        .find(|(display_id, _)| *display_id == id)
        .map(|(_, inset)| *inset)
        .unwrap_or(0.0);
    let learned = learned_top_bands().get(&id).copied().unwrap_or(0) as f64;
    let candidate = base_inset.max(learned) + correction;
    if !(0.0..=MAX_BAND_CORRECTION).contains(&candidate) {
        clear_band_evidence_at(display_ids, target);
        return;
    }

    let candidate = candidate.round() as i32;
    let evidence = PENDING_BAND_EVIDENCE.get_or_init(|| Mutex::new(Default::default()));
    let observations = if let Ok(mut evidence) = evidence.lock() {
        let entry = evidence.entry(id).or_insert(PendingBandEvidence {
            candidate,
            observations: 0,
        });
        if (entry.candidate - candidate).abs() <= CONSISTENT_BAND_EPSILON {
            entry.observations = entry.observations.saturating_add(1);
            entry.candidate = (entry.candidate + candidate) / 2;
        } else {
            *entry = PendingBandEvidence {
                candidate,
                observations: 1,
            };
        }
        entry.observations
    } else {
        return;
    };
    if observations < REQUIRED_BAND_OBSERVATIONS {
        log::debug!(
            "top-band candidate for display {}: {}px total after observation {}/{}",
            id,
            candidate,
            observations,
            REQUIRED_BAND_OBSERVATIONS
        );
        return;
    }

    let band = if let Ok(mut evidence) = evidence.lock() {
        evidence
            .remove(&id)
            .map(|entry| entry.candidate)
            .unwrap_or(candidate)
    } else {
        candidate
    };
    let map = LEARNED_TOP_BANDS.get_or_init(|| Mutex::new(Default::default()));
    if let Ok(mut map) = map.lock() {
        map.insert(id, band);
    }
    log::info!(
        "learned top band for display {}: {}px from {} consistent top-row tile readbacks (target y {}, actual y {}, correction {:.1}px)",
        id,
        band,
        REQUIRED_BAND_OBSERVATIONS,
        target.y,
        actual.y,
        correction
    );
}

/// An attempt without a fresh measured clamp breaks the evidence sequence,
/// so pending samples cannot combine across unrelated write outcomes.
pub fn clear_top_band_evidence_for_target(display_ids: &[u32], target: &pengwm_core::layout::Rect) {
    clear_band_evidence_at(display_ids, target);
}

fn clear_band_evidence_at(display_ids: &[u32], rect: &pengwm_core::layout::Rect) {
    let cx = rect.x + rect.width / 2.0;
    let cy = rect.y + rect.height / 2.0;
    let display = display_ids.iter().find(|&&id| {
        let frame = CGDisplay::new(id).bounds();
        cx >= frame.origin.x
            && cx <= frame.origin.x + frame.size.width
            && cy >= frame.origin.y
            && cy <= frame.origin.y + frame.size.height
    });
    if let (Some(&id), Some(evidence)) = (display, PENDING_BAND_EVIDENCE.get()) {
        if let Ok(mut evidence) = evidence.lock() {
            evidence.remove(&id);
        }
    }
}

/// The display set of `.active_displays()` is not re-enumerated inside
/// the lock; the executor hands it in.
///
/// Prune learned values and incomplete evidence for displays no longer live.
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
    if let Some(evidence) = PENDING_BAND_EVIDENCE.get() {
        if let Ok(mut evidence) = evidence.lock() {
            evidence.retain(|id, _| keep.contains(id));
        }
    }
}

fn refresh_physical_geometry(ids: &[u32], insets: &[(u32, f64)]) {
    let current: std::collections::HashMap<u32, PhysicalDisplayGeometry> = ids
        .iter()
        .map(|&id| {
            let frame = CGDisplay::new(id).bounds();
            let inset = insets
                .iter()
                .find(|(display_id, _)| *display_id == id)
                .map(|(_, inset)| *inset)
                .unwrap_or(0.0);
            (
                id,
                PhysicalDisplayGeometry {
                    raw_origin: (frame.origin.x as i32, frame.origin.y as i32),
                    raw_size: (frame.size.width as u32, frame.size.height as u32),
                    appkit_inset: inset,
                },
            )
        })
        .collect();
    let state = LAST_PHYSICAL_GEOMETRY.get_or_init(|| Mutex::new(Default::default()));
    let mut reset = Vec::new();
    if let Ok(mut state) = state.lock() {
        for (&id, geometry) in &current {
            let changed = state.get(&id).is_some_and(|previous| {
                previous.raw_origin != geometry.raw_origin
                    || previous.raw_size != geometry.raw_size
                    || (previous.appkit_inset - geometry.appkit_inset).abs() > 0.5
            });
            if changed {
                reset.push(id);
            }
        }
        state.retain(|id, _| current.contains_key(id));
        state.extend(current);
    }
    if !reset.is_empty() {
        if let Some(bands) = LEARNED_TOP_BANDS.get() {
            if let Ok(mut bands) = bands.lock() {
                for id in &reset {
                    bands.remove(id);
                }
            }
        }
        if let Some(evidence) = PENDING_BAND_EVIDENCE.get() {
            if let Ok(mut evidence) = evidence.lock() {
                for id in &reset {
                    evidence.remove(id);
                }
            }
        }
        log::info!(
            "cleared learned top-band evidence after physical geometry changed on displays {:?}",
            reset
        );
    }
    let keep: Vec<u32> = ids.to_vec();
    if let Some(bands) = LEARNED_TOP_BANDS.get() {
        if let Ok(mut bands) = bands.lock() {
            bands.retain(|id, _| keep.contains(id));
        }
    }
    if let Some(evidence) = PENDING_BAND_EVIDENCE.get() {
        if let Ok(mut evidence) = evidence.lock() {
            evidence.retain(|id, _| keep.contains(id));
        }
    }
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
    CGDisplay::active_displays().unwrap_or_default()
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
    if let Some(evidence) = PENDING_BAND_EVIDENCE.get() {
        if let Ok(mut evidence) = evidence.lock() {
            evidence.clear();
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
    refresh_physical_geometry(&ids, &base);
    let learned = learned_top_bands();

    ids.iter()
        .map(|&id| {
            let display = CGDisplay::new(id);
            let bounds = display.bounds();
            let raw_origin = (bounds.origin.x as i32, bounds.origin.y as i32);
            let raw_size = (bounds.size.width as u32, bounds.size.height as u32);
            let mut origin = raw_origin;
            let mut size = raw_size;

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

            let snapshot = DisplayGeometryInputs {
                raw_origin,
                raw_size,
                appkit_inset: appkit,
                learned_band: band as i32,
            };
            let last = LAST_DISPLAY_GEOMETRY.get_or_init(|| Mutex::new(Default::default()));
            if let Ok(mut last) = last.lock() {
                if let Some(previous) = last.insert(id, snapshot) {
                    if previous != snapshot {
                        log::info!(
                            "display {} geometry inputs changed: raw={:?}+{:?}, visibleFrame top inset={:.1}px, learned top band={}px; tile area={:?}+{:?}",
                            id, raw_origin, raw_size, appkit, band, origin, size
                        );
                    }
                }
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

/// Force AppKit's cached screen list to re-derive from the window server.
/// The daemon runs its own CFRunLoop, not `NSApp`'s event loop, so the
/// system never routes `NSApplicationDidChangeScreenParametersNotification`
/// through this process the way it does for normal apps — AppKit's
/// in-process screen cache froze at boot. Consequences observed live:
/// insets read 0 for a re-plugged display, and pill placement refused
/// to honor a frame on it (default placement on the primary instead).
/// The notification posts synchronously through NSNotificationCenter,
/// so its handlers run — AppKit's screen cache included — before this
/// returns. Same pattern `main.rs` already uses for the
/// didFinishLaunching poke.
pub fn refresh_appkit_screen_list() {
    use objc2_foundation::{NSNotificationCenter, NSString};
    let center = NSNotificationCenter::defaultCenter();
    unsafe {
        center.postNotificationName_object(
            &NSString::from_str("NSApplicationDidChangeScreenParametersNotification"),
            None,
        );
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

    let mut reconfigured = false;
    if flags & kCGDisplayAddFlag != 0 {
        log::info!("Display added: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorAdded(display));
        reconfigured = true;
    }
    if flags & kCGDisplayRemoveFlag != 0 {
        log::info!("Display removed: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorRemoved(display));
        reconfigured = true;
    }
    if flags & kCGDisplaySetModeFlag != 0 {
        log::info!("Display resized: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorResized(display));
        reconfigured = true;
    }
    if reconfigured {
        // The topology reconfiguration is exactly when AppKit's cached
        // screen list must refresh (see the fn doc); do it on this
        // thread, which is the main one the callback fires on.
        refresh_appkit_screen_list();
        log::debug!("AppKit screen list refreshed after display reconfiguration");
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

        // --- requires repeated evidence from a real renormalization --
        // The learned value includes the existing AppKit inset plus the
        // extra displacement. One observation remains pending.
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
        assert!(!learned_top_bands().contains_key(&id));
        learn_top_band_from_write(&[id], &target, &renormalized);
        let base_inset = ns_workspace::screen_top_insets()
            .into_iter()
            .find(|(display_id, _)| *display_id == id)
            .map(|(_, inset)| inset)
            .unwrap_or(0.0);
        let learned = (base_inset + 30.0).round() as i32;
        assert_eq!(learned_top_bands().get(&id), Some(&learned));
        // Out-of-range corrections clear pending evidence and teach nothing.
        let park_clamp = pengwm_core::layout::Rect {
            x: frame.origin.x + 10.0,
            y: frame.origin.y + 500.0,
            width: 800.0,
            height: 600.0,
        };
        learn_top_band_from_write(&[id], &target, &park_clamp);
        assert_eq!(
            learned_top_bands().get(&id),
            Some(&learned),
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
            Some(&learned),
            "a width pin that left y alone must not teach anything"
        );

        // --- topology prune --------------------------------------
        // A display that left the live CG set loses its band, so a
        // re-plug re-teaches from live renorms instead of inheriting
        // its previous session's value.
        prune_learned_top_bands(&[]);
        assert!(learned_top_bands().is_empty());
        learn_top_band_from_write(&[id], &target, &renormalized);
        assert!(learned_top_bands().is_empty());
        learn_top_band_from_write(&[id], &target, &renormalized);
        assert_eq!(learned_top_bands().get(&id), Some(&learned));
        prune_learned_top_bands(&[id]);
        assert_eq!(learned_top_bands().get(&id), Some(&learned));
        clear_learned_top_bands_for_tests();
        assert!(learned_top_bands().is_empty(), "cleared for neighbors");
    }
}
