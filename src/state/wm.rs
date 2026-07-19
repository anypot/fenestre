//! Core runtime state for Fenestre.
//!
//! Owns all mutable compositor state: River protocol proxies, windows, outputs,
//! seats, runtime keybindings, focus state, configuration, layout trees, and
//! pending River-managed state changes. Scene-related fields (`last_manage_scene`,
//! `last_render_scene`, `render_order_cache`) and
//! the declarative reconciler (`desired_scene`, `apply_manage`, `apply_render`)
//! are defined in `scene.rs` as an extension impl on `WMState`.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use super::events::Event;
use super::keybindings::{KeyBinding, XkbBindingId};
use super::output::{Output, OutputId};
use super::pointerbindings::{PointerBinding, PointerBindingId};
use super::scene::SceneSnapshot;
use super::seat::{InteractiveOp, Seat, SeatId};
use super::window::{Window, WindowId};
use crate::config::Config;
use crate::layout::{LayoutTree, Rect, WindowState};

/// Default edges for a resize when the pointer position or window geometry
/// cannot be determined: bottom-right resize (bottom=2 | right=8 = 10).
const DEFAULT_RESIZE_EDGES: u32 = 0b1010;

/// Result of computing an interactive operation's new rect.
pub(super) struct OpRect {
    pub(super) window_id: WindowId,
    pub(super) rect: Rect,
}

/// Owns all mutable compositor state for the window manager.
///
/// `WMState` stores River protocol proxies, windows, outputs, seats,
/// runtime keybindings, focus state, configuration, layout trees, and pending
/// River-managed state changes. The scene snapshot fields (`last_manage_scene`,
/// `last_render_scene`, `render_order_cache`) are
/// owned by the `scene` module — mutate them only through `apply_manage` or
/// `apply_render`.
///
/// Protocol event handlers mutate this state. Configuration reconciliation updates
/// runtime keybindings and queues window-rule reapplication.
pub(crate) struct WMState {
    pub(super) wm: Option<crate::protocol::river::river_window_management_v1::client::river_window_manager_v1::RiverWindowManagerV1>,
    pub(super) xkb_bindings: Option<crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_bindings_v1::RiverXkbBindingsV1>,
    pub(super) layer_shell: Option<crate::protocol::river::river_layer_shell_v1::client::river_layer_shell_v1::RiverLayerShellV1>,

    pub(super) config: Option<Config>,
    pub(super) config_path: Option<PathBuf>,

    pub(super) windows: HashMap<WindowId, Window>,
    pub(super) outputs: HashMap<OutputId, Output>,
    pub(super) seats: BTreeMap<SeatId, Seat>,
    pub(super) keybindings: HashMap<XkbBindingId, KeyBinding>,
    /// Runtime River pointer bindings (Super+drag to move/resize), keyed by id.
    pub(super) pointer_bindings: HashMap<PointerBindingId, PointerBinding>,
    pub(super) output_trees: HashMap<OutputId, LayoutTree>,

    pub(super) focused_window: Option<WindowId>,
    pub(super) focused_output: Option<OutputId>,
    pub(super) focus_stack: Vec<WindowId>,

    pub(super) current_seat: Option<SeatId>,

    /// True when desired xkb bindings differ from configured River binding objects.
    pub(super) xkb_bindings_dirty: bool,
    /// River xkb binding protocol objects queued for destruction during the next manage sequence.
    pub(super) pending_xkb_binding_destroys: Vec<crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_binding_v1::RiverXkbBindingV1>,
    /// True when desired pointer bindings differ from configured River binding objects.
    pub(super) pointer_bindings_dirty: bool,
    /// River pointer binding protocol objects queued for destruction during the next manage sequence.
    pub(super) pending_pointer_binding_destroys: Vec<crate::protocol::river::river_window_management_v1::client::river_pointer_binding_v1::RiverPointerBindingV1>,
    /// Window rules loaded from config, applied per-window when metadata is known.
    pub(super) window_rules: Option<super::rule::WindowRules>,
    /// Window ID to focus during the next manage sequence.
    pub(super) pending_focus: Option<WindowId>,
    /// Window IDs to close during the next manage sequence.
    pub(super) pending_closes: Vec<WindowId>,

    /// Pending split direction for the next spawned window.
    pub(super) pending_split: Option<crate::layout::SplitDirection>,

    /// Cached sorted window IDs for render stacking order.
    ///
    /// Owned by the scene module — mutate via `apply_manage` / `apply_render`.
    pub(super) render_order_cache: Vec<WindowId>,

    /// Snapshot of the last desired scene from a manage cycle, used to diff
    /// manage-phase effects (dimensions, fullscreen, server-side decorations).
    ///
    /// Owned by the scene module — mutate via `apply_manage` / `apply_render`.
    pub(super) last_manage_scene: SceneSnapshot,
    /// Set whenever the layer-shell default may need re-emitting (focus change,
    /// first focused output, or a freshly created layer-shell proxy); flushed
    /// (and cleared) inside `apply_manage`. Keeps `set_default` inside manage
    /// sequences per the River protocol constraint.
    ///
    /// Owned by the scene module — set via `set_focused_output` / adapter, cleared
    /// in `apply_manage`.
    pub(super) layer_shell_default_dirty: bool,
    /// Snapshot of the last desired scene from a render cycle, used to diff
    /// render-phase effects (position, z-order, borders).
    ///
    /// Owned by the scene module — mutate via `apply_manage` / `apply_render`.
    pub(super) last_render_scene: SceneSnapshot,

    /// Next identifiers for internal ID allocation.
    next_window_id: WindowId,
    next_output_id: OutputId,
    next_seat_id: SeatId,
    next_xkb_binding_id: XkbBindingId,
    next_pointer_binding_id: PointerBindingId,

    /// Proxy-to-ID indexes for O(1) lookup by Wayland object.
    pub(super) windows_by_proxy: HashMap<crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1, WindowId>,
    pub(super) outputs_by_proxy: HashMap<crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1, OutputId>,
    pub(super) seats_by_proxy: HashMap<crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1, SeatId>,

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
            layer_shell: None,
            config: None,
            config_path: None,

            windows: HashMap::new(),
            outputs: HashMap::new(),
            seats: BTreeMap::new(),
            keybindings: HashMap::new(),
            pointer_bindings: HashMap::new(),
            output_trees: HashMap::new(),

            focused_window: None,
            focused_output: None,
            focus_stack: Vec::new(),

            current_seat: None,

            xkb_bindings_dirty: false,
            pending_xkb_binding_destroys: Vec::new(),
            pointer_bindings_dirty: false,
            pending_pointer_binding_destroys: Vec::new(),
            window_rules: None,
            pending_focus: None,
            pending_closes: Vec::new(),
            pending_split: None,

            next_window_id: WindowId(0),
            next_output_id: OutputId(0),
            next_seat_id: SeatId(0),
            next_xkb_binding_id: XkbBindingId(0),
            next_pointer_binding_id: PointerBindingId(0),
            render_order_cache: Vec::new(),
            last_manage_scene: Vec::new(),
            layer_shell_default_dirty: false,
            last_render_scene: Vec::new(),
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

    /// Allocate the next internal River pointer binding identifier.
    pub(super) fn next_pointer_binding_id(&mut self) -> PointerBindingId {
        let id = self.next_pointer_binding_id;
        self.next_pointer_binding_id.0 += 1;
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
            .and_then(|o| o.tiling_rect())
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

    /// Run window rules against a newly updated window.
    ///
    /// Called from `handle_event` when metadata arrives so rules can match as
    /// soon as identifiers are known.
    pub(super) fn evaluate_window_rules(&mut self, window_id: WindowId) {
        let Some(output_id) = self.windows.get(&window_id).map(|w| w.output_id) else {
            return;
        };
        if self.window_rules.is_none() {
            return;
        }
        let Some(output_rect) = self.outputs.get(&output_id).and_then(|o| o.rect()) else {
            return;
        };
        let float_ratio = self.default_float_ratio();
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
                .evaluate(window, tree, output_rect, float_ratio)
        };

        if changed && let Some(wm) = &self.wm {
            wm.manage_dirty();
        }
    }

