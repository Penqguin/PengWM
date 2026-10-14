use super::common::*;
use crate::state::store::RoutingVerdict;
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
#[should_panic(expected = "parked xor popup-member must hold")]
fn identity_invariant_catches_a_drifted_verdict() {
    // ADR-0003's harness check: a Popup verdict that is neither parked
    // nor a member of any popup set is exactly the drift the check
    // exists to catch; every funnel ends its tests through the assert.
    let mut sm = setup(1);
    sm.store.inject(300, 42);
    sm.store.note_routed(300, RoutingVerdict::Popup);
    sm.assert_identity_invariant();
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
fn sweep_untracks_window_whose_destroyed_notification_was_missed() {
    // 1Password exit / Preview-close from Finder: the window closes but no
    // destroyed notification arrives. The app's listing still answers
    // (other windows present), so the sweep untracks the dead window and
    // its siblings re-tile in the same pass — no 10s grace, no stranded
    // tree entry waiting for an unrelated layout event.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.unlist_window(42, 100);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        !sm.store.contains(100),
        "a tracked window its app no longer lists is dead"
    );
    assert!(sm.workspaces[0].find_window(100).is_none());
    assert!(
        sm.workspaces[0].find_window(200).is_some(),
        "the surviving sibling keeps its tile"
    );
    assert_eq!(sm.workspaces[0].window_count(), 1);
}

#[test]
fn sweep_untracks_the_app_whose_only_window_closed() {
    // A PDF closed in Preview, the 1Password vault closed: the destroyed
    // notification is missed and the app stays running with *zero*
    // windows. Its empty-but-answered listing is death evidence — the
    // pre-Listing rule ("zero listing is no judgment") stranded the dead
    // window in the tree forever, freezing the layout.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    assert_eq!(sm.workspaces[0].window_count(), 1);
    // Preview keeps running: the empty answer IS the app's verdict.
    handle.unlist_window(42, 100);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        !sm.store.contains(100),
        "an answered-but-empty listing untracks its app's tracked windows"
    );
    assert_eq!(
        sm.workspaces[0].window_count(),
        0,
        "the dead window left the tree, retiling is free to shrink"
    );
}

#[test]
fn unanswered_listing_makes_no_judgment_in_the_sweep() {
    // AX answering nothing (the blackout signature) is not a mass close —
    // the sweep must not untrack on it. The app that *answers* with zero
    // windows is a different fact, judged by
    // `sweep_untracks_the_app_whose_only_window_closed`.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_ax_blackout(true);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        sm.store.contains(100),
        "a NoAnswer listing must not count as evidence of death"
    );
}

#[test]
fn unknown_destroyed_probe_untracks_the_dead_window() {
    // A destroyed notification fired for an element we could not map to a
    // window id (stale ref / CFEqual mismatch). The pid still says who
    // reported the death: the probe polls that pid and the unlisted
    // window dies immediately rather than waiting for the next sweep.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.unlist_window(42, 100);
    sm.on_unknown_window_destroyed(42);
    assert!(!sm.store.contains(100));
    assert!(sm.workspaces[0].find_window(100).is_none());
    assert!(
        sm.workspaces[0].find_window(200).is_some(),
        "probe untracks only what the listing actually lost"
    );
}

#[test]
fn unknown_destroyed_probe_untracks_the_app_whose_only_window_closed() {
    // The 1Password stale-ref close: a destroyed notification fired for
    // an element we could not map, and the probe's poll answers "zero
    // windows" — the app's own verdict after its only window closed. The
    // empty answer judges; the dead window leaves immediately.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.unlist_window(42, 100);
    sm.on_unknown_window_destroyed(42);
    assert!(
        !sm.store.contains(100),
        "an answered-empty probe untracks the closed-only-window app"
    );
}

#[test]
fn unknown_destroyed_probe_with_no_answer_makes_no_judgment() {
    // Event during a blackout (theoretically impossible — a dead AX can't
    // deliver notifications — but the probe must still be safe): the
    // unanswered poll is not judged. An answered-empty poll is, by
    // contrast — see `sweep_untracks_the_app_whose_only_window_closed`.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_ax_blackout(true);
    sm.on_unknown_window_destroyed(42);
    assert!(sm.store.contains(100));
}

#[test]
fn unknown_destroyed_probe_ignores_excluded_and_unknown_pids() {
    // An excluded app owes us no windows; an app with no tracked windows
    // has nothing to lose. Both are cheap no-ops, not errors.
    let (mut sm, _handle) = setup_with_handle(1);
    sm.on_unknown_window_destroyed(0);
    sm.on_unknown_window_destroyed(9999);
    assert!(sm.store.contains(100) && sm.store.contains(200));
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

#[test]
fn every_untrack_path_forges_the_os_side_eviction() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    assert!(!handle.forgotten().contains(&100));

    // Destroyed notification arrives: the element cache must not keep the
    // window's retained ref — the leak that grew the daemon per closed
    // window in apps whose AX destroyed notifications never fire.
    sm.on_window_destroyed(100);
    assert!(handle.forgotten().contains(&100));

    // And the missed-notification sweep path evicts too (it funnels
    // through the same `on_window_destroyed`).
    handle.unlist_window(42, 200);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(handle.forgotten().contains(&200));
}

#[test]
fn app_termination_evicts_every_window_of_the_pid() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // AppTerminated sends WindowDestroyed events per window; near the
    // dispatch they drain through the same funnel.
    handle.clear_forgotten_for_test();
    sm.on_app_terminated(42);
    let forgotten = handle.forgotten();
    assert!(forgotten.contains(&100) && forgotten.contains(&200));
}
