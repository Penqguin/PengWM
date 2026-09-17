use super::*;
use crate::adapter::DisplayInfo;
use crate::adapter_test::TestAdapter;
use crate::config::keybinds::KeybindConfig;
use crate::config::{BarConfig, WorkspaceEntry};
use pengwm_core::command::{Command, DaemonResponse, LayoutMode};
use pengwm_core::config::BarPosition;
use pengwm_core::tree::Direction;
use pengwm_core::tree::SplitDirection;

fn make_adapter(display_count: u32) -> TestAdapter {
    let mut adapter = TestAdapter::new();
    if display_count == 1 {
        adapter.displays = vec![DisplayInfo {
            id: 1,
            origin: (0, 0),
            size: (1920, 1080),
        }];
    }
    if display_count == 2 {
        adapter.displays = vec![
            DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (1920, 1080),
            },
            DisplayInfo {
                id: 2,
                origin: (1920, 0),
                size: (1920, 1080),
            },
        ];
    }
    adapter.frontmost = Some(42);
    adapter.running_apps = vec![42];
    adapter
        .windows
        .borrow_mut()
        .entry(42)
        .or_default()
        .extend(vec![100, 200]);
    adapter.window_pids.borrow_mut().insert(100, 42);
    adapter.window_pids.borrow_mut().insert(200, 42);
    adapter
}

fn setup(display_count: u32) -> StateManager {
    let (tx, _) = mpsc::channel(64);
    let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
    let adapter = make_adapter(display_count);
    let (bar_tx, _) = mpsc::channel(64);
    StateManager::new(
        tx,
        keybinds,
        Box::new(adapter),
        BarSender::from_channel(bar_tx),
        None,
        vec![],
    )
}

#[test]
fn creates_workspaces_from_displays() {
    let sm = setup(1);
    // The five default named workspaces, all on the primary monitor.
    let names: Vec<&str> = sm.workspaces.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Development", "Browsing", "Notes", "Music", "Messaging"]
    );
    assert!(sm.workspaces.iter().all(|w| w.monitor_id == 1));
}

#[test]
fn creates_workspace_set_per_display() {
    let sm = setup(2);
    assert_eq!(sm.workspaces.len(), 10);
    assert!(sm.workspaces[..5].iter().all(|w| w.monitor_id == 1));
    assert!(sm.workspaces[5..].iter().all(|w| w.monitor_id == 2));
    assert_eq!(sm.displays.active().get(&1), Some(&0));
    assert_eq!(sm.displays.active().get(&2), Some(&5));
}

#[test]
fn tracks_existing_windows_at_init() {
    let sm = setup(1);
    // tracked in pid maps
    assert_eq!(sm.store.len(), 2);
    assert_eq!(sm.store.all_window_pids().get(&100), Some(&42));
    assert_eq!(sm.store.all_window_pids().get(&200), Some(&42));
    // not yet added to workspace tree (event loop hasn't consumed init events)
    assert_eq!(sm.workspaces[0].window_count(), 0);
}

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
    assert_eq!(sm.store.get(100), Some(0));
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
fn move_focused_to_workspace_moves_window() {
    let mut sm = setup(2);
    // 5 named workspaces per display.
    assert_eq!(sm.workspaces.len(), 10);

    // Add two windows — they go to whatever workspace active_workspace_idx() picks
    sm.on_window_created(100, 42);
    sm.on_window_created(300, 42);

    // Find which workspace received them (avoids HashMap ordering assumptions)
    let source = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(100).is_some())
        .expect("window 100 should be in some workspace");
    assert_eq!(sm.workspaces[source].window_count(), 2);

    let target = if source == 0 { 1 } else { 0 };

    sm.move_focused_to_workspace(target);

    assert_eq!(sm.workspaces[source].window_count(), 1);
    assert_eq!(sm.workspaces[target].window_count(), 1);
}

#[test]
fn created_window_overflows_to_next_workspace() {
    let mut sm = setup(2);
    sm.displays.set_max_tiles(2);
    sm.workspaces[0].add_window(100, None);
    sm.workspaces[0].add_window(200, None);
    sm.workspaces[1].add_window(300, None);

    sm.on_window_created(400, 42);

    assert!(sm.workspaces[0].find_window(400).is_none());
    assert!(
        sm.workspaces[1].find_window(400).is_some(),
        "full ws-0 should overflow into ws-1"
    );
    assert_eq!(sm.workspaces[0].window_count(), 2);
    assert_eq!(sm.workspaces[1].window_count(), 2);
}

