use std::time::Duration;

use super::common::*;

#[test]
fn system_woke_resyncs_and_retiles() {
    // Sleep invalidates AX refs + display geometry while `applied_rects`
    // still claims "already at target". Wake must drop the cache, discover
    // untracked windows via the normal created path, and rewrite.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let writes_after_tile = handle.writes();
    // Identical layout skips before wake.
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(handle.writes(), writes_after_tile);
    // Window that appeared mid-sleep: OS knows it, store does not.
    handle.inject_window(42, 300);
    // Window moved mid-sleep: OS truth disagrees with the cleared cache, so
    // the read-before-write check can't skip it and a rewrite is forced.
    // (Without this, placed windows correctly skip and no write happens.)
    handle.displace(200, 200.0, 200.0);
    sm.on_system_woke();
    // The resync is deferred until AX answers; the tick drives it.
    sm.on_tick();
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(300).is_some()),
        "wake must tile windows missed during sleep"
    );
    assert!(
        handle.writes() > writes_after_tile,
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
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_fault(
        100,
        Fault::Transient("kAXErrorFailure (live resize)".into()),
    );
    // Displace the OS-side rect so the read-before-write check can't skip:
    // the write must actually be attempted to observe the contention. (The
    // failed writes never update it, so one displacement covers all three
    // layouts below.)
    handle.displace(100, 500.0, 500.0);

    // Force a rewrite attempt (mirrors the displaced path).
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
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
        sm.layout_cache.throttle_armed(100),
        "first failure should arm the log throttle"
    );

    // Immediate retry (the next layout while still contested): still tracked,
    // throttle entry unchanged (repeat logs at debug, no spam).
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(sm.store.contains(100));
    assert!(sm.layout_cache.throttle_armed(100));

    // Contention clears: the write lands and the throttle resets.
    handle.clear_fault(100);
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        !sm.layout_cache.throttle_armed(100),
        "success must clear the throttle so the next episode logs fresh"
    );
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()));
}

#[test]
fn drift_failure_stays_tracked_and_retries_without_poisoning_cache() {
    // Login-time Firefox: first write drifts (not-yet-resizable) and must
    // NOT poison `applied_rects` — otherwise manual re-tile skips forever
    // and only a full quit + reopen (fresh WindowId) heals it.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_fault(100, Fault::Drift);
    // Displace the OS-side rect so the read-before-write check can't skip:
    // the write must actually be attempted to observe the drift. (Drift
    // failures never update the OS rect, so one displacement covers both
    // layouts below.)
    handle.displace(100, 500.0, 500.0);

    // Force a rewrite attempt (mirrors the displaced path).
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(sm.store.contains(100), "drift must not untrack the window");
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "drift must not drop the window from the tree"
    );
    assert!(
        sm.layout_cache.throttle_armed(100),
        "drift should arm the log throttle as transient"
    );

    // Drift clears (Firefox becomes resizable): next layout lands.
    handle.clear_fault(100);
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        !sm.layout_cache.throttle_armed(100),
        "success must clear the throttle"
    );
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()));
}

#[test]
fn wake_skips_windows_already_at_target() {
    // Post-wake full rewrite is the Firefox storm: `on_system_woke` clears
    // `applied_rects`, but windows that haven't moved must still skip via
    // the read-before-write check — no write, no reflow.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let writes_after_tile = handle.writes();
    sm.on_system_woke();
    sm.on_tick();
    assert!(
        !sm.wake_resync_pending(),
        "AX answered, so the resync committed"
    );
    assert_eq!(
        handle.writes(),
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
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    let writes_after_tile = handle.writes();
    // Externally displace the OS rect far from target, past grace.
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    handle.displace(100, 200.0, 200.0);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        handle.writes() > writes_after_tile,
        "misplaced sweep must force a rewrite"
    );
}

