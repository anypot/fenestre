//! Scene snapshot & manage/render logic.
//!
//! Owns the scene computation and the manage/render effect sequences for
//! `WMState`. The four scene-related fields on `WMState` are owned by this
//! module — mutate them only through `apply_manage` or `apply_render`.
use std::collections::HashMap;

use super::WMState;
use super::effects::Effect;
use super::output::OutputId;
use super::seat::{InteractiveOp, SeatId};
use super::window::WindowId;
use crate::layout::{Rect, SplitDirection, WindowState};
use wayland_client::QueueHandle;

/// A single window's entry in a desired scene snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SceneEntry {
    pub(crate) window_id: WindowId,
    pub(crate) output_id: OutputId,
    pub(crate) rect: Rect,
    pub(crate) state: WindowState,
    pub(crate) z: (u8, u8, u32),
    pub(crate) border: Option<(u32, i32, u32, u32, u32, u32)>,
}

pub(super) type SceneSnapshot = Vec<SceneEntry>;

impl WMState {
    /// Compute the desired scene as a read-only function of current state.
    ///
    /// Each output tree must already be arranged: the manage phase calls
    /// `LayoutTree::arranged_windows`, which populates node geometry from
    /// `output_rect`. This function only reads that geometry plus per-window
    /// decoration and focus state, snapshots each window's intended rect, state,
    /// z-priority, and border appearance, and returns the full scene. It
    /// performs no mutation. The manage and render callers each diff this
    /// against their own last snapshot (`last_manage_scene` /
    /// `last_render_scene`) to produce protocol effects.
    pub(super) fn desired_scene(&self) -> SceneSnapshot {
        let mut scene = Vec::new();
        let decorations = self.config.as_ref().map(|c| c.decorations).unwrap_or(true);
        let border_width = self
            .config
            .as_ref()
            .and_then(|c| c.border_width)
            .unwrap_or(0);
        let (rgba_focused, rgba_unfocused) = self
            .config
            .as_ref()
            .map(|c| c.border_rgba())
            .unwrap_or(((0xff, 0xff, 0xff, 0xff), (0xff, 0xff, 0xff, 0xff)));

        for (output_id, tree) in self.output_trees.iter() {
            if !self.outputs.contains_key(output_id) {
                continue;
            }
            let arranged = tree.arranged_windows_readonly();
            let full_rect = self.outputs.get(output_id).and_then(|o| o.rect());
            for (window_id, window_rect, state) in arranged {
                let window_id = WindowId(window_id);
                let Some(window) = self.windows.get(&window_id) else {
                    continue;
                };

                // Fullscreen windows cover the whole physical output, so their
                // scene rect must be the full output rect, not the tiling rect
                // (which excludes layer-shell exclusive zones). This ensures
                // exiting fullscreen correctly detects a rect change and emits
                // ProposeDimensions / SetPosition to restore the tiled geometry.
                let rect = if matches!(state, WindowState::Fullscreen { .. }) {
                    full_rect.unwrap_or(window_rect)
                } else {
                    window_rect
                };

                let mode_priority = mode_priority(&state);
                let focus_priority = if self.focused_window == Some(window_id) {
                    1
                } else {
                    0
                };
                let z = (mode_priority, focus_priority, window_id.0);

                let use_decor = window.use_client_decorations(decorations);
                let border = if use_decor {
                    None
                } else {
                    let focused_argb = self
                        .config
                        .as_ref()
                        .and_then(|c| c.border_color_focused)
                        .unwrap_or(0xffffffff);
                    let (rgba, edges, width) = if self.focused_window == Some(window_id)
                        && let Some(direction) = self.pending_split
                    {
                        let preview_color = self
                            .config
                            .as_ref()
                            .and_then(|c| c.layout.preview_border_color)
                            .unwrap_or(focused_argb);
                        let preview_width = self
                            .config
                            .as_ref()
                            .and_then(|c| c.layout.preview_border_width)
                            .unwrap_or(border_width);
                        let rgba = crate::config::argb_to_rgba(preview_color);
                        let edge = preview_edge(direction);
                        (rgba, edge, preview_width)
                    } else if self.focused_window == Some(window_id) {
                        (rgba_focused, super::effects::ALL_EDGES, border_width)
                    } else {
                        (rgba_unfocused, super::effects::ALL_EDGES, border_width)
                    };
                    Some((edges, width, rgba.0, rgba.1, rgba.2, rgba.3))
                };

                scene.push(SceneEntry {
                    window_id,
                    output_id: *output_id,
                    rect,
                    state,
                    z,
                    border,
                });
            }
        }

        scene
    }

