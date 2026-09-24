use serde::{Deserialize, Serialize};

use crate::tree::{Arena, Direction, NodeData, NodeId, SplitDirection, WindowId};

use super::{equal_shares, Workspace};

/// One resize-pane step as a share of the split (5%).
pub const RESIZE_STEP: f64 = 0.05;
/// Minimum share a pane keeps when a neighbor grows into it (10%).
pub const MIN_PANE_SHARE: f64 = 0.10;
/// Clamp range for the `main-ratio` knob.
pub const MAIN_RATIO_MIN: f64 = 0.2;
pub const MAIN_RATIO_MAX: f64 = 0.8;

/// Clamp a `main-ratio` value into the sane range. Shared by the workspace
/// preset math and the daemon's config handling so the two can't drift.
pub fn clamp_main_ratio(r: f64) -> f64 {
    r.clamp(MAIN_RATIO_MIN, MAIN_RATIO_MAX)
}

/// A named tmux-style arrangement applied wholesale to a workspace tree.
/// `apply_preset` rewrites the tree into a canonical shape; the `main-*`
/// presets give the first window the `main_ratio` share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutPreset {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

impl LayoutPreset {
    /// Kebab-case name shared by keybind action strings and the CLI.
    pub fn name(self) -> &'static str {
        match self {
            LayoutPreset::EvenHorizontal => "even-horizontal",
            LayoutPreset::EvenVertical => "even-vertical",
            LayoutPreset::MainHorizontal => "main-horizontal",
            LayoutPreset::MainVertical => "main-vertical",
            LayoutPreset::Tiled => "tiled",
        }
    }

    pub fn parse_name(s: &str) -> Option<Self> {
        match s {
            "even-horizontal" => Some(LayoutPreset::EvenHorizontal),
            "even-vertical" => Some(LayoutPreset::EvenVertical),
            "main-horizontal" => Some(LayoutPreset::MainHorizontal),
            "main-vertical" => Some(LayoutPreset::MainVertical),
            "tiled" => Some(LayoutPreset::Tiled),
            _ => None,
        }
    }

    pub fn all() -> [LayoutPreset; 5] {
        [
            LayoutPreset::EvenHorizontal,
            LayoutPreset::EvenVertical,
            LayoutPreset::MainHorizontal,
            LayoutPreset::MainVertical,
            LayoutPreset::Tiled,
        ]
    }
}

impl Workspace {
    /// Rewrite the tree into the canonical shape for `preset`, keeping window
    /// order (first window becomes main for the `main-*` presets) and focus.
    /// A preset is an explicit arrangement: it clears monocle and is the only
    /// operation that re-equalizes shares.
    pub fn apply_preset(&mut self, preset: LayoutPreset, main_ratio: f64) {
        let mut order = Vec::new();
        if let Some(root) = self.root {
            self.collect_window_ids(root, &mut order);
        }
        if order.is_empty() {
            return;
        }
        let focused_wid = self.focused_window_id();
        let main = clamp_main_ratio(main_ratio);

        self.arena = Arena::new();
        self.root = None;
        self.focused_node = None;
        self.monocle = false;

        let nodes: Vec<NodeId> = order.iter().map(|&wid| self.alloc_window(wid)).collect();
        let first = nodes[0];
        let root = match preset {
            LayoutPreset::EvenHorizontal => {
                self.alloc_split(SplitDirection::Horizontal, nodes, equal_shares(order.len()))
            }
            LayoutPreset::EvenVertical => {
                self.alloc_split(SplitDirection::Vertical, nodes, equal_shares(order.len()))
            }
            LayoutPreset::MainVertical => self.alloc_main(
                SplitDirection::Vertical,
                SplitDirection::Horizontal,
                nodes,
                main,
            ),
            LayoutPreset::MainHorizontal => self.alloc_main(
                SplitDirection::Horizontal,
                SplitDirection::Vertical,
                nodes,
                main,
            ),
            LayoutPreset::Tiled => self.alloc_tiled(nodes),
        };
        self.root = Some(root);
        match focused_wid {
            Some(wid) => self.focus_window(wid),
            None => self.set_focused_node(first),
        }
    }

    /// Push the divider in `direction` one step (5%): the arrow-side edge of
    /// the focused window moves with the arrow. Inward presses grow the
    /// focused window; outward presses at the screen edge shrink it (no wrap).
    /// Clamped so no pane drops below `MIN_PANE_SHARE`. Returns false when
    /// nothing moved.
    pub fn resize_focused(&mut self, direction: Direction) -> bool {
        self.adjust_ratio(direction, RESIZE_STEP)
    }

