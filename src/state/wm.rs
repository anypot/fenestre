//! Core runtime state for Fenestre.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use super::keybindings::{KeyBinding, XkbBindingId};
use super::output::{Output, OutputId};
use super::seat::{Seat, SeatId};
use super::window::{Window, WindowId};
use crate::config::Config;
use crate::layout::{LayoutTree, Rect};
use crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1;
use crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1;
use crate::protocol::river::river_window_management_v1::client::river_window_manager_v1::RiverWindowManagerV1;
use crate::protocol::river::river_window_management_v1::client::river_window_v1::Edges;
use crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1;
use crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_binding_v1::RiverXkbBindingV1;
use crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_bindings_v1::RiverXkbBindingsV1;
use log::debug;
use wayland_client::QueueHandle;

/// Owns all mutable compositor state for the window manager.
///
/// `WMState` stores River protocol proxies, windows, outputs, seats,
/// runtime keybindings, focus state, configuration, and pending River-managed
/// state changes that are applied during manage/render sequences.
///
/// Protocol event handlers mutate this state. Configuration reconciliation updates
/// runtime keybindings and queues window-rule reapplication.
pub(crate) struct WMState {
    pub(super) wm: Option<RiverWindowManagerV1>,
    pub(super) xkb_bindings: Option<RiverXkbBindingsV1>,
    pub(super) config: Option<Config>,
    pub(super) config_path: Option<PathBuf>,

    pub(super) windows: HashMap<WindowId, Window>,
    pub(super) outputs: HashMap<OutputId, Output>,
    pub(super) seats: BTreeMap<SeatId, Seat>,
    pub(super) keybindings: HashMap<XkbBindingId, KeyBinding>,
    pub(super) output_trees: HashMap<OutputId, LayoutTree>,

    pub(super) focused_window: Option<WindowId>,
    pub(super) focused_output: Option<OutputId>,
    pub(super) focus_stack: Vec<WindowId>,

    pub(super) current_seat: Option<SeatId>,

    /// True when desired xkb bindings differ from configured River binding objects.
    pub(super) xkb_bindings_dirty: bool,
    /// River xkb binding protocol objects queued for destruction during the next manage sequence.
    pub(super) pending_xkb_binding_destroys: Vec<RiverXkbBindingV1>,
    /// Window rules loaded from config, applied per-window when metadata is known.
    pub(super) window_rules: Option<super::rule::WindowRules>,
    /// Window ID to focus during the next manage sequence.
    pub(super) pending_focus: Option<WindowId>,
    /// Window IDs to close during the next manage sequence.
    pub(super) pending_closes: Vec<WindowId>,

    /// Cached sorted window IDs for render stacking order.
    pub(super) render_order_cache: Vec<WindowId>,

    /// Next identifiers for internal ID allocation.
    next_window_id: WindowId,
    next_output_id: OutputId,
    next_seat_id: SeatId,
    next_xkb_binding_id: XkbBindingId,

    /// Proxy-to-ID indexes for O(1) lookup by Wayland object.
    pub(super) windows_by_proxy: HashMap<RiverWindowV1, WindowId>,
    pub(super) outputs_by_proxy: HashMap<RiverOutputV1, OutputId>,
    pub(super) seats_by_proxy: HashMap<RiverSeatV1, SeatId>,

    /// Windows grouped by output for O(1) lookup of which windows belong to
    /// a given output. Currently keyed by `OutputId`; if River exposes a
    /// workspace concept, consider evolving this into a `SurfaceId` enum
    /// (Output / Workspace / OutputWorkspace) so the same index can serve
    /// both output-dimension and workspace-dimension lookups.
    pub(super) windows_by_output: HashMap<OutputId, HashSet<WindowId>>,
}

impl WMState {
    /// Create a new `WMState` and load default configuration.
    pub(crate) fn new() -> Self {
        let mut state = Self {
            wm: None,
            xkb_bindings: None,
            config: None,
            config_path: None,

            windows: HashMap::new(),
            outputs: HashMap::new(),
            seats: BTreeMap::new(),
            keybindings: HashMap::new(),
            output_trees: HashMap::new(),

            focused_window: None,
            focused_output: None,
            focus_stack: Vec::new(),

            current_seat: None,

            xkb_bindings_dirty: false,
            pending_xkb_binding_destroys: Vec::new(),
            window_rules: None,
            pending_focus: None,
            pending_closes: Vec::new(),

            next_window_id: WindowId(0),
            next_output_id: OutputId(0),
            next_seat_id: SeatId(0),
            next_xkb_binding_id: XkbBindingId(0),
            render_order_cache: Vec::new(),
            windows_by_proxy: HashMap::new(),
            outputs_by_proxy: HashMap::new(),
            seats_by_proxy: HashMap::new(),
            windows_by_output: HashMap::new(),
        };

        state.load_default_config();

        state
    }

    /// Allocate the next internal window identifier.
    pub(super) fn next_window_id(&mut self) -> WindowId {
        let id = self.next_window_id;
        self.next_window_id.0 += 1;
        id
    }

    /// Allocate the next internal output identifier.
    pub(super) fn next_output_id(&mut self) -> OutputId {
        let id = self.next_output_id;
        self.next_output_id.0 += 1;
        id
    }

    /// Allocate the next internal seat identifier.
    pub(super) fn next_seat_id(&mut self) -> SeatId {
        let id = self.next_seat_id;
        self.next_seat_id.0 += 1;
        id
    }

    /// Allocate the next internal River xkb binding identifier.
    pub(super) fn next_xkb_binding_id(&mut self) -> XkbBindingId {
        let id = self.next_xkb_binding_id;
        self.next_xkb_binding_id.0 += 1;
        id
    }

    /// Add a window to the per-output index.
    pub(super) fn index_window_in_output(&mut self, window_id: WindowId, output_id: OutputId) {
        self.windows_by_output
            .entry(output_id)
            .or_default()
            .insert(window_id);
    }

    /// Remove a window from the per-output index.
    pub(super) fn remove_window_from_output_index(
        &mut self,
        window_id: WindowId,
        output_id: OutputId,
    ) {
        if let Some(set) = self.windows_by_output.get_mut(&output_id) {
            set.remove(&window_id);
            if set.is_empty() {
                self.windows_by_output.remove(&output_id);
            }
        }
    }

    /// Move a window from one output index entry to another.
    pub(super) fn move_window_between_outputs(
        &mut self,
        window_id: WindowId,
        from: OutputId,
        to: OutputId,
    ) {
        self.remove_window_from_output_index(window_id, from);
        self.index_window_in_output(window_id, to);
    }

    /// Return the set of window IDs registered for a specific output, if any.
    pub(super) fn windows_for_output(&self, output_id: OutputId) -> Option<&HashSet<WindowId>> {
        self.windows_by_output.get(&output_id)
    }

    /// Build a fresh `LayoutTree` for `output_id`, using the output's real
    /// geometry when known (or a zero-area fallback) and applying the current
    /// layout config. Shared by `tree_for_output` and `ensure_tree_for_output`
    /// so tree creation stays in one place.
    fn new_tree_for_output(&self, output_id: OutputId) -> LayoutTree {
        let rect = self
            .outputs
            .get(&output_id)
            .and_then(|o| o.rect())
            .unwrap_or(Rect::new(0, 0, 0, 0));
        let mut tree = LayoutTree::new(rect);
        if let Some(cfg) = self.config.as_ref() {
            tree.set_layout_config(cfg.layout.clone());
        }
        tree
    }

    /// Get the `LayoutTree` for a specific output, creating one if needed.
    pub(crate) fn tree_for_output(&mut self, output_id: OutputId) -> Option<&mut LayoutTree> {
        if !self.outputs.contains_key(&output_id) {
            return None;
        }
        if !self.output_trees.contains_key(&output_id) {
            let tree = self.new_tree_for_output(output_id);
            self.output_trees.insert(output_id, tree);
        }
        self.output_trees.get_mut(&output_id)
    }