#[test]
fn created_window_all_workspaces_full_stays_untracked() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(2);
    for (i, ws) in sm.workspaces.iter_mut().enumerate() {
        ws.add_window((1000 + i * 2) as WindowId, None);
        ws.add_window((1001 + i * 2) as WindowId, None);
    }

    sm.on_window_created(400, 42);

    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(400).is_none()),
        "no workspace has room, so the window stays untracked"
    );
    assert_eq!(
        sm.store.all_window_pids().get(&400),
        Some(&42),
        "still pid-tracked"
    );
}

#[test]
fn created_window_overflow_wraps_to_first_workspace() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(2);
    // Fill the active workspace (1) and every one after it, leaving only
    // the first workspace with room, so overflow wraps around to ws-0.
    sm.workspaces[1].add_window(300, None);
    sm.workspaces[1].add_window(500, None);
    sm.workspaces[2].add_window(301, None);
    sm.workspaces[2].add_window(501, None);
    sm.workspaces[3].add_window(302, None);
    sm.workspaces[3].add_window(502, None);
    sm.workspaces[4].add_window(303, None);
    sm.workspaces[4].add_window(503, None);
    sm.displays.active_mut().insert(1, 1);
    sm.store.set_windows_for_pid(42, vec![300]);

    sm.on_window_created(400, 42);

    assert_eq!(sm.active_workspace_idx(), 1, "ws-1 should be active");
    assert!(
        sm.workspaces[0].find_window(400).is_some(),
        "full ws-1 should wrap around into ws-0"
    );
}

#[test]
fn move_to_full_workspace_redirects_to_next_with_room() {
    let mut sm = setup(2);
    sm.displays.set_max_tiles(2);
    sm.workspaces
        .push(Workspace::new("ws-3".into(), 3, (3840, 0), (1920, 1080)));
    sm.displays.active_mut().insert(3, 2);
    sm.workspaces[0].add_window(100, None);
    sm.workspaces[1].add_window(300, None);
    sm.workspaces[1].add_window(500, None);
    sm.store.set_windows_for_pid(42, vec![100]);
    sm.on_window_focused(100);

    sm.move_focused_to_workspace(1);

    assert_eq!(sm.workspaces[0].window_count(), 0);
    assert_eq!(sm.workspaces[1].window_count(), 2);
    assert!(
        sm.workspaces[2].find_window(100).is_some(),
        "move to full ws-1 should land in ws-3"
    );
}

#[test]
fn move_to_full_workspace_aborts_when_no_room_anywhere() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(2);
    for (i, ws) in sm.workspaces.iter_mut().enumerate() {
        ws.add_window((1000 + i * 2) as WindowId, None);
        ws.add_window((1001 + i * 2) as WindowId, None);
    }
    sm.store.set_windows_for_pid(42, vec![1000]);
    sm.on_window_focused(1000);

    sm.move_focused_to_workspace(1);

    assert!(
        sm.workspaces[0].find_window(1000).is_some(),
        "window should stay put when all workspaces are full"
    );
    assert!(sm.workspaces.iter().all(|ws| ws.window_count() == 2));
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
fn bar_reserved_rect_top_strip_on_primary_display() {
    let mut sm = setup(1);
    *sm.bar.config_mut() = BarConfig {
        position: BarPosition::Top,
        thickness: 24,
        visible: true,
        enabled: true,
        ..Default::default()
    };
    sm.bar.set_spawned(true);
    sm.bar.set_visible(true);
    let rect = sm.bar_reserved_rect().unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (0.0, 0.0, 1920.0, 24.0)
    );
}

#[test]
fn bar_reserved_rect_bottom_and_right() {
    let mut sm = setup(1);
    sm.bar.set_spawned(true);
    sm.bar.set_visible(true);
    *sm.bar.config_mut() = BarConfig {
        position: BarPosition::Bottom,
        thickness: 30,
        visible: true,
        enabled: true,
        ..Default::default()
    };
    let rect = sm.bar_reserved_rect().unwrap();
    assert_eq!((rect.x, rect.y), (0.0, 1080.0 - 30.0));
    assert_eq!((rect.width, rect.height), (1920.0, 30.0));

    sm.bar.config_mut().position = BarPosition::Right;
    sm.bar.config_mut().thickness = 40;
    let rect = sm.bar_reserved_rect().unwrap();
    assert_eq!((rect.x, rect.y), (1920.0 - 40.0, 0.0));
    assert_eq!((rect.width, rect.height), (40.0, 1080.0));
}