    fn adjust_ratio(&mut self, direction: Direction, delta: f64) -> bool {
        let from = match self.focused_node {
            Some(id) => id,
            None => return false,
        };
        let axis = direction.axis();
        let forward = direction.is_forward();
        // Nearest ancestor split on this axis + which branch we're under —
        // the same walk `find_neighbor` uses, but we shift shares instead of
        // moving focus.
        let mut current = from;
        let (split_id, branch_id) = loop {
            let pid = match self.arena.get(current).and_then(|n| n.parent) {
                Some(p) => p,
                None => return false,
            };
            let is_target = matches!(
                &self.arena.get(pid).map(|p| &p.data),
                Some(NodeData::Split { direction: d, .. }) if *d == axis
            );
            if is_target {
                break (pid, current);
            }
            current = pid;
        };
        let (pos, n) = match self.arena.get(split_id) {
            Some(s) => match s.children.iter().position(|&c| c == branch_id) {
                Some(p) => (p, s.children.len()),
                None => return false,
            },
            None => return false,
        };
        if n < 2 {
            return false;
        }
        // Divider-push: move the divider on the arrow side in the arrow
        // direction. Inward presses grow the focused pane; outward presses at
        // the screen edge shrink it into the adjacent neighbor (no wrap).
        let (grow, shrink) = if forward {
            if pos + 1 < n {
                (pos, pos + 1)
            } else {
                (pos - 1, pos)
            }
        } else if pos > 0 {
            (pos, pos - 1)
        } else {
            (pos + 1, pos)
        };
        let node = self.arena.get_mut(split_id).unwrap();
        if let NodeData::Split { ratios, .. } = &mut node.data {
            if grow >= ratios.len() || shrink >= ratios.len() {
                return false;
            }
            let avail = ratios[shrink] - MIN_PANE_SHARE;
            if avail <= 1e-9 {
                return false;
            }
            let step = delta.min(avail);
            ratios[grow] += step;
            ratios[shrink] -= step;
            true
        } else {
            false
        }
    }

    /// Root split on `main_axis` with the first window taking the `main`
    /// share; the rest stack on the cross axis. One- and two-window cases
    /// collapse to a flat split (no degenerate single-child wrapper).
    fn alloc_main(
        &mut self,
        main_axis: SplitDirection,
        stack_axis: SplitDirection,
        nodes: Vec<NodeId>,
        main: f64,
    ) -> NodeId {
        debug_assert!(!nodes.is_empty());
        if nodes.len() == 1 {
            return nodes.into_iter().next().unwrap();
        }
        let stack = if nodes.len() == 2 {
            nodes[1]
        } else {
            let rest = nodes[1..].to_vec();
            let n = rest.len();
            self.alloc_split(stack_axis, rest, equal_shares(n))
        };
        self.alloc_split(main_axis, vec![nodes[0], stack], vec![main, 1.0 - main])
    }

    /// Grid: rows of vertical splits under a horizontal outer split.
    /// Degenerate counts collapse (one row → that row is the root).
    fn alloc_tiled(&mut self, nodes: Vec<NodeId>) -> NodeId {
        debug_assert!(!nodes.is_empty());
        if nodes.len() == 1 {
            return nodes.into_iter().next().unwrap();
        }
        let cols = (nodes.len() as f64).sqrt().ceil() as usize;
        let mut rows = Vec::new();
        for chunk in nodes.chunks(cols) {
            let chunk = chunk.to_vec();
            if chunk.len() == 1 {
                rows.push(chunk[0]);
            } else {
                let k = chunk.len();
                rows.push(self.alloc_split(SplitDirection::Vertical, chunk, equal_shares(k)));
            }
        }
        if rows.len() == 1 {
            return rows.into_iter().next().unwrap();
        }
        let k = rows.len();
        self.alloc_split(SplitDirection::Horizontal, rows, equal_shares(k))
    }

    fn alloc_window(&mut self, window_id: WindowId) -> NodeId {
        self.arena.alloc(NodeData::Window {
            window_id,
            is_focused: false,
        })
    }

    fn alloc_split(
        &mut self,
        direction: SplitDirection,
        children: Vec<NodeId>,
        ratios: Vec<f64>,
    ) -> NodeId {
        debug_assert_eq!(children.len(), ratios.len());
        let id = self.arena.alloc(NodeData::Split { direction, ratios });
        for &child in &children {
            self.arena.get_mut(child).unwrap().parent = Some(id);
        }
        self.arena.get_mut(id).unwrap().children = children;
        id
    }

    fn collect_window_ids(&self, node_id: NodeId, out: &mut Vec<WindowId>) {
        let Some(node) = self.arena.get(node_id) else {
            return;
        };
        match &node.data {
            NodeData::Window { window_id, .. } => out.push(*window_id),
            NodeData::Split { .. } => {
                for &child in &node.children.clone() {
                    self.collect_window_ids(child, out);
                }
            }
        }
    }
}
