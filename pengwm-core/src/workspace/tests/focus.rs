use super::common::*;
use crate::tree::{Direction, NodeData, SplitDirection};
use crate::workspace::Workspace;

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
fn apply_split_direction_pends_when_window_focused() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    ws.focus_window(a);
    ws.apply_split_direction(SplitDirection::Horizontal);
    let c = ws.add_window(300, None);
    let parent = ws.arena.get(c).unwrap().parent.unwrap();
    assert!(
        matches!(
            &ws.arena.get(parent).unwrap().data,
            NodeData::Split {
                direction: SplitDirection::Horizontal,
                ..
            }
        ),
        "pending split direction drives the next window's parent"
    );
}

#[test]
fn apply_split_direction_reorients_focused_split() {
    let mut ws = make_workspace();
    let a = ws.add_window(100, None);
    let _b = ws.add_window(200, None);
    let root_split = ws.arena.get(a).unwrap().parent.unwrap();
    assert!(matches!(
        &ws.arena.get(root_split).unwrap().data,
        NodeData::Split {
            direction: SplitDirection::Vertical,
            ..
        }
    ));
    ws.focused_node = Some(root_split);
    ws.apply_split_direction(SplitDirection::Horizontal);
    assert!(matches!(
        &ws.arena.get(root_split).unwrap().data,
        NodeData::Split {
            direction: SplitDirection::Horizontal,
            ..
        }
    ));
}
