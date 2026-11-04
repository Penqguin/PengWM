use super::common::*;
#[test]
fn popup_is_centered_overlay_above_tiling() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.add_window(200, None);
    assert!(ws.add_popup(300));
    let rects = ws.layout(10.0, 10.0);
    let popup = &rects[&300];
    // Usable 1920x1080 inset by 10 → 1900x1060; popup 75% centered.
    assert_eq!(popup.width, 1900.0 * 0.75);
    assert_eq!(popup.height, 1060.0 * 0.75);
    assert_eq!(popup.x, 10.0 + (1900.0 - 1900.0 * 0.75) / 2.0);
    assert_eq!(popup.y, 10.0 + (1060.0 - 1060.0 * 0.75) / 2.0);
    // Tiling underneath is unaffected: tree members keep tiled rects.
    let tile = &rects[&100];
    assert_eq!(tile.width, 945.0, "master-stack half minus gap");
    assert_ne!(tile, popup);
}

#[test]
fn popup_not_counted_against_tiles_and_membership_idempotent() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    assert!(ws.add_popup(300));
    assert!(!ws.add_popup(300), "idempotent");
    assert!(!ws.add_popup(100), "refuses a tiled window");
    assert_eq!(ws.window_count(), 1, "popups don't consume max_tiles");
    assert!(ws.is_popup(300));
    assert_eq!(ws.popup_ids(), vec![300]);
    assert!(ws.remove_popup(300));
    assert!(!ws.remove_popup(300), "remove idempotent");
    assert!(ws.popup_ids().is_empty());
}

#[test]
fn popup_ratio_controls_overlay_size() {
    let mut ws = make_workspace();
    ws.popup_ratio = 0.5;
    ws.add_popup(300);
    let rects = ws.layout(10.0, 10.0);
    assert_eq!(rects[&300].width, 1900.0 * 0.5);
    assert_eq!(rects[&300].height, 1060.0 * 0.5);
}

#[test]
fn popup_ratio_clamped_at_use() {
    let mut ws = make_workspace();
    ws.popup_ratio = 0.0;
    ws.add_popup(300);
    let rects = ws.layout(10.0, 10.0);
    assert_eq!(
        rects[&300].width,
        1900.0 * 0.1,
        "floor keeps the overlay visible"
    );
}

#[test]
fn magnify_and_popup_share_overlay_geometry() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.toggle_magnify();
    assert!(ws.add_popup(300));
    let rects = ws.layout(0.0, 0.0);
    assert_eq!(rects[&100], rects[&300], "one centered-overlay computation");
}
