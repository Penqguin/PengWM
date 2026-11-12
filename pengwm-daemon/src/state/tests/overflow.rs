use super::common::*;
use crate::adapter::{LiveWindowIdentity, WindowClass};
use crate::config::NewWindowPlacement;
use pengwm_core::command::Command;
use pengwm_core::tree::Direction;
use pengwm_core::workspace::PendingKind;

#[test]
fn fresh_configured_app_uses_current_workspace_by_default() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(77, "com.apple.Safari".into());
    sm.on_command(Command::Workspace { id: 3 }, None);
    sm.on_window_created(777, 77);
    assert!(sm.workspaces[2].find_window(777).is_some());
    assert_eq!(
        sm.new_window_placement,
        NewWindowPlacement::CurrentWorkspace
    );
}

#[test]
fn overflow_carries_until_it_tiles_then_stays_home() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200, 300] {
        sm.on_window_created(id, 42);
    }
    assert_eq!(sm.workspaces[0].pending.len(), 2);
    assert_eq!(handle.last_focused(), Some(300));
    sm.workspaces[1].add_window(900, None);
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert_eq!(sm.workspaces[1].pending.len(), 2);
    assert!(!sm.workspaces[0].owns(200));
    sm.on_command(Command::Workspace { id: 3 }, None);
    assert!(sm.workspaces[2].find_window(200).is_some());
    assert!(sm.workspaces[2].pending_window(300).is_some());
    sm.on_command(Command::Workspace { id: 4 }, None);
    assert!(sm.workspaces[2].find_window(200).is_some());
    assert!(sm.workspaces[3].find_window(300).is_some());
    sm.assert_identity_invariant();
}

#[test]
fn closing_a_tile_fills_oldest_slot_without_switching() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200, 300] {
        sm.on_window_created(id, 42);
    }
    sm.on_window_destroyed(100);
    assert!(sm.workspaces[0].find_window(200).is_some());
    assert!(sm.workspaces[0].pending_window(300).is_some());
    sm.on_window_created(300, 42);
    assert_eq!(sm.workspaces[0].pending.len(), 1);
}

#[test]
fn settled_return_wins_slot_and_keeps_home() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(1);
    sm.on_window_created(100, 42);
    sm.on_window_hidden(100);
    sm.on_window_created(200, 42);
    sm.on_window_created(300, 42);
    sm.on_window_shown(100);
    assert_eq!(
        sm.workspaces[0].pending_window(100).unwrap().kind,
        PendingKind::Returning
    );
    sm.on_window_destroyed(200);
    assert!(sm.workspaces[0].find_window(100).is_some());
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(sm.workspaces[0].find_window(100).is_some());
    assert!(sm.workspaces[1].find_window(300).is_some());
}

#[test]
fn minimized_traveling_window_pauses_carry_and_resumes_at_origin() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.on_window_hidden(200);
    sm.on_command(Command::Workspace { id: 2 }, None);
    sm.on_window_shown(200);
    assert!(sm.workspaces[0].pending_window(200).is_some());
    assert!(!sm.workspaces[1].owns(200));
    sm.on_command(Command::Workspace { id: 1 }, None);
    sm.on_command(Command::Workspace { id: 3 }, None);
    assert!(sm.workspaces[2].find_window(200).is_some());
}