#[test]
fn bar_reserved_rect_none_when_hidden() {
    let mut sm = setup(1);
    sm.bar.set_visible(false);
    assert_eq!(sm.bar_reserved_rect(), None);
}

#[test]
fn bar_reserved_rect_none_when_not_spawned() {
    let mut sm = setup(1);
    sm.bar.set_visible(true);
    sm.bar.set_spawned(false);
    assert_eq!(sm.bar_reserved_rect(), None);
}

#[test]
fn apply_bar_reservation_reserves_primary_workspace() {
    let mut sm = setup(2);
    sm.bar.set_spawned(true);
    sm.bar.set_visible(true);
    *sm.bar.config_mut() = BarConfig {
        position: BarPosition::Top,
        thickness: 20,
        visible: true,
        enabled: true,
        ..Default::default()
    };
    sm.apply_bar_reservation();
    // Display 1 (primary) owns workspaces 0-4, display 2 owns 5-9.
    assert!(
        sm.workspaces[0].reserved_rect().is_some(),
        "primary monitor workspace should be reserved"
    );
    assert!(
        sm.workspaces[5].reserved_rect().is_none(),
        "secondary monitor workspace should not be reserved"
    );
    sm.bar.set_visible(false);
    sm.apply_bar_reservation();
    assert!(sm.workspaces[0].reserved_rect().is_none());
}

#[test]
fn toggle_bar_command_flips_visibility_and_reservation() {
    let mut sm = setup(1);
    sm.bar.set_spawned(true);
    let was_visible = sm.bar.is_visible();
    let (rtx, _) = mpsc::channel(1);
    sm.on_command(Command::ToggleBar, Some(rtx));
    assert_ne!(sm.bar.is_visible(), was_visible);
    // Reservations match the new visibility.
    assert_eq!(sm.bar_reserved_rect().is_some(), sm.bar.is_visible());
}

#[test]
fn publish_bar_state_reports_active_workspace_and_split() {
    let (tx, _) = mpsc::channel(64);
    let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
    let mut adapter = make_adapter(1);
    adapter.frontmost = Some(42);
    let (bar_tx, mut bar_rx) = mpsc::channel(64);
    let mut sm = StateManager::new(
        tx,
        keybinds,
        Box::new(adapter),
        BarSender::from_channel(bar_tx),
        None,
        vec![],
    );

    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // Drain the startup + creation publishes, keep the latest.
    let mut last: Option<BarMessage> = None;
    while let Ok(msg) = bar_rx.try_recv() {
        last = Some(msg);
    }
    let state = match last {
        Some(BarMessage::State(s)) => s,
        other => panic!("expected a State publish, got {:?}", other),
    };
    assert_eq!(state.workspaces.len(), 5);
    assert_eq!(state.workspaces[0].window_count, 2);
    assert!(state.workspaces[0].active);
    assert_eq!(
        state.split_direction,
        Some(SplitDirection::Vertical),
        "two windows on a widescreen monitor split vertically"
    );
}

#[test]
fn publish_bar_state_populates_window_app_names() {
    let (tx, _) = mpsc::channel(64);
    let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
    let adapter = make_adapter(1);
    adapter.app_names.borrow_mut().insert(42, "Safari".into());
    adapter
        .bundle_ids
        .borrow_mut()
        .insert(42, "com.apple.Safari".into());
    let (bar_tx, mut bar_rx) = mpsc::channel(64);
    let mut sm = StateManager::new(
        tx,
        keybinds,
        Box::new(adapter),
        BarSender::from_channel(bar_tx),
        None,
        vec![],
    );

    sm.on_window_created(100, 42);
    sm.on_window_created(300, 43);
    // 43 has no display name and no bundle id -> falls back to "unknown".
    // 42 is Safari, which the default routing sends to the Browsing
    // workspace (index 1); 43's window lands in the active workspace (0).
    let mut last: Option<BarMessage> = None;
    while let Ok(msg) = bar_rx.try_recv() {
        last = Some(msg);
    }
    let state = match last {
        Some(BarMessage::State(s)) => s,
        other => panic!("expected a State publish, got {:?}", other),
    };
    assert_eq!(state.workspaces[0].windows, vec!["unknown"]);
    assert_eq!(state.workspaces[1].windows, vec!["Safari"]);
}

#[test]
fn on_window_created_routes_configured_app_to_its_workspace() {
    let mut sm = setup(1);
    sm.os.inject_bundle_id(77, "com.google.Chrome".into());
    sm.os.inject_app_name(77, "Chrome".into());

    sm.on_window_created(777, 77);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(777).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Browsing");
}

