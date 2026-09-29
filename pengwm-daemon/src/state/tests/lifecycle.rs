use super::common::*;
use pengwm_core::command::Command;
use tokio::sync::mpsc;

#[test]
fn on_window_created_tracks_pid_and_adds_to_workspace() {
    let mut sm = setup(1);
    sm.on_window_created(300, 42);
    assert_eq!(sm.store.all_window_pids().get(&300), Some(&42));
    assert!(sm.workspaces[0].find_window(300).is_some());
}

#[test]
fn on_window_destroyed_removes_tracking_and_window() {
    let mut sm = setup(1);
    sm.on_window_destroyed(100);
    assert!(!sm.store.contains(100));
    assert!(sm.workspaces[0].find_window(100).is_none());
}

#[test]
fn on_window_focused_updates_active_workspace() {
    let mut sm = setup(1);
    sm.on_window_created(300, 42);
    sm.on_window_focused(300);
    let ws = &sm.workspaces[0];
    assert_eq!(ws.focused_node, ws.find_window(300));
}

#[test]
fn on_app_launched_attaches_observer_and_tracks_windows() {
    let mut sm = setup(1);
    // TestAdapter pre-populated with pid 42 having windows 100, 200
    // on_app_launched for pid 99 should attach observer and query windows
    sm.on_app_launched(99);
    // The observer was attached (no-op in TestAdapter, but method was called)
    // No windows returned for this pid since TestAdapter has none for pid 99
    assert!(!sm.store.all_pids().contains_key(&99));
}

#[test]
fn on_app_terminated_detaches_observer_and_cleans_windows() {
    let mut sm = setup(1);
    // Start with pid 42 having windows 100, 200
    assert!(sm.store.contains(100));
    sm.on_app_terminated(42);
    assert!(!sm.store.contains(100));
    assert!(!sm.store.contains(200));
}

#[test]
fn on_app_activated_updates_frontmost_pid() {
    let mut sm = setup(1);
    sm.on_app_activated(99);
    assert_eq!(sm.frontmost_pid, Some(99));
}

#[test]
fn on_window_hidden_removes_from_tree_but_keeps_pid() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    assert_eq!(sm.workspaces[0].window_count(), 2);
    sm.on_window_hidden(100);
    assert!(sm.workspaces[0].find_window(100).is_none());
    assert_eq!(sm.store.all_window_pids().get(&100), Some(&42));
    assert!(sm.store.is_hidden(100));
    assert_eq!(sm.workspaces[0].window_count(), 1);
}

#[test]
fn on_window_shown_retiles_into_original_workspace() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.on_window_hidden(100);
    sm.on_window_shown(100);
    assert!(sm.workspaces[0].find_window(100).is_some());
    assert!(!sm.store.is_hidden(100));
    assert_eq!(sm.workspaces[0].window_count(), 2);
}

#[test]
fn on_window_shown_ignores_untracked_window() {
    let mut sm = setup(1);
    sm.on_window_shown(999);
    assert!(sm.workspaces[0].find_window(999).is_none());
}

#[test]
fn on_window_shown_skips_window_already_tracked() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_shown(100);
    assert_eq!(sm.workspaces[0].window_count(), 1);
}

#[test]
fn destroyed_hidden_window_cleans_hidden_workspace() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_hidden(100);
    assert!(sm.store.is_hidden(100));
    sm.on_window_destroyed(100);
    assert!(!sm.store.is_hidden(100));
    assert!(!sm.store.contains(100));
}

// Reconcile logic is now unit-tested in hidden.rs via predicate injection.
// StateManager integration for reconcile is exercised through on_window_hidden/shown.
// The three previous reconcile tests (hidden_windows.insert + last_reconcile) are
// migrated to hidden::tests::pending_* and hidden::tests::should_reconcile_*

#[test]
fn hidden_focus_and_move_ignored() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.on_window_hidden(100);
    assert!(sm.store.is_hidden(100));
    // Focus from hidden should not flip active
    let before = sm.displays.active().get(&1).copied();
    sm.on_window_focused(100);
    assert_eq!(sm.displays.active().get(&1).copied(), before);
    // Move from hidden should not affect drag
    sm.on_window_moved(100, 1919.0, 1079.0);
    // No panic, still hidden
    assert!(sm.store.is_hidden(100));
}

#[test]
fn reveal_all_clears_tracker_and_retiles() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.on_window_hidden(100);
    assert!(sm.store.is_hidden(100));
    assert!(sm.workspaces[0].find_window(100).is_none());
    let (rtx, _) = mpsc::channel(1);
    sm.on_command(Command::RevealAll, Some(rtx));
    assert!(
        !sm.store.is_hidden(100),
        "RevealAll must clear HiddenTracker"
    );
    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "window should be re-tiled"
    );
    // Second reveal is idempotent
    let (rtx2, _) = mpsc::channel(1);
    sm.on_command(Command::RevealAll, Some(rtx2));
    assert!(!sm.store.is_hidden(100));
}

#[test]
fn reveal_all_via_hidden_drain_is_idempotent() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_hidden(100);
    sm.reveal_all();
    assert!(!sm.store.is_hidden(100));
    sm.reveal_all();
    assert!(!sm.store.is_hidden(100));
}

#[test]
fn missed_window_created_sweep_tiles_new_window() {
    // Firefox tear-off / incognito: OS has the window but no
    // WindowCreated event was ever delivered (missing AX notification
    // or transient non-manageable subrole at creation time). The
    // background sweep must discover it via poll and tile it.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // OS-side only: adapter knows 300, StateManager does not.
    handle.inject_window(42, 300);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(300).is_some()),
        "sweep should tile the missed Firefox window 300"
    );
}

#[test]
fn app_activated_tiles_new_window_immediately() {
    // Fast path: a tear-off usually focuses its new window, firing
    // AppActivated even when WindowCreated was missed.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.inject_window(42, 300);
    sm.on_app_activated(42);
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(300).is_some()),
        "activation poll should tile the missed window without waiting for sweep"
    );
}
