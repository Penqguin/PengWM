use super::common::*;
use crate::adapter::DisplayInfo;
use crate::config::{MonitorRef, WorkspaceEntry};
use crate::state::display::DisplaySet;
use pengwm_core::command::Command;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;

// -----------------------------------------------------------------------
// Bootstrap
// -----------------------------------------------------------------------

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

// -----------------------------------------------------------------------
// Routing + capacity (DisplaySet)
// -----------------------------------------------------------------------

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
    let mut ds = DisplaySet::new(vec![
        WorkspaceEntry {
            name: "Dev".into(),
            apps: vec![],
            monitor: Some(MonitorRef::Index(1)),
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
