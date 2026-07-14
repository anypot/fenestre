//! Binary space partitioning layout engine.

use crate::config::LayoutConfig;

/// Axis along which a split divides its two children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SplitDirection {
    Vertical,
    Horizontal,
}

/// Direction used for focus navigation, window moves, and resizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Window layout state, carrying its own data so illegal states are
/// unrepresentable.
///
/// `Floating` / `PseudoTiled` hold the window's rect; `Fullscreen` holds the
/// pre-fullscreen state in `restore` so toggling fullscreen off returns to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowState {
    Tiled,
    Floating { rect: Rect },
    PseudoTiled { rect: Rect },
    Fullscreen { restore: Box<WindowState> },
}

impl WindowState {
    fn participates_in_tiling(&self) -> bool {
        matches!(self, Self::Tiled | Self::PseudoTiled { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

#[derive(Debug, Clone)]
struct LayoutSplit {
    direction: SplitDirection,
    ratio: f64,
}

#[derive(Debug, Clone)]
struct LayoutNode {
    window: Option<u32>,
    state: WindowState,
    rect: Rect,
    has_tiling: bool,
    split: Option<LayoutSplit>,
    children: Option<(Box<LayoutNode>, Box<LayoutNode>)>,
}

#[derive(Debug, Clone)]
pub(crate) struct LayoutTree {
    root: Option<Box<LayoutNode>>,
    focused: Option<u32>,
    output_rect: Rect,
    raw_output_rect: Rect,
    layout_config: LayoutConfig,
    next_id: u32,
}
#[derive(Debug, Clone)]
enum RemoveOutcome {
    /// The target window was removed from this node or subtree.
    /// If this was a leaf, it's now empty. If it was a split, one child was removed.
    LeafRemoved,
    /// This node should be replaced with the returned node (e.g., collapse to sibling).
    Replaced(Box<LayoutNode>),
    /// The parent should update tiling flags but no collapse is needed.
    Modified,
    /// The target window was not found in this subtree.
    NotFound,
}

impl Rect {
    /// Construct a rectangle from its position and size components.
    pub(crate) fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn right_edge(self) -> i32 {
        self.x + self.width
    }

    fn bottom_edge(self) -> i32 {
        self.y + self.height
    }

    fn horizontal_overlap(self, other: Rect) -> bool {
        self.x <= other.right_edge() && other.x <= self.right_edge()
    }

    fn vertical_overlap(self, other: Rect) -> bool {
        self.y <= other.bottom_edge() && other.y <= self.bottom_edge()
    }

    fn is_right_of(self, other: Rect) -> bool {
        self.x >= other.right_edge() && self.vertical_overlap(other)
    }

    fn is_left_of(self, other: Rect) -> bool {
        self.right_edge() <= other.x && self.vertical_overlap(other)
    }

    fn is_below(self, other: Rect) -> bool {
        self.y >= other.bottom_edge() && self.horizontal_overlap(other)
    }

    fn is_above(self, other: Rect) -> bool {
        self.bottom_edge() <= other.y && self.horizontal_overlap(other)
    }

    fn distance_from(self, other: Rect, direction: FocusDirection) -> i32 {
        match direction {
            FocusDirection::Right => self.x - other.right_edge(),
            FocusDirection::Left => other.x - self.right_edge(),
            FocusDirection::Down => self.y - other.bottom_edge(),
            FocusDirection::Up => other.y - self.bottom_edge(),
        }
    }
}

impl LayoutNode {
    /// Create a leaf node (no split, just a window).
    fn leaf(window: Option<u32>) -> Self {
        Self {
            window,
            state: WindowState::Tiled,
            rect: Rect::new(0, 0, 0, 0),
            has_tiling: window.is_some(),
            split: None,
            children: None,
        }
    }

    /// Create a split node with two children.
    fn split(
        split: LayoutSplit,
        first_child: Box<LayoutNode>,
        second_child: Box<LayoutNode>,
    ) -> Self {
        let has_tiling = first_child.has_tiling || second_child.has_tiling;
        Self {
            window: None,
            state: WindowState::Tiled,
            rect: Rect::new(0, 0, 0, 0),
            has_tiling,
            split: Some(split),
            children: Some((first_child, second_child)),
        }
    }

    #[cfg(test)]
    fn first_child(&self) -> Option<&LayoutNode> {
        self.children.as_ref().map(|(first, _)| first.as_ref())
    }

    #[cfg(test)]
    fn second_child(&self) -> Option<&LayoutNode> {
        self.children.as_ref().map(|(_, second)| second.as_ref())
    }
}

impl LayoutTree {
    /// Create a new layout tree for the given output geometry.
    pub(crate) fn new(output_rect: Rect) -> Self {
        Self {
            root: None,
            focused: None,
            output_rect,
            raw_output_rect: output_rect,
            layout_config: LayoutConfig::default(),
            next_id: 0,
        }
    }

    /// Update the layout configuration (gap and margins).
    ///
    /// This is called from `WMState::load_config` when the user config is
    /// (re)loaded so the layout tree picks up new gap and margin values
    /// without recreating the tree.
    pub(crate) fn set_layout_config(&mut self, config: LayoutConfig) {
        self.layout_config = config;
        self.apply_margins();
    }

    /// Update the output rectangle and recompute all window layouts.
    ///
    /// Stores the raw output rect and re-derives `output_rect` from the
    /// current margins + gap, then arranges all windows.
    pub(crate) fn set_output_rect(&mut self, rect: Rect) {
        self.raw_output_rect = rect;
        self.apply_margins();
        self.arrange();
    }

    /// Translate every floating window's rect by `(dx, dy)`. Used when reassigning
    /// windows between outputs at different logical positions so floating windows
    /// keep their relative placement. Pseudo-tiled geometry is positionally
    /// derived from the output in `reassign_output`, so it is not translated here.
    /// No output clamping is applied: reassignment may run before the destination
    /// has real dimensions, and the next `set_output_rect`/arrange re-clamps anyway.
    pub(crate) fn translate_floating_rects(&mut self, dx: i32, dy: i32) {
        fn shift(node: &mut LayoutNode, dx: i32, dy: i32) {
            if let WindowState::Floating { rect } = &mut node.state {
                rect.x = rect.x.saturating_add(dx);
                rect.y = rect.y.saturating_add(dy);
            }
            if let Some((first, second)) = node.children.as_mut() {
                shift(first, dx, dy);
                shift(second, dx, dy);
            }
        }
        if let Some(root) = self.root.as_mut() {
            shift(root, dx, dy);
        }
    }

    /// Compute `output_rect` from the raw output geometry and the current
    /// layout config.
    ///
    /// The tiling area is inset by `margin_*` plus `gap` on every side so
    /// there is visible space between the tiled area and the output edge.
    /// Width and height are clamped to non-negative to handle tiny outputs.
    fn apply_margins(&mut self) {
        let cfg = &self.layout_config;
        let rect = self.raw_output_rect;
        let gap = cfg.gap.unwrap_or(0);
        let margin_left = cfg.margin_left.unwrap_or(0);
        let margin_right = cfg.margin_right.unwrap_or(0);
        let margin_top = cfg.margin_top.unwrap_or(0);
        let margin_bottom = cfg.margin_bottom.unwrap_or(0);
        self.output_rect = Rect::new(
            rect.x + margin_left + gap,
            rect.y + margin_top + gap,
            (rect.width - margin_left - margin_right - 2 * gap).max(0),
            (rect.height - margin_top - margin_bottom - 2 * gap).max(0),
        );
    }

    /// Insert a window into the tree, splitting the focused node's rect.
    /// Arranges the tree first so the split direction reflects current geometry.
    /// Returns false if the window already exists.
    pub(crate) fn insert_window(&mut self, id: u32) -> bool {
        if self.contains_window(id) {
            return false;
        }

        self.arrange();

        let Some(focused) = self.focused else {
            self.root = Some(Box::new(LayoutNode::leaf(Some(id))));
            self.focused = Some(id);
            return true;
        };

        if self.replace_placeholder(id) {
            self.focused = Some(id);
            return true;
        }

        let Some(focused_node) = self.take_window_node(focused) else {
            return false;
        };
        let parent = LayoutNode::split(
            LayoutSplit {
                direction: self.split_direction(focused),
                ratio: 0.5,
            },
            focused_node,
            Box::new(LayoutNode::leaf(Some(id))),
        );
        self.replace_window_node(focused, Box::new(parent));
        self.focused = Some(id);
        true
    }

    /// Determine split direction based on the focused window's rectangle aspect ratio.
    fn split_direction(&self, id: u32) -> SplitDirection {
        self.node_rect(id)
            .map(|rect| {
                if rect.width >= rect.height {
                    SplitDirection::Vertical
                } else {
                    SplitDirection::Horizontal
                }
            })
            .unwrap_or(SplitDirection::Vertical)
    }

    /// Split the focused window into two, inserting a placeholder.
    /// Returns the placeholder window ID.
    pub(crate) fn split_focused(&mut self, direction: SplitDirection) -> Option<u32> {
        self.arrange();

        let focused = self.focused?;
        let focused_node = self.take_window_node(focused)?;
        let placeholder_id = self.next_id;
        self.next_id += 1;

        let parent = LayoutNode::split(
            LayoutSplit {
                direction,
                ratio: 0.5,
            },
            focused_node,
            Box::new(LayoutNode::leaf(None)),
        );
        self.replace_window_node(focused, Box::new(parent))
            .then_some(placeholder_id)
    }

    /// Remove a window from the tree, collapsing empty splits.
    /// Handles three cases: leaf removed, split replaced by sibling, or modified in place.
    pub(crate) fn remove_window(&mut self, id: u32) -> bool {
        let mut root = self.root.take();
        let outcome = root
            .as_mut()
            .map_or(RemoveOutcome::NotFound, |r| remove_from_node(r, id));

        match outcome {
            RemoveOutcome::LeafRemoved => {
                // Root was a single window that got removed
                self.focused = None;
                true
            }
            RemoveOutcome::Replaced(new_node) => {
                // Root was a split that collapsed to a sibling
                self.root = Some(new_node);
                if self.focused == Some(id) {
                    self.focused = self.first_window();
                }
                if let Some(root) = self.root.as_mut() {
                    update_ancestor_tiling_flags(root);
                }
                true
            }
            RemoveOutcome::Modified => {
                // Tree was modified but no structural change - just update tiling flags
                self.root = root;
                if self.focused == Some(id) {
                    self.focused = self.first_window();
                }
                if let Some(root) = self.root.as_mut() {
                    update_ancestor_tiling_flags(root);
                }
                true
            }
            RemoveOutcome::NotFound => {
                self.root = root;
                false
            }
        }
    }

    /// Cycle focus to the next window in the tree, wrapping around.
    pub(crate) fn focus_next(&mut self) -> bool {
        self.focus_relative(1)
    }

    /// Cycle focus to the previous window in the tree, wrapping around.
    pub(crate) fn focus_previous(&mut self) -> bool {
        self.focus_relative(-1)
    }

    /// Set focus to `id` if it is present in the tree; returns whether focus changed.
    pub(crate) fn focus_window(&mut self, id: u32) -> bool {
        if self.contains_window(id) {
            self.focused = Some(id);
            true
        } else {
            false
        }
    }

    /// Get the currently focused window ID.
    pub(crate) fn focused_window(&self) -> Option<u32> {
        self.focused
    }

    /// Focus the nearest window in the given direction based on geometry.
    /// Uses distance-from-edge calculations to find the closest window.
    pub(crate) fn focus_direction(&mut self, direction: FocusDirection) -> bool {
        self.arrange();

        let Some(focused) = self.focused else {
            return false;
        };
        let Some(target) = self.select_best_target(focused, direction) else {
            return false;
        };
        self.focused = Some(target);
        true
    }

    fn select_best_target(&self, focused: u32, direction: FocusDirection) -> Option<u32> {
        let focused_rect = self.node_rect(focused)?;
        let mut best: Option<(u32, i32)> = None;

        for (id, rect) in self.visible_nodes() {
            if id == focused || !rect.is_side_of(focused_rect, direction) {
                continue;
            }
            let distance = rect.distance_from(focused_rect, direction);
            if best.is_none_or(|(best_id, best_distance)| {
                distance < best_distance || (distance == best_distance && id < best_id)
            }) {
                best = Some((id, distance));
            }
        }

        best.map(|(id, _)| id)
    }

    /// Visually swap the currently focused window with the window located
    /// in the given direction.
    ///
    /// The tiled positions themselves are not moved; instead the *window IDs*
    /// and their per-node state (`state`, `has_tiling`) are exchanged between
    /// the two leaf nodes.
    /// `arrange()` is called up front to compute a baseline layout.
    /// The actual window-to-rect association is finalized by `arrange()`
    /// on the next frame, after this function returns and the caller
    /// triggers a manage/render cycle, so the windows end up in each
    /// other's former slots.
    ///
    /// Returns `false` if there is no focused window, only one visible window,
    /// or no valid target in the requested direction.
    ///
    /// # Safety notes
    ///
    /// This function is `pub(crate)` but contains an `unsafe` block.
    /// The safe helper `find_window_node_mut` cannot be used here because
    /// Rust's borrow checker rejects holding two `&mut LayoutNode` references
    /// obtained from a recursive traversal of the tree, even though the two
    /// windows are guaranteed to live in disjoint sub-trees.
    ///
    /// The workaround is:
    /// 1. Traverse the tree mutably (required to satisfy the borrow checker),
    ///    collecting raw `*mut LayoutNode` pointers to the two leaf nodes.
    /// 2. Release the mutable borrow by ending the recursive call.
    /// 3. Re-hydrate the pointers as `&mut LayoutNode` inside a single
    ///    `unsafe` block and use `std::mem::swap` to exchange the fields.
    ///
    /// The following invariants make this safe:
    /// - `focused` and `target` are distinct window IDs, so the two pointers
    ///   point to different nodes.
    /// - The recursive `collect` closure only mutates traversal state
    ///   (`out_a` / `out_b`), not the nodes themselves, so the tree remains
    ///   structurally intact while we gather the pointers.
    /// - After the swaps, `update_ancestor_tiling_flags` is called on the
    ///   root to restore the `has_tiling` invariant on every ancestor.
    pub(crate) fn swap_windows(&mut self, direction: FocusDirection) -> bool {
        self.arrange();

        let Some(focused) = self.focused else {
            return false;
        };
        let visible = self.visible_windows();
        if visible.len() <= 1 {
            return false;
        }
        let Some(target) = self.select_best_target(focused, direction) else {
            return false;
        };

        let mut node_a: Option<*mut LayoutNode> = None;
        let mut node_b: Option<*mut LayoutNode> = None;

        fn collect(
            node: &mut LayoutNode,
            a: u32,
            b: u32,
            out_a: &mut Option<*mut LayoutNode>,
            out_b: &mut Option<*mut LayoutNode>,
        ) {
            if out_a.is_some() && out_b.is_some() {
                return;
            }
            if node.window == Some(a) {
                *out_a = Some(node);
            } else if node.window == Some(b) {
                *out_b = Some(node);
            }
            if let Some((f, s)) = node.children.as_mut() {
                collect(f, a, b, out_a, out_b);
                collect(s, a, b, out_a, out_b);
            }
        }

        if let Some(root) = self.root.as_mut() {
            collect(root, focused, target, &mut node_a, &mut node_b);
        }

        if let (Some(a), Some(b)) = (node_a, node_b) {
            // SAFETY:
            // - `a` and `b` are distinct leaf nodes because `focused != target`.
            // - They were obtained from a completed mutable traversal, so
            //   no other live borrows of those nodes exist at this point.
            // - We only swap POD-like fields (Option<u32>, enums, bools, Rects);
            //   no double-fetch or invariant-breaking transient state is
            //   ever observed through borrowed references elsewhere.
            // - `update_ancestor_tiling_flags` is called immediately
            //   afterwards to restore `has_tiling` on all ancestors.
            unsafe {
                std::mem::swap(&mut (*a).window, &mut (*b).window);
                std::mem::swap(&mut (*a).state, &mut (*b).state);
                std::mem::swap(&mut (*a).has_tiling, &mut (*b).has_tiling);
            }
            if let Some(root) = self.root.as_mut() {
                update_ancestor_tiling_flags(root);
            }
            true
        } else {
            false
        }
    }

    /// Whether `id`'s state is `Floating`. Returns false if the window is absent.
    pub(crate) fn window_is_floating(&self, id: u32) -> bool {
        self.find_window_node(id)
            .map(|n| matches!(n.state, WindowState::Floating { .. }))
            .unwrap_or(false)
    }

    /// Whether `id`'s state is `Fullscreen`. Returns false if the window is absent.
    pub(crate) fn window_is_fullscreen(&self, id: u32) -> bool {
        self.find_window_node(id)
            .map(|n| matches!(n.state, WindowState::Fullscreen { .. }))
            .unwrap_or(false)
    }

    /// Return the node's current `state` for a window, or `None` if the window
    /// is not in the tree.
    pub(crate) fn window_state(&self, id: u32) -> Option<&WindowState> {
        self.find_window_node(id).map(|n| &n.state)
    }

    /// Nudge a `Floating` window by `delta_x`/`delta_y` in `direction`, clamped
    /// to the output bounds. Returns false if the window is absent or not floating.
    pub(crate) fn move_floating_window(
        &mut self,
        id: u32,
        direction: FocusDirection,
        delta_x: i32,
        delta_y: i32,
    ) -> bool {
        let (output_x, output_y, output_width, output_height) = (
            self.output_rect.x,
            self.output_rect.y,
            self.output_rect.width,
            self.output_rect.height,
        );
        let Some(node) = self.find_window_node_mut(id) else {
            return false;
        };

        let WindowState::Floating { rect } = &mut node.state else {
            return false;
        };

        let dx = match direction {
            FocusDirection::Left => -delta_x,
            FocusDirection::Right => delta_x,
            _ => 0,
        };
        let dy = match direction {
            FocusDirection::Up => -delta_y,
            FocusDirection::Down => delta_y,
            _ => 0,
        };

        rect.x += dx;
        rect.y += dy;

        let min_x = output_x;
        let max_x = output_x + output_width - rect.width;
        let min_y = output_y;
        let max_y = output_y + output_height - rect.height;

        if min_x <= max_x {
            rect.x = rect.x.clamp(min_x, max_x);
        }
        if min_y <= max_y {
            rect.y = rect.y.clamp(min_y, max_y);
        }

        true
    }

    /// Adjust the split ratio of the focused window's parent split in `direction`
    /// by `delta`, clamped to `[0.1, 0.9]`. Returns false if there is no focused
    /// window or no resizable parent split in that direction.
    pub(crate) fn resize_ratio(&mut self, direction: FocusDirection, delta: f64) -> bool {
        self.arrange();

        let Some(focused) = self.focused else {
            return false;
        };

        let Some((split, is_first)) = self.find_parent_split(focused) else {
            return false;
        };

        let ratio_delta = match (split.direction, is_first, direction) {
            (SplitDirection::Vertical, true, FocusDirection::Right) => delta,
            (SplitDirection::Vertical, true, FocusDirection::Left) => -delta,
            (SplitDirection::Vertical, false, FocusDirection::Left) => -delta,
            (SplitDirection::Vertical, false, FocusDirection::Right) => delta,
            (SplitDirection::Horizontal, true, FocusDirection::Down) => delta,
            (SplitDirection::Horizontal, true, FocusDirection::Up) => -delta,
            (SplitDirection::Horizontal, false, FocusDirection::Up) => -delta,
            (SplitDirection::Horizontal, false, FocusDirection::Down) => delta,
            _ => return false,
        };

        let new_ratio = (split.ratio + ratio_delta).clamp(0.1, 0.9);
        if (new_ratio - split.ratio).abs() < f64::EPSILON {
            return false;
        }
        split.ratio = new_ratio;
        self.arrange();
        true
    }

    /// Find the first ancestor split supporting the given resize direction and return
    /// the tiled window ID at the opposite side of that split.
    ///
    /// This enables resize navigation: when a user tries to resize in an unsupported
    /// direction (e.g., Up on a vertical split), we walk upward to find an ancestor
    /// split that does support that direction (e.g., a horizontal split) and focus
    /// the sibling window on the opposite side.
    ///
    /// Returns `None` if no ancestor supports the direction or if the found sibling
    /// is not tiled.
    pub(crate) fn focus_to_resize_target(&self, direction: FocusDirection) -> Option<u32> {
        let focused = self.focused?;

        /// Find the first ancestor split that supports the given direction.
        /// Returns the split node, whether focused is in first child, and the sibling node.
        fn find_ancestor_split(
            node: &LayoutNode,
            focused: u32,
            direction: FocusDirection,
        ) -> Option<(&LayoutSplit, bool, &LayoutNode)> {
            let (first, second) = node.children.as_ref()?;

            let focused_in_first = contains_window(first, focused);
            let focused_in_second = contains_window(second, focused);

            if !focused_in_first && !focused_in_second {
                return None;
            }

            let is_first = focused_in_first;
            let focused_child = if is_first { first } else { second };
            let sibling = if is_first { second } else { first };

            // Check if this split direction supports the resize direction.
            let split_supports_direction = matches!(
                (node.split.as_ref()?.direction, direction),
                (
                    SplitDirection::Vertical,
                    FocusDirection::Left | FocusDirection::Right
                ) | (
                    SplitDirection::Horizontal,
                    FocusDirection::Up | FocusDirection::Down
                )
            );

            if split_supports_direction {
                return Some((node.split.as_ref()?, is_first, sibling));
            }

            find_ancestor_split(focused_child, focused, direction)
        }

        let (_, _, sibling) = self
            .root
            .as_ref()
            .and_then(|root| find_ancestor_split(root, focused, direction))?;

        let sibling_id = if let Some(window) = sibling.window {
            window
        } else {
            find_tiled_window(sibling)?
        };

        if self
            .find_window_node(sibling_id)
            .is_some_and(|n| n.state.participates_in_tiling())
        {
            Some(sibling_id)
        } else {
            None
        }
    }

    /// Resize a `Floating` window by `delta_percent` of the output size in
    /// `direction`. Returns false if the window is absent or not floating.
    pub(crate) fn resize_floating_window(
        &mut self,
        id: u32,
        direction: FocusDirection,
        delta_percent: f32,
        is_expand: bool,
    ) -> bool {
        let output_width = self.output_rect.width;
        let output_height = self.output_rect.height;

        let Some(node) = self.find_window_node_mut(id) else {
            return false;
        };

        let WindowState::Floating { rect } = &mut node.state else {
            return false;
        };

        let raw_x = (output_width as f32 * delta_percent / 100.0).round();
        let raw_y = (output_height as f32 * delta_percent / 100.0).round();
        let delta_x = (raw_x.clamp(0.0, output_width as f32 / 2.0) as i32).max(1);
        let delta_y = (raw_y.clamp(0.0, output_height as f32 / 2.0) as i32).max(1);

        match direction {
            FocusDirection::Left => {
                if is_expand {
                    rect.x -= delta_x;
                    rect.width += delta_x;
                } else {
                    rect.x += delta_x;
                    rect.width = (rect.width - delta_x).max(1);
                }
            }
            FocusDirection::Right => {
                if is_expand {
                    rect.width += delta_x;
                } else {
                    rect.width = (rect.width - delta_x).max(1);
                }
            }
            FocusDirection::Up => {
                if is_expand {
                    rect.y -= delta_y;
                    rect.height += delta_y;
                } else {
                    rect.y += delta_y;
                    rect.height = (rect.height - delta_y).max(1);
                }
            }
            FocusDirection::Down => {
                if is_expand {
                    rect.height += delta_y;
                } else {
                    rect.height = (rect.height - delta_y).max(1);
                }
            }
        }

        true
    }

    fn find_parent_split(&mut self, target: u32) -> Option<(&mut LayoutSplit, bool)> {
        fn find(node: &mut LayoutNode, target: u32) -> Option<(&mut LayoutSplit, bool)> {
            if let Some((first, second)) = node.children.as_mut() {
                let in_first = contains_window(first, target);
                let in_second = contains_window(second, target);

                if in_first && first.window == Some(target) {
                    return Some((node.split.as_mut().unwrap(), true));
                }
                if in_second && second.window == Some(target) {
                    return Some((node.split.as_mut().unwrap(), false));
                }

                if in_first {
                    return find(first, target);
                }
                if in_second {
                    return find(second, target);
                }
            }
            None
        }

        self.root.as_deref_mut().and_then(|root| find(root, target))
    }

    /// Toggle fullscreen state for a window, preserving its previous state in `restore`.
    pub(crate) fn toggle_fullscreen(&mut self, id: u32) -> bool {
        let Some(node) = self.find_window_node_mut(id) else {
            return false;
        };

        let old = std::mem::replace(&mut node.state, WindowState::Tiled);
        node.state = match old {
            WindowState::Fullscreen { restore } => *restore,
            other => WindowState::Fullscreen {
                restore: Box::new(other),
            },
        };

        node.has_tiling = node.state.participates_in_tiling();

        if let Some(root) = self.root.as_mut() {
            update_ancestor_tiling_flags(root);
        }
        true
    }

    /// Toggle floating state for a window with the given rect.
    pub(crate) fn toggle_floating(&mut self, id: u32, rect: Rect) -> bool {
        self.toggle_state(id, WindowState::Floating { rect })
    }

    /// Toggle pseudo-tiled state for a window with the given rect.
    pub(crate) fn toggle_pseudo_tiled(&mut self, id: u32, rect: Rect) -> bool {
        self.toggle_state(id, WindowState::PseudoTiled { rect })
    }

    /// Handle state toggle for a window.
    fn toggle_state(&mut self, id: u32, target: WindowState) -> bool {
        let Some(node) = self.find_window_node_mut(id) else {
            return false;
        };

        let old = std::mem::replace(&mut node.state, WindowState::Tiled);
        node.state = match old {
            WindowState::Fullscreen { restore: _ } => target,
            WindowState::Floating { .. } => {
                if matches!(target, WindowState::Floating { .. }) {
                    WindowState::Tiled
                } else {
                    target
                }
            }
            WindowState::PseudoTiled { .. } => {
                if matches!(target, WindowState::PseudoTiled { .. }) {
                    WindowState::Tiled
                } else {
                    target
                }
            }
            _ => target,
        };

        node.has_tiling = node.state.participates_in_tiling();

        if let Some(root) = self.root.as_mut() {
            update_ancestor_tiling_flags(root);
        }
        true
    }

    /// Explicitly set a window to a given state (no toggle behavior).
    pub(crate) fn set_window_state(&mut self, id: u32, target: WindowState) -> bool {
        let Some(node) = self.find_window_node_mut(id) else {
            return false;
        };

        node.state = target;

        node.has_tiling = node.state.participates_in_tiling();

        if let Some(root) = self.root.as_mut() {
            update_ancestor_tiling_flags(root);
        }
        true
    }

    /// Apply layout and return window rectangles for the current focused window.
    pub(crate) fn arrange(&mut self) -> Vec<(u32, Rect)> {
        self.arranged_windows()
            .into_iter()
            .map(|(id, rect, _)| (id, rect))
            .collect()
    }

    /// Apply layout and return window rectangles with their states for the manage/render sequence.
    pub(crate) fn arranged_windows(&mut self) -> Vec<(u32, Rect, WindowState)> {
        if let Some(root) = self.root.as_mut() {
            apply_rects(root, self.output_rect, self.layout_config.gap.unwrap_or(0));
        }

        let mut windows = Vec::new();
        if let Some(root) = self.root.as_ref() {
            collect_windows_with_state(root, self.output_rect, &mut windows);
        }
        windows
    }

    /// Read-only variant of [`LayoutTree::arranged_windows`].
    ///
    /// Returns each window's current rect/state without re-arranging the tree,
    /// so it can be called from a `&self` context (e.g. the reconciler's
    /// `desired_scene`). Callers must have arranged the tree first (via
    /// `arranged_windows`, which populates node geometry from `output_rect`).
    pub(crate) fn arranged_windows_readonly(&self) -> Vec<(u32, Rect, WindowState)> {
        let mut windows = Vec::new();
        if let Some(root) = self.root.as_ref() {
            collect_windows_with_state(root, self.output_rect, &mut windows);
        }
        windows
    }

    #[cfg(test)]
    pub(crate) fn root_window(&self) -> Option<u32> {
        self.root.as_ref().and_then(|root| root.window)
    }

    fn focus_relative(&mut self, offset: isize) -> bool {
        let windows = self.visible_windows();
        if windows.is_empty() {
            return false;
        }

        let index = self
            .focused
            .and_then(|focused| windows.iter().position(|id| *id == focused))
            .unwrap_or(0);
        let next_index = if offset > 0 {
            (index + 1) % windows.len()
        } else {
            (index + windows.len() - 1) % windows.len()
        };

        self.focused = Some(windows[next_index]);
        true
    }

    /// Replace a placeholder node with a real window, returning true if found.
    fn replace_placeholder(&mut self, window: u32) -> bool {
        fn replace(node: &mut LayoutNode, window: u32) -> bool {
            if node.window.is_none() && node.children.is_none() {
                node.window = Some(window);
                node.has_tiling = true;
                return true;
            }

            if let Some((first, second)) = node.children.as_mut() {
                replace(first, window) || replace(second, window)
            } else {
                false
            }
        }

        if self.root.as_mut().is_some_and(|root| replace(root, window)) {
            if let Some(root) = self.root.as_mut() {
                update_ancestor_tiling_flags(root);
            }
            true
        } else {
            false
        }
    }

    /// Take ownership of a window node, returning None if not found.
    fn take_window_node(&mut self, id: u32) -> Option<Box<LayoutNode>> {
        fn take(node: &mut LayoutNode, id: u32) -> Option<Box<LayoutNode>> {
            if node.window == Some(id) {
                return Some(Box::new(LayoutNode {
                    window: node.window,
                    state: node.state.clone(),
                    rect: node.rect,
                    has_tiling: node.has_tiling,
                    split: node.split.clone(),
                    children: node.children.take(),
                }));
            }

            if let Some((first, second)) = node.children.as_mut() {
                if let Some(taken) = take(first, id) {
                    return Some(taken);
                }
                if let Some(taken) = take(second, id) {
                    return Some(taken);
                }
            }

            None
        }

        self.root.as_mut().and_then(|root| take(root, id))
    }

    /// Replace a window node with a new node, returning true if found.
    fn replace_window_node(&mut self, id: u32, node: Box<LayoutNode>) -> bool {
        fn replace(node: &mut LayoutNode, id: u32, replacement: Box<LayoutNode>) -> bool {
            if node.window == Some(id) {
                *node = *replacement;
                return true;
            }

            if let Some((first, second)) = node.children.as_mut() {
                replace(first, id, replacement.clone()) || replace(second, id, replacement)
            } else {
                false
            }
        }

        self.root
            .as_mut()
            .is_some_and(|root| replace(root, id, node))
    }

    /// Check if a window exists in the tree.
    fn contains_window(&self, id: u32) -> bool {
        self.find_window_node(id).is_some()
    }

    fn find_window_node(&self, id: u32) -> Option<&LayoutNode> {
        fn find(node: &LayoutNode, id: u32) -> Option<&LayoutNode> {
            if node.window == Some(id) {
                return Some(node);
            }

            node.children
                .as_ref()
                .and_then(|(first, second)| find(first, id).or_else(|| find(second, id)))
        }

        self.root.as_deref().and_then(|root| find(root, id))
    }

    fn find_window_node_mut(&mut self, id: u32) -> Option<&mut LayoutNode> {
        fn find(node: &mut LayoutNode, id: u32) -> Option<&mut LayoutNode> {
            if node.window == Some(id) {
                return Some(node);
            }

            node.children
                .as_mut()
                .and_then(|(first, second)| find(first, id).or_else(|| find(second, id)))
        }

        self.root.as_deref_mut().and_then(|root| find(root, id))
    }

    fn node_rect(&self, id: u32) -> Option<Rect> {
        self.find_window_node(id).map(|node| node.rect)
    }

    /// Return the first window found in the tree (depth-first), or `None` if empty.
    pub(crate) fn first_window(&self) -> Option<u32> {
        fn first(node: &LayoutNode) -> Option<u32> {
            if let Some(window) = node.window {
                return Some(window);
            }

            // Search both subtrees so a leftover placeholder/empty leaf in the
            // first child can never hide a real window in the second child.
            if let Some((first_child, second_child)) = node.children.as_ref() {
                return first(first_child).or_else(|| first(second_child));
            }

            None
        }

        self.root.as_deref().and_then(first)
    }

    /// Return the IDs of all windows in the tree.
    pub(crate) fn visible_windows(&self) -> Vec<u32> {
        let mut windows = Vec::new();
        if let Some(root) = self.root.as_ref() {
            collect_windows(root, &mut windows);
        }
        windows
    }

    fn visible_nodes(&self) -> Vec<(u32, Rect)> {
        let mut nodes = Vec::new();
        if let Some(root) = self.root.as_ref() {
            collect_rects(root, self.output_rect, &mut nodes);
        }
        nodes
    }
}

trait FocusSide {
    fn is_side_of(&self, other: Rect, direction: FocusDirection) -> bool;
}

impl FocusSide for Rect {
    fn is_side_of(&self, other: Rect, direction: FocusDirection) -> bool {
        match direction {
            FocusDirection::Right => self.is_right_of(other),
            FocusDirection::Left => self.is_left_of(other),
            FocusDirection::Down => self.is_below(other),
            FocusDirection::Up => self.is_above(other),
        }
    }
}

/// Apply layout rectangles to all nodes in the tree, respecting tiling state.
/// Non-tiling windows (floating, fullscreen) receive zero-sized rectangles to not affect siblings.
fn apply_rects(node: &mut LayoutNode, rect: Rect, gap: i32) {
    let Some((split, children)) = node.split.clone().zip(node.children.as_mut()) else {
        if let WindowState::Floating { rect: float_rect } = &node.state {
            node.rect = *float_rect;
        } else {
            node.rect = rect;
        }
        return;
    };

    node.rect = rect;

    let (first_rect, second_rect) = match (children.0.has_tiling, children.1.has_tiling) {
        (true, true) => split_rect(rect, split.direction, split.ratio, gap),
        (true, false) => (rect, Rect::new(0, 0, 0, 0)),
        (false, true) => (Rect::new(0, 0, 0, 0), rect),
        (false, false) => (Rect::new(0, 0, 0, 0), Rect::new(0, 0, 0, 0)),
    };

    apply_rects(&mut children.0, first_rect, gap);
    apply_rects(&mut children.1, second_rect, gap);
}

/// Update has_tiling flags from leaves upward to reflect current tree state.
fn update_ancestor_tiling_flags(root: &mut LayoutNode) {
    fn update(node: &mut LayoutNode) {
        if let Some((first, second)) = node.children.as_mut() {
            update(first);
            update(second);
            node.has_tiling = first.has_tiling || second.has_tiling;
        }
    }
    update(root);
}

/// Center a size within a slot rectangle while respecting minimum dimensions.
pub(crate) fn capped_rect(slot: Rect, size: Rect) -> Rect {
    let width = size.width.clamp(1, slot.width.max(1));
    let height = size.height.clamp(1, slot.height.max(1));

    Rect::new(
        slot.x + (slot.width - width) / 2,
        slot.y + (slot.height - height) / 2,
        width,
        height,
    )
}

/// Split a rectangle into two parts based on direction and ratio.
///
/// The `available` dimension is reduced by `gap` so the two children are
/// separated by that many pixels. The gap is placed as an offset on the
/// second child's leading edge, which keeps the first child flush with
/// the parent's leading edge.
fn split_rect(rect: Rect, direction: SplitDirection, ratio: f64, gap: i32) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.1, 0.9);

    match direction {
        SplitDirection::Vertical => {
            let available = (rect.width - gap).max(0);
            let first_width = ((available as f64) * ratio)
                .floor()
                .clamp(0.0, available as f64) as i32;
            let first = Rect::new(rect.x, rect.y, first_width, rect.height);
            let second = Rect::new(
                rect.x + first_width + gap,
                rect.y,
                available - first_width,
                rect.height,
            );
            (first, second)
        }
        SplitDirection::Horizontal => {
            let available = (rect.height - gap).max(0);
            let first_height = ((available as f64) * ratio)
                .floor()
                .clamp(0.0, available as f64) as i32;
            let first = Rect::new(rect.x, rect.y, rect.width, first_height);
            let second = Rect::new(
                rect.x,
                rect.y + first_height + gap,
                rect.width,
                available - first_height,
            );
            (first, second)
        }
    }
}

/// Convert window state to its actual rendering rectangle.
fn rect_for_state(state: &WindowState, rect: Rect, output_rect: Rect) -> Rect {
    match state {
        WindowState::Fullscreen { .. } => output_rect,
        WindowState::Floating { rect: float_rect } => *float_rect,
        WindowState::PseudoTiled { rect: pseudo_rect } => capped_rect(rect, *pseudo_rect),
        WindowState::Tiled => rect,
    }
}

/// Collect window IDs with their rectangles for visible nodes traversal.
fn collect_rects(node: &LayoutNode, output_rect: Rect, rects: &mut Vec<(u32, Rect)>) {
    if let Some(window) = node.window {
        let rect = rect_for_state(&node.state, node.rect, output_rect);
        rects.push((window, rect));
    }
    if let Some((first, second)) = node.children.as_ref() {
        collect_rects(first, output_rect, rects);
        collect_rects(second, output_rect, rects);
    }
}

fn collect_windows_with_state(
    node: &LayoutNode,
    output_rect: Rect,
    windows: &mut Vec<(u32, Rect, WindowState)>,
) {
    if let Some(window) = node.window {
        let rect = rect_for_state(&node.state, node.rect, output_rect);
        windows.push((window, rect, node.state.clone()));
    }
    if let Some((first, second)) = node.children.as_ref() {
        collect_windows_with_state(first, output_rect, windows);
        collect_windows_with_state(second, output_rect, windows);
    }
}

/// Collect window IDs for visible windows traversal (focus order).
fn collect_windows(node: &LayoutNode, windows: &mut Vec<u32>) {
    if let Some(window) = node.window {
        windows.push(window);
    }
    if let Some((first, second)) = node.children.as_ref() {
        collect_windows(first, windows);
        collect_windows(second, windows);
    }
}

/// Check whether a subtree contains a window with the given ID.
fn contains_window(node: &LayoutNode, target: u32) -> bool {
    if node.window == Some(target) {
        return true;
    }
    if let Some((first, second)) = node.children.as_ref() {
        return contains_window(first, target) || contains_window(second, target);
    }
    false
}

/// Find the first tiled window in a subtree, skipping placeholders and non-tiling windows.
fn find_tiled_window(node: &LayoutNode) -> Option<u32> {
    if let Some(window) = node.window
        && node.state.participates_in_tiling()
    {
        return Some(window);
    }
    if let Some((first, second)) = node.children.as_ref() {
        find_tiled_window(first).or_else(|| find_tiled_window(second))
    } else {
        None
    }
}

/// Apply the outcome of removing a window from a child node to the sibling.
///
/// For `Replaced`, the child node is updated to the replacement. For `LeafRemoved`,
/// the child has already been emptied in place (it is now a placeholder leaf); the
/// current split must collapse to the sibling so we never leave a dangling empty
/// placeholder behind. If neither child has tiling windows after the operation, the
/// sibling is replaced with an empty leaf.
fn apply_child_outcome(
    child: &mut Box<LayoutNode>,
    sibling: &mut Box<LayoutNode>,
    outcome: RemoveOutcome,
) -> RemoveOutcome {
    match outcome {
        RemoveOutcome::Replaced(new_child) => {
            *child = new_child;
        }
        RemoveOutcome::LeafRemoved => {
            // `child` is now an empty placeholder leaf. Collapse the current split
            // down to the sibling so the sibling takes the freed space and no
            // empty node is left behind. The sibling keeps its window/state, so
            // this is correct for tiling siblings *and* floating/fullscreen ones.
            return RemoveOutcome::Replaced(std::mem::replace(
                sibling,
                Box::new(LayoutNode::leaf(None)),
            ));
        }
        RemoveOutcome::Modified => return RemoveOutcome::Modified,
        RemoveOutcome::NotFound => unreachable!(),
    }

    let has_tiling = child.has_tiling || sibling.has_tiling;
    if !has_tiling {
        let empty = Box::new(LayoutNode::leaf(None));
        RemoveOutcome::Replaced(std::mem::replace(sibling, empty))
    } else {
        RemoveOutcome::Modified
    }
}

/// Remove a window from the subtree rooted at this node.
///
/// Recursively searches for the window in the tree and removes it if found.
/// Returns an outcome indicating what action the parent should take:
///
/// - `LeafRemoved`: The window was found and removed from this leaf node.
///   The node's window is now None. If this was the last window in the tree,
///   the parent should clear the root.
///
/// - `Replaced(node)`: The child subtree collapsed to an empty state (a leaf with
///   no window and no children). The returned node is the sibling that should take
///   its place. This happens when a window is removed and that child becomes empty.
///
/// - `Modified`: The removal modified a child subtree but no collapse occurred.
///   The child still has tiling windows (has_tiling=true), so only tiling flags
///   need to be updated. The parent should propagate this up without collapsing.
///
/// - `NotFound`: The window was not found in this subtree.
///
/// The key distinction: `Replaced` signals a structural change (collapse), while
/// `Modified` signals a content change (window removed but structure preserved).
/// This ensures that when a tiled window is removed, remaining tiled siblings
/// correctly extend to take the freed space.
fn remove_from_node(node: &mut LayoutNode, id: u32) -> RemoveOutcome {
    if let Some((first, second)) = node.children.as_mut() {
        match remove_from_node(first, id) {
            RemoveOutcome::NotFound => match remove_from_node(second, id) {
                RemoveOutcome::NotFound => RemoveOutcome::NotFound,
                outcome => apply_child_outcome(second, first, outcome),
            },
            outcome => apply_child_outcome(first, second, outcome),
        }
    } else if node.window == Some(id) {
        node.window = None;
        node.has_tiling = false;
        RemoveOutcome::LeafRemoved
    } else {
        RemoveOutcome::NotFound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(layout: &LayoutTree) -> Vec<(u32, Rect)> {
        let mut layout = layout.clone();
        layout.arrange()
    }

    #[test]
    fn empty_tree_inserts_first_window_as_root_and_focuses_it() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 100, 100));

        assert!(layout.insert_window(1));

        assert_eq!(layout.focused_window(), Some(1));
        assert_eq!(layout.root_window(), Some(1));
        assert_eq!(visible(&layout), vec![(1, Rect::new(0, 0, 100, 100))]);
    }