    /// Ensure a `LayoutTree` exists for the given output ID, creating one with
    /// the output's real geometry when available, or a zero-area fallback
    /// rectangle when no real output is registered yet (e.g. orphan outputs
    /// that have not yet received a real `river_output`).
    pub(crate) fn ensure_tree_for_output(&mut self, output_id: OutputId) -> &mut LayoutTree {
        if !self.output_trees.contains_key(&output_id) {
            let tree = self.new_tree_for_output(output_id);
            self.output_trees.insert(output_id, tree);
        }
        self.output_trees.get_mut(&output_id).unwrap()
    }

    /// Get the `LayoutTree` for the currently focused output.
    ///
    /// Self-heals a stale `focused_output` (e.g. an output that was removed)
    /// by falling back to the first remaining output before resolving the tree,
    /// so focus commands never silently no-op on a dangling output reference.
    pub(super) fn focused_tree(&mut self) -> Option<&mut LayoutTree> {
        self.ensure_focused_output();
        let focused_output = self.focused_output?;
        self.tree_for_output(focused_output)
    }

    /// Resolve a River window proxy to its per-output `LayoutTree`.
    pub(super) fn tree_for_window_proxy(
        &mut self,
        proxy: &RiverWindowV1,
    ) -> Option<(WindowId, &mut LayoutTree)> {
        let window_id = self.windows_by_proxy.get(proxy).copied()?;
        let output_id = self.windows.get(&window_id)?.output_id;
        let tree = self.tree_for_output(output_id)?;
        Some((window_id, tree))
    }

    /// Run window rules against a newly updated window proxy.
    ///
    /// Called from `Handlers` when metadata arrives so rules can match as
    /// soon as identifiers are known.
    pub(super) fn evaluate_window_rules(&mut self, proxy: &RiverWindowV1) {
        let Some(window_id) = self.windows_by_proxy.get(proxy).copied() else {
            return;
        };
        let Some(output_id) = self.windows.get(&window_id).map(|w| w.output_id) else {
            return;
        };
        if self.window_rules.is_none() {
            return;
        }
        let Some(output_rect) = self.outputs.get(&output_id).and_then(|o| o.rect()) else {
            return;
        };
        let Some(tree) = self.output_trees.get_mut(&output_id) else {
            return;
        };

        let changed = {
            let Some(window) = self.windows.get_mut(&window_id) else {
                return;
            };
            self.window_rules
                .as_ref()
                .unwrap()
                .evaluate(window, tree, output_rect)
        };

        if changed && let Some(wm) = &self.wm {
            wm.manage_dirty();
        }
    }

    /// Move all windows from one output's tree into another output's tree.
    ///
    /// This is used when outputs are removed or reassigned. It preserves each
    /// window's mode (floating, fullscreen, pseudo-tiled) and focus state, so
    /// that windows survive output hotplug without being destroyed.
    ///
    /// The source output's tree is removed from `output_trees`, so callers
    /// should not rely on it existing afterward.
    pub(crate) fn reassign_output(&mut self, from: OutputId, to: OutputId) {
        // Only reassign into a real, known output. We check the output map
        // rather than `tree_for_output` so we do not create a destination tree
        // prematurely.
        if !self.outputs.contains_key(&to) {
            return;
        }

        let Some(from_tree) = self.output_trees.remove(&from) else {
            debug!(target: "fenestre::state::wm", "reassign_output: source output {:?} has no tree, nothing to move", from);
            return;
        };

        let to_rect = self
            .outputs
            .get(&to)
            .and_then(|o| o.rect())
            .unwrap_or(Rect::new(0, 0, 0, 0));
        let from_pos = self
            .outputs
            .get(&from)
            .and_then(|o| o.position)
            .unwrap_or((0, 0));
        let to_pos = self
            .outputs
            .get(&to)
            .and_then(|o| o.position)
            .unwrap_or((0, 0));
        let (dx, dy) = (
            to_pos.0.saturating_sub(from_pos.0),
            to_pos.1.saturating_sub(from_pos.1),
        );

        let window_ids = from_tree.visible_windows();

        // Move every window's record onto the destination output.
        for &win_id in &window_ids {
            self.move_window_between_outputs(WindowId(win_id), from, to);
            if let Some(window) = self.windows.get_mut(&WindowId(win_id)) {
                window.set_output_id(to);
            }
        }

        let previously_focused = self.focused_window;

        // Re-insert (rebuilding splits from the destination geometry) only when
        // the destination already has real dimensions. Otherwise the destination
        // is a freshly (re)created output whose geometry is not yet known, and
        // re-inserting against a zero-area tree would collapse every split to
        // Vertical — instead we clone the source tree to preserve its topology.
        let dest_has_geometry = self.outputs.get(&to).and_then(|o| o.rect()).is_some();
        let to_focus_before = if dest_has_geometry {
            self.reassign_with_rebuild(to, &from_tree, &window_ids, dx, dy, to_rect)
        } else {
            self.reassign_clone_topology(to, from_tree, dx, dy, to_rect)
        };

        // Reconcile focus (shared for both branches):
        // - if the globally focused window is now on `to` (moved here or already
        //   resident), make it the tree + state focus via `focus_window_id`;
        // - otherwise restore `to`'s own remembered focus so a later
        //   `focus_output(to)` focuses the window the user last had there.
        if let Some(focused_id) = previously_focused {
            let focused_now_on_to = self
                .windows
                .get(&focused_id)
                .is_some_and(|w| w.output_id == to);
            if focused_now_on_to {
                self.focus_window_id(focused_id);
            } else if let Some(remembered) = to_focus_before
                && let Some(tree) = self.tree_for_output(to)
            {
                let _ = tree.focus_window(remembered);
            }
        }

        self.request_manage_dirty();
    }

