//! Output reassignment logic for `WMState`.
//!
//! Moves windows from one output's layout tree into another, preserving each
//! window's mode (floating, fullscreen, pseudo-tiled) and focus state so windows
//! survive output hotplug without being destroyed.

use std::collections::HashMap;

use super::output::OutputId;
use super::window::WindowId;
use super::wm::WMState;
use crate::layout::{LayoutTree, Rect, WindowState};
use log::debug;

/// Plan describing the layout state each moved window must return to on the
/// destination tree.
#[derive(Clone, Copy)]
enum ReassignPlan {
    Tiled,
    Floating(Rect),
    PseudoTiled(Rect),
    FullscreenTiled,
    FullscreenPseudo(Rect),
    FullscreenFloating(Rect),
}

impl WMState {
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
            debug!(target: "fenestre::state::reassign", "reassign_output: source output {:?} has no tree, nothing to move", from);
            return;
        };

        let to_rect = self
            .outputs
            .get(&to)
            .and_then(|o| o.tiling_rect())
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
    /// re-deriving their modes from the tree's `WindowState`, adapting splits to the
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
        let float_ratio = self.default_float_ratio();

        let mut plans = HashMap::new();
        for win_id in window_ids {
            if let Some(window) = self.windows.get(&WindowId(*win_id)) {
                let pseudo_rect = window.pseudo_tiled_rect(to_rect, float_ratio);
                let plan = match from_tree.window_state(*win_id) {
                    Some(state) => match state {
                        WindowState::Floating { rect } => ReassignPlan::Floating(Rect::new(
                            rect.x.saturating_add(dx),
                            rect.y.saturating_add(dy),
                            rect.width,
                            rect.height,
                        )),
                        WindowState::PseudoTiled { .. } => ReassignPlan::PseudoTiled(pseudo_rect),
                        WindowState::Fullscreen { restore } => match restore.as_ref() {
                            WindowState::PseudoTiled { .. } => {
                                ReassignPlan::FullscreenPseudo(pseudo_rect)
                            }
                            WindowState::Floating { rect } => {
                                ReassignPlan::FullscreenFloating(Rect::new(
                                    rect.x.saturating_add(dx),
                                    rect.y.saturating_add(dy),
                                    rect.width,
                                    rect.height,
                                ))
                            }
                            _ => ReassignPlan::FullscreenTiled,
                        },
                        _ => ReassignPlan::Tiled,
                    },
                    None => ReassignPlan::Tiled,
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
                        let _ = to_tree.toggle_pseudo_tiled(*win_id, rect);
                        let _ = to_tree.toggle_fullscreen(*win_id);
                    }
                    ReassignPlan::FullscreenFloating(rect) => {
                        let _ = to_tree.toggle_floating(*win_id, rect);
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

    /// Default floating / pseudo-tiled size fraction for windows that have not
    /// reported their own dimensions.
    pub(super) fn default_float_ratio(&self) -> f32 {
        self.config
            .as_ref()
            .and_then(|c| c.layout.default_float_ratio)
            .unwrap_or(0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::output::Output;
    use crate::state::window::Window;

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

        state.reassign_output(o1, o2);

        let o2_tree = state.tree_for_output(o2).unwrap();
        let fullscreen_state = o2_tree.window_state(w1.0).unwrap().clone();
        let crate::layout::WindowState::Fullscreen { restore } = fullscreen_state else {
            panic!("expected fullscreen");
        };
        assert_eq!(*restore, crate::layout::WindowState::PseudoTiled { rect });
        assert!(o2_tree.window_is_fullscreen(w1.0));

        // Un-fullscreening should return to PseudoTiled, not Tiled.
        o2_tree.toggle_fullscreen(w1.0);
        let state_after = o2_tree
            .arranged_windows()
            .into_iter()
            .find(|(id, _, _)| *id == w1.0)
            .map(|(_, _, s)| s);
        assert_eq!(
            state_after,
            Some(crate::layout::WindowState::PseudoTiled { rect })
        );
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

    /// A realistic reassignment where the source output holds a *mixed* set of
    /// window modes (tiled / floating / pseudo-tiled / fullscreen). The
    /// destination already has real geometry, so `reassign_output` takes the
    /// `reassign_with_rebuild` path. Every window must keep its mode on the
    /// destination, and floating rects must survive unchanged when source and
    /// destination share the same logical origin.
    #[test]
    fn reassign_with_rebuild_preserves_mixed_mode_set() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, w, h) in [(o1, 1920, 1080), (o2, 1920, 1080)] {
            let mut out = Output::new(o);
            out.set_dimensions(w, h);
            state.outputs.insert(o, out);
        }
        state.focused_output = Some(o1);

        let tiled = WindowId(1);
        let floating = WindowId(2);
        let pseudo = WindowId(3);
        let full = WindowId(4);
        for w in [tiled, floating, pseudo, full] {
            state.windows.insert(w, Window::new(w, o1));
            state.tree_for_output(o1).unwrap().insert_window(w.0);
            state.push_focus(w);
        }

        // Make the source tree hold every mode variant.
        let o1_rect = crate::layout::Rect::new(0, 0, 1920, 1080);
        let ratio = state.default_float_ratio();
        let float_rect = crate::layout::Rect::new(100, 100, 400, 300);
        state
            .tree_for_output(o1)
            .unwrap()
            .toggle_floating(floating.0, float_rect);
        let pseudo_rect = state
            .windows
            .get(&pseudo)
            .unwrap()
            .pseudo_tiled_rect(o1_rect, ratio);
        state
            .tree_for_output(o1)
            .unwrap()
            .toggle_pseudo_tiled(pseudo.0, pseudo_rect);
        // Fullscreen with a plain Tiled base state.
        state.tree_for_output(o1).unwrap().toggle_fullscreen(full.0);

        state.reassign_output(o1, o2);

        let o2_tree = state.tree_for_output(o2).unwrap();
        assert_eq!(
            o2_tree.window_state(tiled.0).cloned(),
            Some(crate::layout::WindowState::Tiled)
        );
        assert_eq!(
            o2_tree.window_state(floating.0).cloned(),
            Some(crate::layout::WindowState::Floating { rect: float_rect })
        );
        assert_eq!(
            o2_tree.window_state(pseudo.0).cloned(),
            Some(crate::layout::WindowState::PseudoTiled { rect: pseudo_rect })
        );
        assert!(o2_tree.window_is_fullscreen(full.0));

        // Floating rect preserved (source/dest share origin -> no translation).
        let crate::layout::WindowState::Floating { rect } =
            o2_tree.window_state(floating.0).unwrap().clone()
        else {
            panic!("expected floating");
        };
        assert_eq!(rect, float_rect);

        // Fullscreen restore state preserved as Tiled (not clobbered).
        let crate::layout::WindowState::Fullscreen { restore } =
            o2_tree.window_state(full.0).unwrap().clone()
        else {
            panic!("expected fullscreen");
        };
        assert_eq!(*restore, crate::layout::WindowState::Tiled);

        // Un-fullscreening returns to the base Tiled state.
        o2_tree.toggle_fullscreen(full.0);
        assert_eq!(
            o2_tree.window_state(full.0).cloned(),
            Some(crate::layout::WindowState::Tiled)
        );
    }

    /// Reassign a fullscreen-over-floating window through the *rebuild* path
    /// (destination has real geometry). The fullscreen base state must survive
    /// as `Floating` and un-fullscreening must return to the exact float rect.
    /// This covers the `FullscreenFloating` plan branch of `reassign_with_rebuild`.
    #[test]
    fn reassign_with_rebuild_preserves_fullscreen_floating_base() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, w, h) in [(o1, 1920, 1080), (o2, 1920, 1080)] {
            let mut out = Output::new(o);
            out.set_dimensions(w, h);
            state.outputs.insert(o, out);
        }
        state.focused_output = Some(o1);

        let w = WindowId(1);
        state.windows.insert(w, Window::new(w, o1));
        let tree = state.tree_for_output(o1).unwrap();
        tree.insert_window(w.0);
        let float_rect = crate::layout::Rect::new(200, 150, 500, 400);
        tree.toggle_floating(w.0, float_rect);
        tree.toggle_fullscreen(w.0);

        state.reassign_output(o1, o2);

        let o2_tree = state.tree_for_output(o2).unwrap();
        assert!(o2_tree.window_is_fullscreen(w.0));
        let crate::layout::WindowState::Fullscreen { restore } =
            o2_tree.window_state(w.0).unwrap().clone()
        else {
            panic!("expected fullscreen");
        };
        assert_eq!(
            *restore,
            crate::layout::WindowState::Floating { rect: float_rect }
        );

        // Un-fullscreening returns to floating at the same rect.
        o2_tree.toggle_fullscreen(w.0);
        assert_eq!(
            o2_tree.window_state(w.0).cloned(),
            Some(crate::layout::WindowState::Floating { rect: float_rect })
        );
    }

    /// Reassign a fullscreen-over-pseudo-tiled window through the *rebuild*
    /// path. The fullscreen base state must survive as `PseudoTiled`. (The
    /// existing `reassign_output_preserves_fullscreen_base_state` only exercises
    /// the dimension-less `clone` path, so this covers the
    /// `FullscreenPseudo` branch of `reassign_with_rebuild`.)
    #[test]
    fn reassign_with_rebuild_preserves_fullscreen_pseudo_base() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        for (o, w, h) in [(o1, 1920, 1080), (o2, 1920, 1080)] {
            let mut out = Output::new(o);
            out.set_dimensions(w, h);
            state.outputs.insert(o, out);
        }
        state.focused_output = Some(o1);

        let w = WindowId(1);
        state.windows.insert(w, Window::new(w, o1));
        let tree = state.tree_for_output(o1).unwrap();
        tree.insert_window(w.0);
        let rect = crate::layout::Rect::new(0, 0, 100, 100);
        tree.toggle_pseudo_tiled(w.0, rect);
        tree.toggle_fullscreen(w.0);

        state.reassign_output(o1, o2);

        let o2_tree = state.tree_for_output(o2).unwrap();
        assert!(o2_tree.window_is_fullscreen(w.0));
        let crate::layout::WindowState::Fullscreen { restore } =
            o2_tree.window_state(w.0).unwrap().clone()
        else {
            panic!("expected fullscreen");
        };
        let expected_pseudo = crate::layout::Rect::new(480, 270, 960, 540);
        assert_eq!(
            *restore,
            crate::layout::WindowState::PseudoTiled {
                rect: expected_pseudo
            }
        );

        // Un-fullscreening returns to pseudo-tiled.
        o2_tree.toggle_fullscreen(w.0);
        assert_eq!(
            o2_tree.window_state(w.0).cloned(),
            Some(crate::layout::WindowState::PseudoTiled {
                rect: expected_pseudo
            })
        );
    }

    /// Reassign into a dimension-less (recreated) output while a window is
    /// floating. The floating rect must be translated by the output position
    /// delta `(dx, dy)` so the window stays under the cursor / in place on the
    /// new output. This covers the floating-translation branch of
    /// `reassign_clone_topology`.
    #[test]
    fn reassign_into_dimensionless_output_translates_floating_rects() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);

        // Source at the origin with a real floating window.
        let mut out1 = Output::new(o1);
        out1.set_dimensions(1920, 1080);
        out1.set_position(0, 0);
        state.outputs.insert(o1, out1);

        let floating = WindowId(1);
        state.windows.insert(floating, Window::new(floating, o1));
        let tree = state.tree_for_output(o1).unwrap();
        tree.insert_window(floating.0);
        let src_rect = crate::layout::Rect::new(100, 50, 400, 300);
        tree.toggle_floating(floating.0, src_rect);
        state.push_focus(floating);

        // Recreate o2 at a different position, geometry not yet known.
        let mut out2 = Output::new(o2);
        out2.set_position(1920, 0);
        state.outputs.insert(o2, out2);

        state.reassign_output(o1, o2);

        // Give the recreated output real geometry and arrange.
        let o2_rect = crate::layout::Rect::new(1920, 0, 1920, 1080);
        state.tree_for_output(o2).unwrap().set_output_rect(o2_rect);

        // Floating rect must be shifted by (dx=1920, dy=0).
        let crate::layout::WindowState::Floating { rect: dst_rect } = state
            .tree_for_output(o2)
            .unwrap()
            .window_state(floating.0)
            .unwrap()
            .clone()
        else {
            panic!("expected floating");
        };
        assert_eq!(dst_rect, crate::layout::Rect::new(100 + 1920, 50, 400, 300));
    }
}