    /// Resolve a window ID to its per-output `LayoutTree`.
    pub(super) fn tree_for_window_id(
        &mut self,
        window_id: WindowId,
    ) -> Option<(WindowId, &mut LayoutTree)> {
        let output_id = self.windows.get(&window_id)?.output_id;
        let tree = self.tree_for_output(output_id)?;
        Some((window_id, tree))
    }

    /// Find an output by internal ID.
    pub(super) fn find_output_mut_by_proxy_id(
        &mut self,
        output_id: OutputId,
    ) -> Option<(OutputId, &mut Output)> {
        self.outputs
            .get_mut(&output_id)
            .map(|output| (output_id, output))
    }

    /// Find a seat by internal ID.
    pub(super) fn find_seat_mut_by_id(&mut self, seat_id: SeatId) -> Option<(SeatId, &mut Seat)> {
        self.seats.get_mut(&seat_id).map(|seat| (seat_id, seat))
    }

    /// Remove an output by internal ID.
    pub(super) fn remove_output_by_id(
        &mut self,
        output_id: OutputId,
        fallback_output: Option<OutputId>,
    ) -> Option<OutputId> {
        let removed_was_focused = self.focused_output == Some(output_id);
        self.outputs.remove(&output_id);

        if removed_was_focused {
            self.focused_output = fallback_output.or_else(|| self.outputs.keys().next().copied());
        }

        Some(output_id)
    }