#[test]
fn monitor_focus_leaves_overflow_but_explicit_workspace_request_carries() {
    let mut sm = setup(2);
    sm.displays.set_max_tiles(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.on_command(
        Command::FocusDisplay {
            direction: Direction::Right,
        },
        None,
    );
    assert!(sm.workspaces[0].pending_window(200).is_some());
    sm.on_command(
        Command::FocusDisplay {
            direction: Direction::Left,
        },
        None,
    );
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(sm.workspaces[1].find_window(200).is_some());
    assert_eq!(sm.displays.focused_output(), Some(2));
}

#[test]
fn cycle_close_and_explicit_move_work_for_overlays() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200, 300] {
        sm.on_window_created(id, 42);
    }
    sm.on_command(Command::CycleWindow { backward: false }, None);
    assert_eq!(handle.last_focused(), Some(100));
    sm.on_command(Command::CycleWindow { backward: true }, None);
    assert_eq!(handle.last_focused(), Some(300));
    sm.workspaces[1].add_window(900, None);
    sm.on_command(Command::MoveWindowToWorkspace { id: 2 }, None);
    assert_eq!(
        sm.workspaces[1].pending_window(300).unwrap().kind,
        PendingKind::Explicit
    );
    sm.on_command(Command::Workspace { id: 2 }, None);
    sm.on_command(Command::Workspace { id: 3 }, None);
    assert!(sm.workspaces[1].pending_window(300).is_some());
    assert!(sm.workspaces[2].find_window(200).is_some());
}

#[test]
fn native_desktop_gate_defers_tiling_and_carry() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.set_first_desktop(1, false);
    sm.refresh_space_gate(std::time::Instant::now(), true);
    sm.on_window_created(300, 42);
    assert!(sm.workspaces[0].pending_window(300).is_some());
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(sm.workspaces[0].pending_window(300).is_some());
    handle.set_first_desktop(1, true);
    sm.refresh_space_gate(std::time::Instant::now(), true);
    assert!(sm.workspaces[0].find_window(300).is_some());
}

fn identity(id: u64) -> LiveWindowIdentity {
    LiveWindowIdentity {
        boot: "boot-a".into(),
        process_start: (123, 45),
        pid: 42,
        window_id: id,
        accessibility_id: format!("window-{id}"),
    }
}

#[test]
fn restart_roundtrip_keeps_more_than_eight_windows_and_overflow_order() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(2);
    for id in 100..112 {
        handle.inject_window(42, id);
        handle.inject_window_kind(id, WindowClass::Standard);
        handle.inject_identity(identity(id));
        sm.on_window_created(id, 42);
    }
    sm.on_window_hidden(103);
    let path = sm.session_path.clone();
    sm.save_session(&path).unwrap();
    let session = crate::state::session::load_from(&path).unwrap();
    assert_eq!(session.live_windows.len(), 12);
    let (mut restored, restored_handle) = setup_with_handle(1);
    restored.displays.set_max_tiles(2);
    for id in 100..112 {
        restored_handle.inject_window(42, id);
        restored_handle.inject_window_kind(id, WindowClass::Standard);
        restored_handle.inject_identity(identity(id));
        restored.store.register(id, 42);
    }
    restored.restore_live_windows(&session.live_windows);
    assert!(restored.workspaces[0].find_window(100).is_some());
    assert!(restored.workspaces[0].find_window(101).is_some());
    assert_eq!(
        restored.workspaces[0]
            .pending
            .iter()
            .map(|p| p.window_id)
            .collect::<Vec<_>>(),
        (102..112).collect::<Vec<_>>()
    );
    restored.on_window_destroyed(100);
    assert!(restored.workspaces[0].find_window(102).is_some());
    restored.assert_identity_invariant();
}

#[test]
fn restart_rejects_recycled_identity_and_unverified_records() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_window_kind(100, WindowClass::Standard);
    handle.inject_identity(identity(100));
    sm.on_window_created(100, 42);
    sm.move_focused_to_workspace(2);
    let records = sm.capture_live_windows();
    let (mut restored, handle) = setup_with_handle(1);
    let mut changed = identity(100);
    changed.process_start.0 += 1;
    handle.inject_identity(changed);
    handle.inject_window_kind(100, WindowClass::Standard);
    restored.restore_live_windows(&records);
    restored.on_window_created(100, 42);
    assert!(restored.workspaces[0].find_window(100).is_some());
    assert!(!restored.workspaces[2].owns(100));
}