    /// Rebuild the destination tree by re-inserting the moved windows and
    /// re-deriving their modes from `WindowMode`, adapting splits to the
    /// destination geometry. Used when the destination output already has real
    /// dimensions. Returns the destination tree's focus before the move so the
    /// caller can restore it.
    fn reassign_with_rebuild(
        &mut self,
        to: OutputId,
        from_tree: &LayoutTree,
        window_ids: &[u32],
        dx: i32,
        dy: i32,
        to_rect: Rect,
    ) -> Option<u32> {
        // Plan describing the layout state each moved window must return to on
        // the destination tree.
        #[derive(Clone, Copy)]
        enum ReassignPlan {
            Tiled,
            Floating(Rect),
            PseudoTiled(Rect),
            FullscreenTiled,
            FullscreenPseudo(Option<Rect>),
            FullscreenFloating(Option<Rect>),
        }

        let mut plans = HashMap::new();
        for win_id in window_ids {
            if let Some(window) = self.windows.get(&WindowId(*win_id)) {
                let pseudo_rect = window.pseudo_tiled_rect(to_rect);
                let plan = match window.mode {
                    super::window::WindowMode::Floating {
                        x,
                        y,
                        width,
                        height,
                    } => ReassignPlan::Floating(crate::layout::Rect::new(
                        x.saturating_add(dx),
                        y.saturating_add(dy),
                        width,
                        height,
                    )),
                    super::window::WindowMode::PseudoTiled => ReassignPlan::PseudoTiled(
                        pseudo_rect.expect("pseudo_tiled_rect always returns Some"),
                    ),
                    super::window::WindowMode::Fullscreen => {
                        match from_tree.window_base_state(*win_id) {
                            Some(crate::layout::WindowState::PseudoTiled) => {
                                ReassignPlan::FullscreenPseudo(pseudo_rect)
                            }
                            Some(crate::layout::WindowState::Floating) => {
                                ReassignPlan::FullscreenFloating(
                                    from_tree.window_floating_rect(*win_id).map(|r| {
                                        crate::layout::Rect::new(
                                            r.x.saturating_add(dx),
                                            r.y.saturating_add(dy),
                                            r.width,
                                            r.height,
                                        )
                                    }),
                                )
                            }
                            _ => ReassignPlan::FullscreenTiled,
                        }
                    }
                    super::window::WindowMode::Tiled => ReassignPlan::Tiled,
                };
                plans.insert(*win_id, plan);
            }
        }

        let to_tree = self.tree_for_output(to).expect("destination tree exists");
        let remembered = to_tree.focused_window();
        for win_id in window_ids {
            to_tree.insert_window(*win_id);
        }
        for win_id in window_ids {
            if let Some(plan) = plans.get(win_id) {
                match *plan {
                    ReassignPlan::Tiled => {}
                    ReassignPlan::Floating(rect) => {
                        let _ = to_tree.toggle_floating(*win_id, rect);
                    }
                    ReassignPlan::PseudoTiled(rect) => {
                        let _ = to_tree.toggle_pseudo_tiled(*win_id, rect);
                    }
                    ReassignPlan::FullscreenTiled => {
                        let _ = to_tree.toggle_fullscreen(*win_id);
                    }
                    ReassignPlan::FullscreenPseudo(rect) => {
                        if let Some(r) = rect {
                            let _ = to_tree.toggle_pseudo_tiled(*win_id, r);
                        }
                        let _ = to_tree.toggle_fullscreen(*win_id);
                    }
                    ReassignPlan::FullscreenFloating(rect) => {
                        if let Some(r) = rect {
                            let _ = to_tree.toggle_floating(*win_id, r);
                        }
                        let _ = to_tree.toggle_fullscreen(*win_id);
                    }
                }
            }
        }
        to_tree.arrange();
        remembered
    }

    /// Preserve the source tree's exact topology by cloning it into the
    /// destination (translating floating rects by (dx, dy)). Used when the
    /// destination output has no dimensions yet, so re-insertion would collapse
    /// every split to Vertical. Returns the cloned tree's focus so the caller
    /// can restore it.
    fn reassign_clone_topology(
        &mut self,
        to: OutputId,
        mut from_tree: LayoutTree,
        dx: i32,
        dy: i32,
        to_rect: Rect,
    ) -> Option<u32> {
        from_tree.set_output_rect(to_rect);
        if dx != 0 || dy != 0 {
            from_tree.translate_floating_rects(dx, dy);
        }
        let remembered = from_tree.focused_window();
        // Only clone when the destination has no tree yet. A pre-existing
        // destination tree (even a zero-area one) keeps its own windows;
        // overwriting it would silently drop them.
        self.output_trees.entry(to).or_insert(from_tree);
        remembered
    }

    /// Mark River window-manager state as dirty.
    ///
    /// This asks River to start a new manage sequence so pending state changes
    /// can be applied during the next manage phase.
    pub(super) fn request_manage_dirty(&self) {
        if let Some(wm) = self.wm.as_ref() {
            wm.manage_dirty();
        }
    }

    /// Apply pending BSP layout and window-management requests in a manage sequence.
    pub(super) fn apply_manage(&mut self, _qh: &QueueHandle<Self>) {
        self.ensure_focused_output();

        let decorations = self.config.as_ref().map(|c| c.decorations).unwrap_or(true);

        for (output_id, tree) in self.output_trees.iter_mut() {
            let Some(output) = self.outputs.get(output_id) else {
                continue;
            };
            let Some(output_proxy) = output.river_output.as_ref() else {
                continue;
            };
            if let Some(rect) = output.rect() {
                tree.set_output_rect(rect);
            }
            let arranged = tree.arranged_windows();
            for (window_id, window_rect, state) in arranged {
                let window_id = WindowId(window_id);
                let Some(window) = self.windows.get_mut(&window_id) else {
                    continue;
                };

                window.set_layout_rect(window_rect);
                if let Some(river_window) = window.river_window.as_ref() {
                    if !window.use_client_decorations(decorations) {
                        river_window.use_ssd();
                    }
                    match state {
                        crate::layout::WindowState::Tiled
                        | crate::layout::WindowState::Floating => {
                            if window.mode == super::window::WindowMode::Fullscreen {
                                river_window.exit_fullscreen();
                            }
                            river_window.propose_dimensions(window_rect.width, window_rect.height);
                        }
                        crate::layout::WindowState::PseudoTiled => {
                            if window.mode == super::window::WindowMode::Fullscreen {
                                river_window.exit_fullscreen();
                            }
                            let (width, height) = window.preferred_dimensions(window_rect);
                            river_window.propose_dimensions(width, height);
                        }
                        crate::layout::WindowState::Fullscreen => {
                            if window.mode != super::window::WindowMode::Fullscreen {
                                river_window.fullscreen(output_proxy);
                            }
                        }
                    }
                }
                window.mode = match state {
                    crate::layout::WindowState::Tiled => super::window::WindowMode::Tiled,
                    crate::layout::WindowState::PseudoTiled => {
                        super::window::WindowMode::PseudoTiled
                    }
                    crate::layout::WindowState::Floating => super::window::WindowMode::Floating {
                        x: window_rect.x,
                        y: window_rect.y,
                        width: window_rect.width,
                        height: window_rect.height,
                    },
                    crate::layout::WindowState::Fullscreen => super::window::WindowMode::Fullscreen,
                };
            }
        }

        self.render_order_cache.clear();

        // Keep pending_focus queued until the focus can actually be applied.
        // Seat creation requests another manage sequence, which will retry the
        // pending focus. We only clear `pending_focus` when the seat issued the
        // River focus request for real; if the seat or window proxy is missing
        // the request is silently dropped by `Seat::focus_window` and we must
        // keep `pending_focus` so the next manage sequence retries it.
        if let Some(window_id) = self.pending_focus {
            if !self.windows.contains_key(&window_id) {
                self.pending_focus = None;
            } else if let Some(seat_id) = self.current_seat
                && let Some(seat) = self.seats.get(&seat_id)
                && let Some(window) = self.windows.get(&window_id)
                && seat.focus_window(window)
            {
                self.pending_focus = None;
            }
        }

        let pending_closes = std::mem::take(&mut self.pending_closes);
        for window_id in pending_closes {
            if let Some((_, window)) = self.find_window_mut_by_id(window_id)
                && let Some(river_window) = window.river_window.as_ref()
            {
                river_window.close();
            }
        }
    }

