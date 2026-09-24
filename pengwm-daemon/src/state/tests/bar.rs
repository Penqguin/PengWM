use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::super::StateManager;
use super::common::*;
use crate::bar_server::BarSender;
use crate::config::keybinds::KeybindConfig;
use crate::config::BarConfig;
use pengwm_core::command::{BarMessage, Command};
use pengwm_core::config::BarPosition;
use pengwm_core::tree::SplitDirection;

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
        test_prefix(),
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
        test_prefix(),
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
