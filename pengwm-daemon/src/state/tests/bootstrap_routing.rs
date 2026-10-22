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
fn creates_global_pool_shared_across_displays() {
    let sm = setup(2);
    // One tree per name globally; output 1 shows entry 0, output 2 shows entry 1.
    assert_eq!(sm.workspaces.len(), 5);
    assert_eq!(sm.displays.active().get(&1), Some(&0));
    assert_eq!(sm.displays.active().get(&2), Some(&1));
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
    // 5 named workspaces globally.
    assert_eq!(sm.workspaces.len(), 5);

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
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(77, "com.google.Chrome".into());
    handle.inject_app_name(77, "Chrome".into());

    sm.on_window_created(777, 77);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(777).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Browsing");
}

#[test]
fn on_window_created_routes_firefox_developer_edition_to_browsing() {
    // Firefox Developer Edition is a separate app (separate bundle id and
    // display name) — it must route to Browsing by default, not fall
    // through to the active workspace.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(78, "org.mozilla.firefoxdeveloperedition".into());
    handle.inject_app_name(78, "Firefox Developer Edition".into());

    sm.on_window_created(778, 78);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(778).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Browsing");
    // Either identifier alone is enough (bundle id match or name match).
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(78, "org.mozilla.other".into());
    handle.inject_app_name(78, "firefox developer edition".into());
    sm.on_window_created(779, 78);
    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(779).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Browsing");
}

#[test]
fn on_window_created_routing_matches_app_name_case_insensitively() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_app_name(88, "spotify".into());
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
    let (mut sm, handle) = setup_with_handle(1);
    // Route Firefox to Browsing (ws-1) then switch back to Development (ws-0).
    handle.inject_bundle_id(77, "org.mozilla.firefox".into());
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
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(77, "org.mozilla.firefox".into());
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
        1,
    );
    // Global pool: one tree per entry; Dev hinted at 1, Browse homes to
    // primary; output 1 shows Dev, output 2 shows Browse.
    assert_eq!(wss.len(), 2);
    assert_eq!(wss[0].name, "Dev");
    assert_eq!(wss[0].monitor_id, 1);
    assert_eq!(wss[1].name, "Browse");
    assert_eq!(wss[1].monitor_id, 2);
}

// -----------------------------------------------------------------------
// Placement memory (state::placement)
// -----------------------------------------------------------------------

use crate::state::session::WindowPlacement;

#[test]
fn placement_memory_rehomes_before_config_routing() {
    // The record is what the user *did* last session (a manual move), so it
    // outranks the config list (what the app *is*).
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(78, "org.mozilla.firefoxdeveloperedition".into());
    handle.inject_title(778, "MDN — tree shaking".to_string());
    sm.seed_placement_memory_for_test(vec![WindowPlacement {
        bundle: Some("org.mozilla.firefoxdeveloperedition".into()),
        title: Some("MDN — tree shaking".into()),
        workspace: "Notes".into(),
    }]);

    sm.on_window_created(778, 78);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(778).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Notes");
    assert_eq!(sm.placement_memory_len_for_test(), 0, "hit consumed once");
}

#[test]
fn placement_memory_miss_falls_through_to_config_with_record_intact() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(78, "org.mozilla.firefoxdeveloperedition".into());
    handle.inject_title(778, "MDN — tree shaking".to_string());
    sm.seed_placement_memory_for_test(vec![WindowPlacement {
        bundle: Some("org.mozilla.firefoxdeveloperedition".into()),
        title: Some("Some other title".into()),
        workspace: "Notes".into(),
    }]);

    sm.on_window_created(778, 78);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(778).is_some())
        .expect("routed window should be tracked");
    assert_eq!(
        sm.workspaces[idx].name, "Browsing",
        "config routing owns the miss"
    );
    assert_eq!(
        sm.placement_memory_len_for_test(),
        1,
        "a miss must not consume the record"
    );
}

#[test]
fn placement_memory_hit_is_consumed_once_for_identical_windows() {
    // Two identically-titled windows of one bundle (two "New Tab" tabs):
    // only the first re-homes; the second routes normally instead of
    // stealing the record again.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(77, "org.mozilla.firefox".into());
    handle.inject_title(777, "New Tab".to_string());
    handle.inject_title(888, "New Tab".to_string());
    sm.seed_placement_memory_for_test(vec![WindowPlacement {
        bundle: Some("org.mozilla.firefox".into()),
        title: Some("New Tab".into()),
        workspace: "Notes".into(),
    }]);

    sm.on_window_created(777, 77);
    sm.on_window_created(888, 77);

    let first = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(777).is_some())
        .expect("first window tracked");
    let second = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(888).is_some())
        .expect("second window tracked");
    assert_eq!(sm.workspaces[first].name, "Notes");
    assert_eq!(
        sm.workspaces[second].name, "Browsing",
        "config routing, record spent"
    );
}