    /// Apply pending render-state requests in a render sequence.
    pub(super) fn apply_render(&mut self, qh: &QueueHandle<Self>) {
        if self.render_order_cache.is_empty() {
            self.update_render_order_cache();
        }

        let config = self.config.as_ref();
        let decor_default = config.map(|c| c.decorations).unwrap_or(true);
        let border_width = config.and_then(|c| c.border_width).unwrap_or(0);
        let border_color_focused = config
            .and_then(|c| c.border_color_focused)
            .unwrap_or(0xffffffff);
        let border_color_unfocused = config
            .and_then(|c| c.border_color_unfocused)
            .unwrap_or(0xffffffff);
        let (rgba_focused, rgba_unfocused) = config
            .map(|c| c.border_rgba())
            .unwrap_or(((0xff, 0xff, 0xff, 0xff), (0xff, 0xff, 0xff, 0xff)));

        // Per-window border cache keyed on (width, effective_color).
        // Cached state is invalidated when a window switches between
        // client-side and server-side decorations so stale compositor
        // borders are not left behind.
        for window_id in &self.render_order_cache {
            let Some(window) = self.windows.get_mut(window_id) else {
                continue;
            };
            let Some(river_window) = window.river_window.as_ref() else {
                continue;
            };

            if window.node.is_none() {
                window.node = Some(river_window.get_node(qh, ()));
            }

            if let (Some(node), Some(rect)) = (window.node.as_ref(), window.layout_rect) {
                node.set_position(rect.x, rect.y);
                if matches!(
                    window.mode,
                    super::window::WindowMode::Floating { .. }
                        | super::window::WindowMode::Fullscreen
                ) {
                    node.place_top();
                }
            }

            let use_decor = window.use_client_decorations(decor_default);
            if use_decor {
                if window.last_border.is_some() {
                    window.last_border = None;
                }
            } else {
                let effective_color = if self.focused_window == Some(*window_id) {
                    border_color_focused
                } else {
                    border_color_unfocused
                };
                let desired = Some((border_width, effective_color));
                if window.last_border == desired {
                    continue;
                }
                window.last_border = desired;

                let (r, g, b, a) = if self.focused_window == Some(*window_id) {
                    rgba_focused
                } else {
                    rgba_unfocused
                };

                if border_width > 0 {
                    river_window.set_borders(Edges::all(), border_width, r, g, b, a);
                } else {
                    river_window.set_borders(Edges::empty(), 0, 0, 0, 0, 0);
                }
            }
        }
    }

    /// Rebuild the cached render order based on current window states.
    fn update_render_order_cache(&mut self) {
        self.render_order_cache.clear();
        self.render_order_cache.extend(self.windows.keys().copied());
        self.render_order_cache.sort_unstable_by_key(|id| {
            render_stack_priority(self.windows.get(id), self.focused_window)
        });
    }

    pub(crate) fn ensure_focused_output(&mut self) {
        if self.focused_output.is_none()
            || !self.outputs.contains_key(&self.focused_output.unwrap())
        {
            self.focused_output = self.outputs.keys().next().copied();
        }
        // Note: we intentionally do NOT create a `LayoutTree` here. `tree_for_output`
        // builds a tree (and clones `LayoutConfig`) on a miss, and callers like
        // `active_output_rect` only need read-only geometry, not a tree. Trees are
        // created lazily at real insertion points (`ensure_tree_for_output` /
        // `tree_for_output` in the handlers and `apply_manage`).
    }

    /// Get the active output's geometry rectangle.
    ///
    /// Self-heals a stale `focused_output` before resolving the rectangle so
    /// callers (e.g. move/resize) observe a valid output.
    pub(super) fn active_output_rect(&mut self) -> Option<Rect> {
        self.ensure_focused_output();
        let output_id = self.focused_output?;
        let output = self.outputs.get(&output_id)?;
        output.rect()
    }

    /// Find an output by its River output proxy.
    pub(super) fn find_output_mut_by_proxy(
        &mut self,
        proxy: &RiverOutputV1,
    ) -> Option<(OutputId, &mut Output)> {
        let id = *self.outputs_by_proxy.get(proxy)?;
        self.outputs.get_mut(&id).map(|output| (id, output))
    }

    /// Find a window by internal ID.
    pub(super) fn find_window_mut_by_id(
        &mut self,
        id: WindowId,
    ) -> Option<(WindowId, &mut Window)> {
        self.windows.get_mut(&id).map(|window| (id, window))
    }

    /// Find a window by its River window proxy.
    pub(super) fn find_window_mut_by_proxy(
        &mut self,
        proxy: &RiverWindowV1,
    ) -> Option<(WindowId, &mut Window)> {
        let id = *self.windows_by_proxy.get(proxy)?;
        self.windows.get_mut(&id).map(|window| (id, window))
    }

    /// Apply a metadata update to a window and re-evaluate rules.
    pub(super) fn apply_window_metadata(
        &mut self,
        proxy: &RiverWindowV1,
        f: impl FnOnce(&mut Window),
    ) {
        if let Some((_, window)) = self.find_window_mut_by_proxy(proxy) {
            f(window);
            self.evaluate_window_rules(proxy);
        }
    }

    /// Remove a window by its River window proxy.
    ///
    /// Also clears `focused_window` if the removed window was focused.
    /// Reconcile the layout tree and global focus pointers after a window is
    /// closed.
    ///
    /// Removes `window_id` from its output's `LayoutTree` *first* so the tree's
    /// preferred new focus (`first_window()` when the removed window was
    /// tree-focused) is captured before the global focus bookkeeping runs. The
    /// layout tree, not the cross-output focus stack, is the correct source of
    /// truth for where focus should go now.
    ///
    /// It then drops `window_id` from the window map and the focus stack and
    /// routes global focus through `focus_window_id`, keeping `focused_window`,
    /// `focused_output`, `focus_stack`, `tree.focused`, and `pending_focus` in
    /// sync. When the tree yields no focusable window (its output is now empty),
    /// the existing focus stack's top is kept as `pending_focus`.
    ///
    /// This does NOT remove `window_id` from `windows_by_proxy` or destroy the
    /// River proxy; the caller does that (the production `Event::Closed` path,
    /// or tests which have no proxy).
    pub(super) fn close_window_focus_reconcile(&mut self, window_id: WindowId) {
        // Resolve the output while the window is still in the map, so we can
        // find its `LayoutTree`.
        let output_id = self.windows.get(&window_id).map(|w| w.output_id);

        // Only reroute global focus when the closed window was the *globally*
        // focused one. Closing a non-focused window (on the focused or any other
        // output) must NOT yank focus to a different output. `remove_window`
        // below only changes a tree's `focused` when the removed window was that
        // tree's focus, so a closed non-focused window yields a non-None
        // `new_layout_focus` from its own tree and would otherwise steal focus.
        let was_globally_focused = self.focused_window == Some(window_id);

        // Remove from the layout tree FIRST to capture the tree's chosen new
        // focus before any global focus bookkeeping runs.
        let new_layout_focus = if let Some(output_id) = output_id {
            if let Some(tree) = self.output_trees.get_mut(&output_id) {
                let _ = tree.remove_window(window_id.0);
                tree.focused_window().map(WindowId)
            } else {
                None
            }
        } else {
            None
        };

        // Drop the window from the window map and the focus stack.
        self.windows.remove(&window_id);
        if let Some(output_id) = output_id {
            self.remove_window_from_output_index(window_id, output_id);
        }
        self.remove_focus_for_window(window_id);

        // Route focus through `focus_window_id` (which keeps every focus pointer
        // and the destination output's `LayoutTree` focus in sync) ONLY when the
        // closed window was the globally focused one. Closing a background window
        // (on the focused or any other output) must never move focus.
        //
        // When it *was* globally focused we still need to drive focus somewhere:
        // prefer `new_layout_focus` (the next window on the same output, from the
        // tree), but if that output just emptied, `remove_focus_for_window` already
        // fell back to the global focus stack, which may now point at a *different*
        // output whose `LayoutTree` focus is stale. In that case re-sync to the
        // fallback window so the now-focused output's tree focus matches global
        // focus (otherwise focus direction/stack bookkeeping diverges).
        if was_globally_focused {
            let next = new_layout_focus
                .filter(|id| self.windows.contains_key(id))
                .or_else(|| {
                    self.focused_window
                        .filter(|id| self.windows.contains_key(id))
                });
            if let Some(nf) = next {
                self.focus_window_id(nf);
            } else {
                self.pending_focus = self.focused_window;
            }
        }
    }

