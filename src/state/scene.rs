//! Scene snapshot & manage/render logic.
//!
//! Owns the scene computation and the manage/render effect sequences for
//! `WMState`. The four scene-related fields on `WMState` are owned by this
//! module — mutate them only through `apply_manage` or `apply_render`.
use std::collections::HashMap;

use super::WMState;
use super::effects::Effect;
use super::output::OutputId;
use super::window::WindowId;
use crate::layout::{Rect, WindowState};
use wayland_client::QueueHandle;

/// A single window's entry in a desired scene snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SceneEntry {
    pub(crate) window_id: WindowId,
    pub(crate) output_id: OutputId,
    pub(crate) rect: Rect,
    pub(crate) state: WindowState,
    pub(crate) z: (u8, u8, u32),
    pub(crate) border: Option<(i32, u32, u32, u32, u32)>,
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
            for (window_id, window_rect, state) in arranged {
                let window_id = WindowId(window_id);
                let Some(window) = self.windows.get(&window_id) else {
                    continue;
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
                    let rgba = if self.focused_window == Some(window_id) {
                        rgba_focused
                    } else {
                        rgba_unfocused
                    };
                    Some((border_width, rgba.0, rgba.1, rgba.2, rgba.3))
                };

                scene.push(SceneEntry {
                    window_id,
                    output_id: *output_id,
                    rect: window_rect,
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
        let mut effects = Vec::new();

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

        if let Some(default_output) = self.focused_output
            && self.last_layer_shell_default != Some(default_output)
            && self.layer_shell.is_some()
            && self
                .outputs
                .get(&default_output)
                .is_some_and(|o| o.river_layer_shell_output.is_some())
        {
            effects.push(Effect::SetLayerShellDefault {
                output_id: default_output,
            });
            self.last_layer_shell_default = Some(default_output);
        }

        self.last_manage_scene = desired;
        self.render_order_cache.clear();
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
                if let Some((width, r, g, b, a)) = entry.border {
                    effects.push(Effect::SetBorders {
                        window_id: *window_id,
                        edges: super::effects::ALL_EDGES,
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

            if last.z != entry.z
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
                && let Some((width, r, g, b, a)) = entry.border
            {
                effects.push(Effect::SetBorders {
                    window_id: *window_id,
                    edges: super::effects::ALL_EDGES,
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
