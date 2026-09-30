use std::time::Duration;

use super::common::*;
use pengwm_core::command::Command;
use pengwm_core::workspace::Workspace;

#[test]
fn hide_workspace_uses_bottom_edge_rect() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::BottomEdge);
    sm.on_window_created(100, 42);
    assert!(sm.workspaces[0].find_window(100).is_some());
    let tiled = handle.rect(100).expect("tiled rect");
    // Switch to a different workspace so ws-0 gets hidden
    sm.on_command(Command::Workspace { id: 2 }, None);
    let expected = pengwm_core::layout::hidden_rect((0, 0), (1920, 1080));
    let rect = handle.rect(100).expect("hidden window rect should exist");
    // Position-only hide: position at the clamped corner, size preserved.
    assert_eq!((rect.x, rect.y), (expected.x, expected.y));
    assert_eq!((rect.width, rect.height), (tiled.width, tiled.height));
}

#[test]
fn hide_workspace_far_offscreen_when_configured() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::FarOffscreen);
    sm.on_window_created(100, 42);
    let tiled = handle.rect(100).expect("tiled rect");
    sm.on_command(Command::Workspace { id: 2 }, None);
    let expected = pengwm_core::layout::far_offscreen_rect();
    let rect = handle.rect(100).expect("hidden window rect should exist");
    assert_eq!((rect.x, rect.y), (expected.x, expected.y));
    assert_eq!((rect.width, rect.height), (tiled.width, tiled.height));
}

#[test]
fn hide_workspace_per_monitor_second_display() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.set_hidden_strategy_for_test(crate::config::HiddenStrategy::BottomEdge);
    // Global pool: workspace 1 is visible on display 2 (origin 1920,0).
    sm.workspaces[1].add_window(999, None);
    sm.store.register(999, 42);
    sm.hide_workspace(1);
    let expected = pengwm_core::layout::hidden_rect((1920, 0), (1920, 1080));
    let rect = handle.rect(999).expect("hidden window rect should exist");
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
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    let writes_after_tile = handle.writes();
    assert!(writes_after_tile >= 2, "tiling should write each window");

    // Same layout again (focus change, tick, publish path) → no writes.
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        writes_after_tile,
        "identical layout must not rewrite windows"
    );

    // A real change (new window) writes again.
    sm.on_window_created(300, 42);
    assert!(
        handle.writes() > writes_after_tile,
        "changed layout must write"
    );
}

#[test]
fn moved_window_is_reasserted_on_next_layout() {
    // A displaced window (user drag, app move) must be rewritten, not
    // skipped — otherwise drag snap-back silently stops working.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    let writes_after_tile = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        writes_after_tile,
        "identical layout must not rewrite"
    );
    // Displace past the grace window so it reads as external, not as our
    // own animation settling: the OS-side rect must genuinely disagree
    // before the moved note observes it, or the note (like the
    // read-before-write check) correctly skips.
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    handle.displace(100, 500.0, 500.0);
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        handle.writes() > writes_after_tile,
        "displaced window must be re-asserted"
    );
}

#[test]
fn move_within_grace_window_stays_skipped() {
    // Moves arriving right after our own write are animation settle, not
    // a drag — invalidating on them would restart the Firefox storm.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    let writes_after_tile = handle.writes();
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        writes_after_tile,
        "settle moves inside the grace window must not force a rewrite"
    );
}

#[test]
fn hide_then_switch_back_retiles_instead_of_skipping() {
    // The dangerous direction for skip-if-unchanged: a window hidden on
    // switch-away must come back on switch-back, not compare equal to a
    // stale tiled entry.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    let tiled = handle.rect(100).expect("tiled rect");
    sm.on_command(Command::Workspace { id: 2 }, None);
    let hidden = handle.rect(100).expect("hidden rect");
    assert_ne!(tiled, hidden, "switch-away should hide the window");
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(
        handle.rect(100),
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
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // OS-side close with no WM notification.
    sm.os.close_window(100);
    handle.set_fault(100, Fault::Gone);
    // Invalidate the skip-if-unchanged entry so the next layout actually
    // attempts the write and observes the failure (mirrors the displaced
    // path; avoids depending on workspace capacity).
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.on_window_moved(100, 500.0, 500.0);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        sm.store.contains(100),
        "first miss must start the gone grace, not untrack"
    );
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "grace window must keep its tree position"
    );
    // Still missing past the grace: genuinely gone, untrack.
    sm.layout_cache
        .age_gone_for_test(100, Duration::from_secs(30));
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
fn transient_blackout_heals_without_retiling_as_new() {
    // Post-wake / transient AX blackout: the write fails AND the OS doesn't
    // list the window for one layout, then the same WindowId is back. The
    // window must stay tracked throughout (tree position preserved) and be
    // rewritten in place — no untrack/rediscover cycle.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // Blackout: OS-side invisible + writes failing. Drop the cache entry to
    // mirror the post-wake cleared `applied_rects`.
    sm.os.close_window(100);
    handle.set_fault(100, Fault::Gone);
    sm.layout_cache.drop_applied_for_test(100);
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        sm.store.contains(100),
        "transient miss must not untrack the window"
    );
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "blackout must not drop the window from the tree"
    );
    // Blackout lifts, same WindowId back: rewrite in place.
    handle.inject_window(42, 100);
    handle.clear_fault(100);
    let writes_before_heal = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    assert!(
        handle.writes() > writes_before_heal,
        "healed window must be rewritten"
    );
    assert!(sm.store.contains(100));
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "healed window keeps its tree position"
    );
}