    /// Apply pending BSP layout and window-management requests in a manage sequence.
    pub(super) fn apply_manage(&mut self, _qh: &QueueHandle<Self>) -> Vec<Effect> {
        self.ensure_focused_output();

        // Drive the interactive pointer-operation state machine. Pending ops are
        // started here (the `op_start_pointer` call may only be made inside a
        // manage sequence), and ops flagged for ending are ended via `op_end`.
        let mut effects = self.transition_pointer_ops();

        // Apply any floating rects pending from `op_delta` events to the layout
        // trees exactly once per manage sequence (rather than mutating the tree on
        // every pointer-motion delta). Window closures use `set_window_state`,
        // which rebuilds tiling flags, so we only touch each affected tree here.
        for (output_id, tree) in self.output_trees.iter_mut() {
            let mut updated = false;
            for seat in self.seats.values_mut() {
                // Only consume the pending float once we know this output owns
                // the affected window; `take()` is evaluated before the
                // membership guard, so checking first avoids silently dropping
                // floats whose window lives on a later-iterated output.
                let belongs = seat.pending_float.as_ref().is_some_and(|(wid, _)| {
                    self.windows
                        .get(wid)
                        .is_some_and(|w| w.output_id == *output_id)
                });
                if belongs && let Some((window_id, rect)) = seat.pending_float.take() {
                    tree.set_window_state(window_id.0, WindowState::Floating { rect });
                    updated = true;
                }
            }
            if updated {
                self.render_order_cache.clear();
            }
        }

        for (output_id, tree) in self.output_trees.iter_mut() {
            let Some(output) = self.outputs.get(output_id) else {
                continue;
            };
            if let Some(rect) = output.tiling_rect() {
                tree.set_output_rect(rect);
            }
            for (window_id, window_rect, _state) in tree.arranged_windows() {
                if let Some(window) = self.windows.get_mut(&WindowId(window_id)) {
                    window.set_layout_rect(window_rect);
                }
            }
        }

        let desired = self.desired_scene();

        let last_map: HashMap<WindowId, &SceneEntry> = self
            .last_manage_scene
            .iter()
            .map(|e| (e.window_id, e))
            .collect();

        for entry in &desired {
            let Some(last) = last_map.get(&entry.window_id) else {
                match entry.state {
                    WindowState::Fullscreen { .. } => {
                        effects.push(Effect::Fullscreen {
                            window_id: entry.window_id,
                            output_id: entry.output_id,
                        });
                    }
                    _ => {
                        if matches!(
                            entry.state,
                            WindowState::Tiled
                                | WindowState::Floating { .. }
                                | WindowState::PseudoTiled { .. }
                        ) {
                            effects.push(Effect::ProposeDimensions {
                                window_id: entry.window_id,
                                width: entry.rect.width,
                                height: entry.rect.height,
                            });
                        }
                    }
                }
                if entry.border.is_some() {
                    effects.push(Effect::UseSsd {
                        window_id: entry.window_id,
                    });
                }
                continue;
            };

            if last.state != entry.state {
                match entry.state {
                    WindowState::Fullscreen { .. } => {
                        effects.push(Effect::Fullscreen {
                            window_id: entry.window_id,
                            output_id: entry.output_id,
                        });
                    }
                    _ => {
                        if matches!(last.state, WindowState::Fullscreen { .. }) {
                            effects.push(Effect::ExitFullscreen {
                                window_id: entry.window_id,
                            });
                        }
                    }
                }
            }

            if last.rect != entry.rect
                && matches!(
                    entry.state,
                    WindowState::Tiled
                        | WindowState::Floating { .. }
                        | WindowState::PseudoTiled { .. }
                )
            {
                effects.push(Effect::ProposeDimensions {
                    window_id: entry.window_id,
                    width: entry.rect.width,
                    height: entry.rect.height,
                });
            }

            if last.border.is_none() && entry.border.is_some() {
                effects.push(Effect::UseSsd {
                    window_id: entry.window_id,
                });
            }
        }

        if let Some(window_id) = self.pending_focus {
            if !self.windows.contains_key(&window_id) {
                self.pending_focus = None;
            } else if let Some(seat_id) = self.current_seat
                && let Some(seat) = self.seats.get(&seat_id)
                && let Some(window) = self.windows.get(&window_id)
                && seat.river_seat.is_some()
                && window.river_window.is_some()
            {
                self.pending_focus = None;
                effects.push(Effect::FocusWindow { window_id });
            }
        }

        let pending_closes = std::mem::take(&mut self.pending_closes);
        for window_id in pending_closes {
            if self.windows.contains_key(&window_id) {
                effects.push(Effect::Close { window_id });
            }
        }

        if self.layer_shell_default_dirty
            && let Some(default_output) = self.focused_output
            && self.layer_shell.is_some()
            && self
                .outputs
                .get(&default_output)
                .is_some_and(|o| o.river_layer_shell_output.is_some())
        {
            effects.push(Effect::SetLayerShellDefault {
                output_id: default_output,
            });
        }
        self.layer_shell_default_dirty = false;

        self.last_manage_scene = desired;
        self.render_order_cache.clear();
        effects
    }

