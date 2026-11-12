//! Focus settling (docs/specs/0005-focus-timing.md): WM focus memory follows
//! the actually-focused window — click or keybind — via a settle re-read,
//! never last-arrival; with no OS focus it falls back to the last focused
//! window (visible member), otherwise the workspace seat stands. The
//! observed poisoning case: `126 → 30556 → 61` click-bounce with the
//! stale event arriving last.

use std::time::{Duration, Instant};

use super::common::setup_with_handle;

/// The poisoning case: three focused events inside the storm (the last
/// is the stale one), frontmost app's actual focus = 126. After settle,
/// the WM commits 126 — the OS truth — not last-arrival 61.
#[test]
fn click_bounce_commits_os_truth_not_last_arrival() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    // The OS's settled truth: frontmost pid 42's focused window is 200.
    handle.set_frontmost(Some(42));
    handle.set_focused_window(42, Some(200));

    // The click storm: real (200), drift (100), stale (100 again) —
    // every one of them dispatch-marked.
    let now = Instant::now();
    sm.on_window_focused_event_at(200, now);
    sm.on_window_focused_event_at(100, now + Duration::from_millis(10));
    sm.on_window_focused_event_at(100, now + Duration::from_millis(20));
    sm.commit_settled_focus(now + Duration::from_millis(270));

    assert_eq!(
        sm.workspaces[0].focused_window_id(),
        Some(200),
        "WM focus memory must follow the OS's actually-focused window"
    );
}

/// No window in focus at commit time (desktop clicked / focus nowhere):
/// the last focused window takes over, if it is still a visible member —
/// NOT the storm's last stale arrival.
#[test]
fn no_os_focus_falls_back_to_last_focused_window() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);

    // Establish real focus THROUGH the storm channel (event-marked, so
    // the settle commits it and owns the last-focused memory).
    handle.set_frontmost(Some(42));
    handle.set_focused_window(42, Some(200));
    let now = Instant::now();
    sm.on_window_focused_event_at(200, now);
    sm.commit_settled_focus(now + Duration::from_millis(250));

    // Now everything falls out of focus: no frontmost app, no window.
    handle.set_frontmost(None);
    // A stale focus event still arrives (the pre-settle 100 noise):
    sm.on_window_focused_event_at(100, now + Duration::from_millis(300));
    sm.commit_settled_focus(now + Duration::from_millis(550));

    assert_eq!(
        sm.workspaces[0].focused_window_id(),
        Some(200),
        "no OS focus → last focused window (200), never stale 100"
    );
}

/// Ladder (c): the last focused window's workspace went hidden — the
/// seat stands (no cross-workspace dragging, the i3 rule).
#[test]
fn hidden_last_focus_leaves_the_workspace_seat() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_frontmost(Some(42));
    handle.set_focused_window(42, Some(200));
    let now = Instant::now();
    sm.on_window_focused_event_at(200, now);
    sm.commit_settled_focus(now + Duration::from_millis(250));

    // 200 moves to a hidden workspace (explicit command): its fallback
    // path must NOT drag it back into view after the settle.
    sm.on_command(
        pengwm_core::command::Command::MoveWindowToWorkspace { id: 2 },
        None,
    );
    handle.set_frontmost(None);
    sm.on_window_focused_event_at(100, now + Duration::from_millis(300));
    sm.commit_settled_focus(now + Duration::from_millis(550));

    // The i3 invariant: a hidden workspace's window never receives
    // focus — and nothing switched workspaces behind the user's back
    // (monitor 1 still shows workspace 1).
    assert_eq!(
        sm.displays.active().get(&1),
        Some(&0),
        "seat stands — no reverse-switch from the focus fallback"
    );
}

/// The settle gate: a tick inside the quiet window commits nothing new —
/// the drain is a no-op until 250 ms of quiet has passed (each storm
/// event re-arms it, and the immediate path already held the truth).
#[test]
fn unsettled_dirty_window_does_not_commit_early() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
    handle.set_frontmost(Some(42));
    handle.set_focused_window(42, Some(200));
    let now = Instant::now();
    sm.on_window_focused_event_at(200, now);
    sm.commit_settled_focus(now + Duration::from_millis(250));

    // A fresh storm arrival: immediate path commits it (low-latency),
    // and 249ms later the drain must still leave it alone.
    sm.on_window_focused_event_at(100, now + Duration::from_millis(300));
    sm.commit_settled_focus(now + Duration::from_millis(549));
    assert_eq!(
        sm.workspaces[0].focused_window_id(),
        Some(100),
        "no settle yet → the drain stays out of the way"
    );

    // At exactly 250ms of quiet, OS truth replaces the stale observation.
    sm.commit_settled_focus(now + Duration::from_millis(550));
    assert_eq!(
        sm.workspaces[0].focused_window_id(),
        Some(200),
        "settled → OS truth commits"
    );
}
