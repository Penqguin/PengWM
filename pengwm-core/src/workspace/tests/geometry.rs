use super::common::*;
use crate::layout::Rect;

#[test]
fn no_reservation_uses_full_monitor() {
    let ws = make_workspace();
    let rect = ws.usable_rect(full_monitor_rect(&ws));
    assert_eq!(rect, full_monitor_rect(&ws));
}

#[test]
fn top_bar_reserves_top_strip() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.set_reserved_rect(Some(Rect::new(0.0, 0.0, 1920.0, 30.0)));
    let rects = ws.layout(5.0, 10.0);
    let r = &rects[&100];
    assert_eq!(r.y, 30.0 + 10.0, "layout starts below the bar");
    assert_eq!(r.height, 1080.0 - 30.0 - 20.0);
}

#[test]
fn bottom_bar_reserves_bottom_strip() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.set_reserved_rect(Some(Rect::new(0.0, 1050.0, 1920.0, 30.0)));
    let rects = ws.layout(5.0, 10.0);
    let r = &rects[&100];
    assert_eq!(r.y, 10.0);
    assert_eq!(r.height, 1050.0 - 20.0, "tiling stops above the bar");
}

#[test]
fn left_bar_reserves_left_strip() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.set_reserved_rect(Some(Rect::new(0.0, 0.0, 40.0, 1080.0)));
    let rects = ws.layout(5.0, 10.0);
    let r = &rects[&100];
    assert_eq!(r.x, 40.0 + 10.0);
    assert_eq!(r.width, 1920.0 - 40.0 - 20.0);
}

#[test]
fn right_bar_reserves_right_strip() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.set_reserved_rect(Some(Rect::new(1880.0, 0.0, 40.0, 1080.0)));
    let rects = ws.layout(5.0, 10.0);
    let r = &rects[&100];
    assert_eq!(r.x, 10.0);
    assert_eq!(r.width, 1880.0 - 20.0);
}

#[test]
fn magnify_is_centered_popup_respecting_reservation() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.add_window(200, None);
    ws.focus_window(100);
    ws.toggle_magnify();
    ws.set_reserved_rect(Some(Rect::new(0.0, 0.0, 1920.0, 30.0)));
    let rects = ws.layout(5.0, 10.0);
    let r = &rects[&100];
    // Usable 1920x1050 inset by 10 → 1900x1030; popup 75% centered.
    assert_eq!(r.width, 1900.0 * 0.75);
    assert_eq!(r.height, 1030.0 * 0.75);
    assert_eq!(r.y, 30.0 + 10.0 + (1030.0 - 1030.0 * 0.75) / 2.0);
    // Sibling still tiled underneath (not offscreen).
    assert!(rects.contains_key(&200));
}

#[test]
fn clearing_reservation_restores_layout() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.set_reserved_rect(Some(Rect::new(0.0, 0.0, 1920.0, 30.0)));
    ws.set_reserved_rect(None);
    let rects = ws.layout(5.0, 10.0);
    let r = &rects[&100];
    assert_eq!(r.y, 10.0);
    assert_eq!(r.height, 1080.0 - 20.0);
}
