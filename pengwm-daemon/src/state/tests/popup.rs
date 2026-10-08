//! Popup behavior end-to-end through `StateManager`: classification-driven
//! routing, workspace-bound hides, the switch-back raise, and the funnels
//! popups are excluded from.

use std::time::Duration;

use super::super::StateManager;
use super::common::*;
use crate::adapter::WindowClass;
use pengwm_core::command::Command;
use pengwm_core::layout::Rect;

/// Expected centered-overlay target for display 1 (1920x1080, no bar): the
/// gap-inset usable area at `ratio`, centered. Mirrors the shared
/// `centered_overlay_rect` computation.
fn centered_overlay(sm: &StateManager, ratio: f64) -> Rect {
    let g = sm.gap_outer;
    let inset = Rect::new(g, g, 1920.0 - g * 2.0, 1080.0 - g * 2.0);
    let w = inset.width * ratio;
    let h = inset.height * ratio;
    Rect::new(
        inset.x + (inset.width - w) / 2.0,
        inset.y + (inset.height - h) / 2.0,
        w,
        h,
    )
}

/// A popup drawn at its own position on display 1's center, for the
/// containing-monitor attach rule.
fn popup_rect_on_display1() -> Rect {
    Rect::new(960.0, 540.0, 400.0, 300.0)
}

#[test]
fn dialog_window_becomes_workspace_popup() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.workspaces[0].popup_ratio = 0.75;
    handle.inject_window_kind(300, WindowClass::Floating);
    handle.inject_rect(300, popup_rect_on_display1());
    sm.on_window_created(300, 42);
    assert!(sm.workspaces[0].is_popup(300), "dialog becomes a popup");
    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(300).is_none()),
        "popup never tiles"
    );
    assert_eq!(
        handle.rect(300),
        Some(centered_overlay(&sm, 0.75)),
        "placed as a centered overlay"
    );
}

#[test]
fn restricted_app_windows_pop_out_instead_of_tiling() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.workspaces[0].popup_ratio = 0.75;
    sm.set_restricted_apps_for_test(vec!["com.example.util".into()]);
    handle.inject_bundle_id(42, "com.example.util".into());
    handle.inject_rect(300, popup_rect_on_display1());
    // A *Standard* window of a restricted app still pops out.
    handle.inject_window_kind(300, WindowClass::Standard);
    sm.on_window_created(300, 42);
    assert!(sm.workspaces[0].is_popup(300), "restricted app pops out");
    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(300).is_none()),
        "restricted app never tiles"
    );
    assert_eq!(handle.rect(300), Some(centered_overlay(&sm, 0.75)));
    // Sanity: a Standard window from a *non*-restricted pid still tiles.
    sm.on_window_created(100, 43);
    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "non-restricted windows tile normally"
    );
}

#[test]
fn popup_hides_with_workspace_and_returns_on_top() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.workspaces[0].popup_ratio = 0.75;
    sm.on_window_created(100, 42);
    handle.inject_window_kind(300, WindowClass::Floating);
    handle.inject_rect(300, popup_rect_on_display1());
    sm.on_window_created(300, 42);
    let centered = handle.rect(300).expect("popup placed");

    // Switch away: the popup parks with its workspace like every window.
    sm.on_command(Command::Workspace { id: 2 }, None);
    let hidden = handle.rect(300).expect("popup parked");
    assert_ne!(hidden, centered, "switch-away hides the popup");

    // Switch back: re-centered AND raised to the front of the stack.
    sm.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(handle.rect(300), Some(centered), "switch-back re-centers");
    assert!(handle.raised().contains(&300), "popup comes back on top");
}

