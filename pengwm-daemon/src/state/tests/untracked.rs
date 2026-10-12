use super::common::*;
use pengwm_core::command::Command;
use pengwm_core::layout::Rect;

#[test]
fn untracked_windows_stay_out_of_every_tree() {
    // ADR-0002: an untracked window is store-tracked but tree-less — like
    // a capacity-overflow window. Routing must not tile it, must not make
    // it a workspace-bound popup, and (being outside every tree) it never
    // appears in BarState, whose workspaces list only tree/popup members.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_window_kind(300, crate::adapter::WindowClass::Untracked);
    sm.on_window_created(300, 42);
    assert!(
        sm.store.contains(300),
        "untracked stays tracked — pid maps and eviction funnels keep working"
    );
    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(300).is_none()),
        "untracked never joins a tree"
    );
    assert!(
        sm.workspaces.iter().all(|ws| !ws.is_popup(300)),
        "untracked is not a workspace-bound popup"
    );
}

#[test]
fn float_apps_promote_windows_at_routing() {
    // Tier B: a Standard window of a `float_apps` bundle becomes untracked
    // at routing time. Config checked at creation only
    // (first-classification-wins).
    let (mut sm, handle) = setup_with_handle(1);
    sm.set_float_apps_for_test(vec!["com.apple.custom.floater".into()]);
    handle.inject_bundle_id(42, "com.apple.custom.floater".into());
    sm.on_window_created(300, 42);
    assert!(sm.store.contains(300));
    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(300).is_none()),
        "a float_apps bundle's window must not tile"
    );
}

#[test]
fn float_level_overlays_false_downgrades_level_gated_class() {
    // The kill-switch: untracked *only* because of the CG-level gate tiles
    // again when the gate is disabled (and the bundle is not float-listed).
    let (mut sm, handle) = setup_with_handle(1);
    sm.set_float_level_overlays_for_test(false);
    handle.inject_window_kind(300, crate::adapter::WindowClass::Untracked);
    sm.on_window_created(300, 42);
    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(300).is_some()),
        "with the gate off, a non-float-listed untracked window tiles again"
    );
}

#[test]
fn untracked_windows_are_raised_after_a_workspace_switch() {
    // The raise funnel: macOS leaves stacking unspecified after un-hide,
    // so every untracked window on the monitor whose visible workspace
    // changes gets one AXRaise after the switch completes — the popup
    // switch-back-raise mechanism, extended to the untracked class.
    let (mut sm, handle) = setup_with_handle(1);
    handle.inject_window_kind(300, crate::adapter::WindowClass::Untracked);
    sm.on_window_created(300, 42);
    handle.inject_rect(300, Rect::new(800.0, 400.0, 200.0, 100.0));
    sm.on_command(Command::Workspace { id: 2 }, None);
    assert!(
        handle.raised().contains(&300),
        "the untracked overlay is re-raised above the newly shown workspace"
    );
}
