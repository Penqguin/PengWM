use super::*;
use crate::config::BarPosition;
use crate::tree::{Arena, NodeData, NodeId, SplitDirection};
use std::collections::HashMap;

fn single_window_arena() -> (Arena, NodeId) {
    let mut arena = Arena::new();
    let id = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: true,
    });
    (arena, id)
}

fn two_window_arena() -> (Arena, NodeId) {
    let mut arena = Arena::new();
    let a = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: false,
    });
    let b = arena.alloc(NodeData::Window {
        window_id: 2,
        is_focused: false,
    });
    let split = arena.alloc(NodeData::Split {
        direction: SplitDirection::Vertical,
        ratios: vec![0.5, 0.5],
    });
    arena.get_mut(a).unwrap().parent = Some(split);
    arena.get_mut(b).unwrap().parent = Some(split);
    arena.get_mut(split).unwrap().children = vec![a, b];
    (arena, split)
}

fn three_window_flattened_arena() -> (Arena, NodeId) {
    let mut arena = Arena::new();
    let a = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: false,
    });
    let b = arena.alloc(NodeData::Window {
        window_id: 2,
        is_focused: false,
    });
    let c = arena.alloc(NodeData::Window {
        window_id: 3,
        is_focused: false,
    });
    let split = arena.alloc(NodeData::Split {
        direction: SplitDirection::Vertical,
        ratios: vec![1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    });
    arena.get_mut(a).unwrap().parent = Some(split);
    arena.get_mut(b).unwrap().parent = Some(split);
    arena.get_mut(c).unwrap().parent = Some(split);
    arena.get_mut(split).unwrap().children = vec![a, b, c];
    (arena, split)
}

#[test]
fn layout_single_window() {
    let (arena, root) = single_window_arena();
    let mut output = HashMap::new();
    let bounding = inset_rect(Rect::new(0.0, 0.0, 1920.0, 1080.0), 10.0);

    calculate_layout(root, bounding, &arena, &mut output, 10.0);

    assert_eq!(output.len(), 1);
    let r = output[&1];
    assert_eq!(r.x, 10.0);
    assert_eq!(r.y, 10.0);
    assert_eq!(r.width, 1900.0);
    assert_eq!(r.height, 1060.0);
}

#[test]
fn layout_single_window_zero_gap() {
    let (arena, root) = single_window_arena();
    let mut output = HashMap::new();
    let bounding = Rect::new(0.0, 0.0, 100.0, 100.0);

    calculate_layout(root, bounding, &arena, &mut output, 0.0);

    assert_eq!(output[&1], Rect::new(0.0, 0.0, 100.0, 100.0));
}

#[test]
fn layout_two_vertical_no_gap() {
    let (arena, root) = two_window_arena();
    let mut output = HashMap::new();
    let bounding = Rect::new(0.0, 0.0, 100.0, 100.0);

    calculate_layout(root, bounding, &arena, &mut output, 0.0);

    assert_eq!(output[&1], Rect::new(0.0, 0.0, 50.0, 100.0));
    assert_eq!(output[&2], Rect::new(50.0, 0.0, 50.0, 100.0));
}

#[test]
fn layout_two_vertical_with_gap() {
    let (arena, root) = two_window_arena();
    let mut output = HashMap::new();
    let bounding = inset_rect(Rect::new(0.0, 0.0, 200.0, 100.0), 10.0);

    calculate_layout(root, bounding, &arena, &mut output, 10.0);

    let r1 = output[&1];
    let r2 = output[&2];
    assert!((r1.x - 10.0).abs() < 1e-6);
    assert!((r1.width - 85.0).abs() < 1e-4);
    assert!((r2.x - 105.0).abs() < 1e-4);
    assert!((r2.width - 85.0).abs() < 1e-4);
}

