use std::time::Duration;

use super::common::*;

#[test]
fn system_woke_resyncs_and_retiles() {
    // Sleep invalidates AX refs + display geometry while `applied_rects`
    // still claims "already at target". Wake must drop the cache, discover
    // untracked windows via the normal created path, and rewrite.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let writes_after_tile = sm.os.set_rect_calls_for_test();
    // Identical layout skips before wake.
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(sm.os.set_rect_calls_for_test(), writes_after_tile);
    // Window that appeared mid-sleep: OS knows it, store does not.
    sm.os.inject_window(42, 300);
    // Window moved mid-sleep: OS truth disagrees with the cleared cache, so
    // the read-before-write check can't skip it and a rewrite is forced.
    // (Without this, placed windows correctly skip and no write happens.)
    sm.os.displace_window_for_test(200, 200.0, 200.0);
    sm.on_system_woke();
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(300).is_some()),
        "wake must tile windows missed during sleep"
    );
    assert!(
        sm.os.set_rect_calls_for_test() > writes_after_tile,
        "wake must force rewrite despite unchanged layout"
    );
}

#[test]
fn transient_resize_failure_stays_tracked_and_retries() {
    // Live resize: the OS rejects size writes with kAXErrorFailure while
    // the user holds the resize handle, then accepts them once it settles.
    // The window must stay tracked (not untracked like a gone window), the
    // repeat failure must be throttled, and the next layout after the
    // contention clears must succeed.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.os.fail_transient_for_test(100);
    // Displace the OS-side rect so the read-before-write check can't skip:
    // the write must actually be attempted to observe the contention. (The
    // failed writes never update it, so one displacement covers all three
    // layouts below.)
    sm.os.displace_window_for_test(100, 500.0, 500.0);

    // Force a rewrite attempt (mirrors the displaced path).
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        sm.store.contains(100),
        "transient failure must not untrack the window"
    );
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "transient failure must not drop the window from the tree"
    );
    assert!(
        sm.layout_fail_logged.contains_key(&100),
        "first failure should arm the log throttle"
    );

    // Immediate retry (the next layout while still contested): still tracked,
    // throttle entry unchanged (repeat logs at debug, no spam).
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(sm.store.contains(100));
    assert!(sm.layout_fail_logged.contains_key(&100));

    // Contention clears: the write lands and the throttle resets.
    sm.os.clear_transient_for_test(100);
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        !sm.layout_fail_logged.contains_key(&100),
        "success must clear the throttle so the next episode logs fresh"
    );
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()));
}

#[test]
fn drift_failure_stays_tracked_and_retries_without_poisoning_cache() {
    // Login-time Firefox: first write drifts (not-yet-resizable) and must
    // NOT poison `applied_rects` — otherwise manual re-tile skips forever
    // and only a full quit + reopen (fresh WindowId) heals it.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.os.fail_drift_for_test(100);
    // Displace the OS-side rect so the read-before-write check can't skip:
    // the write must actually be attempted to observe the drift. (Drift
    // failures never update the OS rect, so one displacement covers both
    // layouts below.)
    sm.os.displace_window_for_test(100, 500.0, 500.0);

    // Force a rewrite attempt (mirrors the displaced path).
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(sm.store.contains(100), "drift must not untrack the window");
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "drift must not drop the window from the tree"
    );
    assert!(
        sm.layout_fail_logged.contains_key(&100),
        "drift should arm the log throttle as transient"
    );

    // Drift clears (Firefox becomes resizable): next layout lands.
    sm.os.clear_drift_for_test(100);
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        !sm.layout_fail_logged.contains_key(&100),
        "success must clear the throttle"
    );
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()));
}

#[test]
fn wake_skips_windows_already_at_target() {
    // Post-wake full rewrite is the Firefox storm: `on_system_woke` clears
    // `applied_rects`, but windows that haven't moved must still skip via
    // the read-before-write check — no write, no reflow.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let writes_after_tile = sm.os.set_rect_calls_for_test();
    sm.on_system_woke();
    assert_eq!(
        sm.os.set_rect_calls_for_test(),
        writes_after_tile,
        "wake must not rewrite windows already at target"
    );
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "skipped window stays tiled"
    );
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(200).is_some()),
        "skipped window stays tiled"
    );
}

#[test]
fn misplaced_sweep_retiles_tracked_window_without_new_id() {
    // Tracked-but-misplaced: OS rect disagrees with target (drifted login
    // window). The 2s sweep must invalidate `applied_rects` and rewrite —
    // no fresh WindowId required.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let writes_after_tile = sm.os.set_rect_calls_for_test();
    // Externally displace the OS rect far from target, past grace.
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.os.displace_window_for_test(100, 200.0, 200.0);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        sm.os.set_rect_calls_for_test() > writes_after_tile,
        "misplaced sweep must force a rewrite"
    );
}