#[test]
fn placement_memory_rehomes_across_reboot_pid_drift() {
    // A reboot changes pids. The record matches bundle+title, so the
    // window still re-homes — and the record's owning pid difference does
    // not veto.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(9102, "org.mozilla.firefoxdeveloperedition".into());
    handle.inject_title(9103, "MDN — tree shaking".to_string());
    sm.seed_placement_memory_for_test(vec![WindowPlacement {
        bundle: Some("org.mozilla.firefoxdeveloperedition".into()),
        title: Some("MDN — tree shaking".into()),
        workspace: "Messaging".into(),
    }]);

    sm.on_window_created(9103, 9102);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(9103).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Messaging");
}

#[test]
fn placement_memory_record_for_missing_workspace_name_misses() {
    // The workspace was renamed between save and restore: a stale name is
    // a miss, not a panic or a fallback-to-index.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(78, "org.mozilla.firefoxdeveloperedition".into());
    handle.inject_title(778, "MDN — tree shaking".to_string());
    sm.seed_placement_memory_for_test(vec![WindowPlacement {
        bundle: Some("org.mozilla.firefoxdeveloperedition".into()),
        title: Some("MDN — tree shaking".into()),
        workspace: "Gaming".into(),
    }]);

    sm.on_window_created(778, 78);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(778).is_some())
        .expect("routed window should be tracked");
    assert_eq!(sm.workspaces[idx].name, "Browsing");
}

#[test]
fn placement_memory_captures_tileable_windows_with_evidence() {
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(77, "org.mozilla.firefox".into());
    handle.inject_title(777, "New Tab".to_string());
    sm.on_window_created(777, 77); // → Browsing

    let records = sm.capture_placements();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].workspace, "Browsing");
    assert_eq!(records[0].bundle.as_deref(), Some("org.mozilla.firefox"));
    assert_eq!(records[0].title.as_deref(), Some("New Tab"));
}

// -----------------------------------------------------------------------
// Routing restack (candidate 3): a member of a hidden workspace is offscreen
// -----------------------------------------------------------------------

#[test]
fn window_routed_into_hidden_workspace_takes_the_hide_placement() {
    // The initial hide pass ran inside `new()` while the trees were empty;
    // Firefox routes to Browsing (hidden — display 1 shows Development).
    // Without the routing tail it would float visibly over Development
    // until the next switch.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_bundle_id(77, "org.mozilla.firefox".into());
    handle.inject_title(777, "New Tab".to_string());

    sm.on_window_created(777, 77);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(777).is_some())
        .expect("routed window should be tracked");
    assert!(!sm.displays.is_visible(idx, &sm.workspaces));
    let rect = handle.rect(777).expect("routed window has an OS rect");
    // BottomEdge default: bottom-right 1×1 (the dark title-bar strip).
    assert!(
        rect.width == 1.0 && rect.height == 1.0 && rect.y >= 1079.0 && rect.x >= 1919.0,
        "routed-into-hidden window should sit at the hide placement, got {rect:?}"
    );
}

#[test]
fn window_routed_into_visible_workspace_is_not_hidden_offscreen() {
    let (mut sm, handle) = setup_with_handle(1);
    // Unassigned app: routes to the active workspace (Development, visible).
    handle.inject_bundle_id(99, "com.example.unassigned".into());
    handle.inject_title(999, "Local note".to_string());
    handle.inject_rect(
        999,
        pengwm_core::layout::Rect::new(100.0, 100.0, 800.0, 600.0),
    );

    sm.on_window_created(999, 99);

    let idx = sm
        .workspaces
        .iter()
        .position(|ws| ws.find_window(999).is_some())
        .expect("routed window should be tracked");
    assert!(sm.displays.is_visible(idx, &sm.workspaces));
    let rect = handle.rect(999).unwrap();
    // Tiled into the visible workspace (layout applied on route), therefore
    // on-screen — a hide placement would be the bottom-right 1×1 strip.
    assert!(
        rect.y < 1080.0 && rect.x < 1920.0 && rect.width > 1.0,
        "visible-workspace routing must not hide, got {rect:?}"
    );
}