#[test]
fn on_window_created_routing_matches_app_name_case_insensitively() {
    let mut sm = setup(1);
    sm.os.inject_app_name(88, "spotify".into());

    sm.on_window_created(888, 88);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(888).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Music");
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

#[test]
fn workspace_switch_hides_all_other_workspaces_on_monitor() {
    let mut sm = setup(1);
    // Route Firefox to Browsing (ws-1) then switch back to Development (ws-0).
    sm.os.inject_bundle_id(77, "org.mozilla.firefox".into());
    sm.on_window_created(777, 77);
    let browsing_idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.name == "Browsing")
        .unwrap();
    assert!(sm.workspaces[browsing_idx].find_window(777).is_some());

    // Switch to Development via command.
    sm.on_command(Command::Workspace { id: 1 }, None);
    let dev_idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.name == "Development")
        .unwrap();
    assert_eq!(sm.displays.active().get(&1), Some(&dev_idx));
    // Browsing windows should still be only in Browsing, not dragged to Dev.
    assert!(sm.workspaces[dev_idx].find_window(777).is_none());
    assert!(sm.workspaces[browsing_idx].find_window(777).is_some());
    assert_eq!(sm.workspaces[dev_idx].window_count(), 0);
}

#[test]
fn workspace_switch_debounces_stale_focus() {
    let mut sm = setup(1);
    sm.os.inject_bundle_id(77, "org.mozilla.firefox".into());
    sm.on_window_created(777, 77);
    let browsing_idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.name == "Browsing")
        .unwrap();
    let dev_idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.name == "Development")
        .unwrap();
    // Start on browsing, then switch to dev — sets debounce.
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(sm.displays.active().get(&1), Some(&browsing_idx));
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(sm.displays.active().get(&1), Some(&dev_idx));
    // Stale focus for the firefox window that was just hidden should not flip active back.
    sm.on_window_focused(777);
    assert_eq!(
        sm.displays.active().get(&1),
        Some(&dev_idx),
        "debounced focus should not drag active back to browsing"
    );
}

#[test]
fn per_monitor_workspace_entries_respected() {
    let mut ds = crate::state::display::DisplaySet::new(vec![
        WorkspaceEntry {
            name: "Dev".into(),
            apps: vec![],
            monitor: Some(crate::config::MonitorRef::Index(1)),
            autostart: vec![],
        },
        WorkspaceEntry {
            name: "Browse".into(),
            apps: vec![],
            monitor: None,
            autostart: vec![],
        },
    ]);
    let mut wss = Vec::new();
    ds.init_workspaces(
        &mut wss,
        &[
            DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (1920, 1080),
            },
            DisplayInfo {
                id: 2,
                origin: (1920, 0),
                size: (1920, 1080),
            },
        ],
    );
    // Dev only on monitor 1, Browse on both.
    assert_eq!(wss.len(), 3);
    assert_eq!(wss[0].name, "Dev");
    assert_eq!(wss[0].monitor_id, 1);
    assert_eq!(wss[1].name, "Browse");
    assert_eq!(wss[1].monitor_id, 1);
    assert_eq!(wss[2].name, "Browse");
    assert_eq!(wss[2].monitor_id, 2);
}

#[test]
fn hide_workspace_uses_bottom_edge_rect() {
    let mut sm = setup(1);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::BottomEdge);
    sm.on_window_created(100, 42);
    assert!(sm.workspaces[0].find_window(100).is_some());
    // Switch to a different workspace so ws-0 gets hidden
    sm.on_command(Command::Workspace { id: 2 }, None);
    let expected = pengwm_core::layout::hidden_rect((0, 0), (1920, 1080));
    let rect = sm
        .os
        .window_rect_for_test(100)
        .expect("hidden window rect should exist");
    assert_eq!(
        rect, expected,
        "hidden window should be at bottom-right clamped rect"
    );
}

#[test]
fn hide_workspace_far_offscreen_when_configured() {
    let mut sm = setup(1);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::FarOffscreen);
    sm.on_window_created(100, 42);
    sm.on_command(Command::Workspace { id: 2 }, None);
    let expected = pengwm_core::layout::far_offscreen_rect();
    let rect = sm
        .os
        .window_rect_for_test(100)
        .expect("hidden window rect should exist");
    assert_eq!(rect, expected);
}

