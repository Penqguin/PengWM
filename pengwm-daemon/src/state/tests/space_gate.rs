//! The native-Space gate (ADR-0007): while a display shows any user
//! desktop other than its first, the WM does not act on that display —
//! but a display on its first desktop stays fully managed.

use super::common::*;
use pengwm_core::command::Command;
use pengwm_core::tree::Direction;
use std::time::Duration;

#[test]
fn layout_writes_are_deferred_while_the_display_is_off_its_first_desktop() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);

    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    assert!(!sm.display_may_act(1));

    handle.displace(100, 300.0, 200.0);
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, handle.rect(100).unwrap());
    let writes = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        writes,
        "no OS write may land on a display showing a non-first desktop"
    );
}

#[test]
fn returning_to_the_first_desktop_resyncs_the_layout() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);

    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    handle.displace(100, 300.0, 200.0);
    // Displacement invalidates the applied cache, so a resync must write.
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, handle.rect(100).unwrap());
    let writes = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(handle.writes(), writes);

    handle.set_first_desktop(1, true);
    sm.refresh_space_gate_for_test();
    assert!(sm.display_may_act(1));
    assert!(
        handle.writes() > writes,
        "the return resync must re-assert the visible workspace from the model"
    );
}

#[test]
fn commands_acting_on_a_gated_display_are_refused() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let focused = sm.workspaces[0].focused_window_id();

    handle.set_first_desktop(1, false);
    // `on_command` refreshes the gate itself: a keybind right after the
    // swipe must not act on the Space the user just left.
    sm.on_command(
        Command::Focus {
            direction: Direction::Left,
        },
        None,
    );
    assert_eq!(
        sm.workspaces[0].focused_window_id(),
        focused,
        "the model must not move focus while the display is gated"
    );
    assert_eq!(handle.last_focused(), None, "no OS focus call either");
}

#[test]
fn a_gated_display_does_not_freeze_its_neighbor() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.on_window_created(100, 42);
    handle.set_first_desktop(2, false);
    sm.refresh_space_gate_for_test();

    handle.displace(100, 300.0, 200.0);
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, handle.rect(100).unwrap());
    let writes = handle.writes();
    sm.apply_layout(0);
    assert!(
        handle.writes() > writes,
        "display 1 is on its first desktop and stays fully managed"
    );
}

#[test]
fn focus_and_switch_commands_gate_at_the_resolved_display() {
    let (mut sm, handle) = setup_with_handle(2);
    assert_eq!(sm.displays.active().get(&1), Some(&0));
    assert_eq!(sm.displays.active().get(&2), Some(&1));
    sm.on_window_created(100, 42);
    // Put a window on the second output's workspace while it is allowed.
    sm.on_command(Command::MoveWindowToWorkspace { id: 2 }, None);
    assert!(sm.workspaces[1].find_window(100).is_some());

    // Display 2 is off its first desktop: switches to its workspace and
    // focus-display calls aimed at it are refused.
    handle.set_first_desktop(2, false);
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(sm.displays.focused_output(), Some(1));
    sm.on_command(
        Command::FocusDisplay {
            direction: Direction::Right,
        },
        None,
    );
    assert_eq!(sm.displays.focused_output(), Some(1));

    // Swap which display is gated: display 2 is back and focusable while
    // display 1 is now the one the WM leaves alone.
    handle.set_first_desktop(2, true);
    handle.set_first_desktop(1, false);
    sm.on_command(
        Command::FocusDisplay {
            direction: Direction::Right,
        },
        None,
    );
    assert_eq!(
        sm.displays.focused_output(),
        Some(2),
        "a first-desktop display remains focusable while another is gated"
    );
}

#[test]
fn deferred_hides_drain_when_the_display_returns() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    // Admission into a hidden workspace while the display is away: move
    // the model membership without touching the OS.
    sm.workspaces[0].remove_window(100);
    sm.workspaces[1].add_window(100, None);
    let before = handle.rect(100);

    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    sm.hide_window_in_workspace(100, 1);
    assert!(sm.deferred_hides.contains_key(&100));
    assert_eq!(handle.rect(100), before, "no hide write while gated");

    handle.set_first_desktop(1, true);
    sm.refresh_space_gate_for_test();
    assert!(sm.deferred_hides.is_empty());
    assert_ne!(
        handle.rect(100),
        before,
        "the deferred hide placement lands on return"
    );
}

#[test]
fn reveal_stack_skips_a_gated_display() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_window_kind(300, crate::adapter::WindowClass::Untracked);
    sm.on_window_created(300, 42);
    handle.inject_rect(
        300,
        pengwm_core::layout::Rect::new(800.0, 400.0, 200.0, 100.0),
    );
    let raised = handle.raised().len();

    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    sm.reveal_stack(Some(1));
    sm.reveal_stack(None);
    assert_eq!(
        handle.raised().len(),
        raised,
        "no re-stack may land on a gated display"
    );
}

#[test]
fn untrack_judgment_is_deferred_until_the_display_returns() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);

    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    // The app answers its listing without the window, and the element no
    // longer answers (the fake drops the rect with the listing) — the
    // shape of a real close, but observed while the display is away.
    handle.unlist_window(42, 100);
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        sm.store.contains(100),
        "an off-first-desktop omission is not a death verdict"
    );

    handle.set_first_desktop(1, true);
    sm.refresh_space_gate_for_test();
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        !sm.store.contains(100),
        "the same evidence judges once the display is back on its first desktop"
    );
}

#[test]
fn detector_unavailable_fails_open_and_resyncs() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    assert!(!sm.display_may_act(1));

    handle.set_space_detector_unavailable();
    handle.displace(100, 300.0, 200.0);
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, handle.rect(100).unwrap());
    let writes = handle.writes();
    sm.refresh_space_gate_for_test();
    assert!(sm.display_may_act(1), "unavailable detector fails open");
    assert!(
        handle.writes() > writes,
        "the cleared gate resyncs the display it was suppressing"
    );
}

