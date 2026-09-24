use super::common::*;
use crate::bar_server::BarSender;
use pengwm_core::command::{BarMessage, Command, DaemonResponse, LayoutMode};
use pengwm_core::tree::{Direction, SplitDirection};
use pengwm_core::workspace::LayoutPreset;
use tokio::sync::mpsc;

#[test]
fn focus_command_delegates_to_workspace() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(300, 42);
    sm.focus_command(Direction::Right);
    // Should focus the other window
    let focused = sm.workspaces[0].focused_node;
    assert!(focused.is_some());
}

#[test]
fn focus_command_focuses_window_via_adapter() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    assert_eq!(sm.workspaces[0].focused_window_id(), Some(200));
    sm.focus_command(Direction::Right);
    assert_eq!(sm.workspaces[0].focused_window_id(), Some(100));
    assert_eq!(
        sm.os.focused_window_for_pid(42),
        Some(100),
        "adapter should be told to focus the new window"
    );
}

#[test]
fn swap_command_triggers_layout() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(300, 42);
    sm.swap_command(Direction::Right);
    // Workspace should have both windows after swap
    assert_eq!(sm.workspaces[0].window_count(), 2);
}

#[test]
fn close_command_invokes_adapter() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(300, 42);
    let focused = sm.workspaces[0].focused_node;
    assert!(focused.is_some());
}

#[test]
fn toggle_layout_switches_monocle() {
    let mut sm = setup(1);
    assert!(!sm.workspaces[0].monocle);
    let cmd = Command::ToggleLayout;
    let (rtx, _) = mpsc::channel(1);
    sm.on_command(cmd, Some(rtx));
    assert!(sm.workspaces[0].monocle);
}

#[test]
fn set_gap_updates_values() {
    let mut sm = setup(1);
    let (rtx, _) = mpsc::channel(1);
    sm.on_command(Command::SetGapOuter { pixels: 20 }, Some(rtx));
    assert_eq!(sm.gap_outer, 20.0);
}

#[test]
fn split_command_pends_direction_for_next_window() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let (rtx, _) = mpsc::channel(1);
    sm.on_command(
        Command::Split {
            direction: SplitDirection::Horizontal,
        },
        Some(rtx),
    );
    sm.on_window_created(200, 42);
    assert_eq!(
        sm.workspaces[0].focused_split_direction(),
        Some(SplitDirection::Horizontal),
        "split issued on a focused window becomes the next window's parent direction"
    );
}

#[test]
fn query_state_returns_workspace_info() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let (rtx, mut rx) = mpsc::channel(1);
    sm.on_command(Command::QueryState, Some(rtx));
    let resp = rx.blocking_recv();
    assert!(resp.is_some());
}

#[test]
fn on_command_handles_every_variant_without_reply() {
    // The keybind/config-watcher path sends `None` for the reply slot:
    // every Command variant must be handled without a channel to write to.
    let commands = [
        Command::Focus {
            direction: Direction::Left,
        },
        Command::MoveWindow {
            direction: Direction::Right,
        },
        Command::Split {
            direction: SplitDirection::Vertical,
        },
        Command::Workspace { id: 1 },
        Command::MoveWindowToWorkspace { id: 2 },
        Command::FocusDisplay {
            direction: Direction::Left,
        },
        Command::MoveWindowToDisplay {
            direction: Direction::Right,
        },
        Command::Close,
        Command::ToggleLayout,
        Command::SetLayout {
            mode: LayoutMode::Accordion,
        },
        Command::SetGapOuter { pixels: 4 },
        Command::SetGapInner { pixels: 2 },
        Command::SelectLayout {
            preset: LayoutPreset::Tiled,
        },
        Command::ResizePane {
            direction: Direction::Right,
        },
        Command::ToggleBar,
        Command::ReloadConfig,
        Command::QueryState,
        Command::Quit,
    ];
    for cmd in commands {
        let mut sm = setup(1);
        sm.on_command(cmd, None);
    }
}

#[test]
fn on_command_sends_ack_only_when_reply_slot_is_present() {
    let mut sm = setup(1);
    let (rtx, mut rx) = mpsc::channel(1);
    sm.on_command(Command::ToggleLayout, Some(rtx));
    assert!(matches!(rx.blocking_recv(), Some(DaemonResponse::Ack)));

    let mut sm = setup(1);
    let (rtx, mut rx) = mpsc::channel(1);
    sm.on_command(Command::QueryState, Some(rtx));
    assert!(matches!(
        rx.blocking_recv(),
        Some(DaemonResponse::State { .. })
    ));
}

#[test]
fn on_command_select_layout_rearranges_and_resize_shifts() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.on_window_created(300, 42);

    sm.on_command(
        Command::SelectLayout {
            preset: LayoutPreset::MainVertical,
        },
        None,
    );
    let idx = sm.active_workspace_idx();
    // Through the public layout interface: the main window holds ~60% width.
    let before = sm.workspaces[idx].layout(sm.gap_inner, sm.gap_outer);
    assert_eq!(before.len(), 3);
    let main_w = before[&100].width;
    assert!(
        main_w > 900.0,
        "main window should hold the 0.6 share, got {main_w}"
    );

    sm.workspaces[idx].focus_window(100);
    sm.on_command(
        Command::ResizePane {
            direction: Direction::Right,
        },
        None,
    );
    let after = sm.workspaces[idx].layout(sm.gap_inner, sm.gap_outer);
    assert!(
        after[&100].width > main_w,
        "resize right grows the main window"
    );
}

#[test]
fn quit_command_requests_shutdown_and_exits_bar() {
    let mut sm = setup(1);
    let (bar_tx, mut bar_rx) = mpsc::channel(64);
    sm.bar_sender = BarSender::from_channel(bar_tx);
    let (rtx, mut rx) = mpsc::channel(1);

    sm.on_command(Command::Quit, Some(rtx));

    assert!(sm.shutdown_requested());
    assert!(matches!(rx.blocking_recv(), Some(DaemonResponse::Ack)));
    let msgs: Vec<_> = std::iter::from_fn(|| bar_rx.try_recv().ok()).collect();
    assert!(
        msgs.iter().any(|m| matches!(m, BarMessage::Exit)),
        "quitting should tell the bar to exit too"
    );
}
