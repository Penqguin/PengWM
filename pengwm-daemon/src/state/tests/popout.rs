use super::common::*;
use crate::adapter::WindowClass;
use pengwm_core::command::Command;
use pengwm_core::layout::Rect;
use pengwm_core::workspace::PendingKind;

#[test]
fn popout_toggles_the_os_focused_window_without_moving_it_on_exit() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(2);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.workspaces[0].focus_window(200);
    handle.set_focused_window(42, Some(100));
    let popped_rect = handle.rect(100).unwrap();
    let sibling_rect = handle.rect(200).unwrap();

    sm.on_command(Command::TogglePopout, None);

    assert!(!sm.workspaces[0].owns(100));
    assert!(sm.workspaces[0].find_window(200).is_some());
    assert_eq!(handle.rect(100), Some(popped_rect));
    assert_ne!(
        handle.rect(200),
        Some(sibling_rect),
        "the remaining tile reflows"
    );
    assert_eq!(handle.focused_pid_window(42), Some(100));

    let dragged = Rect::new(400.0, 300.0, 600.0, 400.0);
    handle.inject_rect(100, dragged);
    sm.on_window_moved(100, dragged);
    assert!(sm.drag.is_idle());
    let raised_before = handle.raised().len();
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(handle.rect(100), Some(dragged));
    assert_eq!(handle.raised()[raised_before..], [100]);

    sm.on_command(Command::TogglePopout, None);
    assert!(sm.workspaces[1].find_window(100).is_some());
    assert_ne!(handle.rect(100), Some(dragged));
    assert_eq!(handle.focused_pid_window(42), Some(100));
}

#[test]
fn popout_ignores_popups_untracked_windows_and_missing_os_focus() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.workspaces[0].focus_window(100);
    for (id, kind) in [(300, WindowClass::Dialog), (400, WindowClass::Untracked)] {
        handle.inject_window(42, id);
        handle.inject_window_kind(id, kind);
        handle.inject_rect(id, Rect::new(100.0, 100.0, 300.0, 300.0));
        sm.on_window_created(id, 42);
    }

    for focused in [Some(300), Some(400), None] {
        let writes_before = handle.writes();
        handle.set_focused_window(42, focused);
        sm.on_command(Command::TogglePopout, None);
        assert!(sm.workspaces[0].find_window(100).is_some());
        assert!(sm.workspaces[0].is_popup(300));
        assert!(!sm.workspaces[0].owns(400));
        assert_eq!(handle.writes(), writes_before);
    }
}

#[test]
fn popout_accepts_a_waiting_window_and_rejoin_travels_until_tiled() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    assert!(sm.workspaces[0].pending_window(200).is_some());
    handle.set_focused_window(42, Some(200));
    sm.workspaces[1].add_window(300, None);
    sm.workspaces[2].add_window(400, None);

    sm.on_command(Command::TogglePopout, None);
    assert!(!sm.workspaces[0].owns(200));
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(
        !sm.workspaces[1].owns(200),
        "a switch alone does not rejoin"
    );
    assert_eq!(handle.focused_pid_window(42), Some(200));

    sm.on_command(Command::TogglePopout, None);
    assert_eq!(
        sm.workspaces[1].pending_window(200).map(|p| p.kind),
        Some(PendingKind::Traveling)
    );
    assert_eq!(handle.focused_pid_window(42), Some(200));

    sm.on_command(Command::Workspace { id: 3 }, None);
    assert!(sm.workspaces[2].pending_window(200).is_some());
    sm.on_command(Command::Workspace { id: 4 }, None);
    assert!(sm.workspaces[3].find_window(200).is_some());
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert!(
        sm.workspaces[3].find_window(200).is_some(),
        "a tiled window stops traveling"
    );
}

#[test]
fn popout_stays_detached_across_hide_show_and_rediscovery_but_not_close() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_focused_window(42, Some(100));
    sm.on_command(Command::TogglePopout, None);
    handle.inject_hidden_window(100);
    sm.on_window_hidden(100);
    sm.on_window_created(100, 42);
    sm.on_app_launched(42);

    let raised_before = handle.raised().len();
    handle.set_focused_window(42, None);
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(
        !handle.raised()[raised_before..].contains(&100),
        "hidden window is not re-raised"
    );
    handle.clear_hidden_window(100);
    sm.on_window_shown(100);
    sm.on_window_created(100, 42);
    assert!(sm.workspaces.iter().all(|ws| !ws.owns(100)));

    sm.on_window_destroyed(100);
    sm.on_window_created(100, 42);
    assert!(
        sm.workspaces[1].find_window(100).is_some(),
        "a reused ID starts tiled"
    );
}

#[test]
fn popout_rejoins_the_visible_workspace_on_the_windows_display() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.on_window_created(100, 42);
    handle.set_focused_window(42, Some(100));
    sm.on_command(Command::TogglePopout, None);
    handle.inject_rect(100, Rect::new(2200.0, 300.0, 600.0, 400.0));
    assert_eq!(sm.displays.focused_output(), Some(1));

    sm.on_command(Command::TogglePopout, None);
    assert!(sm.workspaces[1].find_window(100).is_some());
    assert!(!sm.workspaces[0].owns(100));
    assert_eq!(handle.focused_pid_window(42), Some(100));
}

#[test]
fn popout_refuses_to_toggle_on_the_windows_non_first_native_desktop() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    handle.set_focused_window(42, Some(100));

    handle.set_first_desktop(1, false);
    sm.on_command(Command::TogglePopout, None);
    assert!(sm.workspaces[0].find_window(100).is_some());

    handle.set_first_desktop(1, true);
    sm.on_command(Command::TogglePopout, None);
    assert!(!sm.workspaces[0].owns(100));
    handle.set_first_desktop(1, false);
    sm.on_command(Command::TogglePopout, None);
    assert!(!sm.workspaces[0].owns(100), "rejoining is gated too");
}

#[test]
fn popout_workspace_switch_keeps_existing_untracked_focus_but_does_not_steal_it() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.workspaces[1].add_window(200, None);
    handle.inject_window(42, 300);
    handle.inject_window_kind(300, WindowClass::Untracked);
    handle.inject_rect(300, Rect::new(100.0, 100.0, 300.0, 300.0));
    sm.on_window_created(300, 42);
    handle.set_focused_window(42, Some(300));

    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(handle.raised().contains(&300));
    assert_eq!(handle.focused_pid_window(42), Some(300));

    handle.set_focused_window(42, Some(200));
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(handle.focused_pid_window(42), Some(100));
}

#[test]
fn popout_untracked_focus_does_not_override_a_cross_display_workspace_switch() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.workspaces[1].add_window(200, None);
    handle.inject_window(42, 300);
    handle.inject_window_kind(300, WindowClass::Untracked);
    handle.inject_rect(300, Rect::new(100.0, 100.0, 300.0, 300.0));
    sm.on_window_created(300, 42);
    handle.set_focused_window(42, Some(300));

    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(sm.displays.focused_output(), Some(2));
    assert_eq!(handle.focused_pid_window(42), Some(200));
}