    /// Remove a seat by internal ID.
    pub(super) fn remove_seat_by_id(&mut self, seat_id: SeatId) -> Option<SeatId> {
        let removed_was_current = self.current_seat == Some(seat_id);
        self.seats.remove(&seat_id);

        if removed_was_current
            || self
                .current_seat
                .is_some_and(|current| !self.seats.contains_key(&current))
        {
            self.current_seat = self.seats.first_key_value().map(|(id, _)| *id);
        }

        Some(seat_id)
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

    /// Resolve the current `WindowState` for a window from its output tree.
    pub(super) fn window_state_for_id(&self, window_id: WindowId) -> Option<&WindowState> {
        let output_id = self.windows.get(&window_id)?.output_id;
        let tree = self.output_trees.get(&output_id)?;
        tree.window_state(window_id.0)
    }

    /// Set `focused_output`, flagging the layer-shell default for re-emission
    /// whenever the value actually changes.
    ///
    /// The flag is consumed (and cleared) only inside `apply_manage`, so the
    /// resulting `set_default` protocol call stays within a manage sequence, as
    /// required by River. Callers that need a manage cycle should still request
    /// one separately.
    pub(super) fn set_focused_output(&mut self, output_id: OutputId) {
        if self.focused_output != Some(output_id) {
            self.focused_output = Some(output_id);
            self.layer_shell_default_dirty = true;
        }
    }

    /// Ensure `focused_output` points at a live output.
    ///
    /// Self-heals a stale `focused_output` (e.g. one whose output was removed)
    /// by falling back to the first remaining output. Does NOT create a
    /// `LayoutTree`: callers that only need read-only geometry use this instead
    /// of `tree_for_output`, which builds a tree on a miss.
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

    /// Find a window by internal ID.
    pub(super) fn find_window_mut_by_id(
        &mut self,
        id: WindowId,
    ) -> Option<(WindowId, &mut Window)> {
        self.windows.get_mut(&id).map(|window| (id, window))
    }

    /// Whether the given seat currently has an interactive operation in progress.
    pub(super) fn seat_op_active(&self, seat_id: SeatId) -> bool {
        self.seats
            .get(&seat_id)
            .is_some_and(|seat| seat.op.is_active())
    }

    /// Reset every seat whose active interactive operation targets `window_id`.
    ///
    /// Called when a window is closed: an op that points at a now-removed window
    /// would otherwise be stranded. If `op_start_pointer` had not yet been sent
    /// (`op_started == false`), the next manage would emit `StartPointerOp` for a
    /// destroyed window; if it had, the op would leak on the seat until a physical
    /// button release and block every future drag for that seat via
    /// `start_interactive_op`'s `seat_op_active` guard. Resetting here drops the
    /// op, its pending float, and any scheduled `op_end` so the seat returns idle.
    pub(super) fn cancel_op_for_window(&mut self, window_id: WindowId) {
        for seat in self.seats.values_mut() {
            if seat.op.window_id() == Some(window_id) {
                seat.op = InteractiveOp::Inactive;
                seat.op_started = false;
                seat.op_ending = false;
                seat.pending_float = None;
            }
        }
    }

    /// Record a pending interactive operation for `seat_id`/`window_id`, capturing
    /// the window's current rect as the operation's anchor. The actual
    /// `op_start_pointer` protocol call is deferred to the next manage sequence.
    /// Returns `true` if the op was recorded (window exists, seat idle).
    fn start_interactive_op(
        &mut self,
        seat_id: SeatId,
        window_id: WindowId,
        op: InteractiveOp,
    ) -> bool {
        if !self.windows.contains_key(&window_id) || self.seat_op_active(seat_id) {
            return false;
        }
        let Some(initial_rect) = self.window_current_rect(window_id) else {
            return false;
        };
        if let Some(seat) = self.seats.get_mut(&seat_id) {
            let op = match op {
                InteractiveOp::Move { window_id, .. } => InteractiveOp::Move {
                    window_id,
                    initial_rect,
                },
                InteractiveOp::Resize {
                    window_id, edges, ..
                } => InteractiveOp::Resize {
                    window_id,
                    edges,
                    initial_rect,
                },
                InteractiveOp::Inactive => return false,
            };
            seat.op = op;
            seat.op_started = false;
            seat.op_ending = false;
        }
        self.request_manage_dirty();
        true
    }

    /// Return the window's current arranged rect, preferring the rect set during
    /// the last manage cycle (`layout_rect`) and falling back to the tree's
    /// current arrangement.
    pub(super) fn window_current_rect(&self, window_id: WindowId) -> Option<Rect> {
        if let Some(window) = self.windows.get(&window_id)
            && let Some(rect) = window.layout_rect
        {
            return Some(rect);
        }
        let output_id = self.windows.get(&window_id)?.output_id;
        self.output_trees
            .get(&output_id)?
            .arranged_windows_readonly()
            .into_iter()
            .find(|(id, _, _)| *id == window_id.0)
            .map(|(_, rect, _)| rect)
    }

    /// Return the rectangle of the output that currently owns `window_id`, or
    /// `None` if the window has no output or the output geometry is unknown.
    pub(super) fn window_output_rect(&self, window_id: WindowId) -> Option<Rect> {
        let output_id = self.windows.get(&window_id)?.output_id;
        self.outputs.get(&output_id)?.rect()
    }

    /// Compute the resize edge bitmask (top=1, bottom=2, left=4, right=8) for a
    /// Super+drag resize, based on the pointer's position within the window.
    ///
    /// The pointer position (global coordinates) is compared against the
    /// window's global rectangle (output position + layout rect). The half of
    /// the window the pointer is on selects the corresponding edge(s), giving a
    /// four-corner resize. Falls back to a bottom-right resize when the pointer
    /// position or window geometry is unavailable.
    pub(super) fn compute_resize_edges(&self, seat_id: SeatId, window_id: WindowId) -> u32 {
        let Some(seat) = self.seats.get(&seat_id) else {
            return DEFAULT_RESIZE_EDGES;
        };
        let Some((px, py)) = seat.pointer_position else {
            return DEFAULT_RESIZE_EDGES;
        };
        let Some(local) = self.window_current_rect(window_id) else {
            return DEFAULT_RESIZE_EDGES;
        };
        let Some(output_id) = self.windows.get(&window_id).map(|w| w.output_id) else {
            return DEFAULT_RESIZE_EDGES;
        };
        let Some(output_rect) = self.outputs.get(&output_id).and_then(|o| o.rect()) else {
            return DEFAULT_RESIZE_EDGES;
        };
        // Global window rect: output origin offset by the window's local rect.
        let wx = output_rect.x + local.x;
        let wy = output_rect.y + local.y;
        let cx = wx + local.width / 2;
        let cy = wy + local.height / 2;

        let mut edges = 0;
        if px < cx {
            edges |= 0b0100; // left
        } else {
            edges |= 0b1000; // right
        }
        if py < cy {
            edges |= 0b0001; // top
        } else {
            edges |= 0b0010; // bottom
        }
        edges
    }

    /// Compute the new rect for a seat's active interactive operation given a
    /// cumulative delta `(dx, dy)` since op start. Returns `None` when the seat
    /// has no active operation or the window is missing.
    ///
    /// For a move, the initial rect is simply translated by `(dx, dy)`. For a
    /// resize, the `edges` bitmask (top=1, bottom=2, left=4, right=8) selects
    /// which borders move; edges opposed to the grabbed ones stay fixed. The
    /// resulting size is clamped to the window's `dimensions_hint`.
    pub(super) fn compute_op_rect(&self, seat_id: SeatId, dx: i32, dy: i32) -> Option<OpRect> {
        let seat = self.seats.get(&seat_id)?;
        let (window_id, edges, initial_rect) = match seat.op {
            InteractiveOp::Move {
                window_id,
                initial_rect,
            } => (window_id, 0, initial_rect),
            InteractiveOp::Resize {
                window_id,
                edges,
                initial_rect,
            } => (window_id, edges, initial_rect),
            InteractiveOp::Inactive => return None,
        };
        let window = self.windows.get(&window_id)?;
        let rect = match edges {
            0 => Rect::new(
                initial_rect.x.saturating_add(dx),
                initial_rect.y.saturating_add(dy),
                initial_rect.width,
                initial_rect.height,
            ),
            edges => {
                let mut x = initial_rect.x;
                let mut y = initial_rect.y;
                let mut width = initial_rect.width;
                let mut height = initial_rect.height;
                if edges & 0b1000 != 0 {
                    width = initial_rect.width.saturating_add(dx);
                }
                if edges & 0b0100 != 0 {
                    x = initial_rect.x.saturating_add(dx);
                    width = initial_rect.width.saturating_sub(dx);
                }
                if edges & 0b0010 != 0 {
                    height = initial_rect.height.saturating_add(dy);
                }
                if edges & 0b0001 != 0 {
                    y = initial_rect.y.saturating_add(dy);
                    height = initial_rect.height.saturating_sub(dy);
                }
                let (cw, ch) = window.clamp_dimensions(width, height);
                // Re-anchor left/top-grabbed edges when clamping shortened them.
                if edges & 0b0100 != 0 {
                    x = initial_rect.x.saturating_add(initial_rect.width - cw);
                }
                if edges & 0b0001 != 0 {
                    y = initial_rect.y.saturating_add(initial_rect.height - ch);
                }
                Rect::new(x, y, cw, ch)
            }
        };
        // Anchor the computed rect to the owning output so cumulative deltas
        // (which are untrusted i32 values from the compositor) can never produce
        // negative or overflowed coordinates that would be sent to the compositor.
        let rect = if let Some(output) = self.window_output_rect(window_id) {
            let max_x = (output.x + output.width).saturating_sub(rect.width);
            let max_y = (output.y + output.height).saturating_sub(rect.height);
            let clamped_x = rect.x.clamp(output.x, max_x.max(output.x));
            let clamped_y = rect.y.clamp(output.y, max_y.max(output.y));
            Rect::new(clamped_x, clamped_y, rect.width, rect.height)
        } else {
            rect
        };
        Some(OpRect { window_id, rect })
    }

    /// Process a domain event, mutating state.
    pub(super) fn handle_event(&mut self, event: Event) {
        match event {
            Event::WindowCreated {
                window_id,
                target_output,
            } => {
                let window = Window::new(window_id, target_output);
                self.windows.insert(window_id, window);
                self.index_window_in_output(window_id, target_output);
                let pending_split = self.pending_split.take();
                self.ensure_tree_for_output(target_output)
                    .insert_window(window_id.0, pending_split);
                self.push_focus(window_id);
                self.pending_focus = Some(window_id);
                self.request_manage_dirty();
            }
            Event::OutputCreated { output_id } => {
                let output = Output::new();
                self.outputs.insert(output_id, output);

                let orphaned: Vec<OutputId> = self
                    .output_trees
                    .keys()
                    .filter(|id| !self.outputs.contains_key(id))
                    .copied()
                    .collect();
                for orphaned_id in orphaned {
                    if self.focused_output == Some(orphaned_id) {
                        self.set_focused_output(output_id);
                    }
                    self.reassign_output(orphaned_id, output_id);
                }

                if self.focused_output.is_none() {
                    self.set_focused_output(output_id);
                }
            }
            Event::SeatCreated { seat_id } => {
                let seat = Seat::new();
                self.seats.insert(seat_id, seat);
                if self.current_seat.is_none() {
                    self.current_seat = Some(seat_id);
                }
                self.reconcile_keybindings();
                self.reconcile_pointer_bindings();
                self.request_manage_dirty();
            }
            Event::WindowClosed { window_id } => {
                self.close_window_focus_reconcile(window_id);
            }
            Event::WindowInteraction { window_id } => {
                if self.focused_window != Some(window_id) {
                    self.focus_window_id(window_id);
                }
            }
            Event::DimensionsHint {
                window_id,
                min_w,
                min_h,
                max_w,
                max_h,
            } => {
                if let Some((_, window)) = self.find_window_mut_by_id(window_id) {
                    window.set_dimensions_hint(min_w, min_h, max_w, max_h);
                }
            }
            Event::AppIdUpdated { window_id, app_id } => {
                if let Some((_, window)) = self.find_window_mut_by_id(window_id) {
                    window.app_id = app_id;
                }
                self.evaluate_window_rules(window_id);
            }
            Event::TitleUpdated { window_id, title } => {
                if let Some((_, window)) = self.find_window_mut_by_id(window_id) {
                    window.title = title;
                }
                self.evaluate_window_rules(window_id);
            }
            Event::ParentUpdated {
                window_id,
                parent_id,
            } => {
                if let Some((_, window)) = self.find_window_mut_by_id(window_id) {
                    window.parent = parent_id;
                };
            }
            Event::DecorationHintUpdated { window_id, hint } => {
                if let Some((_, window)) = self.find_window_mut_by_id(window_id) {
                    window.decoration_hint = Some(hint);
                }
            }
            Event::PidUpdated { window_id, pid } => {
                if let Some((_, window)) = self.find_window_mut_by_id(window_id) {
                    window.pid = pid as i32;
                }
            }
            Event::FullscreenRequested { window_id } => {
                if let Some((_, tree)) = self.tree_for_window_id(window_id)
                    && tree.toggle_fullscreen(window_id.0)
                {
                    if self.focused_window == Some(window_id) {
                        self.pending_split = None;
                        self.render_order_cache.clear();
                    }
                    self.request_manage_dirty();
                }
            }
            Event::ExitFullscreenRequested { window_id } => {
                if let Some((_, tree)) = self.tree_for_window_id(window_id)
                    && tree.toggle_fullscreen(window_id.0)
                {
                    if self.focused_window == Some(window_id) {
                        self.pending_split = None;
                        self.render_order_cache.clear();
                    }
                    self.request_manage_dirty();
                }
            }
            Event::OutputRemoved { output_id } => {
                let reassign_target = self.outputs.keys().find(|k| **k != output_id).copied();
                if let Some(to_id) = reassign_target {
                    self.reassign_output(output_id, to_id);
                }
                self.windows_by_output.remove(&output_id);
                let _ = self.remove_output_by_id(output_id, reassign_target);
                self.request_manage_dirty();
            }
            Event::OutputNameUpdated { output_id, name } => {
                if let Some((_, output)) = self.find_output_mut_by_proxy_id(output_id) {
                    output.wl_output_name = name;
                    self.request_manage_dirty();
                }
            }
            Event::OutputPositionUpdated { output_id, x, y } => {
                if let Some((_, output)) = self.find_output_mut_by_proxy_id(output_id) {
                    output.set_position(x, y);
                    self.request_manage_dirty();
                }
            }
            Event::OutputDimensionsUpdated { output_id, w, h } => {
                if let Some((_, output)) = self.find_output_mut_by_proxy_id(output_id) {
                    output.set_dimensions(w, h);
                    self.request_manage_dirty();
                }
                let window_ids: Vec<WindowId> = self
                    .windows_for_output(output_id)
                    .into_iter()
                    .flatten()
                    .copied()
                    .collect();
                for window_id in window_ids {
                    self.evaluate_window_rules(window_id);
                }
            }
            Event::SeatRemoved { seat_id } => {
                let _ = self.remove_seat_by_id(seat_id);
                self.reconcile_keybindings();
                self.reconcile_pointer_bindings();
                self.request_manage_dirty();
            }
            Event::SeatNameUpdated { seat_id, name } => {
                if let Some((_, seat)) = self.find_seat_mut_by_id(seat_id) {
                    seat.wl_seat_name = name;
                }
            }
            Event::SeatPointerPositionUpdated { seat_id, x, y } => {
                if let Some((_, seat)) = self.find_seat_mut_by_id(seat_id) {
                    seat.pointer_position = Some((x, y));
                }
            }
            Event::SeatLayerShellFocus { seat_id, mode } => {
                if let Some((_, seat)) = self.find_seat_mut_by_id(seat_id) {
                    seat.layer_shell_focus = mode;
                }
                // `focus_none` means River hands focus back to the WM, so re-run
                // manage. The adapter drops `FocusWindow` effects while focus is
                // `Exclusive` and `apply_manage` clears `pending_focus` before they
                // are emitted, so re-queue the current focus here so it is actually
                // re-applied instead of staying desynced from River.
                if mode == crate::state::seat::LayerShellFocus::None
                    && let Some(window_id) = self.focused_window
                {
                    self.pending_focus = Some(window_id);
                }
                self.request_manage_dirty();
            }
            Event::PointerMoveRequested { window_id, seat_id } => {
                // Record a pending move operation. The actual `op_start_pointer`
                // protocol call is deferred to the next manage sequence (it may
                // only be issued inside ManageStart); the window's current rect
                // is captured as the operation's anchor.
                self.start_interactive_op(
                    seat_id,
                    window_id,
                    InteractiveOp::Move {
                        window_id,
                        initial_rect: Rect::new(0, 0, 0, 0),
                    },
                );
            }
            Event::PointerResizeRequested {
                window_id,
                seat_id,
                edges,
            } => {
                // Record a pending resize operation, mirroring the move path.
                // `edges` is the River edges bitmask describing which borders the
                // pointer grabbed; it drives the delta transform in `OpDelta`.
                self.start_interactive_op(
                    seat_id,
                    window_id,
                    InteractiveOp::Resize {
                        window_id,
                        edges,
                        initial_rect: Rect::new(0, 0, 0, 0),
                    },
                );
            }
            Event::OpDelta { seat_id, dx, dy } => {
                // Cumulative delta since op start: transform the recorded
                // `initial_rect` into a new rect and store it as a pending float
                // on the seat. The layout tree is only mutated once, inside the
                // next `apply_manage`, so a drag that emits many `op_delta`
                // events per manage sequence does not traverse the tree each time.
                if let Some(new_rect) = self.compute_op_rect(seat_id, dx, dy) {
                    if let Some(seat) = self.seats.get_mut(&seat_id) {
                        seat.pending_float = Some((new_rect.window_id, new_rect.rect));
                    }
                    self.request_manage_dirty();
                }
            }
            Event::OpRelease { seat_id } => {
                // Buttons released: schedule `op_end` on the next manage sequence.
                // The op stays active so trailing `op_delta` events still apply.
                if let Some(seat) = self.seats.get_mut(&seat_id)
                    && seat.op.is_active()
                {
                    seat.op_ending = true;
                    self.request_manage_dirty();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_outputs_get_independent_trees() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new());
        state.outputs.insert(o2, Output::new());
        state.focused_output = Some(o1);

        let w1 = WindowId(1);
        let w2 = WindowId(2);
        state.windows.insert(w1, Window::new(w1, o1));
        state.windows.insert(w2, Window::new(w2, o2));
        state.tree_for_output(o1).unwrap().insert_window(w1.0, None);
        state.tree_for_output(o2).unwrap().insert_window(w2.0, None);

        assert_eq!(state.tree_for_output(o1).unwrap().focused_window(), Some(1));
        assert_eq!(state.tree_for_output(o2).unwrap().focused_window(), Some(2));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(1));
    }

    #[test]
    fn output_resize_does_not_affect_other_output_tree() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new());
        state.outputs.insert(o2, Output::new());
        state.outputs.get_mut(&o1).unwrap().set_dimensions(800, 600);
        state
            .outputs
            .get_mut(&o2)
            .unwrap()
            .set_dimensions(1920, 1080);
        state.focused_output = Some(o1);

        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        state.tree_for_output(o1).unwrap().insert_window(w1.0, None);

        state
            .tree_for_output(o1)
            .unwrap()
            .set_output_rect(crate::layout::Rect::new(0, 0, 800, 600));
        let arranged = state.tree_for_output(o1).unwrap().arranged_windows();
        assert_eq!(arranged.len(), 1);
    }

