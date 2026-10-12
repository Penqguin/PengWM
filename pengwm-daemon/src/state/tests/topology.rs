//! Topology healing through `StateManager`: `run_topology_sync` (the
//! executor around `DisplaySet::sync`) must make a missed display event a
//! one-sweep exhibit instead of permanent stale geometry. The scenarios
//! below change the adapter's live display list with **no** Monitor event
//! dispatched — what a dropped `try_send`, a before-pass race, or an
//! AirPlay tear-down without a clean RemoveFlag produces — and then heal.

use crate::adapter::DisplayInfo;
use pengwm_core::layout::Rect;

use super::common;

fn display(id: u32, origin: (i32, i32), size: (u32, u32)) -> DisplayInfo {
    DisplayInfo { id, origin, size }
}

#[test]
fn lost_removal_heals_in_one_sync() {
    let (mut sm, handle) = common::setup_with_handle(2);
    // Baseline: output 1 shows entry 0, output 2 shows entry 1; window 300
    // tiles on the second output.
    assert_eq!(sm.displays.active().get(&2), Some(&1));
    sm.on_window_created(300, 42);
    let owner = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(300).is_some())
        .expect("window 300 should be tenant somewhere");
    let dead = sm.workspaces[owner].monitor_id;
    if dead == 2 {
        // Park the window on the (soon gone) second display's coordinates.
        handle.inject_rect(300, Rect::new(2200.0, 100.0, 800.0, 500.0));
    }

    // The display list changes with no event: AirPlay display 2 vanishes.
    handle.set_displays(vec![display(1, (0, 0), (1920, 1080))]);

    sm.run_topology_sync();

    assert!(!sm.displays.active().contains_key(&2));
    // The orphaned workspace was remapped to primary…
    assert!(sm.workspaces.iter().all(|ws| ws.monitor_id == 1));
    // …with fresh primary geometry (no stale origin/size survives).
    assert!(sm
        .workspaces
        .iter()
        .all(|ws| ws.monitor_size() == (1920, 1080)));
    // A window that tiled the dead display is parked on the surviving one.
    if dead == 2 {
        let r = handle.rect(300).expect("window 300 keeps a rect");
        assert!(
            r.x + r.width <= 1920.0,
            "parked/stale window must not name dead-display coordinates, got {r:?}"
        );
        assert!(r.y + r.height <= 1080.0, "got {r:?}");
    }
}

#[test]
fn lost_resize_heals_geometry_and_rewrites_windows() {
    let (mut sm, handle) = common::setup_with_handle(2);
    sm.on_window_created(100, 42);
    let launched = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(100).is_some())
        .expect("window 100 tenant");

    // Display 1 shrinks (AirPlay resolution change), no event dispatched.
    handle.set_displays(vec![
        display(1, (0, 0), (1280, 800)),
        display(2, (1280, 0), (1920, 1080)),
    ]);

    sm.run_topology_sync();

    assert_eq!(sm.workspaces[launched].monitor_size(), (1280, 800));
    // The window's write targets healed geometry: inside the new bounds.
    let r = handle.rect(100).expect("window 100 keeps a rect");
    assert!(
        r.x >= 0.0 && r.y >= 0.0 && (r.x + r.width) <= 1280.0,
        "got {r:?}"
    );
}

#[test]
fn redisplay_with_unchanged_geometry_is_a_noop() {
    // The before/after double pass of a CG reconfiguration: the same
    // topology re-reported must change nothing and run no machinery.
    let mut sm = common::setup(2);
    sm.run_topology_sync();
    let after = sm.displays.active().clone();
    let ids: Vec<u32> = sm.workspaces.iter().map(|ws| ws.monitor_id).collect();
    sm.run_topology_sync();
    assert_eq!(sm.displays.active(), &after);
    let ids2: Vec<u32> = sm.workspaces.iter().map(|ws| ws.monitor_id).collect();
    assert_eq!(ids, ids2);
}
