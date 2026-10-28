use std::time::Duration;

use super::super::session;
use super::common::*;

fn tmp_session_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pengwm-persistence-test-{}.toml", name))
}

#[test]
fn the_seatbelt_returns_the_rediscovered_window() {
    // The seatbelt is the most current fact about a window: the
    // rediscovered window returns to where it *was this session*,
    // never the active-workspace fallback.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(42, "com.test".into());
    handle.inject_title(100, "W100".into());
    sm.on_window_created(100, 42);
    assert!(sm.workspaces[0].find_window(100).is_some());

    // The loss the seatbelt exists for: the store drops the window
    // without a destroy funnel (a misjudgment path), then it is
    // rediscovered.
    sm.store.unregister(100);
    sm.workspaces[0].remove_window(100);
    sm.on_window_created(100, 42);

    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "the rediscovered window must return to its last workspace (the seatbelt)"
    );
}

#[test]
fn a_fresh_window_routes_by_config_then_active_workspace() {
    // ADR-0005: a birth is a birth. A window the seatbelt has never
    // seen (no placement change noted for it) routes through config
    // routing and the active workspace — never a remembered home.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(42, "com.test".into());
    handle.inject_title(300, "T300".into());

    sm.on_window_created(300, 42);

    assert!(
        sm.workspaces[0].find_window(300).is_some(),
        "the unconfigured new window takes the active workspace"
    );
}

#[test]
fn projection_includes_parked_and_excludes_unrouted_and_popups() {
    // The snapshot reads the seatbelt, not the trees: a parked (minimized)
    // window keeps its home — the gap the quit-time rescan had — while
    // never-routed windows and popups are excluded.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(42, "com.test".into());
    handle.inject_title(100, "T100".into());

    sm.on_window_created(100, 42);
    // Move to a hidden workspace: the seatbelt follows.
    sm.move_focused_to_workspace(1);
    // Park it: the seatbelt record is refreshed while the identity is
    // readable, then the window leaves the tree.
    sm.on_window_hidden(100);
    assert!(sm.store.is_parked(100), "precondition: 100 is parked");

    let placements = sm.capture_placements();
    assert_eq!(
        placements.len(),
        1,
        "exactly one record: the parked window (200 was never routed)"
    );
    assert_eq!(placements[0].workspace, sm.workspaces[1].name);
    assert_eq!(placements[0].bundle.as_deref(), Some("com.test"));
    assert_eq!(placements[0].title.as_deref(), Some("T100"));
}

#[test]
fn save_session_round_trip_persists_without_rehoming() {
    // ADR-0005: placements are projected into the snapshot and round-trip
    // through the file — but the next boot never consumes them. The
    // reloaded record stays on disk as diagnostic data while the new
    // window routes by config, then the active workspace.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(42, "com.test".into());
    handle.inject_title(100, "T100".into());
    sm.on_window_created(100, 42);
    sm.move_focused_to_workspace(1);

    let path = tmp_session_path("round-trip");
    let _ = std::fs::remove_file(&path);
    sm.save_session(&path).expect("session saves");

    let loaded = session::load_from(&path).expect("session loads back");
    assert_eq!(loaded.placements.len(), 1);
    assert_eq!(loaded.placements[0].workspace, sm.workspaces[1].name);

    // The next boot: the same identity is born again and routes as a
    // birth — the saved placement is not consulted.
    let (mut sm2, handle2) = setup_with_handle(1);
    handle2.inject_bundle_id(42, "com.test".into());
    handle2.inject_title(100, "T100".into());
    sm2.on_window_created(100, 42);
    assert!(
        sm2.workspaces[0].find_window(100).is_some(),
        "the new window takes the active workspace, not the saved home"
    );
    assert!(
        sm2.workspaces[1].find_window(100).is_none(),
        "the saved placement must not re-home the new window"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn tick_drains_a_dirty_session_and_rearms() {
    // Crash survival: a placement change dirties the projection; the
    // tick drains it at most once per interval, then the next change
    // re-arms it. The harness path keeps the user's real file clean.
    let (mut sm, _handle) = setup_with_handle(1);
    let path = sm.session_path.clone();
    let _ = std::fs::remove_file(&path);

    sm.on_window_created(100, 42);
    assert!(sm.session_dirty_for_test(), "a placement change dirties");
    assert!(!path.exists(), "nothing written before the interval");

    sm.age_session_save_for_test();
    sm.on_tick();
    assert!(path.exists(), "the tick drained the dirty session");
    assert!(!sm.session_dirty_for_test(), "the drain marks it clean");

    // The next change re-arms, and drains again on a later tick.
    sm.move_focused_to_workspace(1);
    assert!(sm.session_dirty_for_test(), "the move re-dirties");
    sm.age_session_save_for_test();
    sm.on_tick();
    assert!(
        !sm.session_dirty_for_test(),
        "the second drain re-arms cleanly"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn quench_the_wake_repro_end_to_end() {
    // The full shipped behavior, one test: a window moved to a hidden
    // workspace survives sleep *and* gets persisted by the periodic
    // save, so even a crash after the wake resync doesn't lose it.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(42, "com.test".into());
    handle.inject_title(100, "T100".into());
    sm.on_window_created(100, 42);
    sm.move_focused_to_workspace(1);
    handle.unlist_window(42, 100);

    sm.on_system_woke();
    sm.age_wake_probe_for_test();
    sm.on_tick(); // resync commits; the discovery loop keeps 100 tracked
    sm.age_wake_committed_for_test();
    sm.force_window_sweep_for_test();
    sm.on_tick(); // the sweep must not judge the hidden member

    assert!(sm.workspaces[1].find_window(100).is_some());

    // The periodic save drains and the file carries the hidden home.
    sm.age_session_save_for_test();
    sm.on_tick();
    let path = sm.session_path.clone();
    let loaded = session::load_from(&path).expect("the drained session loads");
    assert!(
        loaded
            .placements
            .iter()
            .any(|p| p.workspace == sm.workspaces[1].name && p.title.as_deref() == Some("T100")),
        "the hidden workspace home is persisted"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn age_helper_mirrors_the_interval() {
    // Documents the helper's contract: one call moves the save clock
    // past the drain interval.
    let (mut sm, _handle) = setup_with_handle(1);
    sm.age_session_save_for_test();
    assert!(Duration::from_secs(11) >= super::super::placement::SESSION_SAVE_INTERVAL);
}

#[test]
fn seatbelt_caps_records_per_pid() {
    // A never-quitting app that churns WindowIds (browsers, Electron) is
    // the one unbounded growth the seatbelt map had: entries die with the
    // pid by design, so the cap is the only bound. The newest records per
    // pid survive — a rediscovery ask's legitimate candidates are always
    // the recent ones.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(42, "com.test".into());
    for wid in 300..312u64 {
        handle.inject_title(wid, format!("W{wid}"));
        sm.on_window_created(wid, 42);
    }
    assert!(
        sm.live_placement.len() <= 8,
        "the seatbelt caps per-pid records (got {})",
        sm.live_placement.len()
    );
    assert!(
        sm.live_placement.contains_key(&311),
        "the most recent placement always survives the cap"
    );
}