    /// Emit effects that advance each seat's interactive pointer operation.
    ///
    /// A pending operation (recorded by `PointerMoveRequested` /
    /// `PointerResizeRequested`) is started here via `StartPointerOp`, which
    /// maps to River's `op_start_pointer` — a manage-sequence-only call. A
    /// resize op also emits `InformResizeStart`. An operation flagged by
    /// `OpRelease` is ended via `EndPointerOp` (`op_end`), followed by
    /// `InformResizeEnd` for resizes.
    pub(super) fn transition_pointer_ops(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();

        // First pass: read pending/active state and emit protocol effects. This
        // only borrows `self.seats` immutably, so we can iterate the keys
        // directly without snapshotting them into a Vec first.
        let mut pending: Vec<(SeatId, bool, bool)> = Vec::new();
        for seat_id in self.seats.keys().copied() {
            let (needs_start, needs_end, is_resize) = {
                let Some(seat) = self.seats.get(&seat_id) else {
                    continue;
                };
                (
                    seat.op.is_active() && !seat.op_started,
                    seat.op_ending,
                    matches!(seat.op, InteractiveOp::Resize { .. }),
                )
            };

            if needs_start {
                effects.push(Effect::StartPointerOp { seat_id });
                if is_resize
                    && let Some(window_id) = self.seats.get(&seat_id).and_then(|s| s.op.window_id())
                {
                    effects.push(Effect::InformResizeStart { window_id });
                }
            }

            if needs_end {
                effects.push(Effect::EndPointerOp { seat_id });
                if is_resize
                    && let Some(window_id) = self.seats.get(&seat_id).and_then(|s| s.op.window_id())
                {
                    effects.push(Effect::InformResizeEnd { window_id });
                }
            }

            if needs_start || needs_end {
                pending.push((seat_id, needs_start, needs_end));
            }
        }

        // Second pass: apply the seat-state mutations now that the immutable
        // iteration over `self.seats` has ended.
        for (seat_id, needs_start, needs_end) in pending {
            if needs_start && let Some(seat) = self.seats.get_mut(&seat_id) {
                seat.op_started = true;
            }
            if needs_end && let Some(seat) = self.seats.get_mut(&seat_id) {
                seat.op = InteractiveOp::Inactive;
                seat.op_started = false;
                seat.op_ending = false;
            }
        }

        effects
    }