#[test]
fn hide_workspace_per_monitor_second_display() {
    let mut sm = setup(2);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::BottomEdge);
    // Window routed to display 2 via manual add
    sm.workspaces[5].add_window(999, None);
    sm.store.register(999, 42);
    // Hide workspace 5 (on display 2 origin 1920,0)
    sm.displays.active_mut().insert(2, 6);
    // Hide the former visible on display 2
    sm.hide_workspace(5);
    let expected = pengwm_core::layout::hidden_rect((1920, 0), (1920, 1080));
    let rect = sm
        .os
        .window_rect_for_test(999)
        .expect("hidden window rect should exist");
    assert_eq!(
        rect, expected,
        "display-2 window should hide at its own monitor corner"
    );
}

#[test]
fn monocle_sibling_stays_far_offscreen() {
    let mut ws = Workspace::new("test".into(), 1, (0, 0), (1920, 1080));
    ws.add_window(100, None);
    ws.add_window(200, None);
    ws.monocle = true;
    ws.focus_window(100);
    let rects = ws.layout(5.0, 10.0);
    let off = pengwm_core::layout::far_offscreen_rect();
    assert_eq!(
        rects[&200], off,
        "monocle sibling must remain far offscreen, not bottom-edge"
    );
    assert_ne!(rects[&100], off);
}

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
    assert!(sm.store.hidden_is_empty());
}

#[test]
fn reveal_all_via_hidden_drain_is_idempotent() {
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_hidden(100);
    sm.reveal_all();
    assert!(sm.store.hidden_is_empty());
    sm.reveal_all();
    assert!(sm.store.hidden_is_empty());
}

#[test]
fn missed_window_created_sweep_tiles_new_window() {
    // Firefox tear-off / incognito: OS has the window but no
    // WindowCreated event was ever delivered (missing AX notification
    // or transient non-manageable subrole at creation time). The
    // background sweep must discover it via poll and tile it.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // OS-side only: adapter knows 300, StateManager does not.
    sm.os.inject_window(42, 300);
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
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.os.inject_window(42, 300);
    sm.on_app_activated(42);
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(300).is_some()),
        "activation poll should tile the missed window without waiting for sweep"
    );
}

#[test]
fn apply_layout_skips_windows_already_at_target() {
    // Redundant layouts must not touch the AX API: Firefox reflows on
    // every write and crawls under the repeat storm.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let writes_after_tile = sm.os.set_rect_calls_for_test();
    assert!(writes_after_tile >= 2, "tiling should write each window");

    // Same layout again (focus change, tick, publish path) → no writes.
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        sm.os.set_rect_calls_for_test(),
        writes_after_tile,
        "identical layout must not rewrite windows"
    );

    // A real change (new window) writes again.
    sm.on_window_created(300, 42);
    assert!(
        sm.os.set_rect_calls_for_test() > writes_after_tile,
        "changed layout must write"
    );
}

#[test]
fn moved_window_is_reasserted_on_next_layout() {
    // A displaced window (user drag, app move) must be rewritten, not
    // skipped — otherwise drag snap-back silently stops working.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let writes_after_tile = sm.os.set_rect_calls_for_test();
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        sm.os.set_rect_calls_for_test(),
        writes_after_tile,
        "identical layout must not rewrite"
    );
    // Displace past the grace window so it reads as external, not as our
    // own animation settling.
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        sm.os.set_rect_calls_for_test() > writes_after_tile,
        "displaced window must be re-asserted"
    );
}

#[test]
fn move_within_grace_window_stays_skipped() {
    // Moves arriving right after our own write are animation settle, not
    // a drag — invalidating on them would restart the Firefox storm.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let writes_after_tile = sm.os.set_rect_calls_for_test();
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        sm.os.set_rect_calls_for_test(),
        writes_after_tile,
        "settle moves inside the grace window must not force a rewrite"
    );
}

#[test]
fn hide_then_switch_back_retiles_instead_of_skipping() {
    // The dangerous direction for skip-if-unchanged: a window hidden on
    // switch-away must come back on switch-back, not compare equal to a
    // stale tiled entry.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let tiled = sm.os.window_rect_for_test(100).expect("tiled rect");
    sm.on_command(Command::Workspace { id: 2 }, None);
    let hidden = sm.os.window_rect_for_test(100).expect("hidden rect");
    assert_ne!(tiled, hidden, "switch-away should hide the window");
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(
        sm.os.window_rect_for_test(100),
        Some(tiled),
        "switch-back must retile, not skip as unchanged"
    );
}
