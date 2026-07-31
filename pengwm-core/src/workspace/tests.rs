use super::*;
use crate::tree::Direction;

fn make_workspace() -> Workspace {
    Workspace::new("test".into(), 1, (0, 0), (1920, 1080))
}

// -----------------------------------------------------------------------
// add_window
// -----------------------------------------------------------------------

#[test]
fn add_first_window() {
    let mut ws = make_workspace();
    let id = ws.add_window(100, None);
    assert_eq!(ws.root, Some(id));
    assert_eq!(ws.focused_node, Some(id));
    assert_eq!(ws.window_count(), 1);
}

#[test]
fn add_second_window_different_dir() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, None);
    let b = ws.add_window(200, Some(SplitDirection::Vertical));

    assert_eq!(ws.window_count(), 2);
    let parent_id = ws.arena.get(a).unwrap().parent.unwrap();
    assert!(ws.arena.is_leaf(a));
    assert!(ws.arena.is_leaf(b));
    assert_eq!(ws.arena.get(parent_id).unwrap().children.len(), 2);
}

#[test]
fn add_window_same_dir_flatten() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, Some(SplitDirection::Vertical));
    let _b = ws.add_window(200, Some(SplitDirection::Vertical));
    let c = ws.add_window(300, Some(SplitDirection::Vertical));

    assert_eq!(ws.window_count(), 3);
    let parent_a = ws.arena.get(a).unwrap().parent.unwrap();
    let parent_c = ws.arena.get(c).unwrap().parent.unwrap();
    assert_eq!(parent_a, parent_c);
    let parent = ws.arena.get(parent_a).unwrap();
    assert_eq!(parent.children.len(), 3);
}

#[test]
fn add_alternating_dir_creates_nested_split() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, Some(SplitDirection::Vertical));
    let _b = ws.add_window(200, Some(SplitDirection::Vertical));
    let c = ws.add_window(300, Some(SplitDirection::Horizontal));

    assert_eq!(ws.window_count(), 3);
    let parent_a = ws.arena.get(a).unwrap().parent.unwrap();
    let parent_c = ws.arena.get(c).unwrap().parent.unwrap();
    assert_ne!(
        parent_a, parent_c,
        "a and c should be under different splits (nested, not flattened)"
    );
    let child_list = ws.arena.get(parent_a).unwrap().children.clone();
    assert_eq!(child_list.len(), 2);
    assert!(child_list.contains(&a));
}

// -----------------------------------------------------------------------
// remove_window
// -----------------------------------------------------------------------

#[test]
fn remove_last_window() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.remove_window(100);
    assert!(ws.root.is_none());
    assert!(ws.focused_node.is_none());
    assert_eq!(ws.window_count(), 0);
}

#[test]
fn remove_window_collapse() {
    let mut ws = make_workspace();
    let _a = ws.add_window(100, Some(SplitDirection::Vertical));
    let b = ws.add_window(200, Some(SplitDirection::Vertical));
    ws.remove_window(100);

    assert_eq!(ws.window_count(), 1);
    assert_eq!(ws.arena.get(b).unwrap().parent, None);
    assert_eq!(ws.root, Some(b));
}

#[test]
fn remove_window_unfocused() {
    let mut ws = make_workspace();
    let _a = ws.add_window(100, Some(SplitDirection::Vertical));
    let b = ws.add_window(200, Some(SplitDirection::Vertical));
    ws.focus_window(200);
    ws.remove_window(100);

    assert_eq!(ws.window_count(), 1);
    assert_eq!(ws.focused_node, Some(b));
}

#[test]
fn remove_left_window_keeps_vertical_root() {
    let mut ws = make_workspace();
    // Auto-layout: VSplit(100, HSplit(200, 300)) — 100 left, 200/300 right.
    let _a = ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    let _c = ws.add_window(300, None);

    ws.remove_window(100);

    assert_eq!(ws.window_count(), 2);
    let root = ws.root.unwrap();
    let root_data = &ws.arena.get(root).unwrap().data;
    assert!(
        matches!(
            root_data,
            NodeData::Split {
                direction: SplitDirection::Vertical,
                ..
            }
        ),
        "root should stay Vertical after closing the left window, got {:?}",
        root_data
    );
    assert_eq!(ws.arena.get(root).unwrap().children.len(), 2);
    assert!(ws.find_window(200).is_some());
    assert!(ws.find_window(300).is_some());
}

#[test]
fn remove_left_window_nested_keeps_vertical_root() {
    let mut ws = make_workspace();
    // Auto-layout: VSplit(100, HSplit(200, VSplit(300, 400))).
    let _a = ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    let _c = ws.add_window(300, None);
    let _d = ws.add_window(400, None);

    ws.remove_window(100);

    assert_eq!(ws.window_count(), 3);
    let root = ws.root.unwrap();
    let root_data = &ws.arena.get(root).unwrap().data;
    assert!(
        matches!(
            root_data,
            NodeData::Split {
                direction: SplitDirection::Vertical,
                ..
            }
        ),
        "root should stay Vertical after closing the left window, got {:?}",
        root_data
    );
    // Root holds one window (200) and a nested split with 300/400.
    assert_eq!(ws.arena.get(root).unwrap().children.len(), 2);
}

