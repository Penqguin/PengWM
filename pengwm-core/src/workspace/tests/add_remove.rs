use super::common::*;
use crate::tree::{Direction, NodeData, SplitDirection};
use crate::workspace::MIN_PANE_SHARE;

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
    // Auto-layout master-stack: VSplit(100, HSplit(200, 300, 400)).
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
    // After master-stack collapse, root is Vertical with remaining windows.
    // Depending on collapse strategy it may be 2 (VSplit + stack) or 3 (flat).
    let child_len = ws.arena.get(root).unwrap().children.len();
    assert!(
        child_len == 2 || child_len == 3,
        "root should have 2 or 3 children after master-stack collapse, got {}",
        child_len
    );
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
        assert!((ratios[0] - 0.5).abs() < f64::EPSILON);
        assert!((ratios[1] - 0.5).abs() < f64::EPSILON);
    } else {
        panic!("expected split");
    }
}

#[test]
fn resize_grows_focused_one_step() {
    let mut ws = make_workspace();
    ws.add_window(100, Some(SplitDirection::Vertical));
    ws.add_window(200, Some(SplitDirection::Vertical));
    ws.focus_window(100);
    assert!(ws.resize_focused(Direction::Right));
    assert!(ws.root.is_some());
    let root = ws.root.unwrap();
    if let NodeData::Split { ratios, .. } = &ws.arena.get(root).unwrap().data {
        assert!((ratios[0] - 0.55).abs() < 1e-9);
        assert!((ratios[1] - 0.45).abs() < 1e-9);
    } else {
        panic!("expected split");
    }
}

#[test]
fn resize_clamps_at_minimum_share() {
    let mut ws = make_workspace();
    ws.add_window(100, Some(SplitDirection::Vertical));
    ws.add_window(200, Some(SplitDirection::Vertical));
    ws.focus_window(100);
    for _ in 0..20 {
        ws.resize_focused(Direction::Right);
    }
    let root = ws.root.unwrap();
    if let NodeData::Split { ratios, .. } = &ws.arena.get(root).unwrap().data {
        assert!(ratios[1] >= MIN_PANE_SHARE - 1e-9);
    } else {
        panic!("expected split");
    }
    assert!(
        !ws.resize_focused(Direction::Right),
        "donor exhausted: no-op"
    );
}

#[test]
fn resize_noop_without_split_on_axis() {
    let mut ws = make_workspace();
    ws.add_window(100, None);
    assert!(!ws.resize_focused(Direction::Right));
}

#[test]
fn resize_survives_add_and_remove() {
    let mut ws = make_workspace();
    ws.add_window(100, Some(SplitDirection::Vertical));
    ws.add_window(200, Some(SplitDirection::Vertical));
    ws.focus_window(100);
    ws.resize_focused(Direction::Right);
    // Add: newcomer takes an equal slice, relative sizes preserved.
    ws.focus_window(200);
    ws.add_window(300, Some(SplitDirection::Vertical));
    let root = ws.root.unwrap();
    if let NodeData::Split { ratios, .. } = &ws.arena.get(root).unwrap().data {
        assert_eq!(ratios.len(), 3);
        assert!(
            ratios[0] > ratios[1],
            "manual 0.55/0.45 tilt survives add: {ratios:?}"
        );
    } else {
        panic!("expected split");
    }
    // Remove: survivors rescale proportionally, tilt preserved.
    ws.remove_window(300);
    let root = ws.root.unwrap();
    if let NodeData::Split { ratios, .. } = &ws.arena.get(root).unwrap().data {
        assert_eq!(ratios.len(), 2);
        assert!(ratios[0] > ratios[1], "tilt survives remove: {ratios:?}");
        let sum: f64 = ratios.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    } else {
        panic!("expected split");
    }
}

#[test]
fn resize_outward_at_edge_shrinks() {
    let mut ws = make_workspace();
    ws.add_window(100, Some(SplitDirection::Vertical));
    ws.add_window(200, Some(SplitDirection::Vertical));
    // Left edge + Left shrinks focused (was wrap-around grow).
    ws.focus_window(100);
    assert!(ws.resize_focused(Direction::Left));
    let root = ws.root.unwrap();
    if let NodeData::Split { ratios, .. } = &ws.arena.get(root).unwrap().data {
        assert!((ratios[0] - 0.45).abs() < 1e-9);
        assert!((ratios[1] - 0.55).abs() < 1e-9);
    } else {
        panic!("expected split");
    }
    // Right edge + Right shrinks focused.
    ws.focus_window(200);
    assert!(ws.resize_focused(Direction::Right));
    let root = ws.root.unwrap();
    if let NodeData::Split { ratios, .. } = &ws.arena.get(root).unwrap().data {
        assert!((ratios[0] - 0.50).abs() < 1e-9);
        assert!((ratios[1] - 0.50).abs() < 1e-9);
    } else {
        panic!("expected split");
    }
}

#[test]
fn resize_right_pane_left_grows() {
    let mut ws = make_workspace();
    ws.add_window(100, Some(SplitDirection::Vertical));
    ws.add_window(200, Some(SplitDirection::Vertical));
    // Right-side window + Left pushes its left edge left (grows).
    ws.focus_window(200);
    assert!(ws.resize_focused(Direction::Left));
    let root = ws.root.unwrap();
    if let NodeData::Split { ratios, .. } = &ws.arena.get(root).unwrap().data {
        assert!((ratios[0] - 0.45).abs() < 1e-9);
        assert!((ratios[1] - 0.55).abs() < 1e-9);
    } else {
        panic!("expected split");
    }
}
