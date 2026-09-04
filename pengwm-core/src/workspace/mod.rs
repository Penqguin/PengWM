use crate::layout::Rect;
use crate::tree::{Arena, Direction, NodeData, NodeId, SplitDirection, WindowId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn offscreen_rect(_reference: Rect) -> Rect {
    // Far off-screen for monocle siblings — must stay fully invisible
    // even when clamped, unlike hide_workspace which deliberately uses
    // hidden_rect (bottom-right clamped strip) as a daemon-down escape hatch.
    crate::layout::far_offscreen_rect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub monitor_id: u32,
    pub focused_node: Option<NodeId>,
    pub monocle: bool,
    pending_split: Option<SplitDirection>,
    root: Option<NodeId>,
    arena: Arena,
    monitor_origin: (i32, i32),
    monitor_size: (u32, u32),
    /// Global-coordinate region of the monitor that is off-limits to windows
    /// (e.g. a status bar strip). Applied in `layout()` before the gap inset.
    reserved: Option<Rect>,
}

impl Workspace {
    pub fn new(name: String, monitor_id: u32, origin: (i32, i32), size: (u32, u32)) -> Self {
        Self {
            name,
            monitor_id,
            focused_node: None,
            monocle: false,
            pending_split: None,
            root: None,
            arena: Arena::new(),
            monitor_origin: origin,
            monitor_size: size,
            reserved: None,
        }
    }

    // -----------------------------------------------------------------------
    // Window management
    // -----------------------------------------------------------------------

    pub fn add_window(&mut self, window_id: WindowId, direction: Option<SplitDirection>) -> NodeId {
        if self.root.is_none() {
            let id = self.arena.alloc(NodeData::Window {
                window_id,
                is_focused: true,
            });
            self.root = Some(id);
            self.focused_node = Some(id);
            return id;
        }

        // Default master-stack: 1 window on the left, remaining stacked on the
        // right. Only use the generic split logic when an explicit direction
        // was requested (via keybind or pending_split).
        let explicit = direction.is_some() || self.pending_split.is_some();
        if !explicit {
            if let Some(id) = self.try_add_master_stack(window_id) {
                return id;
            }
        }

        let dir = direction
            .or_else(|| self.pending_split.take())
            .unwrap_or_else(|| self.next_direction());
        let focused = self
            .focused_node
            .expect("focused_node set when root exists");

        let flatten = self
            .arena
            .get(focused)
            .and_then(|n| n.parent)
            .and_then(|pid| self.arena.get(pid))
            .is_some_and(|p| matches!(&p.data, NodeData::Split { direction: d, .. } if *d == dir));

        if flatten {
            let parent_id = self.arena.get(focused).and_then(|n| n.parent).unwrap();
            let new_id = self.arena.alloc(NodeData::Window {
                window_id,
                is_focused: true,
            });
            self.arena.get_mut(parent_id).unwrap().children.push(new_id);
            self.arena.get_mut(new_id).unwrap().parent = Some(parent_id);
            self.rebalance_ratios(parent_id);
            self.set_focused_node(new_id);
            return new_id;
        }

        let old_parent = self.arena.get(focused).and_then(|n| n.parent);
        let new_id = self.arena.alloc(NodeData::Window {
            window_id,
            is_focused: true,
        });
        let split_id = self.arena.alloc(NodeData::Split {
            direction: dir,
            ratios: vec![0.5, 0.5],
        });

        self.arena.get_mut(split_id).unwrap().children = vec![focused, new_id];
        self.arena.get_mut(focused).unwrap().parent = Some(split_id);
        self.arena.get_mut(new_id).unwrap().parent = Some(split_id);

        match old_parent {
            Some(pid) => {
                let parent = self.arena.get_mut(pid).unwrap();
                if let Some(pos) = parent.children.iter().position(|&c| c == focused) {
                    parent.children[pos] = split_id;
                }
                self.arena.get_mut(split_id).unwrap().parent = Some(pid);
            }
            None => {
                self.root = Some(split_id);
            }
        }

        self.set_focused_node(new_id);
        new_id
    }

    /// Master-stack insertion: left master (single window), right stack
    /// (Horizontal split). Returns None if the current tree is not in
    /// master-stack shape and we should fall back to the generic splitter.
    fn try_add_master_stack(&mut self, window_id: WindowId) -> Option<NodeId> {
        let root_id = self.root?;
        // Single window → create Vertical [master, new]
        if self.arena.len() == 1 {
            if !matches!(
                self.arena.get(root_id)?.data,
                NodeData::Window { .. }
            ) {
                return None;
            }
            let new_id = self.arena.alloc(NodeData::Window {
                window_id,
                is_focused: true,
            });
            let split_id = self.arena.alloc(NodeData::Split {
                direction: SplitDirection::Vertical,
                ratios: vec![0.5, 0.5],
            });
            self.arena.get_mut(split_id).unwrap().children = vec![root_id, new_id];
            self.arena.get_mut(root_id).unwrap().parent = Some(split_id);
            self.arena.get_mut(new_id).unwrap().parent = Some(split_id);
            self.root = Some(split_id);
            self.set_focused_node(new_id);
            return Some(new_id);
        }

        // Check master-stack shape: root Vertical with exactly 2 children,
        // left is Window, right is Window or Horizontal.
        let root_node = self.arena.get(root_id)?;
        let (dir, children) = match &root_node.data {
            NodeData::Split { direction, .. } => (*direction, root_node.children.clone()),
            _ => return None,
        };
        if dir != SplitDirection::Vertical || children.len() != 2 {
            return None;
        }
        let left_id = children[0];
        let right_id = children[1];
        let left_is_window = matches!(
            self.arena.get(left_id)?.data,
            NodeData::Window { .. }
        );
        if !left_is_window {
            return None;
        }
        // Clone right data before mutable alloc to avoid borrow conflict
        let right_data = self.arena.get(right_id)?.data.clone();
        let new_id = self.arena.alloc(NodeData::Window {
            window_id,
            is_focused: true,
        });
        match &right_data {
            NodeData::Window { .. } => {
                // Right is single window → convert to Horizontal stack [right, new]
                let stack_id = self.arena.alloc(NodeData::Split {
                    direction: SplitDirection::Horizontal,
                    ratios: vec![0.5, 0.5],
                });
                self.arena.get_mut(stack_id).unwrap().children = vec![right_id, new_id];
                self.arena.get_mut(right_id).unwrap().parent = Some(stack_id);
                self.arena.get_mut(new_id).unwrap().parent = Some(stack_id);
                self.arena.get_mut(root_id).unwrap().children[1] = stack_id;
                self.arena.get_mut(stack_id).unwrap().parent = Some(root_id);
                self.rebalance_ratios(stack_id);
                self.set_focused_node(new_id);
                Some(new_id)
            }
            NodeData::Split { direction, .. } if *direction == SplitDirection::Horizontal => {
                // Right is already a Horizontal stack → append
                self.arena.get_mut(right_id).unwrap().children.push(new_id);
                self.arena.get_mut(new_id).unwrap().parent = Some(right_id);
                self.rebalance_ratios(right_id);
                self.set_focused_node(new_id);
                Some(new_id)
            }
            _ => None,
        }
    }

    pub fn remove_window(&mut self, window_id: WindowId) {
        let node_id = match self.find_window(window_id) {
            Some(id) => id,
            None => return,
        };

        let was_focused = self.focused_node == Some(node_id);

        if self.arena.len() == 1 {
            self.arena.remove(node_id);
            self.root = None;
            self.focused_node = None;
            return;
        }

        let parent_id = self
            .arena
            .get(node_id)
            .and_then(|n| n.parent)
            .expect("non-root node has a parent");
        self.arena.remove(node_id);
        self.collapse_upward(parent_id);

        if self.root.is_none() {
            self.focused_node = None;
        } else if was_focused {
            self.focus_nearest_leaf();
        }
    }

    pub fn focus_window(&mut self, window_id: WindowId) {
        let node_id = match self.find_window(window_id) {
            Some(id) => id,
            None => return,
        };
        self.set_focused_node(node_id);
    }

    pub fn focus_neighbor(&mut self, direction: Direction) {
        if self.monocle {
            let leaves = self.cycle_leaves();
            if leaves.is_empty() {
                return;
            }
            let pos = leaves
                .iter()
                .position(|&id| Some(id) == self.focused_node)
                .unwrap_or(0);
            let target_pos = if direction.is_forward() {
                (pos + 1) % leaves.len()
            } else if pos == 0 {
                leaves.len() - 1
            } else {
                pos - 1
            };
            self.set_focused_node(leaves[target_pos]);
            return;
        }
        let from = match self.focused_node {
            Some(id) => id,
            None => return,
        };
        let target = match self.find_neighbor(from, direction) {
            Some(id) => id,
            None => return,
        };
        self.set_focused_node(target);
    }

    pub fn swap_windows_by_id(&mut self, dragged_id: WindowId, target_id: WindowId) -> bool {
        let dragged_node = match self.find_window(dragged_id) {
            Some(id) => id,
            None => return false,
        };
        let target_node = match self.find_window(target_id) {
            Some(id) => id,
            None => return false,
        };
        if dragged_node == target_node {
            return false;
        }
        let dragged_data = self.arena.get(dragged_node).unwrap().data.clone();
        let target_data = self.arena.get(target_node).unwrap().data.clone();
        self.arena.get_mut(dragged_node).unwrap().data = target_data;
        self.arena.get_mut(target_node).unwrap().data = dragged_data;
        self.focused_node = Some(target_node);
        true
    }

    pub fn swap_window(&mut self, direction: Direction) {
        if self.monocle {
            let leaves = self.cycle_leaves();
            if leaves.len() < 2 {
                return;
            }
            let pos = match leaves.iter().position(|&id| Some(id) == self.focused_node) {
                Some(p) => p,
                None => return,
            };
            let target_pos = if direction.is_forward() {
                (pos + 1) % leaves.len()
            } else if pos == 0 {
                leaves.len() - 1
            } else {
                pos - 1
            };
            let focused = leaves[pos];
            let target = leaves[target_pos];
            // In monocle all windows are stacked — swapping tree positions
            // is invisible if we keep the original WindowId visible. Swap
            // only the WindowId so the *neighbor* becomes the visible
            // fullscreen window, and keep is_focused at the original position.
            let focused_wid = match &self.arena.get(focused).unwrap().data {
                NodeData::Window { window_id, .. } => *window_id,
                _ => return,
            };
            let target_wid = match &self.arena.get(target).unwrap().data {
                NodeData::Window { window_id, .. } => *window_id,
                _ => return,
            };
            if let NodeData::Window { window_id, .. } =
                &mut self.arena.get_mut(focused).unwrap().data
            {
                *window_id = target_wid;
            }
            if let NodeData::Window { window_id, .. } =
                &mut self.arena.get_mut(target).unwrap().data
            {
                *window_id = focused_wid;
            }
            // Flags already correct — focused stays focused but now shows the
            // swapped-in window, so the swap is visible in monocle.
            return;
        }
        let focused = match self.focused_node {
            Some(id) => id,
            None => return,
        };
        let target = match self.find_neighbor(focused, direction) {
            Some(id) => id,
            None => return,
        };
        let old_focused_data = self.arena.get(focused).unwrap().data.clone();
        let old_target_data = self.arena.get(target).unwrap().data.clone();
        self.arena.get_mut(focused).unwrap().data = old_target_data;
        self.arena.get_mut(target).unwrap().data = old_focused_data;
        self.focused_node = Some(target);
    }

    fn cycle_leaves(&self) -> Vec<NodeId> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.collect_leaves(root, &mut out);
        out
    }

    fn collect_leaves(&self, node_id: NodeId, out: &mut Vec<NodeId>) {
        let Some(node) = self.arena.get(node_id) else {
            return;
        };
        match &node.data {
            NodeData::Window { .. } => out.push(node_id),
            NodeData::Split { .. } => {
                for &child in &node.children.clone() {
                    self.collect_leaves(child, out);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tree queries
    // -----------------------------------------------------------------------

    pub fn find_window(&self, window_id: WindowId) -> Option<NodeId> {
        for (id, node) in &self.arena.nodes {
            if let NodeData::Window { window_id: wid, .. } = &node.data {
                if *wid == window_id {
                    return Some(*id);
                }
            }
        }
        None
    }

    pub fn all_windows(&self) -> Vec<WindowId> {
        let mut result = Vec::new();
        for node in self.arena.nodes.values() {
            if let NodeData::Window { window_id: wid, .. } = &node.data {
                result.push(*wid);
            }
        }
        result
    }

    pub fn window_count(&self) -> usize {
        self.all_windows().len()
    }

    pub fn toggle_monocle(&mut self) {
        self.monocle = !self.monocle;
    }

    // -----------------------------------------------------------------------
    // Layout
    // -----------------------------------------------------------------------

    /// Compute global-coordinate rects for every window using the stored monitor
    /// geometry. Handles monocle internally: the focused window fills the monitor
    /// (minus outer gap); all siblings get offscreen rects.
    pub fn layout(&self, gap_inner: f64, gap_outer: f64) -> HashMap<WindowId, Rect> {
        let Some(root) = self.root else {
            return HashMap::new();
        };

        let monitor_rect = Rect::new(
            0.0,
            0.0,
            self.monitor_size.0 as f64,
            self.monitor_size.1 as f64,
        );
        let usable = self.usable_rect(monitor_rect);
        let inset = crate::layout::inset_rect(usable, gap_outer);
        let mut output = HashMap::new();

        if self.monocle {
            if let Some(focused) = self.focused_node {
                if let Some(node) = self.arena.get(focused) {
                    if let NodeData::Window { window_id, .. } = &node.data {
                        let global =
                            crate::layout::screen_local_to_global(inset, self.monitor_origin);
                        output.insert(*window_id, global);
                    }
                }
            }
            // Keep the original size (inset) while moving far off-screen
            // so the window isn't shrunk to 1x1 and can restore without flicker.
            let offscreen = offscreen_rect(inset);
            for node in self.arena.nodes.values() {
                if let NodeData::Window { window_id, .. } = &node.data {
                    output.entry(*window_id).or_insert(offscreen);
                }
            }
        } else {
            crate::layout::calculate_layout(root, inset, &self.arena, &mut output, gap_inner);
            for rect in output.values_mut() {
                *rect = crate::layout::screen_local_to_global(*rect, self.monitor_origin);
            }
        }

        output
    }

    /// Global-coordinate origin of the monitor this workspace tiles on.
    pub fn set_monitor_origin(&mut self, origin: (i32, i32)) {
        self.monitor_origin = origin;
    }

    pub fn monitor_origin(&self) -> (i32, i32) {
        self.monitor_origin
    }

    pub fn monitor_size(&self) -> (u32, u32) {
        self.monitor_size
    }

    pub fn focused_window_id(&self) -> Option<WindowId> {
        self.focused_node.and_then(|nid| {
            if let NodeData::Window { window_id, .. } = &self.arena.get(nid)?.data {
                Some(*window_id)
            } else {
                None
            }
        })
    }

    /// True if the focused node is a Window leaf (not a Split container).
    fn focused_is_window(&self) -> bool {
        self.focused_node.is_some_and(|nid| {
            self.arena
                .get(nid)
                .is_some_and(|n| matches!(n.data, NodeData::Window { .. }))
        })
    }

    /// Set the split direction for the next window added (when a Window is
    /// focused) or re-orient the focused Split container (when one is focused).
    /// The invariant — "a split direction only applies to a Split container,
    /// otherwise it's pending for the next window" — lives here with the tree.
    pub fn apply_split_direction(&mut self, direction: SplitDirection) {
        if self.focused_is_window() {
            self.pending_split = Some(direction);
        } else {
            self.set_split_direction(direction);
        }
    }

    /// Change the direction of the focused Split and flatten if redundant.
    fn set_split_direction(&mut self, direction: SplitDirection) {
        if let Some(node_id) = self.focused_node {
            if let NodeData::Split {
                direction: ref mut dir,
                ..
            } = &mut self.arena.get_mut(node_id).unwrap().data
            {
                *dir = direction;
                self.flatten_split_if_redundant(node_id);
            }
        }
    }

    /// If `split_id` is a Split whose direction matches its parent's, absorb its
    /// children into the parent and remove the now-redundant split.
    fn flatten_split_if_redundant(&mut self, split_id: NodeId) {
        let parent_id = match self.arena.get(split_id).and_then(|n| n.parent) {
            Some(pid) => pid,
            None => return,
        };

        let same_dir = match (
            &self.arena.get(split_id).unwrap().data,
            &self.arena.get(parent_id).unwrap().data,
        ) {
            (NodeData::Split { direction: d1, .. }, NodeData::Split { direction: d2, .. }) => {
                d1 == d2
            }
            _ => return,
        };

        if !same_dir {
            return;
        }

        let children: Vec<NodeId> = self.arena.get(split_id).unwrap().children.clone();
        for &child in &children {
            self.arena.get_mut(child).unwrap().parent = Some(parent_id);
        }
        let parent = self.arena.get_mut(parent_id).unwrap();
        if let Some(pos) = parent.children.iter().position(|&c| c == split_id) {
            parent.children.splice(pos..=pos, children);
        }
        self.arena.get_mut(split_id).unwrap().children.clear();
        self.arena.remove(split_id);
        self.rebalance_ratios(parent_id);
    }

    // -----------------------------------------------------------------------
    // Monitor
    // -----------------------------------------------------------------------

    pub fn update_monitor_geometry(&mut self, origin: (i32, i32), size: (u32, u32)) {
        self.monitor_origin = origin;
        self.monitor_size = size;
    }

    /// Reserve a region of the monitor (in global coordinates) that windows
    /// must avoid — used for the bar strip. `None` clears the reservation.
    pub fn set_reserved_rect(&mut self, global: Option<Rect>) {
        self.reserved = global;
    }

    pub fn reserved_rect(&self) -> Option<Rect> {
        self.reserved
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Subtract the reserved region (an edge strip spanning the monitor) from
    /// the full monitor rect, producing the tiling area.
    fn usable_rect(&self, monitor: Rect) -> Rect {
        let Some(reserved) = self.reserved else {
            return monitor;
        };
        let local = Rect::new(
            reserved.x - self.monitor_origin.0 as f64,
            reserved.y - self.monitor_origin.1 as f64,
            reserved.width,
            reserved.height,
        );
        crate::layout::subtract_strip(monitor, local)
    }

    /// Direction of the focused split container (the focused node itself, or
    /// its nearest split ancestor). `None` when there is no focused split.
    pub fn focused_split_direction(&self) -> Option<SplitDirection> {
        let mut current = self.focused_node?;
        loop {
            let node = self.arena.get(current)?;
            match &node.data {
                NodeData::Split { direction, .. } => return Some(*direction),
                NodeData::Window { .. } => current = node.parent?,
            }
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers (monitor)
    // -----------------------------------------------------------------------

    fn is_widescreen(&self) -> bool {
        self.monitor_size.0 > self.monitor_size.1
    }

    fn next_direction(&self) -> SplitDirection {
        let default = if self.is_widescreen() {
            SplitDirection::Vertical
        } else {
            SplitDirection::Horizontal
        };
        let focused = match self.focused_node {
            Some(id) => id,
            None => return default,
        };
        match self
            .arena
            .get(focused)
            .and_then(|n| n.parent)
            .and_then(|pid| self.arena.get(pid))
            .map(|p| &p.data)
        {
            Some(NodeData::Split { direction, .. }) => match direction {
                SplitDirection::Horizontal => SplitDirection::Vertical,
                SplitDirection::Vertical => SplitDirection::Horizontal,
            },
            _ => default,
        }
    }

    fn set_focused_node(&mut self, node_id: NodeId) {
        if let Some(old_id) = self.focused_node {
            if let Some(old) = self.arena.get_mut(old_id) {
                if let NodeData::Window {
                    ref mut is_focused, ..
                } = old.data
                {
                    *is_focused = false;
                }
            }
        }
        self.focused_node = Some(node_id);
        if let Some(node) = self.arena.get_mut(node_id) {
            if let NodeData::Window {
                ref mut is_focused, ..
            } = node.data
            {
                *is_focused = true;
            }
        }
    }

    fn focus_nearest_leaf(&mut self) {
        match self.root {
            Some(root_id) => {
                let leaf = self.leftmost_leaf(root_id);
                self.set_focused_node(leaf);
            }
            None => {
                self.focused_node = None;
            }
        }
    }

    fn rebalance_ratios(&mut self, split_id: NodeId) {
        let n = self
            .arena
            .get(split_id)
            .map(|n| n.children.len())
            .unwrap_or(0);
        if n > 0 {
            let equal = 1.0 / n as f64;
            if let NodeData::Split { ref mut ratios, .. } =
                &mut self.arena.get_mut(split_id).unwrap().data
            {
                *ratios = vec![equal; n];
            }
        }
    }

    fn collapse_upward(&mut self, start_id: NodeId) {
        let mut current = Some(start_id);
        while let Some(id) = current {
            let (child_count, parent_id) = match self.arena.get(id) {
                Some(node) => (node.children.len(), node.parent),
                None => break,
            };

            if child_count >= 2 {
                self.rebalance_ratios(id);
                break;
            }

            if child_count == 0 {
                let next = parent_id;
                self.arena.remove(id);
                current = next;
                if next.is_none() {
                    self.root = None;
                }
                continue;
            }

            let only_child = self.arena.get(id).unwrap().children[0];

            // If the only remaining child is itself a split, absorb its
            // children into `id` rather than promoting it wholesale. Promoting
            // a differently-oriented split flips the layout (e.g. a vertical
            // left/right split becomes horizontal top/bottom); absorbing keeps
            // `id`'s orientation, e.g. one window on the left and the rest on
            // the right.
            if matches!(
                &self.arena.get(only_child).unwrap().data,
                NodeData::Split { .. }
            ) {
                let grandchildren: Vec<NodeId> =
                    self.arena.get(only_child).unwrap().children.clone();
                {
                    let node = self.arena.get_mut(id).unwrap();
                    node.children = grandchildren.clone();
                }
                for &gc in &grandchildren {
                    self.arena.get_mut(gc).unwrap().parent = Some(id);
                }
                self.arena.get_mut(only_child).unwrap().children.clear();
                self.arena.remove(only_child);
                self.rebalance_ratios(id);
                continue;
            }

            self.arena.get_mut(only_child).unwrap().parent = parent_id;

            match parent_id {
                Some(gp_id) => {
                    let gp = self.arena.get_mut(gp_id).unwrap();
                    if let Some(pos) = gp.children.iter().position(|&c| c == id) {
                        gp.children[pos] = only_child;
                    }
                }
                None => {
                    self.root = Some(only_child);
                }
            }

            self.arena.get_mut(id).unwrap().children.clear();
            self.arena.remove(id);
            current = parent_id;
        }
    }

    fn find_neighbor(&self, from_node: NodeId, direction: Direction) -> Option<NodeId> {
        let target_axis = direction.axis();
        let is_forward = direction.is_forward();

        let mut current = from_node;
        let (split_id, branch_id) = loop {
            let node = self.arena.get(current)?;
            match node.parent {
                Some(pid) => {
                    let parent = self.arena.get(pid)?;
                    if let NodeData::Split { direction: d, .. } = &parent.data {
                        if *d == target_axis {
                            break (pid, current);
                        }
                    }
                    current = pid;
                }
                None => return None,
            }
        };

        let split = self.arena.get(split_id)?;
        let pos = split.children.iter().position(|&c| c == branch_id)?;
        let n_children = split.children.len();

        let target_pos = if is_forward {
            (pos + 1) % n_children
        } else if pos == 0 {
            n_children - 1
        } else {
            pos - 1
        };

        let target_branch = split.children[target_pos];
        Some(if is_forward {
            self.leftmost_leaf(target_branch)
        } else {
            self.rightmost_leaf(target_branch)
        })
    }

    fn leftmost_leaf(&self, node_id: NodeId) -> NodeId {
        let node = self.arena.get(node_id).unwrap();
        match &node.data {
            NodeData::Window { .. } => node_id,
            NodeData::Split { .. } => {
                if node.children.is_empty() {
                    node_id
                } else {
                    self.leftmost_leaf(node.children[0])
                }
            }
        }
    }

    fn rightmost_leaf(&self, node_id: NodeId) -> NodeId {
        let node = self.arena.get(node_id).unwrap();
        match &node.data {
            NodeData::Window { .. } => node_id,
            NodeData::Split { .. } => {
                if node.children.is_empty() {
                    node_id
                } else {
                    self.rightmost_leaf(node.children[node.children.len() - 1])
                }
            }
        }
    }
}


#[cfg(test)]
mod tests;