#[test]
fn layout_two_horizontal() {
    let mut arena = Arena::new();
    let a = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: false,
    });
    let b = arena.alloc(NodeData::Window {
        window_id: 2,
        is_focused: false,
    });
    let split = arena.alloc(NodeData::Split {
        direction: SplitDirection::Horizontal,
        ratios: vec![0.5, 0.5],
    });
    arena.get_mut(a).unwrap().parent = Some(split);
    arena.get_mut(b).unwrap().parent = Some(split);
    arena.get_mut(split).unwrap().children = vec![a, b];

    let mut output = HashMap::new();
    calculate_layout(
        split,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        &arena,
        &mut output,
        0.0,
    );

    assert_eq!(output[&1], Rect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(output[&2], Rect::new(0.0, 50.0, 100.0, 50.0));
}

#[test]
fn layout_three_vertical_equal() {
    let (arena, root) = three_window_flattened_arena();
    let mut output = HashMap::new();
    let bounding = Rect::new(0.0, 0.0, 300.0, 100.0);

    calculate_layout(root, bounding, &arena, &mut output, 0.0);

    assert!((output[&1].width - 100.0).abs() < 1e-4);
    assert!((output[&2].width - 100.0).abs() < 1e-4);
    assert!((output[&3].width - 100.0).abs() < 1e-4);
    assert_eq!(output[&1].x, 0.0);
    assert_eq!(output[&2].x, output[&1].x + output[&1].width);
    assert_eq!(output[&3].x, output[&2].x + output[&2].width);
}

#[test]
fn layout_three_vertical_with_gap() {
    let (arena, root) = three_window_flattened_arena();
    let mut output = HashMap::new();
    let bounding = inset_rect(Rect::new(0.0, 0.0, 320.0, 100.0), 10.0);

    calculate_layout(root, bounding, &arena, &mut output, 10.0);

    let r1 = output[&1];
    let r2 = &output[&2];
    let r3 = &output[&3];
    assert_eq!(r1.x, 10.0);
    let expected_width = (300.0 - 20.0) / 3.0;
    assert!(
        (r1.width - expected_width).abs() < 1e-4,
        "window 1 width should be ~{}, got {}",
        expected_width,
        r1.width
    );
    assert_eq!(
        r2.x,
        r1.x + r1.width + 10.0,
        "expected 10px gap after window 1"
    );
    assert!(
        (r2.width - expected_width).abs() < 1e-4,
        "window 2 width should be ~{}, got {}",
        expected_width,
        r2.width
    );
    assert_eq!(
        r3.x,
        r2.x + r2.width + 10.0,
        "expected 10px gap after window 2"
    );
    assert_eq!(
        r3.x + r3.width,
        310.0,
        "right edge should end at 310 (320 - 10 outer gap)"
    );
}

#[test]
fn layout_non_uniform_ratios() {
    let mut arena = Arena::new();
    let a = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: false,
    });
    let b = arena.alloc(NodeData::Window {
        window_id: 2,
        is_focused: false,
    });
    let split = arena.alloc(NodeData::Split {
        direction: SplitDirection::Vertical,
        ratios: vec![0.7, 0.3],
    });
    arena.get_mut(a).unwrap().parent = Some(split);
    arena.get_mut(b).unwrap().parent = Some(split);
    arena.get_mut(split).unwrap().children = vec![a, b];

    let mut output = HashMap::new();
    calculate_layout(
        split,
        Rect::new(0.0, 0.0, 200.0, 100.0),
        &arena,
        &mut output,
        0.0,
    );

    assert!(
        (output[&1].width - 140.0).abs() < 1e-4,
        "window 1 width: expected ~140, got {}",
        output[&1].width
    );
    assert!(
        (output[&2].width - 60.0).abs() < 1e-4,
        "window 2 width: expected ~60, got {}",
        output[&2].width
    );
    assert_eq!(output[&1].x, 0.0);
    assert_eq!(output[&2].x, output[&1].x + output[&1].width);
}

