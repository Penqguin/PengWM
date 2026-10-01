use super::common::*;
use crate::tree::{NodeData, SplitDirection};
use crate::workspace::{LayoutPreset, Workspace};

fn preset_workspace(n: u32) -> Workspace {
    let mut ws = make_workspace();
    for i in 0..n {
        ws.add_window(100 + i as u64, None);
    }
    ws.focus_window(100);
    ws
}

fn root_split(ws: &Workspace) -> (SplitDirection, Vec<f64>, usize) {
    let root = ws.root.unwrap();
    match &ws.arena.get(root).unwrap().data {
        NodeData::Split { direction, ratios } => (
            *direction,
            ratios.clone(),
            ws.arena.get(root).unwrap().children.len(),
        ),
        NodeData::Window { .. } => panic!("expected root split"),
    }
}

#[test]
fn preset_even_horizontal_is_flat_equal() {
    let mut ws = preset_workspace(3);
    ws.apply_preset(LayoutPreset::EvenHorizontal, 0.6);
    let (dir, ratios, n) = root_split(&ws);
    assert_eq!(dir, SplitDirection::Horizontal);
    assert_eq!(n, 3);
    for r in &ratios {
        assert!((r - 1.0 / 3.0).abs() < 1e-9);
    }
    assert_eq!(ws.focused_window_id(), Some(100));
}

#[test]
fn preset_even_vertical_is_flat_equal() {
    let mut ws = preset_workspace(4);
    ws.apply_preset(LayoutPreset::EvenVertical, 0.6);
    let (dir, ratios, n) = root_split(&ws);
    assert_eq!(dir, SplitDirection::Vertical);
    assert_eq!(n, 4);
    assert!(ratios.iter().all(|r| (r - 0.25).abs() < 1e-9));
}

#[test]
fn preset_main_vertical_honors_ratio() {
    let mut ws = preset_workspace(3);
    ws.apply_preset(LayoutPreset::MainVertical, 0.6);
    let (dir, ratios, n) = root_split(&ws);
    assert_eq!(dir, SplitDirection::Vertical);
    assert_eq!(n, 2);
    assert!((ratios[0] - 0.6).abs() < 1e-9);
    assert!((ratios[1] - 0.4).abs() < 1e-9);
    // First window is main (left), the rest stack horizontally on the right.
    let root = ws.root.unwrap();
    let children = ws.arena.get(root).unwrap().children.clone();
    assert_eq!(ws.focused_window_id(), Some(100));
    let main_wid = match &ws.arena.get(children[0]).unwrap().data {
        NodeData::Window { window_id, .. } => *window_id,
        _ => panic!("main should be a window"),
    };
    assert_eq!(main_wid, 100);
    assert!(matches!(
        &ws.arena.get(children[1]).unwrap().data,
        NodeData::Split {
            direction: SplitDirection::Horizontal,
            ..
        }
    ));
}

#[test]
fn preset_main_horizontal_mirrors() {
    let mut ws = preset_workspace(2);
    ws.apply_preset(LayoutPreset::MainHorizontal, 0.7);
    let (dir, ratios, n) = root_split(&ws);
    assert_eq!(dir, SplitDirection::Horizontal);
    assert_eq!(n, 2);
    assert!((ratios[0] - 0.7).abs() < 1e-9);
}

#[test]
fn preset_tiled_is_grid() {
    let mut ws = preset_workspace(4);
    ws.apply_preset(LayoutPreset::Tiled, 0.6);
    let (dir, _, n) = root_split(&ws);
    assert_eq!(dir, SplitDirection::Horizontal);
    assert_eq!(n, 2, "2x2 grid: two rows under the outer split");
    let root = ws.root.unwrap();
    for &row in &ws.arena.get(root).unwrap().children.clone() {
        assert!(matches!(
            &ws.arena.get(row).unwrap().data,
            NodeData::Split {
                direction: SplitDirection::Vertical,
                ..
            }
        ));
    }
    let rects = ws.layout(0.0, 0.0);
    assert_eq!(rects.len(), 4);
}

#[test]
fn preset_clears_magnify_and_clamps_ratio() {
    let mut ws = preset_workspace(2);
    ws.toggle_magnify();
    assert!(ws.magnified.is_some());
    ws.apply_preset(LayoutPreset::MainVertical, 99.0);
    assert!(ws.magnified.is_none());
    let (_, ratios, _) = root_split(&ws);
    assert!(
        (ratios[0] - 0.8).abs() < 1e-9,
        "ratio clamps to 0.8, got {}",
        ratios[0]
    );
}

#[test]
fn preset_empty_workspace_is_noop() {
    let mut ws = make_workspace();
    ws.apply_preset(LayoutPreset::Tiled, 0.6);
    assert!(ws.root.is_none());
}

#[test]
fn cycle_preset_wraps_in_all_order() {
    let mut ws = preset_workspace(2);
    ws.apply_preset(LayoutPreset::Tiled, 0.6);
    let next = ws.cycle_preset(0.6);
    assert_eq!(next, LayoutPreset::EvenHorizontal);
    // Walk through the rest and wrap back to Tiled.
    for expected in [
        LayoutPreset::EvenVertical,
        LayoutPreset::MainHorizontal,
        LayoutPreset::MainVertical,
        LayoutPreset::Tiled,
    ] {
        assert_eq!(ws.cycle_preset(0.6), expected);
    }
}

#[test]
fn magnify_pins_and_clears_on_remove() {
    let mut ws = preset_workspace(2);
    ws.focus_window(100);
    ws.toggle_magnify();
    assert_eq!(ws.magnified, Some(100));
    ws.focus_window(101);
    assert_eq!(ws.magnified, Some(100), "pinned across focus change");
    ws.toggle_magnify();
    assert_eq!(ws.magnified, Some(101), "toggle pins newly focused");
    ws.remove_window(101);
    assert!(ws.magnified.is_none(), "close clears the pin");
}

#[test]
fn preset_single_window_stays_single() {
    let mut ws = preset_workspace(1);
    ws.apply_preset(LayoutPreset::MainVertical, 0.6);
    assert_eq!(ws.window_count(), 1);
    assert!(matches!(
        &ws.arena.get(ws.root.unwrap()).unwrap().data,
        NodeData::Window { .. }
    ));
}