    #[test]
    fn longest_side_split_chooses_vertical_for_wide_rectangles() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.arrange();

        layout.insert_window(2);

        let root = layout.root.as_ref().unwrap();
        assert_eq!(
            root.split.as_ref().unwrap().direction,
            SplitDirection::Vertical
        );
        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 500, 100)),
                (2, Rect::new(500, 0, 500, 100))
            ]
        );
    }

    #[test]
    fn longest_side_split_chooses_horizontal_for_tall_rectangles() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 100, 1000));
        layout.insert_window(1);
        layout.arrange();

        layout.insert_window(2);

        let root = layout.root.as_ref().unwrap();
        assert_eq!(
            root.split.as_ref().unwrap().direction,
            SplitDirection::Horizontal
        );
        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 100, 500)),
                (2, Rect::new(0, 500, 100, 500))
            ]
        );
    }

    #[test]
    fn manual_vertical_split_creates_vertical_parent_with_focused_and_empty_leaf() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.arrange();

        let placeholder = layout.split_focused(SplitDirection::Vertical).unwrap();

        assert_eq!(placeholder, 0);
        let root = layout.root.as_ref().unwrap();
        assert_eq!(
            root.split.as_ref().unwrap().direction,
            SplitDirection::Vertical
        );
        assert_eq!(root.first_child().unwrap().window, Some(1));
        assert_eq!(root.second_child().unwrap().window, None);
    }

    #[test]
    fn remove_leaf_collapses_parent_into_sibling() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.arrange();

        layout.focus_window(2);
        assert!(layout.remove_window(2));

        assert_eq!(layout.focused_window(), Some(1));
        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 500, 100)),
                (3, Rect::new(500, 0, 500, 100))
            ]
        );
    }

    #[test]
    fn remove_last_leaf_leaves_empty_tree() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 100, 100));
        layout.insert_window(1);

        assert!(layout.remove_window(1));

        assert!(layout.root.is_none());
        assert_eq!(layout.focused_window(), None);
        assert!(visible(&layout).is_empty());
    }

    #[test]
    fn next_and_previous_traverse_tiled_windows() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);

        // insert_window focuses the newly inserted window, so the last
        // window is focused after the third insert.
        assert_eq!(layout.focused_window(), Some(3));

        assert!(layout.focus_next());
        assert_eq!(layout.focused_window(), Some(1));
        assert!(layout.focus_previous());
        assert_eq!(layout.focused_window(), Some(3));
    }

    #[test]
    fn focus_direction_includes_floating_windows() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.arrange();

        // Window 1 is at x=0..500, window 3 at x=750..1000.
        // Make window 2 floating at x=500 so it is to the right of window 1
        // and closer than window 3.
        let float_rect = Rect::new(500, 20, 200, 50);
        assert!(layout.toggle_floating(2, float_rect));

        layout.focus_window(1);

        // Direction right should target the floating window 2 because it is
        // closer than window 3.
        assert!(layout.focus_direction(FocusDirection::Right));
        assert_eq!(layout.focused_window(), Some(2));
    }

    #[test]
    fn focus_direction_works_with_gap() {
        use crate::config::LayoutConfig;

        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.set_layout_config(LayoutConfig {
            gap: Some(10),
            ..LayoutConfig::default()
        });

        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.arrange();

        // Focus window 1, then move right — should reach window 2, not skip to 3
        layout.focus_window(1);
        assert!(layout.focus_direction(FocusDirection::Right));
        assert_eq!(layout.focused_window(), Some(2));

        // Move right again — should reach window 3
        assert!(layout.focus_direction(FocusDirection::Right));
        assert_eq!(layout.focused_window(), Some(3));

        // Move left twice — should go back to 2, then 1
        assert!(layout.focus_direction(FocusDirection::Left));
        assert_eq!(layout.focused_window(), Some(2));
        assert!(layout.focus_direction(FocusDirection::Left));
        assert_eq!(layout.focused_window(), Some(1));
    }

    #[test]
    fn fullscreen_window_bypasses_tiling_space() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_fullscreen(1));

        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 1000, 100)),
                (2, Rect::new(0, 0, 1000, 100))
            ]
        );
    }

    #[test]
    fn output_resize_recomputes_rectangles_without_changing_topology() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.set_output_rect(Rect::new(0, 0, 2000, 200));

        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 1000, 200)),
                (2, Rect::new(1000, 0, 1000, 200))
            ]
        );
    }

    #[test]
    fn placeholder_is_replaced_by_next_inserted_window_and_becomes_focused() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.split_focused(SplitDirection::Vertical).unwrap();

        assert!(layout.insert_window(2));

        assert_eq!(layout.focused_window(), Some(2));
        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 500, 100)),
                (2, Rect::new(500, 0, 500, 100))
            ]
        );
    }

    #[test]
    fn removing_focused_window_refocuses_first_remaining_window() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.arrange();

        layout.focus_window(2);
        assert!(layout.remove_window(2));

        assert_eq!(layout.focused_window(), Some(1));
        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 500, 100)),
                (3, Rect::new(500, 0, 500, 100))
            ]
        );
    }

    #[test]
    fn toggle_floating_off_restores_tiled_state() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.arrange();

        // Toggle floating on
        assert!(layout.toggle_floating(1, Rect::new(10, 10, 200, 50)));
        // Toggle floating off
        assert!(layout.toggle_floating(1, Rect::new(0, 0, 0, 0)));

        // Window 1 should be tiled again, consuming full tiling space
        assert_eq!(visible(&layout), vec![(1, Rect::new(0, 0, 1000, 100))]);
    }

    #[test]
    fn pseudo_tiled_window_is_centered_inside_its_tiling_slot() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.arrange();

        assert!(layout.toggle_pseudo_tiled(2, Rect::new(10, 10, 200, 50)));

        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(0, 0, 500, 100)),
                (2, Rect::new(525, 25, 200, 50)),
                (3, Rect::new(750, 0, 250, 100))
            ]
        );
    }

    #[test]
    fn fullscreen_toggle_preserves_floating_state() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_floating(1, Rect::new(10, 10, 200, 50)));
        assert!(layout.toggle_fullscreen(1));
        assert!(layout.toggle_fullscreen(1));

        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(10, 10, 200, 50)),
                (2, Rect::new(0, 0, 1000, 100))
            ]
        );
    }

    #[test]
    fn fullscreen_toggle_preserves_pseudo_tiled_state() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_pseudo_tiled(1, Rect::new(10, 10, 200, 50)));
        assert!(layout.toggle_fullscreen(1));
        assert!(layout.toggle_fullscreen(1));

        assert_eq!(
            visible(&layout),
            vec![
                (1, Rect::new(150, 25, 200, 50)),
                (2, Rect::new(500, 0, 500, 100))
            ]
        );
    }

    #[test]
    fn remove_tiled_window_extends_remaining_tiled() {
        // When removing a tiled window, remaining tiled windows should extend
        // to take the freed space
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));

        // Add 4 windows - each splits from the previously focused
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.insert_window(4);
        layout.arrange();

        // Before removal: 4 windows sharing space
        let before = visible(&layout);

        assert_eq!(before.len(), 4);

        // Remove window 4 (deepest leaf) - windows 1, 2, 3 should remain
        assert!(layout.remove_window(4));

        let after = visible(&layout);

        assert_eq!(
            after.len(),
            3,
            "Expected 3 windows after removal, got {}: {:?}",
            after.len(),
            after
        );

        // All remaining windows should have non-zero dimensions
        for (id, rect) in &after {
            assert!(
                rect.width > 0 || rect.height > 0,
                "Window {id} has zero rect"
            );
        }

        // Remove window 3 - windows 1, 2 should remain
        assert!(layout.remove_window(3));

        let after = visible(&layout);

        assert_eq!(after.len(), 2, "Expected 2 windows after second removal");

        // Remove window 2 - window 1 should remain full screen
        assert!(layout.remove_window(2));

        let after = visible(&layout);
        assert_eq!(after.len(), 1, "Expected 1 window after third removal");
        assert_eq!(
            after[0].1.width, 1000,
            "Remaining window should be full width"
        );
        assert_eq!(
            after[0].1.height, 100,
            "Remaining window should be full height"
        );
    }

    #[test]
    fn remove_middle_tiled_window_extends_siblings() {
        // Test: spawn 4 windows, close window 2 (middle), verify remaining extend
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));

        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.arrange();

        // Tree: split(leaf(1), split(leaf(2), leaf(3))) with widths 250, 250, 500
        let before = visible(&layout);
        assert_eq!(before.len(), 3);

        // Remove window 2 - windows 1 and 3 should extend
        assert!(layout.remove_window(2));

        let after = visible(&layout);
        assert_eq!(
            after.len(),
            2,
            "Expected 2 windows after removing middle one"
        );

        // Both remaining windows should have non-zero dimensions summing to 1000
        let total_width: i32 = after.iter().map(|(_, r)| r.width).sum();
        assert_eq!(
            total_width, 1000,
            "Total width should be 1000, got {}",
            total_width
        );
    }

    #[test]
    fn focus_direction_after_close_starts_from_correct_window() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.arrange();

        // Focus window 3
        layout.focus_window(3);

        // Remove focused window 3
        assert!(layout.remove_window(3));

        // Now layout focuses window 1 (first_window)
        assert_eq!(layout.focused_window(), Some(1));

        // Direction focus right from 1 should go to the remaining window (2)
        assert!(layout.focus_direction(FocusDirection::Right));
        assert_eq!(layout.focused_window(), Some(2));
    }

    #[test]
    fn spawn_four_windows_then_toggle_floating_on_last_does_not_break_first() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.insert_window(4);
        layout.arrange();

        // Simulate what push_focus does in handlers.rs: only update
        // state.focused_window, NOT layout.focused.
        // layout.focused is now stale (still 1), but the WM will
        // toggle_floating on window 4 because state.focused_window = 4.
        layout.focus_window(4);

        let float_rect = Rect::new(0, 0, 200, 50);
        assert!(layout.toggle_floating(4, float_rect));

        let after = visible(&layout);

        // Window 4 should now be floating at the requested position.
        assert!(after.contains(&(4, float_rect)));

        // Windows 1, 2, 3 should still be tiled and visible with non-zero size.
        let tiled: Vec<_> = after.iter().filter(|(id, _)| *id != 4).collect();
        assert_eq!(tiled.len(), 3, "Expected 3 tiled windows, got: {:?}", after);
        for (id, rect) in tiled {
            assert!(
                rect.width > 0 && rect.height > 0,
                "Tiled window {id} has zero rect: {rect:?}"
            );
        }
    }

    #[test]
    fn gap_inserts_space_between_tiled_windows() {
        use crate::config::LayoutConfig;

        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.set_layout_config(LayoutConfig {
            gap: Some(10),
            ..LayoutConfig::default()
        });

        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        let visible = visible(&layout);
        assert_eq!(visible.len(), 2);

        let (_, rect1) = visible[0];
        let (_, rect2) = visible[1];

        assert_eq!(rect1.x, 10);
        assert_eq!(rect1.width, 485);
        assert_eq!(rect2.x, 505);
        assert_eq!(rect2.width, 485);
    }

    #[test]
    fn margins_inset_the_tiling_area() {
        use crate::config::LayoutConfig;

        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.set_layout_config(LayoutConfig {
            margin_top: Some(10),
            margin_right: Some(20),
            margin_bottom: Some(30),
            margin_left: Some(40),
            ..LayoutConfig::default()
        });

        layout.insert_window(1);
        layout.arrange();

        let visible = visible(&layout);
        assert_eq!(visible.len(), 1);

        let (_, rect) = visible[0];
        assert_eq!(rect.x, 40);
        assert_eq!(rect.y, 10);
        assert_eq!(rect.width, 940);
        assert_eq!(rect.height, 60);
    }

    #[test]
    fn gap_with_margins_combines_correctly() {
        use crate::config::LayoutConfig;

        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.set_layout_config(LayoutConfig {
            gap: Some(10),
            margin_top: Some(5),
            margin_right: Some(10),
            margin_bottom: Some(5),
            margin_left: Some(10),
            default_float_ratio: None,
        });

        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        let visible = visible(&layout);
        assert_eq!(visible.len(), 2);

        let (_, rect1) = visible[0];
        let (_, rect2) = visible[1];

        assert_eq!(rect1.x, 20);
        assert_eq!(rect1.y, 15);
        assert_eq!(rect1.width, 475);
        assert_eq!(rect1.height, 70);
        assert_eq!(rect2.x, 505);
        assert_eq!(rect2.y, 15);
        assert_eq!(rect2.width, 475);
        assert_eq!(rect2.height, 70);
    }

    #[test]
    fn swap_windows_exchanges_positions_of_two_tiled_windows() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(1);
        assert!(layout.swap_windows(FocusDirection::Right));

        let after = visible(&layout);
        assert_eq!(after.len(), 2);

        let window_1_rect = after.iter().find(|(id, _)| *id == 1).map(|(_, r)| *r);
        let window_2_rect = after.iter().find(|(id, _)| *id == 2).map(|(_, r)| *r);

        assert_eq!(window_1_rect, Some(Rect::new(500, 0, 500, 100)));
        assert_eq!(window_2_rect, Some(Rect::new(0, 0, 500, 100)));
    }

    #[test]
    fn swap_windows_returns_false_when_no_target_exists() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.arrange();

        layout.focus_window(1);
        assert!(!layout.swap_windows(FocusDirection::Right));
    }

    #[test]
    fn swap_windows_returns_false_with_single_window() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.arrange();

        layout.focus_window(1);
        assert!(!layout.swap_windows(FocusDirection::Left));
    }

    #[test]
    fn move_floating_window_updates_floating_rect() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_floating(2, Rect::new(10, 10, 200, 50)));

        let delta_x = 100;
        let delta_y = 100;
        assert!(layout.move_floating_window(2, FocusDirection::Right, delta_x, delta_y));
        assert!(layout.move_floating_window(2, FocusDirection::Down, delta_x, delta_y));

        let node = layout.find_window_node(2).unwrap();
        let WindowState::Floating { rect } = &node.state else {
            panic!("expected floating");
        };
        assert_eq!(*rect, Rect::new(110, 110, 200, 50));
    }

    #[test]
    fn resize_vertical_split_first_child_expand_right() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(1);
        assert!(layout.resize_ratio(FocusDirection::Right, 0.1));

        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        let rect2 = arranged
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect1.width > 500,
            "Left child should grow: got {}",
            rect1.width
        );
        assert!(
            rect2.width < 500,
            "Right child should shrink: got {}",
            rect2.width
        );
    }

    #[test]
    fn resize_vertical_split_first_child_shrink_left() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(1);
        assert!(layout.resize_ratio(FocusDirection::Left, 0.1));

        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        let rect2 = arranged
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect1.width < 500,
            "Left child should shrink: got {}",
            rect1.width
        );
        assert!(
            rect2.width > 500,
            "Right child should grow: got {}",
            rect2.width
        );
    }

    #[test]
    fn resize_horizontal_split_first_child_expand_down() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 100, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(1);
        assert!(layout.resize_ratio(FocusDirection::Down, 0.1));

        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        let rect2 = arranged
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect1.height > 500,
            "Top child should grow: got {}",
            rect1.height
        );
        assert!(
            rect2.height < 500,
            "Bottom child should shrink: got {}",
            rect2.height
        );
    }

    #[test]
    fn resize_ratio_clamps_at_boundaries() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(1);
        for _ in 0..20 {
            let _ = layout.resize_ratio(FocusDirection::Right, 0.05);
        }
        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect1.width <= 900,
            "Ratio should clamp, width={}",
            rect1.width
        );

        let mut layout2 = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout2.insert_window(1);
        layout2.insert_window(2);
        layout2.arrange();
        layout2.focus_window(1);
        for _ in 0..20 {
            let _ = layout2.resize_ratio(FocusDirection::Left, 0.05);
        }
        let arranged2 = visible(&layout2);
        let rect1 = arranged2
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect1.width >= 100,
            "Ratio should clamp, width={}",
            rect1.width
        );
    }

    #[test]
    fn resize_floating_window_changes_dimensions() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_floating(2, Rect::new(10, 10, 200, 50)));
        assert!(layout.resize_floating_window(2, FocusDirection::Right, 5.0, true));
        assert!(layout.resize_floating_window(2, FocusDirection::Down, 5.0, true));

        let node = layout.find_window_node(2).unwrap();
        let WindowState::Floating { rect } = &node.state else {
            panic!("expected floating");
        };
        assert!(rect.width > 200, "width={}", rect.width);
        assert!(rect.height > 50, "height={}", rect.height);
    }

    #[test]
    fn resize_floating_window_shrink_left_shifts_position() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_floating(2, Rect::new(100, 100, 200, 50)));
        let WindowState::Floating { rect: before } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };

        assert!(layout.resize_floating_window(2, FocusDirection::Left, 10.0, false)); // shrink

        let WindowState::Floating { rect: after } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };
        assert!(
            after.width < before.width,
            "width should shrink: {} -> {}",
            before.width,
            after.width
        );
        assert!(
            after.x > before.x,
            "x should shift right when shrinking left border: {} -> {}",
            before.x,
            after.x
        );
    }

    #[test]
    fn resize_floating_window_shrink_up_shifts_position() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_floating(2, Rect::new(100, 100, 200, 50)));
        let WindowState::Floating { rect: before } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };

        assert!(layout.resize_floating_window(2, FocusDirection::Up, 10.0, false)); // shrink

        let WindowState::Floating { rect: after } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };
        assert!(
            after.height < before.height,
            "height should shrink: {} -> {}",
            before.height,
            after.height
        );
        assert!(
            after.y > before.y,
            "y should shift down when shrinking top border: {} -> {}",
            before.y,
            after.y
        );
    }

    #[test]
    fn resize_floating_window_expand_left_shifts_position() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_floating(2, Rect::new(200, 100, 200, 50)));
        let WindowState::Floating { rect: before } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };

        assert!(layout.resize_floating_window(2, FocusDirection::Left, 10.0, true)); // expand

        let WindowState::Floating { rect: after } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };
        assert!(
            after.width > before.width,
            "width should expand: {} -> {}",
            before.width,
            after.width
        );
        assert!(
            after.x < before.x,
            "x should shift left when expanding left border: {} -> {}",
            before.x,
            after.x
        );
    }

    #[test]
    fn resize_floating_window_expand_up_shifts_position() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        assert!(layout.toggle_floating(2, Rect::new(100, 200, 200, 50)));
        let WindowState::Floating { rect: before } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };

        assert!(layout.resize_floating_window(2, FocusDirection::Up, 10.0, true)); // expand

        let WindowState::Floating { rect: after } = layout.find_window_node(2).unwrap().state
        else {
            panic!("expected floating");
        };
        assert!(
            after.height > before.height,
            "height should expand: {} -> {}",
            before.height,
            after.height
        );
        assert!(
            after.y < before.y,
            "y should shift up when expanding top border: {} -> {}",
            before.y,
            after.y
        );
    }

    #[test]
    fn resize_single_window_returns_false() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.arrange();

        layout.focus_window(1);
        assert!(!layout.resize_ratio(FocusDirection::Right, 0.05));
    }

    #[test]
    fn resize_vertical_split_second_child_expand_left() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(2);
        // Second child (right) expand left = take space from first = decrease ratio = -delta
        assert!(layout.resize_ratio(FocusDirection::Left, 0.1));

        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        let rect2 = arranged
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect2.width > 500,
            "Right child should grow: got {}",
            rect2.width
        );
        assert!(
            rect1.width < 500,
            "Left child should shrink: got {}",
            rect1.width
        );
    }

    #[test]
    fn resize_vertical_split_second_child_shrink_right() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(2);
        // Second child (right) shrink right = give space to first = increase ratio = +delta
        assert!(layout.resize_ratio(FocusDirection::Right, 0.1));

        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        let rect2 = arranged
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect2.width < 500,
            "Right child should shrink: got {}",
            rect2.width
        );
        assert!(
            rect1.width > 500,
            "Left child should grow: got {}",
            rect1.width
        );
    }

    #[test]
    fn resize_horizontal_split_second_child_expand_up() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 100, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(2);
        // Second child (bottom) expand up = take space from first = decrease ratio = -delta
        assert!(layout.resize_ratio(FocusDirection::Up, 0.1));

        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        let rect2 = arranged
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect2.height > 500,
            "Bottom child should grow: got {}",
            rect2.height
        );
        assert!(
            rect1.height < 500,
            "Top child should shrink: got {}",
            rect1.height
        );
    }

    #[test]
    fn resize_horizontal_split_second_child_shrink_down() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 100, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(2);
        // Second child (bottom) shrink down = give space to first = increase ratio = +delta
        assert!(layout.resize_ratio(FocusDirection::Down, 0.1));

        let arranged = visible(&layout);
        let rect1 = arranged
            .iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, r)| *r)
            .unwrap();
        let rect2 = arranged
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, r)| *r)
            .unwrap();
        assert!(
            rect2.height < 500,
            "Bottom child should shrink: got {}",
            rect2.height
        );
        assert!(
            rect1.height > 500,
            "Top child should grow: got {}",
            rect1.height
        );
    }

    #[test]
    fn focus_to_resize_target_vertical_split_finds_horizontal_ancestor() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.insert_window(4);
        layout.arrange();

        layout.focus_window(4);
        let target = layout.focus_to_resize_target(FocusDirection::Up);
        assert_eq!(target, Some(2));
    }

    #[test]
    fn focus_to_resize_target_horizontal_split_finds_vertical_ancestor() {
        // Tree structure with square output creates: split(v, w1, split(h, w2, split(v, w3, w4)))
        // Window 2 is in a horizontal split (doesn't support Left/Right)
        // Window 2's grandparent is a vertical split (supports Left/Right)
        // Focusing w2 and pressing Left should find the vertical ancestor's sibling = w1
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.insert_window(4);
        layout.arrange();

        // Verify tree: w2 should be in a horizontal split (doesn't support Left)
        let w2_rect = layout.node_rect(2).unwrap();
        assert_eq!(w2_rect.x, 500); // Right of w1

        layout.focus_window(2);
        let target = layout.focus_to_resize_target(FocusDirection::Left);
        assert_eq!(target, Some(1));
    }

    #[test]
    fn focus_to_resize_target_returns_none_when_no_ancestor_supports_direction() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 100));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.arrange();

        layout.focus_window(2);
        let target = layout.focus_to_resize_target(FocusDirection::Up);
        assert_eq!(target, None);
    }

    #[test]
    fn focus_to_resize_target_returns_none_at_root() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.arrange();

        layout.focus_window(1);
        let target = layout.focus_to_resize_target(FocusDirection::Up);
        assert_eq!(target, None);
    }

    #[test]
    fn focus_to_resize_target_skips_non_tiling_siblings() {
        let mut layout = LayoutTree::new(Rect::new(0, 0, 1000, 1000));
        layout.insert_window(1);
        layout.insert_window(2);
        layout.insert_window(3);
        layout.insert_window(4);
        layout.arrange();

        layout.focus_window(4);
        assert!(layout.toggle_floating(2, Rect::new(10, 10, 200, 50)));

        let target = layout.focus_to_resize_target(FocusDirection::Up);
        assert_eq!(target, None);
    }
}