#[test]
fn layout_nested() {
    let mut arena = Arena::new();
    let a = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: false,
    });
    let b = arena.alloc(NodeData::Window {
        window_id: 2,
        is_focused: false,
    });
    let c = arena.alloc(NodeData::Window {
        window_id: 3,
        is_focused: false,
    });
    let inner = arena.alloc(NodeData::Split {
        direction: SplitDirection::Horizontal,
        ratios: vec![0.5, 0.5],
    });
    let outer = arena.alloc(NodeData::Split {
        direction: SplitDirection::Vertical,
        ratios: vec![0.5, 0.5],
    });

    arena.get_mut(a).unwrap().parent = Some(inner);
    arena.get_mut(b).unwrap().parent = Some(inner);
    arena.get_mut(c).unwrap().parent = Some(outer);
    arena.get_mut(inner).unwrap().parent = Some(outer);
    arena.get_mut(inner).unwrap().children = vec![a, b];
    arena.get_mut(outer).unwrap().children = vec![inner, c];

    let mut output = HashMap::new();
    calculate_layout(
        outer,
        Rect::new(0.0, 0.0, 200.0, 100.0),
        &arena,
        &mut output,
        0.0,
    );

    assert_eq!(output[&1], Rect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(output[&2], Rect::new(0.0, 50.0, 100.0, 50.0));
    assert_eq!(output[&3], Rect::new(100.0, 0.0, 100.0, 100.0));
}

#[test]
fn layout_four_alternating_nested_no_gap() {
    let mut arena = Arena::new();
    let a = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: false,
    });
    let b = arena.alloc(NodeData::Window {
        window_id: 2,
        is_focused: false,
    });
    let c = arena.alloc(NodeData::Window {
        window_id: 3,
        is_focused: false,
    });
    let d = arena.alloc(NodeData::Window {
        window_id: 4,
        is_focused: false,
    });

    // Build the alternating tree: VSplit(A, HSplit(B, VSplit(C, D)))
    let inner_v = arena.alloc(NodeData::Split {
        direction: SplitDirection::Vertical,
        ratios: vec![0.5, 0.5],
    });
    let inner_h = arena.alloc(NodeData::Split {
        direction: SplitDirection::Horizontal,
        ratios: vec![0.5, 0.5],
    });
    let outer_v = arena.alloc(NodeData::Split {
        direction: SplitDirection::Vertical,
        ratios: vec![0.5, 0.5],
    });

    // inner_v: C, D
    arena.get_mut(c).unwrap().parent = Some(inner_v);
    arena.get_mut(d).unwrap().parent = Some(inner_v);
    arena.get_mut(inner_v).unwrap().children = vec![c, d];

    // inner_h: B, inner_v
    arena.get_mut(b).unwrap().parent = Some(inner_h);
    arena.get_mut(inner_v).unwrap().parent = Some(inner_h);
    arena.get_mut(inner_h).unwrap().children = vec![b, inner_v];

    // outer_v: A, inner_h
    arena.get_mut(a).unwrap().parent = Some(outer_v);
    arena.get_mut(inner_h).unwrap().parent = Some(outer_v);
    arena.get_mut(outer_v).unwrap().children = vec![a, inner_h];

    let bounding = inset_rect(Rect::new(0.0, 0.0, 1920.0, 1080.0), 10.0);
    let mut output = HashMap::new();
    calculate_layout(outer_v, bounding, &arena, &mut output, 5.0);

    assert_eq!(output.len(), 4);

    // Verify no overlap between any pair of windows
    let rects: Vec<_> = output.iter().collect();
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            let (_, r1) = rects[i];
            let (_, r2) = rects[j];
            let overlap_x = r1.x < r2.x + r2.width && r2.x < r1.x + r1.width;
            let overlap_y = r1.y < r2.y + r2.height && r2.y < r1.y + r1.height;
            if overlap_x && overlap_y {
                panic!(
                    "Overlap between window {} and {}: {:?} vs {:?}",
                    rects[i].0, rects[j].0, r1, r2
                );
            }
        }
    }
}