#[test]
fn displaced_popup_is_not_snapped_back() {
    // Free-drag after placement: the misplaced sweep must leave popups
    // alone, unlike tree windows which are re-asserted.
    let (mut sm, handle) = setup_with_handle(1);
    sm.workspaces[0].popup_ratio = 0.75;
    handle.inject_window_kind(300, WindowClass::Floating);
    // A live popup is in its app's kAXWindows listing like any other
    // window — the sweep's unlisted-untrack would otherwise judge it dead.
    handle.inject_window(42, 300);
    handle.inject_rect(300, popup_rect_on_display1());
    sm.on_window_created(300, 42);
    let centered = handle.rect(300).expect("popup placed");

    handle.displace(300, 300.0, 0.0);
    sm.layout_cache
        .age_applied_for_test(300, Duration::from_secs(5));
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert_ne!(
        handle.rect(300),
        Some(centered),
        "user-dragged popup stays where the user put it"
    );
    assert!(sm.workspaces[0].is_popup(300), "still tracked");
}

#[test]
fn destroyed_popup_leaves_the_workspace() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.workspaces[0].popup_ratio = 0.75;
    handle.inject_window_kind(300, WindowClass::Floating);
    handle.inject_rect(300, popup_rect_on_display1());
    sm.on_window_created(300, 42);
    sm.on_window_destroyed(300);
    assert!(!sm.workspaces[0].is_popup(300), "membership cleared");
    assert!(sm.store.pid_for(300).is_none(), "untracked");
    let rects = sm.workspaces[0].layout(5.0, 10.0);
    assert!(
        !rects.contains_key(&300),
        "no target written for the dead popup"
    );
}

#[test]
fn minimized_popup_restores_into_the_popup_set() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.workspaces[0].popup_ratio = 0.75;
    handle.inject_window_kind(300, WindowClass::Floating);
    handle.inject_rect(300, popup_rect_on_display1());
    sm.on_window_created(300, 42);
    sm.on_window_hidden(300);
    assert!(
        !sm.workspaces[0].is_popup(300),
        "hidden popup leaves the set"
    );
    assert!(sm.store.is_hidden(300), "remembered for restore");
    sm.on_window_shown(300);
    assert!(
        sm.workspaces[0].is_popup(300),
        "restore re-attaches as popup"
    );
    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(300).is_none()),
        "restore never routes a popup into the tree"
    );
}

#[test]
fn missed_app_hidden_popup_reconciles_off_the_set() {
    // The 1s reconcile is the fallback for missed hide notifications; it
    // must cover popups too (membership is tree-or-popup).
    let (mut sm, handle) = setup_with_handle(1);
    sm.workspaces[0].popup_ratio = 0.75;
    handle.inject_window_kind(300, WindowClass::Floating);
    // Live popup: listed by its app (see the sweep's unlisted-untrack).
    handle.inject_window(42, 300);
    handle.inject_rect(300, popup_rect_on_display1());
    sm.on_window_created(300, 42);
    // Missed WindowHidden notification: the OS reports the window hidden.
    handle.inject_hidden_window(300);
    sm.store.force_reconcile_for_test();
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(
        !sm.workspaces[0].is_popup(300),
        "reconcile unsets the popup"
    );
    assert!(sm.store.is_hidden(300), "popup remembered for restore");
}

#[test]
fn popup_attaches_to_monitor_containing_it() {
    // Multi-monitor (Q8): a dialog drawn on display 2 attaches to display
    // 2's workspace, even though the focused output is display 1.
    let (mut sm, handle) = setup_with_handle(2);
    sm.workspaces[0].popup_ratio = 0.75;
    sm.workspaces[1].popup_ratio = 0.75;
    handle.inject_rect(300, Rect::new(2700.0, 300.0, 400.0, 300.0));
    handle.inject_window_kind(300, WindowClass::Floating);
    sm.on_window_created(300, 42);
    assert!(
        sm.workspaces[1].is_popup(300),
        "attaches to the monitor it appeared on"
    );
    assert!(!sm.workspaces[0].is_popup(300));
    assert!(sm.workspaces[1].find_window(300).is_none(), "never tiles");
}