    /// Remove an output by its River output proxy.
    ///
    /// If the removed output was focused, `focused_output` is re-pointed to
    /// `fallback_output` (the output windows were reassigned to, if any),
    /// falling back to the first remaining output (or `None` if none remain).
    /// This is the single owner of the focused-output fallback policy.
    pub(super) fn remove_output_by_proxy(
        &mut self,
        proxy: &RiverOutputV1,
        fallback_output: Option<OutputId>,
    ) -> Option<OutputId> {
        let output_id = *self.outputs_by_proxy.get(proxy)?;

        let removed_was_focused = self.focused_output == Some(output_id);
        self.outputs.remove(&output_id);
        self.outputs_by_proxy.remove(proxy);

        if removed_was_focused {
            self.focused_output = fallback_output.or_else(|| self.outputs.keys().next().copied());
        }

        Some(output_id)
    }

    /// Find a seat by its River seat proxy.
    pub(super) fn find_seat_mut_by_proxy(
        &mut self,
        proxy: &RiverSeatV1,
    ) -> Option<(SeatId, &mut Seat)> {
        let id = *self.seats_by_proxy.get(proxy)?;
        self.seats.get_mut(&id).map(|seat| (id, seat))
    }

    /// Remove a seat by its River seat proxy.
    pub(super) fn remove_seat_by_proxy(&mut self, proxy: &RiverSeatV1) -> Option<SeatId> {
        let seat_id = *self.seats_by_proxy.get(proxy)?;

        let removed_was_current = self.current_seat == Some(seat_id);
        self.seats.remove(&seat_id);
        self.seats_by_proxy.remove(proxy);

        // Recompute current_seat only if the removed seat was current,
        // or if the current seat is no longer present.
        if removed_was_current
            || self
                .current_seat
                .is_some_and(|current| !self.seats.contains_key(&current))
        {
            self.current_seat = self.seats.first_key_value().map(|(id, _)| *id);
        }

        Some(seat_id)
    }

    /// Push a window to the front of the focus stack and mark it focused.
    pub(super) fn push_focus(&mut self, window_id: WindowId) {
        self.focus_stack.retain(|id| *id != window_id);
        self.focus_stack.insert(0, window_id);
        self.focused_window = Some(window_id);
        if let Some(window) = self.windows.get(&window_id) {
            self.focused_output = Some(window.output_id);
        }
    }

    /// Focus a window by ID, updating both the global focus state and the
    /// per-output `LayoutTree`. Queues a pending focus for River and requests
    /// a manage sequence.
    pub(super) fn focus_window_id(&mut self, window_id: WindowId) {
        if !self.windows.contains_key(&window_id) {
            return;
        }

        self.push_focus(window_id);
        if let Some(output_id) = self.windows.get(&window_id).map(|w| w.output_id)
            && let Some(tree) = self.tree_for_output(output_id)
        {
            let _ = tree.focus_window(window_id.0);
        }
        self.pending_focus = Some(window_id);
        self.render_order_cache.clear();
        self.request_manage_dirty();
    }

    /// Remove a window from the focus stack and reconcile `focused_window` /
    /// `focused_output` to the new top of the stack. Also clears `pending_focus`
    /// if it pointed at the removed window.
    pub(super) fn remove_focus_for_window(&mut self, window_id: WindowId) {
        self.focus_stack.retain(|id| *id != window_id);

        if self.focused_window == Some(window_id) {
            self.focused_window = self.focus_stack.first().copied();
            if let Some(new_focused) = self.focused_window
                && let Some(window) = self.windows.get(&new_focused)
            {
                self.focused_output = Some(window.output_id);
            }
        }

        if self.pending_focus == Some(window_id) {
            self.pending_focus = self.focused_window;
        }
    }
}