#[test]
fn layout_four_flattened_vertical_with_gap() {
    let mut arena = Arena::new();
    let a = arena.alloc(NodeData::Window {
        window_id: 1,
        is_focused: false,
    });
    let b = arena.alloc(NodeData::Window {
        window_id: 2,
        is_focused: false,
    });
    let c = arena.alloc(NodeData::Window {
        window_id: 3,
        is_focused: false,
    });
    let d = arena.alloc(NodeData::Window {
        window_id: 4,
        is_focused: false,
    });

    let split = arena.alloc(NodeData::Split {
        direction: SplitDirection::Vertical,
        ratios: vec![0.25, 0.25, 0.25, 0.25],
    });

    arena.get_mut(a).unwrap().parent = Some(split);
    arena.get_mut(b).unwrap().parent = Some(split);
    arena.get_mut(c).unwrap().parent = Some(split);
    arena.get_mut(split).unwrap().children = vec![a, b, c, d];

    let bounding = Rect::new(0.0, 0.0, 1000.0, 100.0);
    let mut output = HashMap::new();
    calculate_layout(split, bounding, &arena, &mut output, 5.0);

    assert_eq!(output.len(), 4);

    // Verify gaps between adjacent windows
    let r1 = &output[&1];
    let r2 = &output[&2];
    let r3 = &output[&3];
    let r4 = &output[&4];

    // Gap between col 1 and col 2
    assert!(
        (r2.x - (r1.x + r1.width) - 5.0).abs() < 1e-4,
        "Expected 5px gap between 1 and 2, got {}",
        r2.x - (r1.x + r1.width)
    );
    // Gap between col 2 and col 3
    assert!(
        (r3.x - (r2.x + r2.width) - 5.0).abs() < 1e-4,
        "Expected 5px gap between 2 and 3, got {}",
        r3.x - (r2.x + r2.width)
    );
    // Gap between col 3 and col 4
    assert!(
        (r4.x - (r3.x + r3.width) - 5.0).abs() < 1e-4,
        "Expected 5px gap between 3 and 4, got {}",
        r4.x - (r3.x + r3.width)
    );

    // Verify no overlap
    for i in 1..=4 {
        for j in i + 1..=4 {
            let r1 = &output[&i];
            let r2 = &output[&j];
            let overlap_x = r1.x < r2.x + r2.width && r2.x < r1.x + r1.width;
            let overlap_y = r1.y < r2.y + r2.height && r2.y < r1.y + r1.height;
            assert!(
                !(overlap_x && overlap_y),
                "Overlap between {} and {}: {:?} vs {:?}",
                i,
                j,
                r1,
                r2
            );
        }
    }
}

#[test]
fn screen_local_to_global_offsets() {
    let local = Rect::new(10.0, 20.0, 100.0, 50.0);
    let global = screen_local_to_global(local, (1440, 0));
    assert_eq!(global, Rect::new(1450.0, 20.0, 100.0, 50.0));
}

#[test]
fn window_at_point_hits_containing_window() {
    let mut rects = HashMap::new();
    rects.insert(1, Rect::new(0.0, 0.0, 100.0, 100.0));
    rects.insert(2, Rect::new(100.0, 0.0, 100.0, 100.0));
    assert_eq!(window_at_point(&rects, 50.0, 50.0, 99), Some(1));
    assert_eq!(window_at_point(&rects, 150.0, 50.0, 99), Some(2));
}

#[test]
fn window_at_point_ignores_excluded_and_misses() {
    let mut rects = HashMap::new();
    rects.insert(1, Rect::new(0.0, 0.0, 100.0, 100.0));
    rects.insert(2, Rect::new(100.0, 0.0, 100.0, 100.0));
    assert_eq!(window_at_point(&rects, 150.0, 50.0, 1), Some(2));
    assert_eq!(window_at_point(&rects, 250.0, 50.0, 1), None);
}

#[test]
fn bar_strip_rect_on_every_edge() {
    let (origin, size) = ((1440, 0), (1920u32, 1080u32));
    assert_eq!(
        bar_strip_rect(origin, size, BarPosition::Top, 32),
        Rect::new(1440.0, 0.0, 1920.0, 32.0)
    );
    assert_eq!(
        bar_strip_rect(origin, size, BarPosition::Bottom, 32),
        Rect::new(1440.0, 1080.0 - 32.0, 1920.0, 32.0)
    );
    assert_eq!(
        bar_strip_rect(origin, size, BarPosition::Left, 24),
        Rect::new(1440.0, 0.0, 24.0, 1080.0)
    );
    assert_eq!(
        bar_strip_rect(origin, size, BarPosition::Right, 24),
        Rect::new(1440.0 + 1920.0 - 24.0, 0.0, 24.0, 1080.0)
    );
}