    /// Apply pending render-state requests in a render sequence.
    pub(super) fn apply_render(&mut self) -> Vec<Effect> {
        if self.render_order_cache.is_empty() {
            self.update_render_order_cache();
        }

        let desired = self.desired_scene();
        let mut effects = Vec::new();

        let last_map: HashMap<WindowId, &SceneEntry> = self
            .last_render_scene
            .iter()
            .map(|e| (e.window_id, e))
            .collect();
        let desired_map: HashMap<WindowId, &SceneEntry> =
            desired.iter().map(|e| (e.window_id, e)).collect();

        for window_id in &self.render_order_cache {
            let Some(window) = self.windows.get_mut(window_id) else {
                continue;
            };
            if window.river_window.is_none() {
                continue;
            }

            if window.node.is_none() {
                effects.push(Effect::EnsureNode {
                    window_id: *window_id,
                });
            }

            let Some(entry) = desired_map.get(window_id) else {
                continue;
            };

            let Some(last) = last_map.get(window_id) else {
                effects.push(Effect::SetPosition {
                    window_id: *window_id,
                    x: entry.rect.x,
                    y: entry.rect.y,
                });
                if matches!(
                    entry.state,
                    WindowState::Floating { .. } | WindowState::Fullscreen { .. }
                ) {
                    effects.push(Effect::PlaceTop {
                        window_id: *window_id,
                    });
                }
                if let Some((edges, width, r, g, b, a)) = entry.border {
                    effects.push(Effect::SetBorders {
                        window_id: *window_id,
                        edges,
                        width,
                        r,
                        g,
                        b,
                        a,
                    });
                }
                continue;
            };

            if let (Some(_node), true) = (window.node.as_ref(), last.rect != entry.rect) {
                effects.push(Effect::SetPosition {
                    window_id: *window_id,
                    x: entry.rect.x,
                    y: entry.rect.y,
                });
            }

            // Only re-stack floating/fullscreen windows on a z-order *promotion*.
            // A demotion (e.g. the previously focused floating window losing
            // focus) also satisfies `last.z != entry.z`, but emitting `PlaceTop`
            // for it would fight the newly focused window for top-of-stack: since
            // `PlaceTop` effects are applied in cache order, a demoted window
            // processed after the promoted one would wrongly end up on top.
            if entry.z > last.z
                && matches!(
                    entry.state,
                    WindowState::Floating { .. } | WindowState::Fullscreen { .. }
                )
            {
                effects.push(Effect::PlaceTop {
                    window_id: *window_id,
                });
            }

            if last.border != entry.border
                && let Some((edges, width, r, g, b, a)) = entry.border
            {
                effects.push(Effect::SetBorders {
                    window_id: *window_id,
                    edges,
                    width,
                    r,
                    g,
                    b,
                    a,
                });
            }
        }

        self.last_render_scene = desired;
        effects
    }

    /// Rebuild the cached render order based on current window states.
    fn update_render_order_cache(&mut self) {
        self.render_order_cache.clear();
        self.render_order_cache.extend(self.windows.keys().copied());

        let mut priorities = Vec::new();
        for id in &self.render_order_cache {
            let state = self.window_state_for_id(*id);
            priorities.push((*id, render_stack_priority(state, self.focused_window, *id)));
        }
        priorities.sort_unstable_by_key(|(_, p)| *p);
        self.render_order_cache = priorities.into_iter().map(|(id, _)| id).collect();
    }
}

/// Map a window's layout state to its z-order mode priority.
///
/// Tiled / PseudoTiled sit at the bottom (0), floating above (1), and
/// fullscreen on top (2). Shared by `desired_scene` and
/// `render_stack_priority` so the priority rules live in exactly one place.
fn mode_priority(state: &WindowState) -> u8 {
    match state {
        WindowState::Tiled | WindowState::PseudoTiled { .. } => 0,
        WindowState::Floating { .. } => 1,
        WindowState::Fullscreen { .. } => 2,
    }
}

/// Map a split direction to the corresponding single edge bitmask for River's `set_borders`.
fn preview_edge(direction: SplitDirection) -> u32 {
    match direction {
        SplitDirection::Right => 0b1000,
        SplitDirection::Left => 0b0100,
        SplitDirection::Down => 0b0010,
        SplitDirection::Up => 0b0001,
    }
}

