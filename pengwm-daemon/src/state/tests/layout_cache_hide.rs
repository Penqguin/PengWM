use std::time::Duration;

use super::common::*;
use pengwm_core::command::Command;
use pengwm_core::workspace::Workspace;

#[test]
fn hide_workspace_uses_bottom_edge_rect() {
    let mut sm = setup(1);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::BottomEdge);
    sm.on_window_created(100, 42);
    assert!(sm.workspaces[0].find_window(100).is_some());
    let tiled = sm.os.window_rect_for_test(100).expect("tiled rect");
    // Switch to a different workspace so ws-0 gets hidden
    sm.on_command(Command::Workspace { id: 2 }, None);
    let expected = pengwm_core::layout::hidden_rect((0, 0), (1920, 1080));
    let rect = sm
        .os
        .window_rect_for_test(100)
        .expect("hidden window rect should exist");
    // Position-only hide: position at the clamped corner, size preserved.
    assert_eq!((rect.x, rect.y), (expected.x, expected.y));
    assert_eq!((rect.width, rect.height), (tiled.width, tiled.height));
}

#[test]
fn hide_workspace_far_offscreen_when_configured() {
    let mut sm = setup(1);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::FarOffscreen);
    sm.on_window_created(100, 42);
    let tiled = sm.os.window_rect_for_test(100).expect("tiled rect");
    sm.on_command(Command::Workspace { id: 2 }, None);
    let expected = pengwm_core::layout::far_offscreen_rect();
    let rect = sm
        .os
        .window_rect_for_test(100)
        .expect("hidden window rect should exist");
    assert_eq!((rect.x, rect.y), (expected.x, expected.y));
    assert_eq!((rect.width, rect.height), (tiled.width, tiled.height));
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
        (rect.x, rect.y),
        (expected.x, expected.y),
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

#[test]
fn apply_layout_untracks_window_gone_from_os() {
    // Missed destroyed notification (window 15991 storm): OS closed the
    // window but no event arrived, so every layout retried and error-spammed.
    // A permanent-gone error + OS no longer listing the window must drop it
    // via the normal destroyed path instead of retrying forever.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // OS-side close with no WM notification.
    sm.os.close_window(100);
    sm.os.fail_rect_for_test(100);
    // Invalidate the skip-if-unchanged entry so the next layout actually
    // attempts the write and observes the failure (mirrors the displaced
    // path; avoids depending on workspace capacity).
    sm.age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(!sm.store.contains(100), "gone window must be untracked");
    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(100).is_none()),
        "gone window must leave the tree"
    );
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(200).is_some()),
        "live windows keep tiling"
    );
}

#[test]
fn hide_is_position_only_and_preserves_size() {
    // Hide must never resize: Firefox reflows on every size write, and the
    // 3x verify loop in `set_window_rect` would triple it. Position-only.
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let tiled = sm.os.window_rect_for_test(100).expect("tiled rect");
    let writes_after_tile = sm.os.set_rect_calls_for_test();
    sm.on_command(Command::Workspace { id: 2 }, None);
    let hidden = sm.os.window_rect_for_test(100).expect("hidden rect");
    assert_ne!((tiled.x, tiled.y), (hidden.x, hidden.y));
    assert_eq!((hidden.width, hidden.height), (tiled.width, tiled.height));
    assert_eq!(
        sm.os.set_rect_calls_for_test(),
        writes_after_tile,
        "hide must not issue full set_window_rect writes"
    );
}

#[test]
fn window_rect_seam_reads_back_tiled_rect() {
    // The verify-and-retry seam: what `apply_layout` wrote must be readable
    // through the same `OsAdapter` interface (no FFI reach-through).
    let mut sm = setup(1);
    sm.on_window_created(100, 42);
    let written = sm.os.window_rect_for_test(100).expect("tiled rect");
    assert_eq!(sm.os.window_rect(100), Some(written));
    assert!(sm.os.window_rect(999).is_none());
}