#[test]
fn monitor_removal_relocates_and_places_traveling_overflow_immediately() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.displays.set_max_tiles(1);
    sm.on_command(
        Command::FocusDisplay {
            direction: Direction::Right,
        },
        None,
    );
    for id in [300, 400, 500] {
        handle.inject_window(42, id);
        sm.on_window_created(id, 42);
    }
    sm.on_window_hidden(500);
    assert!(sm.workspaces[1].pending_window(400).is_some());
    handle.set_displays(vec![crate::adapter::DisplayInfo {
        id: 1,
        origin: (0, 0),
        size: (1920, 1080),
    }]);
    sm.on_monitor_removed(2);
    assert!(sm.workspaces[1].find_window(300).is_some());
    assert!(sm.workspaces[0].find_window(400).is_some());
    let rect = handle.rect(400).unwrap();
    assert!(
        rect.x < 1920.0 && rect.width > 1000.0,
        "relocated tile must be placed immediately: {rect:?}"
    );
    assert_eq!(sm.store.parked_idx(500), Some(0));
    sm.on_window_shown(500);
    assert!(sm.workspaces[0].pending_window(500).is_some());
}

#[test]
fn pending_overlays_participate_in_hidden_reconcile_and_close_commands() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200] {
        sm.on_window_created(id, 42);
    }
    handle.inject_hidden_window(200);
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(sm.store.is_hidden(200));
    assert!(!sm.workspaces[0].owns(200));
    handle.clear_hidden_window(200);
    sm.on_window_shown(200);
    sm.workspaces[0].focus_window(200);
    sm.on_command(Command::Close, None);
    assert!(handle.rect(200).is_none());
    assert!(handle.rect(100).is_some());
}

#[test]
fn config_mode_and_old_session_decode_with_current_defaults() {
    let settings: crate::config::Settings = toml::from_str("max_tiles = 2").unwrap();
    assert_eq!(
        settings.windows.new_window_placement,
        NewWindowPlacement::CurrentWorkspace
    );
    let settings: crate::config::Settings =
        toml::from_str("[windows]\nnew_window_placement = 'app-rules'").unwrap();
    assert_eq!(
        settings.windows.new_window_placement,
        NewWindowPlacement::AppRules
    );
    assert!(
        toml::from_str::<crate::config::Settings>("[windows]\nnew_window_placement = 'other'")
            .is_err()
    );
    let sm = setup(1);
    let session = crate::state::session::snapshot_from(
        &sm.workspaces,
        sm.displays.active(),
        vec![],
        8.0,
        4.0,
    );
    let mut value = toml::Value::try_from(session).unwrap();
    value.as_table_mut().unwrap().remove("live_windows");
    let old: crate::state::session::Session = value.try_into().unwrap();
    assert!(old.live_windows.is_empty());
}

#[test]
fn app_hide_preserves_traveling_intent_before_sibling_events_arrive() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.inject_hidden_window(100);
    handle.inject_hidden_window(200);
    sm.on_window_hidden(100);
    assert!(sm.store.is_hidden(200));
    assert_eq!(
        sm.store.parked_entry(200).unwrap().pending.unwrap().kind,
        PendingKind::Traveling
    );
    sm.on_window_hidden(200);
    handle.clear_hidden_window(100);
    handle.clear_hidden_window(200);
    sm.on_window_shown(100);
    sm.on_window_shown(200);
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(sm.workspaces[0].find_window(100).is_some());
    assert!(sm.workspaces[1].find_window(200).is_some());
}

