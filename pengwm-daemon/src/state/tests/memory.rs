use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::super::StateManager;
use super::common::{make_adapter, setup_with_handle, test_prefix};
use crate::adapter_test::{TestAdapter, TestHandle};
use crate::bar_server::BarSender;
use crate::config::keybinds::KeybindConfig;

/// Harness like `common::setup_with_handle`, with pids excluded from
/// management (the menubar role).
fn setup_with_excluded(display_count: u32, excluded: Vec<i32>) -> (StateManager, TestHandle) {
    let (tx, _) = mpsc::channel(64);
    let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
    let prefix = test_prefix();
    let adapter: TestAdapter = make_adapter(display_count);
    let handle = TestHandle::new(adapter);
    let boxed: Box<dyn crate::adapter::OsAdapter> = Box::new(handle.shared());
    let (bar_tx, _) = mpsc::channel(64);
    (
        StateManager::new(
            tx,
            keybinds,
            prefix,
            boxed,
            BarSender::from_channel(bar_tx),
            excluded,
        ),
        handle,
    )
}

#[test]
fn excluded_app_windows_created_after_bootstrap_get_pruned() {
    // The observer callback caches a retained element for every manageable
    // window, but an excluded app's windows route out before
    // `store.register` — nothing tracked them, so no per-window eviction
    // path ever applied. The tick's cache-vs-store diff is that path: the
    // menubar's windows must not retain elements forever.
    let (mut sm, handle) = setup_with_excluded(1, vec![999]);
    handle.inject_window(999, 500);
    sm.on_window_created(500, 999);
    assert!(
        !sm.store.contains(500),
        "excluded routing must not track the window"
    );
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert_eq!(
        handle.pruned(),
        vec![500],
        "the cached-but-untracked element must be released on the sweep"
    );
    // And the prune is stable: a second sweep finds nothing more to evict,
    // so the excluded app's steady state is balanced, not churn.
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        handle.pruned() == vec![500],
        "steady state must not re-prune anything"
    );
}

#[test]
fn tracked_windows_are_never_pruned() {
    // The prune's only target is cache entries the store doesn't track;
    // tree members, popups, and parked (hidden) windows all stay tracked,
    // so their elements must survive every sweep. Parking is the sharp
    // case: the window left the tree but kept its store entry.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.on_window_hidden(100);
    assert!(sm.store.is_hidden(100));
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        !handle.pruned().contains(&100) && !handle.pruned().contains(&200),
        "parked and tiled windows keep their tracked elements"
    );
    assert!(handle.pruned().is_empty());
}

#[test]
fn dead_pid_sweep_runs_the_termination_funnel() {
    // The app quit and its `NSWorkspaceDidTerminate` notification was
    // missed. NSWorkspace liveness is non-AX evidence — a dead app's own
    // poll is *always* empty, which the listing sweep treats as "no
    // judgment" — so the dead-pid diff is the only cleaner. It must retire
    // the app exactly as a real termination event would.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.simulate_missed_termination(42);
    handle.clear_forgotten_for_test();
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(
        !sm.store.contains(100) && !sm.store.contains(200),
        "a non-running pid's tracked windows are untracked"
    );
    assert!(
        !handle.forgotten().is_empty(),
        "the funnel releases the dead app's cached elements"
    );
    assert!(
        handle.forgotten_pids().iter().any(|&(pid, _)| pid == 42),
        "forget_pid ran for the dead pid"
    );
    // Idempotent: the next reconcile finds no dead pid.
    handle.clear_forgotten_for_test();
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(
        handle.forgotten().is_empty() && handle.forgotten_pids().is_empty(),
        "cleanup is one-shot: the second reconcile finds nothing dead"
    );
}

#[test]
fn dead_pid_sweep_detaches_the_stranded_observer() {
    // An observer-only pid (attached at bootstrap, no windows) whose app
    // died without notifying: the observer — AXObserverRef + app element +
    // retained runloop source — is stranded until this sweep detaches it.
    let (mut sm, handle) = setup_with_handle(1);
    assert!(
        !handle.observers().is_empty(),
        "bootstrap attached an observer"
    );
    handle.simulate_missed_termination(42);
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(
        !handle.observers().contains(&42),
        "a dead pid's observer is detached by the sweep"
    );
}

#[test]
fn dead_pid_sweep_is_suppressed_while_wake_resync_pending() {
    // Same "no judgment during the blackout" rule as the listing sweep:
    // the resync commit re-attaches and re-polls everything, and cleanup
    // waits for the next reconcile.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.simulate_missed_termination(42);
    sm.on_system_woke();
    assert!(sm.wake_resync_pending(), "precondition: resync armed");
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(
        sm.store.contains(100),
        "the dead-pid sweep must not run mid-resync"
    );
}

#[test]
fn tick_replaces_observers_older_than_the_age_bound() {
    // AX accumulates per-registration records for an observer's lifetime
    // with no removal call. Age-bounded replacement is what keeps a
    // long-window-churn app's observer bounded when no wake event ever
    // forces a resync. The tick drives it through the registry seam.
    let (mut sm, handle) = setup_with_handle(1);
    handle.age_observers_for_test(15 * 60);
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert_eq!(
        handle.refreshes(),
        1,
        "the aged observer is replaced once per reconcile"
    );
    // Replacement resets the age: immediately after, none are aged again.
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert_eq!(
        handle.refreshes(),
        1,
        "a freshly replaced observer is not re-aged"
    );
}
