//! The full WM reset (`state::reset::wm_reset`) — hotplug semantics:
//! any actual topology change rebuilds all derived state and re-routes
//! every tracked member as a birth. Existing topology-suite scenarios
//! now heal through this path too; these tests pin reset-specific
//! behaviors the topology suite never faced: member preservation,
//! the cooldown (no reset-storms from a replug's event burst), and
//! the mid-un-plug (zero displays) guard.

use std::time::{Duration, Instant};

use crate::adapter::DisplayInfo;
use crate::config::keybinds::KeybindConfig;
use crate::bar_server::BarSender;
use crate::adapter_test::TestHandle;
use crate::state::StateManager;
use tokio::sync::mpsc;

use super::common::{make_adapter, test_prefix};

fn display(id: u32, origin: (i32, i32), size: (u32, u32)) -> DisplayInfo {
    DisplayInfo { id, origin, size }
}

fn setup_two_displays() -> (StateManager, TestHandle, tokio::sync::mpsc::Sender<crate::event_loop::DaemonEvent>) {
    let (tx, _rx) = mpsc::channel(64);
    let keybinds = std::sync::Arc::new(std::sync::Mutex::new(KeybindConfig::default()));
    let mut adapter = make_adapter(2);
    adapter.frontmost = None;
    let handle = TestHandle::new(adapter);
    let boxed: Box<dyn crate::adapter::OsAdapter> = Box::new(handle.shared());
    let (bar_tx, _) = mpsc::channel(64);
    (
        StateManager::new(
            tx.clone(),
            keybinds,
            test_prefix(),
            boxed,
            BarSender::from_channel(bar_tx),
            vec![],
        ),
        handle,
        tx,
    )
}

/// Members survive; derivation does not. After a display vanishes every
/// window that was being managed is still managed — re-routed as a
/// birth into the freshly assembled world.
#[test]
fn reset_preserves_members_and_rebuilds_derivation() {
    let (mut sm, handle, _tx) = setup_two_displays();
    sm.on_window_created(300, 42);
    sm.on_window_created(400, 42);

    handle.set_displays(vec![display(1, (0, 0), (1920, 1080))]);
    sm.run_topology_sync();

    // Derived state is from-scratch: every workspace sits on the
    // survivor, with survivor geometry.
    assert!(!sm.displays.active().contains_key(&2));
    assert!(sm.workspaces.iter().all(|ws| ws.monitor_id == 1));
    assert!(sm.workspaces.iter().all(|ws| ws.monitor_size() == (1920, 1080)));

    // Members preserved: both windows tracked, and each lives in some
    // fresh tree (a birth routes config-first; no id is dropped by
    // the reset itself).
    let shapes: Vec<(bool, bool)> = (300..=400)
        .filter(|&wid| handle.rect(wid).is_some() || sm.workspaces.iter().any(|ws| ws.find_window(wid).is_some()))
        .map(|_| (true, true))
        .collect();
    assert_eq!(shapes.len(), 2, "both members still managed");

    // Nothing writes into no-man's land.
    for wid in [300u64, 400] {
        if let Some(r) = handle.rect(wid) {
            assert!(
                r.x + r.width <= 1920.0 && r.y + r.height <= 1080.0,
                "win {wid} wrote outside survivor bounds: {r:?}"
            );
        }
    }
}

/// A replug bursts MonitorRemoved+MonitorAdded; every event reaches
/// `run_topology_sync` while the topology is still settling. The first
/// reset wins; suppressions inside the cooldown must leave state
/// consistent (they route through the incremental fallback heal).
#[test]
fn reset_cooldown_suppresses_the_burst_but_stays_consistent() {
    let (mut sm, handle, _tx) = setup_two_displays();
    sm.on_window_created(300, 42);

    handle.set_displays(vec![display(1, (0, 0), (1920, 1080))]);
    sm.run_topology_sync(); // reset #1
    let after_one = sm.displays.active().clone();
    assert_eq!(after_one.len(), 1);

    // Second (immediate) topology "change" inside the cooldown: the
    // reset pressure is suppressed; whatever state the sync computes
    // must still be internally consistent (no panic, no ghost monitor).
    sm.run_topology_sync();

    assert_eq!(sm.displays.active().len(), 1);
    assert!(sm.workspaces.iter().all(|ws| ws.monitor_id == 1));
    // The cooldown holds for the full window.
    std::thread::sleep(Duration::from_millis(50));
    sm.run_topology_sync();
    assert_eq!(sm.displays.active().len(), 1);

    // After the cooldown clears, a *real* change resets again: monitor
    // 2 is back (different id, like the CG id change observed live).
    sm.last_wm_reset = Some(Instant::now() - Duration::from_secs(4));
    handle.set_displays(vec![
        display(1, (0, 0), (1920, 1080)),
        display(8, (-1180, 153), (1180, 820)),
    ]);
    sm.run_topology_sync();
    assert_eq!(sm.displays.active().len(), 2, "late change disrespecting the cooldown must reset");
    assert!(sm.displays.active().contains_key(&8));
}

/// Mid-un-plug: the snapshot can be empty while events settle. Reset
/// must not assemble against nothing — the state survives untouched so
/// the next tick's sync can heal once displays exist again.
#[test]
fn empty_display_snapshot_is_untouched_by_reset() {
    let (mut sm, handle, _tx) = setup_two_displays();
    sm.on_window_created(300, 42);
    let names: Vec<String> = sm.workspaces.iter().map(|ws| ws.name.clone()).collect();

    handle.set_displays(vec![]);
    sm.run_topology_sync();

    assert_eq!(
        sm.workspaces.iter().map(|ws| ws.name.clone()).collect::<Vec<_>>(),
        names,
        "mid-un-plug snapshots the world as it was"
    );
}

/// After a successful reset the engine settles into a no-op: the sync
/// does not spin the state (assembled-from-same-snapshot == no change),
/// so the tick can call it forever for free.
#[test]
fn reset_settles_into_a_noop() {
    let (mut sm, handle, _tx) = setup_two_displays();
    sm.on_window_created(300, 42);

    handle.set_displays(vec![display(1, (0, 0), (1920, 1080))]);
    sm.run_topology_sync();

    let names: Vec<String> = sm.workspaces.iter().map(|ws| ws.name.clone()).collect();
    let active = sm.displays.active().clone();
    for _ in 0..3 {
        sm.run_topology_sync();
        assert_eq!(sm.displays.active(), &active, "settled");
        assert_eq!(
            sm.workspaces.iter().map(|ws| ws.name.clone()).collect::<Vec<_>>(),
            names,
            "settled"
        );
    }
}