#[test]
fn remove_non_left_window_still_collapses() {
    let mut ws = make_workspace();
    // VSplit(100, HSplit(200, 300))
    let _a = ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    let _c = ws.add_window(300, None);

    ws.remove_window(200);

    assert_eq!(ws.window_count(), 2);
    let root = ws.root.unwrap();
    let root_data = &ws.arena.get(root).unwrap().data;
    assert!(
        matches!(
            root_data,
            NodeData::Split {
                direction: SplitDirection::Vertical,
                ..
            }
        ),
        "closing the top-right window should keep the vertical root, got {:?}",
        root_data
    );
    assert!(ws.find_window(100).is_some());
    assert!(ws.find_window(300).is_some());
}

#[test]
fn remove_window_rebalance() {
    let mut ws = make_workspace();
    ws.add_window(100, Some(SplitDirection::Vertical));
    ws.add_window(200, Some(SplitDirection::Vertical));
    ws.add_window(300, Some(SplitDirection::Vertical));
    ws.remove_window(100);

    assert_eq!(ws.window_count(), 2);
    let parent = ws.arena.get(ws.root.unwrap()).unwrap();
    if let NodeData::Split { ratios, .. } = &parent.data {
        assert!((ratios[0] - 0.5).abs() < f32::EPSILON);
        assert!((ratios[1] - 0.5).abs() < f32::EPSILON);
    } else {
        panic!("expected split");
    }
}

// -----------------------------------------------------------------------
// focus_window
// -----------------------------------------------------------------------

#[test]
fn focus_toggle() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, Some(SplitDirection::Vertical));
    let b = ws.add_window(200, Some(SplitDirection::Vertical));

    ws.focus_window(100);
    assert_eq!(ws.focused_node, Some(a));
    assert!(matches!(
        &ws.arena.get(a).unwrap().data,
        NodeData::Window {
            is_focused: true,
            ..
        }
    ));
    assert!(matches!(
        &ws.arena.get(b).unwrap().data,
        NodeData::Window {
            is_focused: false,
            ..
        }
    ));

    ws.focus_window(200);
    assert_eq!(ws.focused_node, Some(b));
    assert!(matches!(
        &ws.arena.get(a).unwrap().data,
        NodeData::Window {
            is_focused: false,
            ..
        }
    ));
    assert!(matches!(
        &ws.arena.get(b).unwrap().data,
        NodeData::Window {
            is_focused: true,
            ..
        }
    ));
}

#[test]
fn focus_invalid_window() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.focus_window(999);
    assert_eq!(ws.focused_node, ws.root);
}

// -----------------------------------------------------------------------
// find_window / all_windows
// -----------------------------------------------------------------------

#[test]
fn find_window_returns_correct_id() {
    let mut ws = make_workspace();
    let _a = ws.add_window(100, None);
    let b = ws.add_window(200, Some(SplitDirection::Vertical));
    assert_eq!(ws.find_window(200), Some(b));
    assert_eq!(ws.find_window(999), None);
}

#[test]
fn all_windows_collects_all() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.add_window(200, Some(SplitDirection::Vertical));
    ws.add_window(300, Some(SplitDirection::Horizontal));
    let mut ids = ws.all_windows();
    ids.sort();
    assert_eq!(ids, vec![100, 200, 300]);
}

// -----------------------------------------------------------------------
// focus_neighbor
// -----------------------------------------------------------------------

#[test]
fn focus_neighbor_same_split() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, Some(SplitDirection::Vertical));
    let b = ws.add_window(200, Some(SplitDirection::Vertical));

    ws.focus_window(100);
    ws.focus_neighbor(Direction::Right);
    assert_eq!(ws.focused_node, Some(b));

    ws.focus_neighbor(Direction::Left);
    assert_eq!(ws.focused_node, Some(a));
}

#[test]
fn focus_neighbor_wraps_around() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, Some(SplitDirection::Vertical));
    let _b = ws.add_window(200, Some(SplitDirection::Vertical));

    ws.focus_window(200);
    ws.focus_neighbor(Direction::Right);
    assert_eq!(ws.focused_node, Some(a));
}

#[test]
fn focus_neighbor_single_window_noop() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, None);
    ws.focus_neighbor(Direction::Right);
    assert_eq!(ws.focused_node, Some(a));
}

#[test]
fn focus_neighbor_nested() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, Some(SplitDirection::Vertical));
    let b = ws.add_window(200, Some(SplitDirection::Vertical));
    let c = ws.add_window(300, Some(SplitDirection::Horizontal));

    ws.focus_window(100);
    ws.focus_neighbor(Direction::Right);
    assert_eq!(
        ws.focused_node,
        Some(b),
        "right from a should go to b (leftmost leaf of sibling split)"
    );

    ws.focus_window(200);
    ws.focus_neighbor(Direction::Down);
    assert_eq!(
        ws.focused_node,
        Some(c),
        "down from b should go to c (next sibling in horizontal split)"
    );

    ws.focus_window(200);
    ws.focus_neighbor(Direction::Left);
    assert_eq!(
        ws.focused_node,
        Some(a),
        "left from b should go to a (previous sibling in root vertical split)"
    );
}