/// Compute the render stack priority for a window.
/// Returns a tuple of (mode_priority, focus_priority, window_id) for deterministic z-ordering.
fn render_stack_priority(
    state: Option<&WindowState>,
    focused_window: Option<super::window::WindowId>,
    window_id: super::window::WindowId,
) -> (u8, u8, u32) {
    let mode_priority = state.map(mode_priority).unwrap_or(0);
    let focus_priority = if focused_window == Some(window_id) {
        1
    } else {
        0
    };

    (mode_priority, focus_priority, window_id.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::state::effects::ALL_EDGES;
    use crate::state::events::Event;
    use crate::state::output::Output;
    use crate::state::window::Window;

    #[test]
    fn pending_split_gives_focused_window_preview_border() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());
        state.focused_output = Some(o1);
        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        state.output_trees.insert(
            o1,
            crate::layout::LayoutTree::new(crate::layout::Rect::new(0, 0, 1000, 100)),
        );
        state.tree_for_output(o1).unwrap().insert_window(1, None);
        state.focus_window_id(w1);
        state.pending_split = Some(crate::layout::SplitDirection::Right);
        state.config = Some(Config {
            layout: crate::config::LayoutConfig {
                preview_border_color: Some(0xff0000ff),
                preview_border_width: Some(3),
                ..Default::default()
            },
            decorations: false,
            border_width: Some(2),
            border_color_focused: Some(0xffffffff),
            border_color_unfocused: Some(0xffffffff),
            keybindings: Vec::new(),
            pointer_bindings: Vec::new(),
            resize_delta_ratio: None,
            resize_delta_percent: None,
            rules: Vec::new(),
            keyboard_layout: None,
            input_devices: Vec::new(),
        });

        let scene = state.desired_scene();

        let entry = scene.iter().find(|e| e.window_id == w1).unwrap();
        assert!(entry.border.is_some());
        let (edges, width, _r, _g, _b, _a) = entry.border.unwrap();
        assert_eq!(edges, 0b1000);
        assert_eq!(width, 3);
    }

    #[test]
    fn pending_split_none_gives_normal_border() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());
        state.focused_output = Some(o1);
        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        state.output_trees.insert(
            o1,
            crate::layout::LayoutTree::new(crate::layout::Rect::new(0, 0, 1000, 100)),
        );
        state.tree_for_output(o1).unwrap().insert_window(1, None);
        state.focus_window_id(w1);
        state.pending_split = None;
        state.config = Some(Config {
            layout: crate::config::LayoutConfig::default(),
            decorations: false,
            border_width: Some(2),
            border_color_focused: Some(0xffffffff),
            border_color_unfocused: Some(0xffffffff),
            keybindings: Vec::new(),
            pointer_bindings: Vec::new(),
            resize_delta_ratio: None,
            resize_delta_percent: None,
            rules: Vec::new(),
            keyboard_layout: None,
            input_devices: Vec::new(),
        });

        let scene = state.desired_scene();

        let entry = scene.iter().find(|e| e.window_id == w1).unwrap();
        assert!(entry.border.is_some());
        let (edges, width, _r, _g, _b, _a) = entry.border.unwrap();
        assert_eq!(edges, ALL_EDGES);
        assert_eq!(width, 2);
    }

    /// Build a `WMState` with one output, one tiled window, and one seat.
    fn setup_pointer_scene_fixture() -> (WMState, OutputId, WindowId, SeatId) {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());
        state.focused_output = Some(o1);
        let w1 = WindowId(1);
        state.windows.insert(w1, Window::new(w1, o1));
        state.output_trees.insert(
            o1,
            crate::layout::LayoutTree::new(crate::layout::Rect::new(0, 0, 1000, 800)),
        );
        state.tree_for_output(o1).unwrap().insert_window(1, None);
        state.tree_for_output(o1).unwrap().arrange();
        state
            .windows
            .get_mut(&w1)
            .unwrap()
            .set_layout_rect(crate::layout::Rect::new(0, 0, 1000, 800));
        let s1 = SeatId(1);
        state.seats.insert(s1, crate::state::seat::Seat::new());
        (state, o1, w1, s1)
    }

    #[test]
    fn transition_starts_move_op() {
        let (mut state, _o, w1, s1) = setup_pointer_scene_fixture();

        state.seats.get_mut(&s1).unwrap().op = crate::state::seat::InteractiveOp::Move {
            window_id: w1,
            initial_rect: crate::layout::Rect::new(0, 0, 1000, 800),
        };

        let effects = state.transition_pointer_ops();

        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::StartPointerOp { seat_id } if *seat_id == s1)),
            "move op must emit StartPointerOp"
        );
        // After starting, the op is marked started.
        assert!(
            state.seats.get(&s1).unwrap().op_started,
            "op marked started"
        );
    }

    #[test]
    fn transition_starts_resize_op_with_inform_resize() {
        let (mut state, _o, w1, s1) = setup_pointer_scene_fixture();

        state.seats.get_mut(&s1).unwrap().op = crate::state::seat::InteractiveOp::Resize {
            window_id: w1,
            edges: 0b1000,
            initial_rect: crate::layout::Rect::new(0, 0, 1000, 800),
        };

        let effects = state.transition_pointer_ops();

        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::StartPointerOp { seat_id } if *seat_id == s1)),
            "resize op must emit StartPointerOp"
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::InformResizeStart { window_id } if *window_id == w1)),
            "resize op must inform the window it is resizing"
        );
    }

    #[test]
    fn transition_ends_op_with_inform_resize_end() {
        let (mut state, _o, w1, s1) = setup_pointer_scene_fixture();

        let seat = state.seats.get_mut(&s1).unwrap();
        seat.op = crate::state::seat::InteractiveOp::Resize {
            window_id: w1,
            edges: 0b1000,
            initial_rect: crate::layout::Rect::new(0, 0, 1000, 800),
        };
        seat.op_started = true;
        seat.op_ending = true;

        let effects = state.transition_pointer_ops();

        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::EndPointerOp { seat_id } if *seat_id == s1)),
            "release must emit EndPointerOp"
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::InformResizeEnd { window_id } if *window_id == w1)),
            "resize end must inform the window"
        );
        // The op is cleared.
        assert!(
            !state.seats.get(&s1).unwrap().op.is_active(),
            "op cleared after end"
        );
    }

    #[test]
    fn transition_noop_when_no_op() {
        let (mut state, _o, _w1, s1) = setup_pointer_scene_fixture();

        let effects = state.transition_pointer_ops();
        assert!(effects.is_empty(), "no effects when no op is active");
        let _ = s1;
    }

    // Regression test: focusing a second floating window must produce a z-order
    // *promotion* for exactly the newly focused window and a *demotion* for the
    // previously focused one. `apply_render` keys its `PlaceTop` emission on
    // this promotion direction (`entry.z > last.z`), so only the focused window
    // is restacked. `apply_render` itself cannot be exercised here because it
    // early-returns for windows without a River proxy (`river_window`), which a
    // unit test cannot construct; this test locks in the z-direction premise the
    // render path depends on.
    #[test]
    fn focusing_second_floating_window_promotes_only_new_focus() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());
        state.focused_output = Some(o1);
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        for (wid, node) in [(w1, 1u32), (w2, 2u32)] {
            state.windows.insert(wid, Window::new(wid, o1));
            state.tree_for_output(o1).unwrap().insert_window(node, None);
        }
        // Both windows floating so their z tuples can diverge on focus.
        state
            .tree_for_output(o1)
            .unwrap()
            .toggle_floating(1, Rect::new(0, 0, 400, 300));
        state
            .tree_for_output(o1)
            .unwrap()
            .toggle_floating(2, Rect::new(400, 0, 400, 300));

        // Focus w1, snapshot its scene (w1 promoted, w2 demoted).
        state.focus_window_id(w1);
        let scene_a = state.desired_scene();
        let z1_a = scene_a.iter().find(|e| e.window_id == w1).unwrap().z;
        let z2_a = scene_a.iter().find(|e| e.window_id == w2).unwrap().z;

        // Focus w2, snapshot again.
        state.focus_window_id(w2);
        let scene_b = state.desired_scene();
        let z1_b = scene_b.iter().find(|e| e.window_id == w1).unwrap().z;
        let z2_b = scene_b.iter().find(|e| e.window_id == w2).unwrap().z;

        // w1 demotes, w2 promotes: the render path emits PlaceTop only where
        // the new z is strictly greater than the old.
        assert!(z1_b <= z1_a, "previously focused window demotes");
        assert!(z2_b > z2_a, "newly focused window promotes");
        assert!(z2_b > z1_b, "newly focused floating window ends on top");
    }

    // Regression test: the layer-shell default must be flagged for re-emission
    // when the first output is created (it becomes the focused output) even
    // though no focus *change* occurs. Without this flag, `apply_manage` would
    // only re-fire `SetLayerShellDefault` on a later focus change, leaving an
    // output-less layer client (e.g. swaybg launched before the first
    // focus-changing manage) without a default output.
    #[test]
    fn output_created_without_prior_focus_flags_layer_shell_default() {
        let mut state = WMState::new();
        assert!(!state.layer_shell_default_dirty);

        let o1 = OutputId(1);
        state.handle_event(Event::OutputCreated { output_id: o1 });

        assert_eq!(state.focused_output, Some(o1));
        assert!(
            state.layer_shell_default_dirty,
            "first OutputCreated must flag the layer-shell default for emission"
        );
    }
}