    /// Build a state with one pseudo-tiled window that has round-tripped through
    /// fullscreen (app reported fullscreen dimensions). Returns the state, the
    /// output id, the window id, and the original pseudo-tiled rect.
    fn setup_pseudo_fullscreen_roundtrip() -> (WMState, OutputId, WindowId, Rect) {
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        let w = WindowId(1);
        state.windows.insert(w, Window::new(w, o));
        state.tree_for_output(o).unwrap().insert_window(w.0, None);
        state.focused_window = Some(w);

        // Spawn pseudo-tiled at the output fraction.
        let output_rect = crate::layout::Rect::new(0, 0, 1920, 1080);
        let ratio = state.default_float_ratio();
        let pseudo = state
            .windows
            .get(&w)
            .unwrap()
            .pseudo_tiled_rect(output_rect, ratio);
        state.tree_for_output(o).unwrap().set_window_state(
            w.0,
            crate::layout::WindowState::PseudoTiled { rect: pseudo },
        );

        // Round-trip through fullscreen. (The app-reported size while
        // fullscreened is no longer stored on the window, so there is nothing to
        // set here; the pseudo size must be preserved via the tree's stored rect.)
        state.tree_for_output(o).unwrap().toggle_fullscreen(w.0);

        (state, o, w, pseudo)
    }

    #[test]
    fn pseudo_tiled_after_fullscreen_keeps_pseudo_size() {
        // Regression: after a fullscreen round-trip, a pseudo-tiled window must
        // keep its pseudo size, not reuse the fullscreen dimensions the app
        // reported while fullscreened. `apply_manage` proposes the arranged
        // `window_rect` directly for pseudo-tiled windows, which stays correct
        // because the pseudo `floating_rect` is preserved across the toggle.
        let (mut state, o, w2, pseudo) = setup_pseudo_fullscreen_roundtrip();

        // Toggle fullscreen back -> pseudo.
        state.tree_for_output(o).unwrap().toggle_fullscreen(w2.0);

        // The arranged pseudo rect must be the original pseudo size, not the
        // stale fullscreen dimensions.
        let (_, window_rect, state_w2) = state
            .tree_for_output(o)
            .unwrap()
            .arranged_windows()
            .into_iter()
            .find(|(id, _, _)| *id == w2.0)
            .unwrap();
        assert_eq!(
            state_w2,
            crate::layout::WindowState::PseudoTiled { rect: pseudo }
        );
        assert_eq!(
            window_rect.width, pseudo.width,
            "pseudo window reused stale fullscreen dimensions"
        );
    }

    #[test]
    fn toggle_from_fullscreen_to_float_keeps_float_size() {
        // Regression: a window toggled straight from fullscreen to floating must
        // restore its pre-fullscreen (output-fraction) size, not reuse the
        // fullscreen dimensions the app reports while fullscreened. This covers
        // the `resolve_toggle_rect` path that `apply_manage` does not.
        let (mut state, _o, w, pseudo) = setup_pseudo_fullscreen_roundtrip();

        // Toggle directly to floating from fullscreen.
        state.toggle_focused_floating();

        let (_, window_rect, state_w) = state
            .tree_for_output(_o)
            .unwrap()
            .arranged_windows()
            .into_iter()
            .find(|(id, _, _)| *id == w.0)
            .unwrap();
        assert_eq!(
            state_w,
            crate::layout::WindowState::Floating { rect: pseudo }
        );
        assert_eq!(
            window_rect.width, pseudo.width,
            "float toggled from fullscreen reused stale fullscreen dimensions"
        );
    }