#[test]
fn pinned_window_backs_off_writes_then_retries_after_backoff() {
    // Busy-app pinned window (stable readbacks, writes futile): three
    // strikes engage the backoff, further layouts skip the write, the
    // backoff timer expiring retries, and a healed window resumes
    // skip-if-unchanged. Tracked throughout, never untracked.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_fault(100, Fault::Pinned);
    // Displace the OS-side rect so the read-before-write check can't skip:
    // the write must actually be attempted to observe the pin. (Pinned
    // failures never update the OS rect, so one displacement covers every
    // layout below.)
    handle.displace(100, 500.0, 500.0);
    sm.layout_cache.drop_applied_for_test(100);

    // Strikes 1–3: each layout attempts the write.
    for strike in 1..=3 {
        let calls = handle.writes();
        sm.apply_layout(sm.active_workspace_idx());
        assert_eq!(
            handle.writes(),
            calls + 1,
            "pre-backoff layout {} must attempt the write",
            strike
        );
        assert!(sm.store.contains(100), "pinned window must stay tracked");
    }
    // Backoff engaged: layouts skip the write entirely.
    let backed_off = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        backed_off,
        "backed-off layouts must not write"
    );
    assert!(sm.store.contains(100));
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "backed-off window keeps its tree position"
    );
    // Backoff expires: the next layout retries the write.
    sm.layout_cache
        .age_pin_for_test(100, Duration::from_secs(30));
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        backed_off + 1,
        "expired backoff must retry"
    );
    // Heals (app drains its event loop): the write lands, the pin clears,
    // and the following layout skips via the applied entry.
    handle.clear_fault(100);
    sm.layout_cache
        .age_pin_for_test(100, Duration::from_secs(30));
    sm.apply_layout(sm.active_workspace_idx());
    let healed = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        healed,
        "healed window must resume skip-if-unchanged"
    );
    assert!(sm.store.contains(100));
}

#[test]
fn pinned_window_with_changed_target_starts_fresh() {
    // A changed target is a fresh situation: pin strikes for the old target
    // must not silence writes for the new one.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_fault(100, Fault::Pinned);
    handle.displace(100, 500.0, 500.0);
    sm.layout_cache.drop_applied_for_test(100);
    for _ in 0..3 {
        sm.apply_layout(sm.active_workspace_idx());
    }
    // Backoff engaged for the old target; a new window changes 100's target.
    let backed_off = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        backed_off,
        "backoff must hold for the unchanged target"
    );
    sm.on_window_created(300, 42);
    // Tiling 300 rewrites the workspace (new targets): 100's write must be
    // attempted again despite the old pin, not skipped.
    assert!(
        handle.writes() > backed_off,
        "changed target must reset the pin and write again"
    );
    assert!(sm.store.contains(100));
}

#[test]
fn read_before_write_skips_placed_windows_without_cache() {
    // Post-wake full rewrite is the Firefox storm: with no `applied_rects`
    // entry but the OS rect already at target, one read must skip the write.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    sm.layout_cache.drop_applied_for_test(100);
    sm.layout_cache.drop_applied_for_test(200);
    let writes_before = handle.writes();
    sm.apply_layout(sm.active_workspace_idx());
    assert_eq!(
        handle.writes(),
        writes_before,
        "placed windows must not be rewritten just because the cache is empty"
    );
}

#[test]
fn hide_is_position_only_and_preserves_size() {
    // Hide must never resize: Firefox reflows on every size write, and the
    // 3x verify loop in `set_window_rect` would triple it. Position-only.
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    let tiled = handle.rect(100).expect("tiled rect");
    let writes_after_tile = handle.writes();
    sm.on_command(Command::Workspace { id: 2 }, None);
    let hidden = handle.rect(100).expect("hidden rect");
    assert_ne!((tiled.x, tiled.y), (hidden.x, hidden.y));
    assert_eq!((hidden.width, hidden.height), (tiled.width, tiled.height));
    assert_eq!(
        handle.writes(),
        writes_after_tile,
        "hide must not issue full set_window_rect writes"
    );
}

#[test]
fn window_rect_seam_reads_back_tiled_rect() {
    // The verify-and-retry seam: what `apply_layout` wrote must be readable
    // through the same `OsAdapter` interface (no FFI reach-through).
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    let written = handle.rect(100).expect("tiled rect");
    assert_eq!(sm.os.window_rect(100), Some(written));
    assert!(sm.os.window_rect(999).is_none());
}