#[test]
fn bar_strip_rect_clamps_thickness_to_one() {
    assert_eq!(
        bar_strip_rect((0, 0), (100, 100), BarPosition::Top, 0),
        Rect::new(0.0, 0.0, 100.0, 1.0)
    );
}

#[test]
fn subtract_strip_top_cuts_correctly() {
    let monitor = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    let strip = Rect::new(0.0, 0.0, 1920.0, 32.0);
    assert_eq!(
        subtract_strip(monitor, strip),
        Rect::new(0.0, 32.0, 1920.0, 1048.0)
    );
}

#[test]
fn subtract_strip_bottom_and_side() {
    let monitor = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert_eq!(
        subtract_strip(monitor, Rect::new(0.0, 90.0, 100.0, 10.0)),
        Rect::new(0.0, 0.0, 100.0, 90.0)
    );
    assert_eq!(
        subtract_strip(monitor, Rect::new(0.0, 0.0, 10.0, 100.0)),
        Rect::new(10.0, 0.0, 90.0, 100.0)
    );
    assert_eq!(
        subtract_strip(monitor, Rect::new(90.0, 0.0, 10.0, 100.0)),
        Rect::new(0.0, 0.0, 90.0, 100.0)
    );
}

#[test]
fn subtract_strip_ignores_non_edge() {
    let monitor = Rect::new(0.0, 0.0, 100.0, 100.0);
    let mid = Rect::new(10.0, 10.0, 10.0, 10.0);
    assert_eq!(subtract_strip(monitor, mid), monitor);
}

#[test]
fn hidden_rect_bottom_right_single_display() {
    assert_eq!(
        hidden_rect((0, 0), (1920, 1080)),
        Rect::new(1919.0, 1079.0, 1.0, 1.0)
    );
}

#[test]
fn hidden_rect_per_monitor_second_display() {
    assert_eq!(
        hidden_rect((1920, 0), (1920, 1080)),
        Rect::new(3839.0, 1079.0, 1.0, 1.0)
    );
}

#[test]
fn hidden_rect_negative_origin() {
    assert_eq!(
        hidden_rect((-1920, 0), (1920, 1080)),
        Rect::new(-1.0, 1079.0, 1.0, 1.0)
    );
}

#[test]
fn far_offscreen_is_negative_100k() {
    assert_eq!(
        far_offscreen_rect(),
        Rect::new(-100_000.0, -100_000.0, 0.0, 0.0)
    );
}

#[test]
fn rects_close_within_epsilon() {
    let a = Rect::new(0.0, 0.0, 960.0, 1040.0);
    let b = Rect::new(1.0, 1.0, 961.0, 1039.0);
    assert!(rects_close(a, b, 2.0));
    assert!(!rects_close(a, b, 0.5));
}

#[test]
fn rects_close_rejects_far_drift() {
    let a = Rect::new(0.0, 0.0, 960.0, 1040.0);
    let b = Rect::new(50.0, 0.0, 960.0, 1040.0);
    assert!(!rects_close(a, b, 2.0));
}

#[test]
fn write_outcome_ok_predicate() {
    assert!(WriteOutcome::Ok.is_ok());
    assert!(!WriteOutcome::Gone.is_ok());
    assert!(!WriteOutcome::Drift {
        target: Rect::new(0.0, 0.0, 1.0, 1.0),
        actual: Rect::new(9.0, 0.0, 1.0, 1.0),
    }
    .is_ok());
}

#[test]
fn rects_displaced_is_negated_close_at_layout_epsilon() {
    let a = Rect::new(0.0, 0.0, 960.0, 1040.0);
    assert!(!rects_displaced(a, a));
    assert!(!rects_displaced(a, Rect::new(1.0, 1.0, 961.0, 1039.0)));
    assert!(rects_displaced(a, Rect::new(50.0, 0.0, 960.0, 1040.0)));
    assert!(rects_displaced(a, Rect::new(0.0, 0.0, 800.0, 1040.0)));
}