    #[test]
    fn toggle_from_fullscreen_to_pseudo_keeps_pseudo_size() {
        // Same regression as above, but toggling to pseudo-tiled directly from
        // fullscreen instead of floating.
        let (mut state, _o, w, pseudo) = setup_pseudo_fullscreen_roundtrip();

        state.toggle_focused_pseudo_tiled();

        let (_, window_rect, state_w) = state
            .tree_for_output(_o)
            .unwrap()
            .arranged_windows()
            .into_iter()
            .find(|(id, _, _)| *id == w.0)
            .unwrap();
        assert_eq!(
            state_w,
            crate::layout::WindowState::PseudoTiled { rect: pseudo }
        );
        assert_eq!(
            window_rect.width, pseudo.width,
            "pseudo toggled from fullscreen reused stale fullscreen dimensions"
        );
    }

    // --- `handle_event` routing tests -------------------------------------
    //
    // These exercise the match arms in `WMState::handle_event` end to end,
    // feeding `Event` variants through the real dispatch path instead of
    // calling the underlying methods directly. They catch bugs where a match
    // arm forgets to invoke a side effect (e.g. `AppIdUpdated` not calling
    // `evaluate_window_rules`).

    /// Build a minimal `WMState` with one output (real geometry) and one
    /// floating window-rule keyed on an exact app_id. The output geometry is
    /// set so `Output::rect` is non-`None`, which `evaluate_window_rules`
    /// requires before it will run rules.
    fn setup_handle_event_fixture() -> (WMState, OutputId, WindowId) {
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        // A rule that floats only when BOTH the exact app_id "floatme" and the
        // exact title "pinned" are present. Requiring the title keeps a window
        // in the pending-metadata state after its app_id arrives, which is what
        // lets `OutputDimensionsUpdated`'s re-evaluation branch be observable.
        let rules = crate::state::rule::WindowRules::new(vec![crate::config::WindowRule {
            app_id: Some(crate::config::RulePattern::exact("floatme")),
            title: Some(crate::config::RulePattern::exact("pinned")),
            target: crate::layout::WindowState::Floating {
                rect: crate::layout::Rect::new(0, 0, 0, 0),
            },
            floating_rect: None,
        }]);
        state.window_rules = Some(rules);

        let w = WindowId(1);
        (state, o, w)
    }