// -----------------------------------------------------------------------
// swap_window
// -----------------------------------------------------------------------

#[test]
fn swap_window_basic() {
    let mut ws = make_workspace();
    let _a = ws.add_window(100, Some(SplitDirection::Vertical));
    let b = ws.add_window(200, Some(SplitDirection::Vertical));

    ws.focus_window(100);
    ws.swap_window(Direction::Right);
    assert_eq!(ws.window_count(), 2);
    let ws_a = ws.all_windows();
    assert!(ws_a.contains(&100));
    assert!(ws_a.contains(&200));
    assert_eq!(ws.focused_node, Some(b));
}

// -----------------------------------------------------------------------
// next_direction (via add_window with None)
// -----------------------------------------------------------------------

#[test]
fn first_split_on_widescreen_defaults_to_vertical() {
    let mut ws = Workspace::new("test".into(), 1, (0, 0), (1920, 1080));
    ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    let root = ws.root.unwrap();
    let root_node = ws.arena.get(root).unwrap();
    assert!(matches!(
        &root_node.data,
        NodeData::Split {
            direction: SplitDirection::Vertical,
            ..
        }
    ));
}

#[test]
fn auto_alternates_on_subsequent_add() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, None);
    let b = ws.add_window(200, None);
    let c = ws.add_window(300, None);

    let root = ws.root.unwrap();
    assert!(
        matches!(
            &ws.arena.get(root).unwrap().data,
            NodeData::Split {
                direction: SplitDirection::Vertical,
                ..
            }
        ),
        "first split should be Vertical on widescreen"
    );

    let parent_a = ws.arena.get(a).unwrap().parent.unwrap();
    let parent_b = ws.arena.get(b).unwrap().parent.unwrap();
    let parent_c = ws.arena.get(c).unwrap().parent.unwrap();

    assert_eq!(parent_a, root, "a should be direct child of root split");
    assert_eq!(
        parent_b, parent_c,
        "b and c should be siblings under nested split"
    );
    let inner_parent = ws.arena.get(parent_b).unwrap().parent.unwrap();
    assert_eq!(inner_parent, root, "inner split should be child of root");
}

// -----------------------------------------------------------------------
// reserved rect
// -----------------------------------------------------------------------

fn full_monitor_rect(ws: &Workspace) -> Rect {
    let (w, h) = ws.monitor_size();
    Rect::new(0.0, 0.0, w as f64, h as f64)
}

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
fn monocle_respects_reservation() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    ws.add_window(200, None);
    ws.monocle = true;
    ws.set_reserved_rect(Some(Rect::new(0.0, 0.0, 1920.0, 30.0)));
    let rects = ws.layout(5.0, 10.0);
    let focused = ws.focused_window_id().unwrap();
    let r = &rects[&focused];
    assert_eq!(r.y, 30.0 + 10.0, "monocle window also avoids the bar");
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

// -----------------------------------------------------------------------
// focused_split_direction
// -----------------------------------------------------------------------

#[test]
fn split_direction_none_for_single_window() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    assert_eq!(ws.focused_split_direction(), None);
}

#[test]
fn split_direction_of_focused_split() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    ws.focus_window(100);
    let root_split = ws.arena.get(a).unwrap().parent.unwrap();
    assert!(matches!(
        &ws.arena.get(root_split).unwrap().data,
        NodeData::Split {
            direction: SplitDirection::Vertical,
            ..
        }
    ));
    assert_eq!(ws.focused_split_direction(), Some(SplitDirection::Vertical));
}

#[test]
fn split_direction_walks_up_nested_split() {
    let mut ws = make_workspace();
    let _a = ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    let _c = ws.add_window(300, None);
    ws.focus_window(100);
    assert_eq!(
        ws.focused_split_direction(),
        Some(SplitDirection::Vertical),
        "a sits directly under the root Vertical split"
    );
    ws.focus_window(300);
    assert_eq!(
        ws.focused_split_direction(),
        Some(SplitDirection::Horizontal),
        "c sits under the nested Horizontal split"
    );
}

#[test]
fn command_toggle_bar_roundtrips() {
    let cmd = crate::command::Command::ToggleBar;
    let json = serde_json::to_string(&cmd).unwrap();
    let back: crate::command::Command = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, crate::command::Command::ToggleBar));
}

#[test]
fn bar_state_roundtrips() {
    use crate::command::{BarMessage, BarState, BarWorkspace};
    let state = BarState {
        workspaces: vec![BarWorkspace {
            name: "ws-1".into(),
            monitor_id: 1,
            window_count: 2,
            active: true,
        }],
        active_workspace: 0,
        split_direction: Some(SplitDirection::Vertical),
        rect: None,
    };
    let json = serde_json::to_string(&BarMessage::State(state)).unwrap();
    let back: BarMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, BarMessage::State(_)));
}
