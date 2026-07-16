//! Core runtime state for Fenestre.
//!
//! Owns all mutable compositor state: River protocol proxies, windows, outputs,
//! seats, runtime keybindings, focus state, configuration, layout trees, and
//! pending River-managed state changes. Scene-related fields (`last_manage_scene`,
//! `last_render_scene`, `last_layer_shell_default`, `render_order_cache`) and
//! the declarative reconciler (`desired_scene`, `apply_manage`, `apply_render`)
//! are defined in `scene.rs` as an extension impl on `WMState`.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use super::events::Event;
use super::keybindings::{KeyBinding, XkbBindingId};
use super::output::{Output, OutputId};
use super::scene::SceneSnapshot;
use super::seat::{Seat, SeatId};
use super::window::{Window, WindowId};
use crate::config::Config;
use crate::layout::{LayoutTree, Rect, WindowState};

/// Owns all mutable compositor state for the window manager.
///
/// `WMState` stores River protocol proxies, windows, outputs, seats,
/// runtime keybindings, focus state, configuration, layout trees, and pending
/// River-managed state changes. The scene snapshot fields (`last_manage_scene`,
/// `last_render_scene`, `last_layer_shell_default`, `render_order_cache`) are
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
    pub(super) output_trees: HashMap<OutputId, LayoutTree>,

    pub(super) focused_window: Option<WindowId>,
    pub(super) focused_output: Option<OutputId>,
    pub(super) focus_stack: Vec<WindowId>,

    pub(super) current_seat: Option<SeatId>,

    /// True when desired xkb bindings differ from configured River binding objects.
    pub(super) xkb_bindings_dirty: bool,
    /// River xkb binding protocol objects queued for destruction during the next manage sequence.
    pub(super) pending_xkb_binding_destroys: Vec<crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_binding_v1::RiverXkbBindingV1>,
    /// Window rules loaded from config, applied per-window when metadata is known.
    pub(super) window_rules: Option<super::rule::WindowRules>,
    /// Window ID to focus during the next manage sequence.
    pub(super) pending_focus: Option<WindowId>,
    /// Window IDs to close during the next manage sequence.
    pub(super) pending_closes: Vec<WindowId>,

    /// Cached sorted window IDs for render stacking order.
    ///
    /// Owned by the scene module — mutate via `apply_manage` / `apply_render`.
    pub(super) render_order_cache: Vec<WindowId>,

    /// Snapshot of the last desired scene from a manage cycle, used to diff
    /// manage-phase effects (dimensions, fullscreen, server-side decorations).
    ///
    /// Owned by the scene module — mutate via `apply_manage` / `apply_render`.
    pub(super) last_manage_scene: SceneSnapshot,
    /// Output last announced as the layer-shell default via `set_default`, used
    /// to emit that effect only on change (it is global across outputs).
    ///
    /// Owned by the scene module — mutate via `apply_manage` / `apply_render`.
    pub(super) last_layer_shell_default: Option<OutputId>,
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
            last_manage_scene: Vec::new(),
            last_layer_shell_default: None,
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
                self.ensure_tree_for_output(target_output)
                    .insert_window(window_id.0);
                self.push_focus(window_id);
                self.pending_focus = Some(window_id);
                self.request_manage_dirty();
            }
            Event::OutputCreated { output_id } => {
                let output = Output::new(output_id);
                self.outputs.insert(output_id, output);

                let orphaned: Vec<OutputId> = self
                    .output_trees
                    .keys()
                    .filter(|id| !self.outputs.contains_key(id))
                    .copied()
                    .collect();
                for orphaned_id in orphaned {
                    if self.focused_output == Some(orphaned_id) {
                        self.focused_output = Some(output_id);
                    }
                    self.reassign_output(orphaned_id, output_id);
                }

                if self.focused_output.is_none() {
                    self.focused_output = Some(output_id);
                }
            }
            Event::SeatCreated { seat_id } => {
                let seat = Seat::new(seat_id);
                self.seats.insert(seat_id, seat);
                if self.current_seat.is_none() {
                    self.current_seat = Some(seat_id);
                }
                self.reconcile_keybindings();
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
                    self.request_manage_dirty();
                }
            }
            Event::ExitFullscreenRequested { window_id } => {
                if let Some((_, tree)) = self.tree_for_window_id(window_id)
                    && tree.toggle_fullscreen(window_id.0)
                {
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
                if mode == super::seat::LayerShellFocus::None
                    && let Some(window_id) = self.focused_window
                {
                    self.pending_focus = Some(window_id);
                }
                self.request_manage_dirty();
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

    /// Build a state with one pseudo-tiled window that has round-tripped through
    /// fullscreen (app reported fullscreen dimensions). Returns the state, the
    /// output id, the window id, and the original pseudo-tiled rect.
    fn setup_pseudo_fullscreen_roundtrip() -> (WMState, OutputId, WindowId, Rect) {
        let mut state = WMState::new();
        let o = OutputId(1);
        let mut out = Output::new(o);
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o, out);
        state.focused_output = Some(o);

        let w = WindowId(1);
        state.windows.insert(w, Window::new(w, o));
        state.tree_for_output(o).unwrap().insert_window(w.0);
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
}