    #[test]
    fn event_window_created_registers_window_tree_and_focus() {
        let (mut state, o, w) = setup_handle_event_fixture();

        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o,
        });

        // Window exists and is indexed to the output.
        assert!(state.windows.contains_key(&w), "window not registered");
        assert_eq!(
            state.windows_for_output(o).map(|s| s.contains(&w)),
            Some(true),
            "window not indexed under its output"
        );
        // It is in the output's layout tree.
        assert!(
            state
                .tree_for_output(o)
                .unwrap()
                .visible_windows()
                .contains(&w.0),
            "window missing from layout tree"
        );
        // It became the focused window and is queued for River focus.
        assert_eq!(state.focused_window, Some(w), "created window not focused");
        assert_eq!(state.pending_focus, Some(w), "focus not queued for manage");
        assert_eq!(state.focused_output, Some(o));
    }

    #[test]
    fn event_window_closed_reconciles_focus() {
        let (mut state, o, a) = setup_handle_event_fixture();
        let b = WindowId(2);

        // Spawn A then B on the same output; B is globally focused last.
        state.handle_event(Event::WindowCreated {
            window_id: a,
            target_output: o,
        });
        state.handle_event(Event::WindowCreated {
            window_id: b,
            target_output: o,
        });
        assert_eq!(state.focused_window, Some(b));

        // Close the globally focused window B; focus must move to A (same
        // output's tree choice), not drop focus or jump elsewhere.
        state.handle_event(Event::WindowClosed { window_id: b });

        assert!(
            !state.windows.contains_key(&b),
            "closed window still present"
        );
        assert!(
            state.windows_for_output(o).map(|s| s.contains(&b)) == Some(false),
            "closed window still indexed under its output"
        );
        assert_eq!(state.focused_window, Some(a), "focus not moved to A");
        assert_eq!(
            state.focused_tree().unwrap().focused_window(),
            Some(a.0),
            "layout focus diverged from global focus after close"
        );
        assert_eq!(state.pending_focus, Some(a), "focus not re-queued");
    }

    #[test]
    fn event_app_id_updated_evaluates_rules() {
        let (mut state, o, w) = setup_handle_event_fixture();

        // Create the window; no metadata yet, so the float rule cannot match.
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o,
        });
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Tiled),
            "window should start tiled before app_id arrives"
        );

        // Deliver the matching app_id. The rule also requires a title, so the
        // window stays in the pending-metadata state: routing must still call
        // `evaluate_window_rules` (storing the app_id) without floating yet.
        state.handle_event(Event::AppIdUpdated {
            window_id: w,
            app_id: Some("floatme".to_string()),
        });

        assert_eq!(
            state.windows.get(&w).unwrap().app_id.as_deref(),
            Some("floatme"),
            "app_id not stored"
        );
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Tiled),
            "AppIdUpdated must not float before the title arrives"
        );

        // Deliver the matching title: the rule now applies and the window floats.
        state.handle_event(Event::TitleUpdated {
            window_id: w,
            title: Some("pinned".to_string()),
        });

        assert_eq!(
            state.windows.get(&w).unwrap().title.as_deref(),
            Some("pinned"),
            "title not stored"
        );
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Floating {
                rect: crate::layout::Rect::new(480, 270, 960, 540)
            }),
            "TitleUpdated did not evaluate window rules"
        );
    }

    #[test]
    fn event_output_dimensions_updated_re_evaluates_window_rules() {
        let (mut state, o, w) = setup_handle_event_fixture();

        // Create the window and deliver the matching app_id. The rule also
        // requires a title, so the window is still pending metadata and the
        // float has NOT been applied yet.
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o,
        });
        state.handle_event(Event::AppIdUpdated {
            window_id: w,
            app_id: Some("floatme".to_string()),
        });
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Tiled),
            "precondition: window still tiled (title pending)"
        );

        // Resizing the output must re-run rules for that output's windows. With
        // the title still missing, the re-evaluation correctly keeps the window
        // deferred (not floated, not finalized) rather than skipping or
        // finalizing early.
        state.handle_event(Event::OutputDimensionsUpdated {
            output_id: o,
            w: 2560,
            h: 1440,
        });

        assert_eq!(
            state.outputs.get(&o).unwrap().rect(),
            Some(crate::layout::Rect::new(0, 0, 2560, 1440)),
            "output dimensions not updated by event"
        );
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Tiled),
            "resize re-evaluation must not float before the title arrives"
        );

        // Now the title arrives: the (re-evaluated) rule applies and the window
        // floats at the NEW output geometry, proving the resize re-ran the rules
        // against the updated output rect.
        state.handle_event(Event::TitleUpdated {
            window_id: w,
            title: Some("pinned".to_string()),
        });
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Floating {
                rect: crate::layout::Rect::new(640, 360, 1280, 720)
            }),
            "OutputDimensionsUpdated did not re-evaluate window rules"
        );
    }

    /// Build a `WMState` with one output (real geometry) holding one window, so
    /// fullscreen toggle events have a window to act on.
    fn setup_fullscreen_fixture() -> (WMState, OutputId, WindowId) {
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        let w = WindowId(1);
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o,
        });
        (state, o, w)
    }

    #[test]
    fn event_output_created_creates_output_and_handles_orphaned_trees() {
        // Default config binds to the `Primary` seat, so with no seats present
        // `reconcile_keybindings` produces no runtime bindings. Creating an
        // output must reconcile keybindings (which, with still no seats, stays
        // empty) without panicking and keeps bindings consistent.
        let mut state = WMState::new();
        assert_eq!(
            state.keybindings.len(),
            0,
            "precondition: no bindings without a seat"
        );

        state.handle_event(Event::OutputCreated {
            output_id: OutputId(1),
        });

        assert!(
            state.outputs.contains_key(&OutputId(1)),
            "output not created"
        );
        // Still no seats, so no bindings, but reconcile ran cleanly.
        assert_eq!(state.keybindings.len(), 0);
    }

    #[test]
    fn event_output_removed_reassigns_windows_and_falls_back_focus() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, x) in [(o1, 0), (o2, 1920)] {
            let mut out = Output::new();
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }

        // Two windows: one on the output we will remove (o1), one resident on
        // o2. `WindowCreated` focuses each new window, so the last created
        // (`resident` on o2) is the globally focused window.
        let moving = WindowId(1);
        let resident = WindowId(2);
        state.handle_event(Event::WindowCreated {
            window_id: moving,
            target_output: o1,
        });
        state.handle_event(Event::WindowCreated {
            window_id: resident,
            target_output: o2,
        });
        assert_eq!(state.focused_output, Some(o2), "precondition: o2 focused");
        assert_eq!(
            state.focused_window,
            Some(resident),
            "precondition: resident globally focused"
        );

        // Remove the focused output o1. Its window must be reassigned to o2,
        // the output map entry dropped, and focus must stay on the surviving
        // output's window (o2 / resident).
        state.handle_event(Event::OutputRemoved { output_id: o1 });

        assert!(
            !state.outputs.contains_key(&o1),
            "removed output still present"
        );
        assert!(
            !state.windows_for_output(o1).is_some_and(|s| !s.is_empty()),
            "windows not cleared from removed output index"
        );
        // The moved window now lives on the surviving output.
        assert_eq!(
            state.windows.get(&moving).unwrap().output_id,
            o2,
            "window not reassigned to surviving output"
        );
        assert!(
            state.windows_for_output(o2).map(|s| s.contains(&moving)) == Some(true),
            "reassigned window not indexed on surviving output"
        );
        // Focus stays on the surviving output's resident window.
        assert_eq!(state.focused_output, Some(o2), "focus did not fall back");
        assert_eq!(
            state.focused_window,
            Some(resident),
            "focus not handed to surviving output's window"
        );
        assert_eq!(
            state.focused_tree().unwrap().focused_window(),
            Some(resident.0),
            "layout focus diverged from global focus after output removal"
        );
        // The moved window survived (not destroyed) and is in o2's tree.
        assert!(
            state.windows.contains_key(&moving),
            "reassigned window was destroyed"
        );
        assert!(
            state
                .tree_for_output(o2)
                .unwrap()
                .visible_windows()
                .contains(&moving.0),
            "reassigned window missing from surviving output's tree"
        );
    }

    #[test]
    fn event_seat_created_triggers_reconcile_keybindings() {
        let mut state = WMState::new();
        assert_eq!(
            state.keybindings.len(),
            0,
            "precondition: no bindings before any seat exists"
        );

        // Creating the first (primary) seat must reconcile keybindings, which
        // recreates the default config's Primary-target bindings for that seat.
        state.handle_event(Event::SeatCreated { seat_id: SeatId(1) });

        assert!(state.seats.contains_key(&SeatId(1)), "seat not created");
        assert!(
            !state.keybindings.is_empty(),
            "SeatCreated did not reconcile bindings"
        );
        // Every reconciled binding belongs to the newly created seat.
        assert!(
            state.keybindings.values().all(|b| b.seat_id == SeatId(1)),
            "reconciled bindings assigned to the wrong seat"
        );
        assert_eq!(
            state.current_seat,
            Some(SeatId(1)),
            "first seat not current"
        );

        // A second seat with no Primary bindings should not duplicate the
        // Primary-target set, but reconcile must still run and keep bindings
        // consistent (still only seat 1's Primary bindings).
        state.handle_event(Event::SeatCreated { seat_id: SeatId(2) });
        assert!(
            state.keybindings.values().all(|b| b.seat_id == SeatId(1)),
            "non-primary seat wrongly received bindings"
        );
    }

    #[test]
    fn event_seat_removed_cleans_bindings_and_hands_off_focus() {
        let mut state = WMState::new();

        // Two seats; both get Primary-target bindings because Primary resolves
        // to the lowest seat id (seat 1 only), so reconcile yields seat-1
        // bindings. To exercise per-seat binding cleanup, drive the binding set
        // through a seat-targeted reconcile: create seat 1, then seat 2.
        state.handle_event(Event::SeatCreated { seat_id: SeatId(1) });
        let before = state.keybindings.len();
        assert!(before > 0, "precondition: bindings exist for seat 1");

        state.handle_event(Event::SeatCreated { seat_id: SeatId(2) });

        // Removing seat 1 (the current seat) must run reconcile again and hand
        // `current_seat` off to the surviving seat.
        state.handle_event(Event::SeatRemoved { seat_id: SeatId(1) });

        assert!(!state.seats.contains_key(&SeatId(1)), "seat not removed");
        assert_eq!(
            state.current_seat,
            Some(SeatId(2)),
            "focus not handed to surviving seat"
        );
        // Reconcile recreated only seat 2's (Primary-target) bindings; none
        // reference the removed seat.
        assert!(
            state.keybindings.values().all(|b| b.seat_id == SeatId(2)),
            "bindings for removed seat were not cleaned up"
        );
    }

    #[test]
    fn event_fullscreen_requested_toggles_window_state() {
        let (mut state, _o, w) = setup_fullscreen_fixture();

        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Tiled),
            "precondition: window starts tiled"
        );

        state.handle_event(Event::FullscreenRequested { window_id: w });

        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Fullscreen {
                restore: Box::new(crate::layout::WindowState::Tiled)
            }),
            "FullscreenRequested did not fullscreen the window"
        );
    }

    #[test]
    fn event_exit_fullscreen_requested_restores_window_state() {
        let (mut state, _o, w) = setup_fullscreen_fixture();

        // Enter fullscreen (preserving the tiled restore state), then exit.
        state.handle_event(Event::FullscreenRequested { window_id: w });
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Fullscreen {
                restore: Box::new(crate::layout::WindowState::Tiled)
            }),
            "precondition: window is fullscreen"
        );

        state.handle_event(Event::ExitFullscreenRequested { window_id: w });

        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Tiled),
            "ExitFullscreenRequested did not restore the window"
        );
    }

    /// Build a `WMState` with one output (real geometry) and a single window on
    /// it, returning `(state, output, window)`.
    fn setup_window_on_output() -> (WMState, OutputId, WindowId) {
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        let w = WindowId(1);
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o,
        });
        (state, o, w)
    }

    #[test]
    fn event_window_interaction_focuses_clicked_window() {
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        // Two windows; the last created (b) is globally focused.
        let a = WindowId(1);
        let b = WindowId(2);
        state.handle_event(Event::WindowCreated {
            window_id: a,
            target_output: o,
        });
        state.handle_event(Event::WindowCreated {
            window_id: b,
            target_output: o,
        });
        assert_eq!(state.focused_window, Some(b), "precondition: b focused");

        // Clicking a (which is not focused) must move focus to it across both
        // the global focus state and the layout tree.
        state.handle_event(Event::WindowInteraction { window_id: a });

        assert_eq!(state.focused_window, Some(a), "interaction did not focus a");
        assert_eq!(
            state.focused_tree().unwrap().focused_window(),
            Some(a.0),
            "layout focus diverged from global focus after interaction"
        );
        assert_eq!(state.pending_focus, Some(a), "focus not queued for manage");
        assert_eq!(state.focus_stack.first().copied(), Some(a));
    }

    #[test]
    fn event_decoration_hint_updated_sets_csd_ssd() {
        let (mut state, _o, w) = setup_window_on_output();

        assert_eq!(
            state.windows.get(&w).unwrap().decoration_hint,
            None,
            "precondition: no decoration hint yet"
        );

        // Hint 1 => client-side decorations.
        state.handle_event(Event::DecorationHintUpdated {
            window_id: w,
            hint: 1,
        });
        assert_eq!(
            state.windows.get(&w).unwrap().decoration_hint,
            Some(1),
            "decoration hint not stored"
        );
        assert!(
            state.windows.get(&w).unwrap().use_client_decorations(false),
            "hint 1 should prefer client-side decorations"
        );

        // Hint 0 => server-side decorations (overrides the false fallback).
        state.handle_event(Event::DecorationHintUpdated {
            window_id: w,
            hint: 0,
        });
        assert_eq!(
            state.windows.get(&w).unwrap().decoration_hint,
            Some(0),
            "decoration hint not updated"
        );
        assert!(
            !state.windows.get(&w).unwrap().use_client_decorations(true),
            "hint 0 should prefer server-side decorations"
        );
    }

    #[test]
    fn event_dimensions_hint_updated_records_preferred_size() {
        let (mut state, o, w) = setup_window_on_output();

        // No hint yet: preferred size is the ratio base.
        let out_rect = state.outputs.get(&o).unwrap().rect().unwrap();
        let (base_w, _base_h) = state
            .windows
            .get(&w)
            .unwrap()
            .preferred_dimensions(out_rect, state.default_float_ratio());
        assert_eq!(base_w, 960, "precondition: default size is ratio base");

        // Deliver a min-width/height hint; the event must store it and the
        // preferred size must clamp up to the hints (never shrink below the
        // ratio base).
        state.handle_event(Event::DimensionsHint {
            window_id: w,
            min_w: 1200,
            min_h: 700,
            max_w: 0,
            max_h: 0,
        });

        let hint = &state.windows.get(&w).unwrap().dimensions_hint;
        assert_eq!(
            (
                hint.min_width,
                hint.min_height,
                hint.max_width,
                hint.max_height
            ),
            (1200, 700, 0, 0),
            "dimensions hint not stored"
        );
        let (w_w, w_h) = state
            .windows
            .get(&w)
            .unwrap()
            .preferred_dimensions(out_rect, state.default_float_ratio());
        assert_eq!(
            (w_w, w_h),
            (1200, 700),
            "DimensionsHint did not influence preferred size"
        );
    }

    #[test]
    fn event_pid_updated_stores_process_id() {
        let (mut state, _o, w) = setup_window_on_output();

        assert_eq!(state.windows.get(&w).unwrap().pid, 0, "precondition: pid 0");
        state.handle_event(Event::PidUpdated {
            window_id: w,
            pid: 4242,
        });
        assert_eq!(
            state.windows.get(&w).unwrap().pid,
            4242,
            "PidUpdated did not store the pid"
        );
    }

    #[test]
    fn event_parent_updated_stores_parent() {
        let (mut state, _o, w) = setup_window_on_output();

        assert_eq!(
            state.windows.get(&w).unwrap().parent,
            None,
            "precondition: no parent"
        );
        state.handle_event(Event::ParentUpdated {
            window_id: w,
            parent_id: Some(WindowId(2)),
        });
        assert_eq!(
            state.windows.get(&w).unwrap().parent,
            Some(WindowId(2)),
            "ParentUpdated did not store the parent"
        );
    }

    #[test]
    fn event_seat_name_updated_stores_name() {
        let mut state = WMState::new();
        state.handle_event(Event::SeatCreated { seat_id: SeatId(1) });
        assert_eq!(
            state.seats.get(&SeatId(1)).unwrap().wl_seat_name,
            0,
            "precondition: seat name 0"
        );

        state.handle_event(Event::SeatNameUpdated {
            seat_id: SeatId(1),
            name: 7,
        });
        assert_eq!(
            state.seats.get(&SeatId(1)).unwrap().wl_seat_name,
            7,
            "SeatNameUpdated did not store the name"
        );
    }

    #[test]
    fn event_seat_pointer_position_updated_stores_cursor() {
        let mut state = WMState::new();
        state.handle_event(Event::SeatCreated { seat_id: SeatId(1) });
        assert_eq!(
            state.seats.get(&SeatId(1)).unwrap().pointer_position,
            None,
            "precondition: no pointer position"
        );

        state.handle_event(Event::SeatPointerPositionUpdated {
            seat_id: SeatId(1),
            x: 123,
            y: 456,
        });
        assert_eq!(
            state.seats.get(&SeatId(1)).unwrap().pointer_position,
            Some((123, 456)),
            "SeatPointerPositionUpdated did not store the cursor position"
        );
    }

    #[test]
    fn event_output_position_updated_moves_output_and_reassigns() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, x) in [(o1, 0), (o2, 1920)] {
            let mut out = Output::new();
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }
        state.focused_output = Some(o1);

        // A window on o1, floated so its absolute position is meaningful.
        let w = WindowId(1);
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o1,
        });
        state.tree_for_output(o1).unwrap().set_window_state(
            w.0,
            crate::layout::WindowState::Floating {
                rect: crate::layout::Rect::new(100, 50, 300, 200),
            },
        );

        // Move o1 by (+500, +40). The output position must update and the window
        // must stay on o1 with its floating rect preserved.
        state.handle_event(Event::OutputPositionUpdated {
            output_id: o1,
            x: 500,
            y: 40,
        });

        assert_eq!(
            state.outputs.get(&o1).unwrap().position,
            Some((500, 40)),
            "output position not updated"
        );
        assert_eq!(
            state.windows.get(&w).unwrap().output_id,
            o1,
            "window must stay on its output after position update"
        );
        // The floating rect keeps its window-local coordinates since the
        // window stayed on the same output.
        assert_eq!(
            state.window_state_for_id(w),
            Some(&crate::layout::WindowState::Floating {
                rect: crate::layout::Rect::new(100, 50, 300, 200)
            }),
            "floating rect not preserved across output position update"
        );
    }

    #[test]
    fn event_seat_layer_shell_focus_updates_mode_and_requeues_focus() {
        let mut state = WMState::new();
        state.handle_event(Event::SeatCreated { seat_id: SeatId(1) });

        // A focused window on an output so the None-mode requeue branch is
        // observable.
        let o1 = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o1, out);
        state.focused_output = Some(o1);
        let w = WindowId(1);
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o1,
        });
        assert_eq!(
            state.focused_window,
            Some(w),
            "precondition: window focused"
        );

        // Exclusive mode: only the seat's layer-shell focus mode is stored; no
        // re-queue of pending focus.
        state.handle_event(Event::SeatLayerShellFocus {
            seat_id: SeatId(1),
            mode: crate::state::seat::LayerShellFocus::Exclusive,
        });
        assert_eq!(
            state.seats.get(&SeatId(1)).unwrap().layer_shell_focus,
            crate::state::seat::LayerShellFocus::Exclusive,
            "layer-shell focus mode not stored"
        );

        // Clear pending focus, then switch to None: River handed focus back to
        // the WM, so the current focus must be re-queued.
        state.pending_focus = None;
        state.handle_event(Event::SeatLayerShellFocus {
            seat_id: SeatId(1),
            mode: crate::state::seat::LayerShellFocus::None,
        });
        assert_eq!(
            state.seats.get(&SeatId(1)).unwrap().layer_shell_focus,
            crate::state::seat::LayerShellFocus::None,
            "layer-shell focus mode not cleared"
        );
        assert_eq!(
            state.pending_focus,
            Some(w),
            "None mode did not re-queue the current focus"
        );
    }

    #[test]
    fn event_output_name_updated_stores_name() {
        let mut state = WMState::new();
        state.handle_event(Event::OutputCreated {
            output_id: OutputId(1),
        });
        assert_eq!(
            state.outputs.get(&OutputId(1)).unwrap().wl_output_name,
            0,
            "precondition: output name 0"
        );

        state.handle_event(Event::OutputNameUpdated {
            output_id: OutputId(1),
            name: 7,
        });
        assert_eq!(
            state.outputs.get(&OutputId(1)).unwrap().wl_output_name,
            7,
            "OutputNameUpdated did not store the name"
        );
    }

    #[test]
    fn fullscreen_scene_uses_full_output_rect_with_layer_shell_zone() {
        // When a layer-shell surface reserves an exclusive zone
        // (e.g. a top bar), the scene must record the full output rect for
        // fullscreen windows, not the shrunken tiling rect. This ensures
        // exiting fullscreen correctly detects a rect change.
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        out.set_non_exclusive_area(0, 40, 1920, 1040);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        let w = WindowId(1);
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o,
        });

        // Arrange the tree so node.rect is up-to-date before reading the scene.
        if let Some(rect) = state.outputs.get(&o).and_then(|o| o.tiling_rect()) {
            state.tree_for_output(o).unwrap().set_output_rect(rect);
        }

        // Tiled: scene rect should be the tiling area (shrunken by bar).
        let scene = state.desired_scene();
        let tiled_entry = scene.iter().find(|e| e.window_id == w).unwrap();
        assert_eq!(
            tiled_entry.rect,
            crate::layout::Rect::new(0, 40, 1920, 1040),
            "tiled window should use tiling rect"
        );

        // Enter fullscreen.
        state.tree_for_output(o).unwrap().toggle_fullscreen(w.0);
        if let Some(rect) = state.outputs.get(&o).and_then(|o| o.tiling_rect()) {
            state.tree_for_output(o).unwrap().set_output_rect(rect);
        }
        let scene = state.desired_scene();
        let fs_entry = scene.iter().find(|e| e.window_id == w).unwrap();
        assert_eq!(
            fs_entry.rect,
            crate::layout::Rect::new(0, 0, 1920, 1080),
            "fullscreen window should use full output rect"
        );

        // Exit fullscreen.
        state.tree_for_output(o).unwrap().toggle_fullscreen(w.0);
        if let Some(rect) = state.outputs.get(&o).and_then(|o| o.tiling_rect()) {
            state.tree_for_output(o).unwrap().set_output_rect(rect);
        }
        let scene = state.desired_scene();
        let restored_entry = scene.iter().find(|e| e.window_id == w).unwrap();
        assert_eq!(
            restored_entry.rect,
            crate::layout::Rect::new(0, 40, 1920, 1040),
            "restored tiled window should use tiling rect"
        );
    }

    /// Build a `WMState` with one output (real geometry) holding one tiled
    /// window, a seat, and an arranged layout rect for the window. Returns
    /// `(state, output, window, seat)`.
    fn setup_pointer_op_fixture() -> (WMState, OutputId, WindowId, SeatId) {
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        let w = WindowId(1);
        state.handle_event(Event::WindowCreated {
            window_id: w,
            target_output: o,
        });
        state.ensure_tree_for_output(o).arrange();

        let s = SeatId(1);
        state.handle_event(Event::SeatCreated { seat_id: s });
        (state, o, w, s)
    }

    #[test]
    fn event_pointer_move_requested_starts_pending_op() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        assert!(!state.seat_op_active(s), "precondition: no op yet");
        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });

        assert!(state.seat_op_active(s), "move op should be active");
        let seat = state.seats.get(&s).unwrap();
        assert!(
            !seat.op_started,
            "op must start in the next manage sequence"
        );
        match &seat.op {
            InteractiveOp::Move {
                window_id,
                initial_rect,
            } => {
                assert_eq!(*window_id, w);
                // The initial rect is the window's current arranged rect.
                assert_eq!(initial_rect.width, 1920);
            }
            _ => panic!("expected a Move op"),
        }
    }

    #[test]
    fn window_closed_during_pending_op_cancels_seat_op() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        // A pointer move is requested but not yet started (op_start_pointer is
        // deferred to the next manage sequence).
        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });
        assert!(state.seat_op_active(s), "precondition: op is pending");

        // Closing the drag-target window must cancel the seat's op so a later
        // manage does not emit StartPointerOp for a destroyed window, and so the
        // op does not leak and block future drags.
        state.handle_event(Event::WindowClosed { window_id: w });

        assert!(
            !state.seat_op_active(s),
            "closing the drag target must cancel the seat's interactive op"
        );
    }

    #[test]
    fn window_closed_during_started_op_cancels_seat_op() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });
        // Advance the op into its started state (op_start_pointer sent).
        let _ = state.transition_pointer_ops();

        state.handle_event(Event::WindowClosed { window_id: w });

        assert!(
            !state.seat_op_active(s),
            "closing the drag target must cancel an already-started op"
        );
    }

    #[test]
    fn op_delta_move_translates_initial_rect() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();
        // A smaller floating window has room to move within the output.
        state
            .ensure_tree_for_output(OutputId(1))
            .toggle_floating(w.0, Rect::new(100, 50, 800, 600));

        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });
        // A move delta of (100, 50) shifts the whole rect by that amount.
        let op_rect = state.compute_op_rect(s, 100, 50).expect("op rect computed");
        assert_eq!(op_rect.window_id, w);
        assert_eq!(op_rect.rect.x, 200, "move should translate x by dx");
        assert_eq!(op_rect.rect.y, 100, "move should translate y by dy");
        assert_eq!(op_rect.rect.width, 800, "move preserves size");
    }

    #[test]
    fn op_delta_move_clamps_to_output() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();
        state
            .ensure_tree_for_output(OutputId(1))
            .toggle_floating(w.0, Rect::new(100, 50, 800, 600));

        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });
        // A huge delta must not produce coordinates outside the output bounds.
        let op_rect = state
            .compute_op_rect(s, 9000, 9000)
            .expect("op rect computed");
        assert_eq!(op_rect.rect.width, 800);
        assert!(op_rect.rect.x >= 0, "x must stay within output");
        assert!(op_rect.rect.y >= 0, "y must stay within output");
        let max_x = 1920 - 800;
        let max_y = 1080 - 600;
        assert_eq!(op_rect.rect.x, max_x, "x clamps to right edge");
        assert_eq!(op_rect.rect.y, max_y, "y clamps to bottom edge");
    }

    #[test]
    fn op_delta_resize_right_edge_grows_width() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        state.handle_event(Event::PointerResizeRequested {
            window_id: w,
            seat_id: s,
            // right edge only (bit 8).
            edges: 0b1000,
        });
        // dx=200 grows the width by 200, keeping x/y fixed.
        let op_rect = state.compute_op_rect(s, 200, 0).expect("op rect computed");
        assert_eq!(op_rect.rect.x, 0);
        assert_eq!(op_rect.rect.width, 1920 + 200, "right edge grows width");
    }

    #[test]
    fn op_delta_resize_left_edge_shrinks_from_left() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        state.handle_event(Event::PointerResizeRequested {
            window_id: w,
            seat_id: s,
            // left edge only (bit 4).
            edges: 0b0100,
        });
        // dx=200 moves the left edge right by 200, shrinking width by 200.
        let op_rect = state.compute_op_rect(s, 200, 0).expect("op rect computed");
        assert_eq!(op_rect.rect.x, 200, "left edge moves right by dx");
        assert_eq!(op_rect.rect.width, 1920 - 200, "left edge shrinks width");
    }

    #[test]
    fn op_delta_resize_clamps_to_dimensions_hint() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        // Constrain the window to a minimum width of 1500.
        state.handle_event(Event::DimensionsHint {
            window_id: w,
            min_w: 1500,
            min_h: 0,
            max_w: 0,
            max_h: 0,
        });
        state.handle_event(Event::PointerResizeRequested {
            window_id: w,
            seat_id: s,
            edges: 0b0100, // left edge, which would shrink width below the min
        });
        // dx=1000 would shrink width to 920; the hint floors it at 1500 and
        // re-anchors the left edge so the right edge stays put.
        let op_rect = state.compute_op_rect(s, 1000, 0).expect("op rect computed");
        assert_eq!(op_rect.rect.width, 1500, "width clamped to min hint");
        assert_eq!(
            op_rect.rect.x,
            1920 - 1500,
            "left edge re-anchored to keep right fixed"
        );
    }

    #[test]
    fn op_release_flags_op_for_ending() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });
        state.handle_event(Event::OpRelease { seat_id: s });

        let seat = state.seats.get(&s).unwrap();
        assert!(seat.op_ending, "release must schedule op_end");
        // The op stays active so trailing deltas still apply.
        assert!(state.seat_op_active(s), "op stays active until op_end");
    }

    #[test]
    fn pointer_op_ignored_while_another_active() {
        let (mut state, _o, w, s) = setup_pointer_op_fixture();

        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });
        // A second move request while the first is active must be ignored.
        state.handle_event(Event::PointerMoveRequested {
            window_id: w,
            seat_id: s,
        });
        let seat = state.seats.get(&s).unwrap();
        match &seat.op {
            InteractiveOp::Move { initial_rect, .. } => {
                assert_eq!(
                    initial_rect.width, 1920,
                    "initial rect unchanged by duplicate request"
                );
            }
            _ => panic!("expected Move op to be retained"),
        }
    }

    #[test]
    fn window_clamp_dimensions_respects_hints() {
        let mut window = Window::new(WindowId(1), OutputId(1));
        window.set_dimensions_hint(200, 100, 800, 600);

        // Below min -> clamped up.
        assert_eq!(window.clamp_dimensions(50, 50), (200, 100));
        // Within range -> unchanged.
        assert_eq!(window.clamp_dimensions(400, 300), (400, 300));
        // Above max -> clamped down.
        assert_eq!(window.clamp_dimensions(1000, 1000), (800, 600));
        // Zero hint on an axis means unconstrained on that axis.
        window.set_dimensions_hint(0, 0, 0, 0);
        assert_eq!(window.clamp_dimensions(123, 456), (123, 456));
    }
}
