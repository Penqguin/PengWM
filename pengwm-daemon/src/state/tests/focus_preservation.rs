//! Non-clock focus policy preserved by the focus module extraction.

use std::time::{Duration, Instant};

use super::common::setup_with_handle;
use crate::state::StateManager;

fn settle(sm: &mut StateManager) {
    // No switch is active in these scenarios. Advancing only the drain's
    // decision instant is sufficient to exercise the existing quiet gate.
    sm.commit_settled_focus(Instant::now() + Duration::from_millis(250));
}

#[test]
fn activation_accepts_immediate_focus_without_arming_settled_work() {
    for activated_focus in [Some(100), None] {
        let (mut sm, handle) = setup_with_handle(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        sm.on_window_focused(200);
        handle.set_focused_window(42, activated_focus);

        sm.on_app_activated(42);

        let expected = activated_focus.unwrap_or(200);
        assert_eq!(sm.frontmost_pid, Some(42));
        assert_eq!(sm.workspaces[0].focused_window_id(), Some(expected));
        assert!(!sm.focus_settle_dirty());

        handle.set_focused_window(42, Some(if expected == 100 { 200 } else { 100 }));
        settle(&mut sm);
        assert_eq!(
            sm.workspaces[0].focused_window_id(),
            Some(expected),
            "activation alone creates no later OS-focus correction"
        );
    }
}

#[test]
fn rejected_os_some_replaces_memory_without_falling_back() {
    for rejected in ["hidden", "parked", "unknown"] {
        let (mut sm, handle) = setup_with_handle(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        handle.set_focused_window(42, Some(200));
        sm.on_window_focused_event(200);
        settle(&mut sm);
        assert_eq!(sm.workspaces[0].focused_window_id(), Some(200));

        let candidate = 300;
        match rejected {
            "hidden" => {
                handle.inject_window(42, candidate);
                sm.on_window_created(candidate, 42);
                // Give the candidate a hidden home without selecting it.
                sm.workspaces[0].remove_window(candidate);
                sm.workspaces[1].add_window(candidate, None);
            }
            "parked" => {
                handle.inject_window(42, candidate);
                sm.on_window_created(candidate, 42);
                handle.inject_hidden_window(candidate);
                sm.on_window_hidden(candidate);
                assert!(sm.store.is_parked(candidate));
            }
            "unknown" => assert!(!sm.store.contains(candidate)),
            _ => unreachable!(),
        }

        sm.on_window_focused_event(100);
        handle.set_focused_window(42, Some(candidate));
        settle(&mut sm);
        assert_eq!(
            sm.workspaces[0].focused_window_id(),
            Some(100),
            "rejected {rejected} Some must not fall back to remembered 200"
        );
        assert!(!sm.focus_settle_dirty());

        handle.set_focused_window(42, None);
        sm.on_window_focused_event(100);
        settle(&mut sm);
        assert_eq!(
            sm.workspaces[0].focused_window_id(),
            Some(100),
            "None after rejected {rejected} Some must not revive old memory 200"
        );
        assert_eq!(sm.displays.active().get(&1), Some(&0));
    }
}

#[test]
fn structural_reset_preserves_armed_work_and_settled_memory() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_focused_window(42, Some(200));
    sm.on_window_focused_event(200);
    settle(&mut sm);

    sm.on_window_focused_event(100);
    assert!(sm.wm_reset().is_some());
    assert!(sm.focus_settle_dirty(), "reset retains the pending drain");
    // Reset re-admits members; choose a contrasting immediate seat without
    // changing the settle obligation or its remembered OS observation.
    sm.on_window_focused(100);
    handle.set_focused_window(42, None);
    settle(&mut sm);

    assert_eq!(sm.workspaces[0].focused_window_id(), Some(200));
    assert!(!sm.focus_settle_dirty());
}

#[test]
fn native_space_limbo_keeps_immediate_and_settled_focus_bookkeeping() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    assert!(!sm.display_may_act(1));
    let writes = handle.writes();
    let focused = handle.last_focused();

    sm.on_window_focused_event(100);
    assert_eq!(sm.workspaces[0].focused_window_id(), Some(100));
    handle.set_focused_window(42, Some(200));
    settle(&mut sm);

    assert_eq!(sm.workspaces[0].focused_window_id(), Some(200));
    assert!(!sm.focus_settle_dirty());
    assert_eq!(handle.writes(), writes);
    assert_eq!(handle.last_focused(), focused);
}
