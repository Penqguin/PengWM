//! Exact-time focus decisions through the same interface production uses.

use std::time::{Duration, Instant};

use super::common::setup_with_handle;
use crate::adapter_test::TestHandle;
use crate::state::StateManager;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn two_outputs() -> (StateManager, TestHandle) {
    let (mut sm, handle) = setup_with_handle(2);
    for window in [100, 200] {
        sm.on_window_created(window, 42);
    }
    handle.inject_window(42, 300);
    sm.on_window_created(300, 42);
    for window in [200, 300] {
        sm.workspaces[0].remove_window(window);
        sm.workspaces[1].add_window(window, None);
    }
    sm.accept_focus_at(100, Instant::now());
    assert_eq!(sm.displays.focused_output(), Some(1));
    (sm, handle)
}

#[test]
fn switch_protection_holds_output_until_exactly_150_ms_but_updates_workspace() {
    let (mut sm, _) = two_outputs();
    let start = Instant::now();
    assert_eq!(sm.workspaces[1].focused_window_id(), Some(300));
    sm.protect_workspace_switch(start);

    sm.accept_focus_at(200, start + ms(149));
    assert_eq!(sm.workspaces[1].focused_window_id(), Some(200));
    assert_eq!(sm.displays.focused_output(), Some(1));

    sm.accept_focus_at(300, start + ms(150));
    assert_eq!(sm.workspaces[1].focused_window_id(), Some(300));
    assert_eq!(sm.displays.focused_output(), Some(2));
}

#[test]
fn later_selection_replaces_switch_protection_deadline() {
    let (mut sm, _) = two_outputs();
    let start = Instant::now();
    sm.protect_workspace_switch(start);
    sm.protect_workspace_switch(start + ms(100));

    sm.accept_focus_at(200, start + ms(150));
    assert_eq!(sm.displays.focused_output(), Some(1));
    sm.accept_focus_at(200, start + ms(249));
    assert_eq!(sm.displays.focused_output(), Some(1));
    sm.accept_focus_at(200, start + ms(250));
    assert_eq!(sm.displays.focused_output(), Some(2));
}

#[test]
fn later_arrival_rearms_quiet_period_until_exactly_250_ms() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_focused_window(42, Some(200));
    let start = Instant::now();
    sm.on_window_focused_event_at(100, start);
    sm.commit_settled_focus(start + ms(249));
    assert_eq!(sm.workspaces[0].focused_window_id(), Some(100));
    assert!(sm.focus_settle_dirty());

    sm.on_window_focused_event_at(100, start + ms(249));
    sm.commit_settled_focus(start + ms(250));
    assert_eq!(sm.workspaces[0].focused_window_id(), Some(100));
    sm.commit_settled_focus(start + ms(498));
    assert!(sm.focus_settle_dirty());
    assert_eq!(sm.workspaces[0].focused_window_id(), Some(100));
    sm.commit_settled_focus(start + ms(499));
    assert!(!sm.focus_settle_dirty());
    assert_eq!(sm.workspaces[0].focused_window_id(), Some(200));
}

#[test]
fn newer_switch_consumes_older_due_work_but_same_time_switch_allows_settle() {
    for switch_offset in [0, 200] {
        let (mut sm, handle) = setup_with_handle(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        handle.set_focused_window(42, Some(200));
        let start = Instant::now();
        sm.on_window_focused_event_at(100, start);
        sm.protect_workspace_switch(start + ms(switch_offset));
        sm.commit_settled_focus(start + ms(250));

        assert!(!sm.focus_settle_dirty());
        let expected = if switch_offset == 0 { 200 } else { 100 };
        assert_eq!(sm.workspaces[0].focused_window_id(), Some(expected));
        sm.commit_settled_focus(start + ms(500));
        assert_eq!(
            sm.workspaces[0].focused_window_id(),
            Some(expected),
            "consumed work must not resume after the newer switch expires"
        );
    }
}

#[test]
fn settled_os_observation_and_none_fallback_reuse_the_decision_instant() {
    for fallback in [false, true] {
        let (mut sm, handle) = two_outputs();
        // Protection is still in the future relative to the wall clock,
        // but expires before the supplied settled decision instant.
        let start = Instant::now() + Duration::from_secs(60);
        handle.set_focused_window(42, Some(200));
        sm.on_window_focused_event_at(200, start - ms(300));
        sm.commit_settled_focus(start - ms(50));
        assert_eq!(sm.displays.focused_output(), Some(2));
        sm.accept_focus_at(100, start);
        sm.protect_workspace_switch(start);
        sm.on_window_focused_event_at(100, start);
        if fallback {
            handle.set_focused_window(42, None);
        }

        sm.commit_settled_focus(start + ms(250));

        assert_eq!(sm.workspaces[1].focused_window_id(), Some(200));
        assert_eq!(
            sm.displays.focused_output(),
            Some(2),
            "nested acceptance must reuse settle time, including fallback={fallback}"
        );
    }
}

#[test]
fn structural_reset_intent_clears_switch_protection() {
    let (mut sm, _) = two_outputs();
    let start = Instant::now();
    sm.protect_workspace_switch(start);
    sm.accept_focus_at(200, start + ms(1));
    assert_eq!(sm.displays.focused_output(), Some(1));

    sm.reset_focus_protection();
    sm.accept_focus_at(200, start + ms(1));
    assert_eq!(sm.displays.focused_output(), Some(2));
}

#[test]
fn destroyed_remembered_window_cannot_receive_none_fallback() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let start = Instant::now();
    handle.set_focused_window(42, Some(200));
    sm.on_window_focused_event_at(200, start);
    sm.commit_settled_focus(start + ms(250));
    sm.on_window_destroyed(200);
    handle.set_focused_window(42, None);
    sm.on_window_focused_event_at(100, start + ms(300));
    let focused = handle.last_focused();

    sm.commit_settled_focus(start + ms(550));

    assert_eq!(sm.workspaces[0].focused_window_id(), Some(100));
    assert!(!sm.store.contains(200));
    assert_eq!(handle.last_focused(), focused);
    assert!(!sm.focus_settle_dirty());
}