#[test]
fn pin_backoff_survives_the_misplaced_sweep() {
    // The post-wake Firefox storm. An app that refuses every write reports
    // Pinned each time, and `reconcile_misplaced_windows` runs every 2s —
    // a pinned window always reads as displaced, so the sweep invalidates
    // it on every tick. If `invalidate` also cleared the strike count, the
    // counter reset to 1 forever: PIN_STRIKES was unreachable, the backoff
    // never engaged, and Firefox took a full 3-attempt rewrite (~15 AX
    // round trips, each a reflow) every 2 seconds indefinitely.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_fault(100, Fault::Pinned);
    handle.displace(100, 500.0, 500.0);
    // Age the skip entry past DISPLACE_GRACE, as 2s of wall clock would.
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));

    let mut per_sweep = Vec::new();
    for _ in 0..6 {
        let before = handle.writes();
        sm.force_window_sweep_for_test();
        sm.on_tick();
        per_sweep.push(handle.writes() - before);
    }
    assert_eq!(
        per_sweep,
        vec![1, 1, 1, 0, 0, 0],
        "backoff must engage after PIN_STRIKES and hold through the sweep"
    );
    // Backoff is a write throttle, not an untrack: the window stays owned
    // and stays tiled, and the workspace keeps its other window placed.
    assert!(sm.store.contains(100));
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()));
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(200).is_some()));

    // Time-bounded, never permanent: when the timer expires the sweep
    // retries exactly once, then goes quiet again for another backoff.
    sm.layout_cache
        .age_pin_for_test(100, Duration::from_secs(30));
    let before = handle.writes();
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert_eq!(handle.writes() - before, 1, "expired backoff retries once");
    let before = handle.writes();
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert_eq!(
        handle.writes() - before,
        0,
        "and re-arms rather than storming"
    );

    // The app finally accepts writes: the pin heals and the window lands.
    handle.clear_fault(100);
    sm.layout_cache
        .age_pin_for_test(100, Duration::from_secs(30));
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        !sm.layout_cache
            .pin_backoff_active(100, std::time::Instant::now()),
        "a landed write clears the pin"
    );
}

#[test]
fn wake_resync_waits_for_ax_instead_of_writing_through_stale_elements() {
    // The bug this exists for: `NSWorkspaceDidWake` arrives while AX is
    // still blacked out. Resyncing inline polled every app, got nothing
    // back (so the stale element refs survived), then wrote layouts
    // through those dead refs — every live window reported Gone and
    // started a 10s death timer. The resync must wait for AX instead.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let writes_after_tile = handle.writes();

    handle.set_ax_blackout(true);
    sm.on_system_woke();
    assert!(
        sm.wake_resync_pending(),
        "wake arms the resync, it does not run it"
    );

    // Several ticks through the blackout: no writes, and crucially no
    // gone-grace timers started on windows that are very much alive.
    for _ in 0..4 {
        sm.age_wake_probe_for_test();
        sm.on_tick();
    }
    assert!(sm.wake_resync_pending(), "still waiting on AX");
    assert_eq!(
        handle.writes(),
        writes_after_tile,
        "must not write through stale elements during the blackout"
    );
    assert!(sm.store.contains(100) && sm.store.contains(200));
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()));
    assert!(sm.workspaces.iter().any(|ws| ws.find_window(200).is_some()));

    // AX comes back, and a window appeared during sleep.
    handle.set_ax_blackout(false);
    handle.inject_window(42, 300);
    handle.displace(200, 300.0, 300.0);
    sm.age_wake_probe_for_test();
    sm.on_tick();

    assert!(!sm.wake_resync_pending(), "resync commits once AX answers");
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(300).is_some()),
        "the committed resync tiles windows missed during sleep"
    );
    assert!(
        handle.writes() > writes_after_tile,
        "the committed resync re-asserts displaced windows"
    );
}

#[test]
fn wake_resync_commits_at_the_deadline_even_if_ax_never_answers() {
    // A probe that can never succeed (genuinely windowless desktop, or an
    // AX subsystem that stays wedged) must not leave the resync armed
    // forever — the deadline forces it through exactly once.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_ax_blackout(true);
    sm.on_system_woke();

    sm.age_wake_probe_for_test();
    sm.on_tick();
    assert!(sm.wake_resync_pending(), "not yet at the deadline");

    sm.age_wake_deadline_for_test();
    sm.on_tick();
    assert!(
        !sm.wake_resync_pending(),
        "deadline commits the resync and disarms it"
    );
}

#[test]
fn double_wake_notification_arms_once() {
    // macOS fires both NSWorkspaceDidWake and ScreensDidWake for one wake.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_ax_blackout(true);
    sm.on_system_woke();
    sm.on_system_woke();
    assert!(sm.wake_resync_pending());
    // One commit clears it; the duplicate did not queue a second resync.
    handle.set_ax_blackout(false);
    sm.age_wake_probe_for_test();
    sm.on_tick();
    assert!(!sm.wake_resync_pending());
}