#[test]
fn restart_preserves_live_tiles_ahead_of_previously_parked_returns() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200] {
        handle.inject_window_kind(id, WindowClass::Standard);
        handle.inject_identity(identity(id));
    }
    sm.on_window_created(100, 42);
    sm.on_window_hidden(100);
    sm.on_window_created(200, 42);
    let records = sm.capture_live_windows();
    let (mut restored, handle) = setup_with_handle(1);
    restored.displays.set_max_tiles(1);
    for id in [100, 200] {
        handle.inject_window_kind(id, WindowClass::Standard);
        handle.inject_identity(identity(id));
    }
    // The user unminimized 100 while the daemon was down. Its former slot
    // belongs to the already-tiled 200, and its established home still holds.
    restored.restore_live_windows(&records);
    assert!(restored.workspaces[0].find_window(200).is_some());
    assert_eq!(
        restored.workspaces[0].pending_window(100).unwrap().kind,
        PendingKind::Returning
    );
}

#[test]
fn rediscovered_overflow_retains_traveling_kind_and_opening_order() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200, 300] {
        sm.on_window_created(id, 42);
    }
    let original = sm.workspaces[0].pending_window(200).unwrap();
    // The seatbelt's real rediscovery shape: an indirect untrack (the
    // listing sweep lost a live window), then the same window polled
    // again. The unconfirmed tombstone keeps its trust, so the pending
    // kind and opening order ride the return.
    sm.on_window_unlisted(200);
    sm.on_window_created(200, 42);
    assert_eq!(sm.workspaces[0].pending_window(200), Some(original));
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(sm.workspaces[1].find_window(200).is_some());
    assert!(sm.workspaces[1].pending_window(300).is_some());
}

#[test]
fn recycled_window_id_after_confirmed_destroy_routes_fresh() {
    let mut sm = setup(1);
    sm.displays.set_max_tiles(2);
    sm.on_window_created(100, 42);
    sm.move_focused_to_workspace(2); // home = workspace 2 (idx 1)
    sm.on_command(Command::Workspace { id: 3 }, None); // active = idx 2
    sm.on_window_destroyed(100); // destroyed notification — the window is gone
                                 // macOS reuses the WindowId for a brand-new window of the same app:
                                 // a birth, not a rediscovery. It must open on the current workspace
                                 // with fresh intent, not inherit the dead window's home.
    sm.on_window_created(100, 42);
    assert!(
        sm.workspaces[2].find_window(100).is_some(),
        "recycled id routes to the current workspace"
    );
    assert!(!sm.workspaces[1].owns(100));
}

#[test]
fn directional_move_carries_a_focused_overlay_to_the_neighbor_display() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.displays.set_max_tiles(1);
    sm.on_window_created(100, 42); // tiles workspace 1
    sm.on_window_created(200, 42); // overflow overlay on workspace 1
    sm.workspaces[0].focus_window(200);
    sm.on_command(
        Command::MoveWindow {
            direction: Direction::Right,
        },
        None,
    );
    assert!(sm.workspaces[0].find_window(100).is_some());
    assert!(!sm.workspaces[0].owns(200));
    // The neighbor display's workspace has room, so the carried overlay
    // tiles there and keeps focus.
    assert!(sm.workspaces[1].find_window(200).is_some());
    assert_eq!(handle.last_focused(), Some(200));
}

#[test]
fn restart_record_for_a_deleted_workspace_falls_back_to_fresh_routing() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_window(42, 100);
    handle.inject_window_kind(100, WindowClass::Standard);
    handle.inject_identity(identity(100));
    sm.on_window_created(100, 42);
    let mut records = sm.capture_live_windows();
    assert_eq!(records.len(), 1);
    records[0].workspace = "deleted-in-the-new-config".into();

    let (mut restored, handle) = setup_with_handle(1);
    handle.inject_window(42, 100);
    handle.inject_window_kind(100, WindowClass::Standard);
    handle.inject_identity(identity(100));
    restored.store.register(100, 42);
    restored.restore_live_windows(&records);
    // The named workspace no longer exists: the record is skipped and
    // bootstrap's birth path routes the window fresh.
    assert!(
        !restored.workspaces.iter().any(|ws| ws.owns(100)),
        "no home may be invented for a deleted workspace name"
    );
    restored.on_window_created(100, 42);
    assert!(restored.workspaces[0].find_window(100).is_some());
}