/// Compute the render stack priority for a window.
/// Returns a tuple of (mode_priority, focus_priority, window_id) for deterministic z-ordering.
fn render_stack_priority(
    window: Option<&super::window::Window>,
    focused_window: Option<super::window::WindowId>,
) -> (u8, u8, u32) {
    let Some(window) = window else {
        return (0, 0, 0);
    };
    let mode_priority = match window.mode {
        super::window::WindowMode::Tiled | super::window::WindowMode::PseudoTiled => 0,
        super::window::WindowMode::Floating { .. } => 1,
        super::window::WindowMode::Fullscreen => 2,
    };
    let focus_priority = if focused_window == Some(window.id) {
        1
    } else {
        0
    };

    (mode_priority, focus_priority, window.id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_focus_makes_new_window_focused() {
        let mut state = WMState::new();
        let window_id = WindowId(1);
        let output_id = OutputId(1);
        state.outputs.insert(output_id, Output::new(output_id));
        state.focused_output = Some(output_id);
        let window = Window::new(window_id, output_id);
        state.windows.insert(window_id, window);
        state.tree_for_output(output_id).unwrap().insert_window(1);
        state.push_focus(window_id);
        state.request_manage_dirty();

        assert_eq!(state.focused_window, Some(window_id));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(1));
        assert_eq!(state.focus_stack, vec![window_id]);
    }

    #[test]
    fn focus_window_id_syncs_state_and_layout() {
        let mut state = WMState::new();
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let output_id = OutputId(1);
        state.outputs.insert(output_id, Output::new(output_id));
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state.windows.insert(w2, Window::new(w2, output_id));
        state.tree_for_output(output_id).unwrap().insert_window(1);
        state.tree_for_output(output_id).unwrap().insert_window(2);

        state.push_focus(w1);
        state.focus_window_id(w2);

        assert_eq!(state.focused_window, Some(w2));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(2));
        assert_eq!(state.focus_stack, vec![w2, w1]);
    }

    #[test]
    fn removing_focused_window_keeps_state_and_layout_consistent() {
        let mut state = WMState::new();
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let w3 = WindowId(3);
        let output_id = OutputId(1);
        state.outputs.insert(output_id, Output::new(output_id));
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state.windows.insert(w2, Window::new(w2, output_id));
        state.windows.insert(w3, Window::new(w3, output_id));
        state.tree_for_output(output_id).unwrap().insert_window(1);
        state.tree_for_output(output_id).unwrap().insert_window(2);
        state.tree_for_output(output_id).unwrap().insert_window(3);

        state.focus_window_id(w3);
        state.focus_window_id(w1);
        assert_eq!(state.focus_stack, vec![w1, w3]);

        state.windows.remove(&w1);
        state.remove_focus_for_window(w1);
        state.tree_for_output(output_id).unwrap().remove_window(1);
        state.focused_window = state.focused_tree().unwrap().focused_window().map(WindowId);
        state.pending_focus = state.focused_window;

        let state_focus = state.focused_window;
        let layout_focus = state.focused_tree().unwrap().focused_window().map(WindowId);

        assert_eq!(
            state_focus, layout_focus,
            "State focused {:?} did not match layout focused {:?}",
            state_focus, layout_focus
        );
    }

    #[test]
    fn removing_non_focused_window_does_not_change_focus() {
        let mut state = WMState::new();
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        let output_id = OutputId(1);
        state.outputs.insert(output_id, Output::new(output_id));
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state.windows.insert(w2, Window::new(w2, output_id));
        state.tree_for_output(output_id).unwrap().insert_window(1);
        state.tree_for_output(output_id).unwrap().insert_window(2);

        state.focus_window_id(w1);

        state.windows.remove(&w2);
        state.remove_focus_for_window(w2);
        state.tree_for_output(output_id).unwrap().remove_window(2);

        assert_eq!(state.focused_window, Some(w1));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(1));
    }

    #[test]
    fn new_window_receives_focus_and_is_focused_in_layout() {
        let mut state = WMState::new();
        let w1 = WindowId(1);
        let output_id = OutputId(1);
        state.outputs.insert(output_id, Output::new(output_id));
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state.tree_for_output(output_id).unwrap().insert_window(1);
        state.push_focus(w1);
        state.request_manage_dirty();

        assert_eq!(state.focused_window, Some(w1));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(1));
    }

    #[test]
    fn two_outputs_get_independent_trees() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new(o1));
        state.outputs.insert(o2, Output::new(o2));
        state.focused_output = Some(o1);

        let w1 = WindowId(1);
        let w2 = WindowId(2);
        state.windows.insert(w1, Window::new(w1, o1));
        state.windows.insert(w2, Window::new(w2, o2));
        state.tree_for_output(o1).unwrap().insert_window(w1.0);
        state.tree_for_output(o2).unwrap().insert_window(w2.0);

        assert_eq!(state.tree_for_output(o1).unwrap().focused_window(), Some(1));
        assert_eq!(state.tree_for_output(o2).unwrap().focused_window(), Some(2));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(1));
    }

    #[test]
    fn reassign_output_moves_windows_and_updates_output_id() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new(o1));
        state.outputs.insert(o2, Output::new(o2));

        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        state.tree_for_output(o1).unwrap().insert_window(w1.0);

        state.reassign_output(o1, o2);
        assert_eq!(state.windows.get(&w1).unwrap().output_id, o2);
        assert!(state.output_trees.contains_key(&o2));
        assert!(!state.output_trees.contains_key(&o1));
    }

    #[test]
    fn pointer_interaction_switches_focused_output() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new(o1));
        state.outputs.insert(o2, Output::new(o2));
        state.focused_output = Some(o1);

        let w1 = WindowId(1);
        let w2 = WindowId(2);
        state.windows.insert(w1, Window::new(w1, o1));
        state.windows.insert(w2, Window::new(w2, o2));
        state.tree_for_output(o1).unwrap().insert_window(w1.0);
        state.tree_for_output(o2).unwrap().insert_window(w2.0);
        state.push_focus(w1);

        state.focus_window_id(w2);
        assert_eq!(state.focused_window, Some(w2));
        assert_eq!(state.focused_output, Some(o2));
    }

    #[test]
    fn output_resize_does_not_affect_other_output_tree() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new(o1));
        state.outputs.insert(o2, Output::new(o2));
        state.outputs.get_mut(&o1).unwrap().set_dimensions(800, 600);
        state
            .outputs
            .get_mut(&o2)
            .unwrap()
            .set_dimensions(1920, 1080);
        state.focused_output = Some(o1);

        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        state.tree_for_output(o1).unwrap().insert_window(w1.0);

        state
            .tree_for_output(o1)
            .unwrap()
            .set_output_rect(crate::layout::Rect::new(0, 0, 800, 600));
        let arranged = state.tree_for_output(o1).unwrap().arranged_windows();
        assert_eq!(arranged.len(), 1);
    }

    #[test]
    fn closing_tree_focused_window_keeps_state_and_layout_consistent() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new(o1));
        state.focused_output = Some(o1);

        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);
        for w in [a, b, c] {
            state.windows.insert(w, Window::new(w, o1));
            state.tree_for_output(o1).unwrap().insert_window(w.0);
            state.push_focus(w);
        }
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(3));
        assert_eq!(state.focused_window, Some(c));

        // Exercise the same reconciliation path the real `Event::Closed` uses.
        state.close_window_focus_reconcile(c);

        // The tree's chosen focus (first remaining window, A) is authoritative
        // and the global state must match it.
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(
            state.focused_tree().unwrap().focused_window(),
            Some(a.0),
            "State focus {:?} diverged from layout focus after close",
            state.focused_window
        );
    }

    /// Closing a non-focused window on a *different* (non-focused) output must
    /// NOT move global focus there. This guards the cross-output focus-theft
    /// bug: `close_window_focus_reconcile` must only reroute focus when the
    /// closed window was the globally focused one.
    #[test]
    fn closing_nonfocused_window_on_other_output_keeps_focus() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, x) in [(o1, 0), (o2, 1920)] {
            let mut out = Output::new(o);
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }
        state.focused_output = Some(o1);

        // Globally focus A on o1.
        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);
        state.windows.insert(a, Window::new(a, o1));
        state.tree_for_output(o1).unwrap().insert_window(a.0);
        state.focus_window_id(a);
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(state.focused_output, Some(o1));

        // Two windows on the other output, with C last focused there.
        for w in [b, c] {
            state.windows.insert(w, Window::new(w, o2));
            state.tree_for_output(o2).unwrap().insert_window(w.0);
            state.push_focus(w);
        }
        state.focus_window_id(a); // re-assert global focus on o1
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(state.focused_output, Some(o1));

        // Close B (non-focused, on the non-focused output o2).
        state.close_window_focus_reconcile(b);

        // Focus must stay on A / o1: closing a background window on another
        // monitor must not yank focus over.
        assert_eq!(state.focused_window, Some(a), "focus stolen on close");
        assert_eq!(
            state.focused_output,
            Some(o1),
            "focused_output stolen on close"
        );
        assert!(
            state.windows.contains_key(&c),
            "sibling on o2 should survive the close"
        );
    }

    /// When the globally focused window lives on the focused output and is
    /// closed, focus must move to another window on that same output (the tree's
    /// chosen next focus), not to a different output.
    #[test]
    fn closing_focused_window_moves_focus_within_output() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, x) in [(o1, 0), (o2, 1920)] {
            let mut out = Output::new(o);
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }
        state.focused_output = Some(o1);

        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);
        for w in [a, b] {
            state.windows.insert(w, Window::new(w, o1));
            state.tree_for_output(o1).unwrap().insert_window(w.0);
        }
        state.windows.insert(c, Window::new(c, o2));
        state.tree_for_output(o2).unwrap().insert_window(c.0);
        state.focus_window_id(a);
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(state.focused_output, Some(o1));

        // Close the globally focused window A on o1.
        state.close_window_focus_reconcile(a);

        // Focus moves to B on o1 (the tree's next focus), not to C on o2.
        assert_eq!(
            state.focused_window,
            Some(b),
            "focus did not move within output"
        );
        assert_eq!(
            state.focused_output,
            Some(o1),
            "focus jumped to other output"
        );
    }

    /// When the globally focused window is moved to another output, its new
    /// output id must be reflected in `focused_output` (via `focus_window_id`),
    /// not left dangling on the old output.
    ///
    /// Reassigning windows into a destination output while the globally focused
    /// window lives on a *third* output must preserve the destination's own
    /// remembered focus. Otherwise `focus_output(to)` would later focus an
    /// arbitrary window moved in from `from` instead of the window the user last
    /// had focused there.
    #[test]
    fn reassign_output_preserves_destination_focus_when_focus_elsewhere() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        let o3 = OutputId(3);
        for (o, x) in [(o1, 0), (o2, 1920), (o3, 3840)] {
            let mut out = Output::new(o);
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }

        // `o2` has two windows; A is its last-focused (remembered) window.
        let a = WindowId(1);
        let b = WindowId(2);
        state.windows.insert(a, Window::new(a, o2));
        state.windows.insert(b, Window::new(b, o2));
        state.tree_for_output(o2).unwrap().insert_window(a.0);
        state.tree_for_output(o2).unwrap().insert_window(b.0);
        // Make A the remembered focus on o2.
        state.tree_for_output(o2).unwrap().focus_window(a.0);
        assert_eq!(
            state.tree_for_output(o2).unwrap().focused_window(),
            Some(a.0)
        );

        // `o1` has a window X that we will reassign into o2.
        let x = WindowId(3);
        state.windows.insert(x, Window::new(x, o1));
        state.tree_for_output(o1).unwrap().insert_window(x.0);

        // Global focus is on o3 (a third output), unrelated to the move.
        let c = WindowId(4);
        state.windows.insert(c, Window::new(c, o3));
        state.tree_for_output(o3).unwrap().insert_window(c.0);
        state.focus_window_id(c);
        assert_eq!(state.focused_window, Some(c));
        assert_eq!(state.focused_output, Some(o3));

        // Reassign o1 -> o2 while focus is on o3.
        state.reassign_output(o1, o2);

        // Windows moved correctly.
        assert_eq!(state.windows.get(&x).unwrap().output_id, o2);
        // Global focus must stay on o3 / c.
        assert_eq!(state.focused_window, Some(c));
        assert_eq!(state.focused_output, Some(o3));
        // o2's own remembered focus (A) must survive, not be clobbered to the
        // last-inserted window X.
        assert_eq!(
            state.tree_for_output(o2).unwrap().focused_window(),
            Some(a.0),
            "destination output's remembered focus was clobbered on reassign"
        );
    }

    #[test]
    fn reassign_output_keeps_focused_window_synced() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new(o1));
        state.outputs.insert(o2, Output::new(o2));

        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        state.tree_for_output(o1).unwrap().insert_window(w1.0);
        state.push_focus(w1);
        state.focused_output = Some(o1);

        state.reassign_output(o1, o2);

        assert_eq!(state.windows.get(&w1).unwrap().output_id, o2);
        assert_eq!(state.focused_window, Some(w1));
        assert_eq!(state.focused_output, Some(o2));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(1));
    }

    #[test]
    fn reassign_output_preserves_fullscreen_base_state() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new(o1));
        state.outputs.insert(o2, Output::new(o2));
        state.focused_output = Some(o1);

        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        let tree = state.tree_for_output(o1).unwrap();
        tree.insert_window(w1.0);
        let rect = crate::layout::Rect::new(0, 0, 100, 100);
        tree.toggle_pseudo_tiled(w1.0, rect);
        tree.toggle_fullscreen(w1.0);
        // Mirror the layout state into the window record, as `apply_manage` does.
        state.windows.get_mut(&w1).unwrap().mode = crate::state::window::WindowMode::Fullscreen;

        state.reassign_output(o1, o2);

        let o2_tree = state.tree_for_output(o2).unwrap();
        // The captured `base_state` must survive as PseudoTiled, not clobber to Tiled.
        assert_eq!(
            o2_tree.window_base_state(w1.0),
            Some(crate::layout::WindowState::PseudoTiled)
        );
        assert!(o2_tree.window_is_fullscreen(w1.0));

        // Un-fullscreening should return to PseudoTiled, not Tiled.
        o2_tree.toggle_fullscreen(w1.0);
        let state_after = o2_tree
            .arranged_windows()
            .into_iter()
            .find(|(id, _, _)| *id == w1.0)
            .map(|(_, _, s)| s);
        assert_eq!(state_after, Some(crate::layout::WindowState::PseudoTiled));
    }

    /// Reassigning windows to another output must preserve split directions,
    /// not collapse every split to Vertical. `reassign_output` inserts with
    /// `insert_window` (which arranges before deciding each split direction),
    /// so the rebuilt tree reflects the destination output's real geometry.
    #[test]
    fn reassign_output_preserves_split_directions() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, w, h) in [(o1, 100, 1000), (o2, 1000, 100)] {
            let mut out = Output::new(o);
            out.set_dimensions(w, h);
            state.outputs.insert(o, out);
        }
        state.focused_output = Some(o1);

        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);
        for w in [a, b, c] {
            state.windows.insert(w, Window::new(w, o1));
            state.tree_for_output(o1).unwrap().insert_window(w.0);
            state.push_focus(w);
        }

        // Source uses a horizontal-first split (tall output): the first window
        // occupies the full width and half the height.
        let src = state.tree_for_output(o1).unwrap().arranged_windows();
        let a_src = src.iter().find(|(id, _, _)| *id == a.0).unwrap();
        assert_eq!(a_src.1.height, 500, "source precondition");

        state.reassign_output(o1, o2);

        // Source is tall (100×1000): horizontal-first split gives the first
        // window full width and half height. Destination is wide (1000×100):
        // the rebuilt tree must adapt to a vertical-first split, so the first
        // window gets full height and half width.
        let dst = state.tree_for_output(o2).unwrap().arranged_windows();
        let a_dst = dst.iter().find(|(id, _, _)| *id == a.0).unwrap();
        assert_eq!(
            a_dst.1.height, 100,
            "reassign did not adapt to destination geometry (height {})",
            a_dst.1.height
        );
        assert_eq!(
            a_dst.1.width, 500,
            "reassign did not adapt to destination geometry (width {})",
            a_dst.1.width
        );
    }

    /// If `focused_output` references a removed output, `focused_tree` must
    /// self-heal to a remaining output so `focus_next` still works instead of
    /// silently no-op'ing on a dangling tree.
    /// Regression test: when an output is removed and later recreated, the
    /// windows are drained from the orphan tree into the new output before the
    /// compositor has sent its geometry. Re-inserting into a zero-area tree
    /// collapsed every split to Vertical; the topology must instead be cloned
    /// from the source so the user's layout survives the recreate.
    #[test]
    fn reassign_output_into_dimensionless_output_preserves_topology() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);

        // Source: a tall output so the first split is Horizontal (windows stacked).
        let mut out1 = Output::new(o1);
        out1.set_dimensions(100, 1000);
        state.outputs.insert(o1, out1);

        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);
        for w in [a, b, c] {
            state.windows.insert(w, Window::new(w, o1));
            state.tree_for_output(o1).unwrap().insert_window(w.0);
            state.push_focus(w);
        }

        // Simulate output removal (orphan tree kept) then a recreate of o2
        // without dimensions yet.
        state.outputs.remove(&o1);
        state.outputs.insert(o2, Output::new(o2));

        state.reassign_output(o1, o2);

        // Give the recreated output real geometry and arrange.
        let rect = crate::layout::Rect::new(0, 0, 1000, 1000);
        state.tree_for_output(o2).unwrap().set_output_rect(rect);
        let dst = state.tree_for_output(o2).unwrap().arranged_windows();

        let a_dst = dst.iter().find(|(id, _, _)| *id == a.0).unwrap();
        let b_dst = dst.iter().find(|(id, _, _)| *id == b.0).unwrap();
        // With the all-vertical bug, `a` and `b` would share the same y (side by
        // side). A preserved horizontal-first topology stacks them (different y).
        assert_ne!(
            a_dst.1.y, b_dst.1.y,
            "recreate collapsed topology to all-vertical (same y: {})",
            a_dst.1.y
        );
        assert_eq!(state.windows.get(&a).unwrap().output_id, o2);
        assert_eq!(state.windows.get(&b).unwrap().output_id, o2);
        assert_eq!(state.windows.get(&c).unwrap().output_id, o2);
    }

    #[test]
    fn focus_next_recovers_from_removed_focused_output() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new(o1));

        let w1 = WindowId(1);
        let w2 = WindowId(2);
        state.windows.insert(w1, Window::new(w1, o1));
        state.windows.insert(w2, Window::new(w2, o1));
        state.tree_for_output(o1).unwrap().insert_window(w1.0);
        state.tree_for_output(o1).unwrap().insert_window(w2.0);

        // Establish focus on w1 (tree.focused == w1, state.focused_window == w1),
        // then simulate its output having been removed out from under us.
        state.focus_window_id(w1);
        state.focused_output = Some(OutputId(2));
        assert_eq!(state.focused_window, Some(w1));

        state.focus_next();

        assert_eq!(state.focused_window, Some(w2));
        assert_eq!(state.focused_output, Some(o1));
    }

    /// Fuzz the single-output "spawn many / focus in directions / close many"
    /// workflow. Pinpoints any step where `tree.focused` (the layout focus used
    /// by `focus_direction`/`swap_windows`) diverges from `state.focused_window`
    /// or points at a window no longer in the tree.
    #[test]
    fn fuzz_focus_never_goes_stale() {
        use crate::layout::FocusDirection;

        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new(o1));
        state.focused_output = Some(o1);
        state.output_trees.insert(
            o1,
            crate::layout::LayoutTree::new(crate::layout::Rect::new(0, 0, 1920, 1080)),
        );

        let mut next = 1u32;
        let mut alive: Vec<WindowId> = Vec::new();

        // Deterministic LCG so the repro is stable.
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        let mut rng = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };

        let dirs = [
            FocusDirection::Left,
            FocusDirection::Right,
            FocusDirection::Up,
            FocusDirection::Down,
        ];

        for step in 0..10000 {
            // Spawn a fresh window when low on them.
            if alive.len() < 3 || rng() % 4 == 0 {
                let id = WindowId(next);
                next += 1;
                state.windows.insert(id, Window::new(id, o1));
                state.tree_for_output(o1).unwrap().insert_window(id.0);
                state.push_focus(id);
                alive.push(id);
            }

            match rng() % 3 {
                0 | 1 => {
                    let d = dirs[rng() % 4];
                    if state.focused_tree().is_some_and(|t| t.focus_direction(d))
                        && let Some(id) = state.focused_tree().and_then(|t| t.focused_window())
                    {
                        state.focus_window_id(WindowId(id));
                    }
                }
                _ => {
                    // Close a random alive window (mirrors the corrected handler).
                    if alive.is_empty() {
                        continue;
                    }
                    let idx = rng() % alive.len();
                    let id = alive.remove(idx);
                    state.close_window_focus_reconcile(id);
                }
            }

            // Invariant: tree.focused must be a window currently in the tree,
            // and it must equal state.focused_window.
            let visible: Vec<u32> = state.focused_tree().unwrap().visible_windows();
            let tree_focus = state.focused_tree().unwrap().focused_window();
            assert!(
                tree_focus.is_none() || visible.contains(&tree_focus.unwrap()),
                "step {step}: tree.focused {:?} not in visible {:?} (alive count {})",
                tree_focus,
                visible,
                alive.len()
            );
            assert_eq!(
                state.focused_window.map(|w| w.0),
                tree_focus,
                "step {step}: divergence state={:?} tree={:?}",
                state.focused_window,
                tree_focus
            );
        }
    }

    /// Multi-output counterpart of `fuzz_focus_never_goes_stale`: two outputs,
    /// windows spawned across both, focus moved in directions, windows closed
    /// on either output. Pinpoints any step where a per-output `tree.focused`
    /// diverges from `state.focused_window` or points at a window no longer in
    /// that output's tree.
    #[test]
    fn fuzz_multioutput_focus_never_goes_stale() {
        use crate::layout::FocusDirection;

        let mut state = WMState::new();

        // Two side-by-side outputs with real geometry.
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        let outputs = [o1, o2];
        for (o, x) in [(o1, 0), (o2, 1920)] {
            let mut out = Output::new(o);
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }

        let mut next = 1u32;
        // Track (output, window) so we can close on the right tree.
        let mut alive: Vec<(OutputId, WindowId)> = Vec::new();

        let mut seed: u64 = 0x1234567890ABCDEF;
        let mut rng = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };

        let dirs = [
            FocusDirection::Left,
            FocusDirection::Right,
            FocusDirection::Up,
            FocusDirection::Down,
        ];

        for step in 0..10000 {
            if alive.len() < 3 || rng() % 4 == 0 {
                let id = WindowId(next);
                next += 1;
                let out = outputs[rng() % outputs.len()];
                state.windows.insert(id, Window::new(id, out));
                state.tree_for_output(out).unwrap().insert_window(id.0);
                state.push_focus(id);
                alive.push((out, id));
            }

            match rng() % 3 {
                0 | 1 => {
                    let d = dirs[rng() % 4];
                    if state.focused_tree().is_some_and(|t| t.focus_direction(d))
                        && let Some(id) = state.focused_tree().and_then(|t| t.focused_window())
                    {
                        state.focus_window_id(WindowId(id));
                    }
                }
                _ => {
                    if alive.is_empty() {
                        continue;
                    }
                    let idx = rng() % alive.len();
                    let (_, id) = alive.remove(idx);
                    state.close_window_focus_reconcile(id);
                }
            }

            // Invariant 1: every output tree's focus must be a window currently
            // in that tree (or None when the tree is empty / not yet created).
            for &o in &outputs {
                let Some(tree) = state.output_trees.get(&o) else {
                    continue;
                };
                let visible: Vec<u32> = tree.visible_windows();
                let tf = tree.focused_window();
                assert!(
                    tf.is_none() || visible.contains(&tf.unwrap()),
                    "step {step}: output {o:?} tree.focused {tf:?} not in visible {visible:?}",
                );
            }

            // Invariant 2: global state focus must equal the focused output's
            // tree focus (no cross-output divergence).
            let tree_focus = state.focused_tree().unwrap().focused_window();
            assert_eq!(
                state.focused_window.map(|w| w.0),
                tree_focus,
                "step {step}: divergence state={:?} tree={:?}",
                state.focused_window,
                tree_focus
            );

            // Invariant 3: focused_output must own the focused window.
            if let Some(fw) = state.focused_window {
                assert_eq!(
                    state.focused_output,
                    state.windows.get(&fw).map(|w| w.output_id),
                    "step {step}: focused_output does not own focused window",
                );
            }
        }
    }

    #[test]
    fn focus_moves_to_other_output_when_focused_output_removed() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, x) in [(o1, 0), (o2, 1920)] {
            let mut out = Output::new(o);
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }

        let a = WindowId(1);
        let b = WindowId(2);
        state.windows.insert(a, Window::new(a, o1));
        state.windows.insert(b, Window::new(b, o2));
        state.tree_for_output(o1).unwrap().insert_window(a.0);
        state.tree_for_output(o2).unwrap().insert_window(b.0);

        // Focus the window on o1, so o1 is the focused output.
        state.focus_window_id(a);
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(state.focused_output, Some(o1));

        // Remove o1 while it holds the focused window: reassign o1 -> o2, then
        // drop o1 (mirrors the handler's Event::Output::Removed path).
        let removed_was_focused = state.focused_output == Some(o1);
        if let Some(to_id) = state.outputs.keys().find(|k| **k != o1).copied() {
            state.reassign_output(o1, to_id);
            if removed_was_focused {
                state.focused_output = Some(to_id);
            }
        }
        state.outputs.remove(&o1);
        state.output_trees.remove(&o1);

        // The focused window survives on o2 and stays focused.
        assert_eq!(state.windows.get(&a).map(|w| w.output_id), Some(o2));
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(state.focused_output, Some(o2));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(a.0));
    }

    #[test]
    fn removing_unfocused_output_keeps_focus() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, x) in [(o1, 0), (o2, 1920)] {
            let mut out = Output::new(o);
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }

        let a = WindowId(1);
        let b = WindowId(2);
        state.windows.insert(a, Window::new(a, o1));
        state.windows.insert(b, Window::new(b, o2));
        state.tree_for_output(o1).unwrap().insert_window(a.0);
        state.tree_for_output(o2).unwrap().insert_window(b.0);

        // Focus the window on o2 (the output we will NOT remove).
        state.focus_window_id(b);
        assert_eq!(state.focused_window, Some(b));
        assert_eq!(state.focused_output, Some(o2));

        // Remove o1 (which holds the unfocused window a).
        let removed_was_focused = state.focused_output == Some(o1);
        if let Some(to_id) = state.outputs.keys().find(|k| **k != o1).copied() {
            state.reassign_output(o1, to_id);
            if removed_was_focused {
                state.focused_output = Some(to_id);
            }
        }
        state.outputs.remove(&o1);
        state.output_trees.remove(&o1);

        // Focus must remain on b / o2.
        assert_eq!(state.focused_window, Some(b));
        assert_eq!(state.focused_output, Some(o2));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(b.0));
    }
}
