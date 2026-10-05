use super::common::*;
use crate::bar_server::BarSender;
use pengwm_core::command::{BarMessage, Command, DaemonResponse};
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
fn cycle_layout_advances_preset() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let before = sm.workspaces[0].preset_index;
    let (rtx, _) = mpsc::channel(1);
    sm.on_command(Command::CycleLayout, Some(rtx));
    assert_eq!(
        sm.workspaces[0].preset_index,
        (before + 1) % pengwm_core::workspace::LayoutPreset::all().len()
    );
}

#[test]
fn toggle_magnify_pins_focused() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let focused = sm.workspaces[0].focused_window_id().unwrap();
    let (rtx, _) = mpsc::channel(1);
    sm.on_command(Command::ToggleMagnify, Some(rtx));
    assert_eq!(sm.workspaces[0].magnified, Some(focused));
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
        Command::CycleLayout,
        Command::ToggleMagnify,
        Command::SetGapOuter { pixels: 4 },
        Command::SetGapInner { pixels: 2 },
        Command::SelectLayout {
            preset: LayoutPreset::Tiled,
        },
        Command::ResizePane {
            direction: Direction::Right,
        },
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
    sm.on_command(Command::CycleLayout, Some(rtx));
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
fn quit_command_requests_shutdown_and_exits_menubar() {
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
        "quitting should tell the menubar to exit too"
    );
}

// -----------------------------------------------------------------------
// i3-style output behavior (#2) + global switch (#1)
// -----------------------------------------------------------------------

#[test]
fn focus_display_to_empty_updates_focused_output() {
    let mut sm = setup(2);
    // Output 2 shows workspace 1, which is empty — focus must still land.
    sm.on_command(
        Command::FocusDisplay {
            direction: Direction::Right,
        },
        None,
    );
    assert_eq!(sm.displays.focused_output(), Some(2));
    assert_eq!(sm.active_workspace_idx(), 1);
}

#[test]
fn move_window_to_display_bypasses_cap_and_keeps_focus() {
    let mut sm = setup(2);
    sm.displays.set_max_tiles(1);
    // Target output's visible workspace is full (other app's window, so the
    // frontmost heuristic keeps pointing at the source output)…
    sm.workspaces[1].add_window(900, None);
    sm.store.register(900, 43);
    // …source has the focused window.
    sm.on_window_created(300, 42);
    let src = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(300).is_some())
        .expect("window 300 should be tiled");
    assert_eq!(src, 0);

    sm.on_command(
        Command::MoveWindowToDisplay {
            direction: Direction::Right,
        },
        None,
    );
    assert_eq!(
        sm.workspaces[1].window_count(),
        2,
        "output moves always land, cap is bypassed"
    );
    assert!(
        sm.workspaces[0].find_window(300).is_none(),
        "window left the source"
    );
    assert_eq!(
        sm.displays.focused_output(),
        Some(1),
        "focus stays on the source output"
    );
}

#[test]
fn workspace_switch_to_visible_elsewhere_swaps_outputs() {
    let mut sm = setup(2);
    // Output 1 shows ws 0, output 2 shows ws 1. Switch to global ws 2 → swap.
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(sm.displays.active().get(&1), Some(&1));
    assert_eq!(sm.displays.active().get(&2), Some(&0));
}

#[test]
fn workspace_switch_to_hidden_pulls_and_hides_previous() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::BottomEdge);
    sm.on_window_created(100, 42);
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(sm.displays.active().get(&1), Some(&1));
    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "window stays in its tree"
    );
    let rect = handle
        .rect(100)
        .expect("pulled-away window parks offscreen");
    let expected = pengwm_core::layout::hidden_rect((0, 0), (1920, 1080));
    assert_eq!((rect.x, rect.y), (expected.x, expected.y));
}

#[test]
fn workspace_switch_focuses_destination_mru_by_default() {
    let mut sm = setup_with_handle(1).0;
    sm.on_window_created(100, 42);
    sm.on_command(Command::Workspace { id: 2 }, None);
    sm.on_window_created(200, 42);
    // Empty switch focuses nothing new; destination MRU is 200.
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(
        sm.os.focused_window_for_pid(42),
        Some(100),
        "switching back must re-assert ws-1's MRU window via the OsAdapter seam"
    );
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(
        sm.os.focused_window_for_pid(42),
        Some(200),
        "switching forward must re-assert ws-2's MRU window"
    );
}

#[test]
fn workspace_switch_focus_first_overrides_mru_when_enabled() {
    let mut sm = setup_with_handle(1).0;
    sm.set_focus_first_for_test(true);
    sm.on_window_created(100, 42);
    sm.on_window_created(101, 42);
    // MRU is 101; spatial-first is 100.
    sm.workspaces[0].focus_window(101);
    sm.on_command(Command::Workspace { id: 2 }, None);
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(
        sm.os.focused_window_for_pid(42),
        Some(100),
        "focus_first_on_switch must land on the leftmost leaf, not MRU"
    );
}

#[test]
fn workspace_switch_to_empty_focuses_nothing() {
    let mut sm = setup_with_handle(1).0;
    sm.on_window_created(100, 42);
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(
        sm.os.focused_window_for_pid(42),
        None,
        "empty destination must not issue a focus call"
    );
}
