//! Focus management for `WMState`.
//!
//! Owns the focus stack, global focus pointers, and close-window reconciliation.

use super::window::WindowId;
use super::wm::WMState;

impl WMState {
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
            self.pending_split = None;
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

    /// Push a window onto the front of the focus stack and mark it focused.
    ///
    /// Updates `focused_window` and `focused_output` (derived from the window's
    /// own output) so the most recently focused window is always at the top.
    /// Low-level helper; callers that also need tree focus and River sync should
    /// use `focus_window_id` instead.
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

        self.pending_split = None;
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

#[cfg(test)]
mod tests {
    use super::super::output::{Output, OutputId};
    use super::super::window::{Window, WindowId};
    use super::*;

    #[test]
    fn push_focus_makes_new_window_focused() {
        let mut state = WMState::new();
        let window_id = WindowId(1);
        let output_id = OutputId(1);
        state.outputs.insert(output_id, Output::new());
        state.focused_output = Some(output_id);
        let window = Window::new(window_id, output_id);
        state.windows.insert(window_id, window);
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(1, None);
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
        state.outputs.insert(output_id, Output::new());
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state.windows.insert(w2, Window::new(w2, output_id));
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(1, None);
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(2, None);

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
        state.outputs.insert(output_id, Output::new());
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state.windows.insert(w2, Window::new(w2, output_id));
        state.windows.insert(w3, Window::new(w3, output_id));
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(1, None);
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(2, None);
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(3, None);

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
        state.outputs.insert(output_id, Output::new());
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state.windows.insert(w2, Window::new(w2, output_id));
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(1, None);
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(2, None);

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
        state.outputs.insert(output_id, Output::new());
        state.focused_output = Some(output_id);
        state.windows.insert(w1, Window::new(w1, output_id));
        state
            .tree_for_output(output_id)
            .unwrap()
            .insert_window(1, None);
        state.push_focus(w1);
        state.request_manage_dirty();

        assert_eq!(state.focused_window, Some(w1));
        assert_eq!(state.focused_tree().unwrap().focused_window(), Some(1));
    }

    #[test]
    fn pointer_interaction_switches_focused_output() {
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
        state.push_focus(w1);

        state.focus_window_id(w2);
        assert_eq!(state.focused_window, Some(w2));
        assert_eq!(state.focused_output, Some(o2));
    }

    #[test]
    fn closing_tree_focused_window_keeps_state_and_layout_consistent() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());
        state.focused_output = Some(o1);

        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);
        for w in [a, b, c] {
            state.windows.insert(w, Window::new(w, o1));
            state.tree_for_output(o1).unwrap().insert_window(w.0, None);
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
            let mut out = Output::new();
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
        state.tree_for_output(o1).unwrap().insert_window(a.0, None);
        state.focus_window_id(a);
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(state.focused_output, Some(o1));

        // Two windows on the other output, with C last focused there.
        for w in [b, c] {
            state.windows.insert(w, Window::new(w, o2));
            state.tree_for_output(o2).unwrap().insert_window(w.0, None);
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
            let mut out = Output::new();
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
            state.tree_for_output(o1).unwrap().insert_window(w.0, None);
        }
        state.windows.insert(c, Window::new(c, o2));
        state.tree_for_output(o2).unwrap().insert_window(c.0, None);
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

    #[test]
    fn focus_next_recovers_from_removed_focused_output() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());

        let w1 = WindowId(1);
        let w2 = WindowId(2);
        state.windows.insert(w1, Window::new(w1, o1));
        state.windows.insert(w2, Window::new(w2, o1));
        state.tree_for_output(o1).unwrap().insert_window(w1.0, None);
        state.tree_for_output(o1).unwrap().insert_window(w2.0, None);

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
        state.outputs.insert(o1, Output::new());
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
                state.tree_for_output(o1).unwrap().insert_window(id.0, None);
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
            let mut out = Output::new();
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
                state
                    .tree_for_output(out)
                    .unwrap()
                    .insert_window(id.0, None);
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
            let mut out = Output::new();
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }

        let a = WindowId(1);
        let b = WindowId(2);
        state.windows.insert(a, Window::new(a, o1));
        state.windows.insert(b, Window::new(b, o2));
        state.tree_for_output(o1).unwrap().insert_window(a.0, None);
        state.tree_for_output(o2).unwrap().insert_window(b.0, None);

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
            let mut out = Output::new();
            out.set_dimensions(1920, 1080);
            out.set_position(x, 0);
            state.outputs.insert(o, out);
        }

        let a = WindowId(1);
        let b = WindowId(2);
        state.windows.insert(a, Window::new(a, o1));
        state.windows.insert(b, Window::new(b, o2));
        state.tree_for_output(o1).unwrap().insert_window(a.0, None);
        state.tree_for_output(o2).unwrap().insert_window(b.0, None);

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

    /// Closing the *globally focused* window when its output empties must fall
    /// back to another output's window (via the global focus stack), not lose
    /// focus or yank it elsewhere spuriously. This is the multi-output
    /// reconciliation branch `was_globally_focused && next == Some(window on o2)`.
    #[test]
    fn closing_focused_window_when_output_empties_falls_back_to_other_output() {
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

        // Only window on o1, and globally focused.
        let a = WindowId(1);
        state.windows.insert(a, Window::new(a, o1));
        state.tree_for_output(o1).unwrap().insert_window(a.0, None);
        state.push_focus(a);

        // Two windows on o2; C is the last focused there (remembered fallback).
        let b = WindowId(2);
        let c = WindowId(3);
        for w in [b, c] {
            state.windows.insert(w, Window::new(w, o2));
            state.tree_for_output(o2).unwrap().insert_window(w.0, None);
            state.push_focus(w);
        }

        // Re-assert global focus on A (o1) while keeping B/C on the stack so the
        // fallback after closing A lands on C (the surviving output's focus).
        state.focus_window_id(a);
        assert_eq!(state.focused_window, Some(a));
        assert_eq!(state.focused_output, Some(o1));

        // Close the only window on o1 -> o1 now empty.
        state.close_window_focus_reconcile(a);

        // Focus must fall back to C on o2 (the global focus stack top).
        assert_eq!(
            state.focused_window,
            Some(c),
            "focus lost when the focused output emptied"
        );
        assert_eq!(
            state.focused_output,
            Some(o2),
            "focus did not move to the surviving output"
        );
        assert!(state.windows.contains_key(&c), "sibling lost on fallback");
        assert!(state.windows.contains_key(&b), "sibling lost on fallback");
        assert!(
            !state.windows.contains_key(&a),
            "closed window still present"
        );
    }

    /// Closing the very last window in the WM must clear global focus and
    /// pending focus cleanly (no dangling reference, no panic). This covers the
    /// `was_globally_focused && next == None` fallback branch.
    #[test]
    fn closing_last_remaining_window_clears_focus() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let mut out = Output::new();
        out.set_dimensions(1920, 1080);
        state.outputs.insert(o1, out);
        state.focused_output = Some(o1);

        let a = WindowId(1);
        state.windows.insert(a, Window::new(a, o1));
        state.tree_for_output(o1).unwrap().insert_window(a.0, None);
        state.focus_window_id(a);
        assert_eq!(state.focused_window, Some(a));

        state.close_window_focus_reconcile(a);

        assert_eq!(
            state.focused_window, None,
            "focus should clear when the last window is closed"
        );
        assert_eq!(state.pending_focus, None, "pending focus should clear");
        assert!(!state.windows.contains_key(&a));
    }

    #[test]
    fn focus_window_id_clears_pending_split() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());
        state.focused_output = Some(o1);
        let w1 = WindowId(1);
        let w2 = WindowId(2);
        state.windows.insert(w1, Window::new(w1, o1));
        state.windows.insert(w2, Window::new(w2, o1));
        state.tree_for_output(o1).unwrap().insert_window(1, None);
        state.tree_for_output(o1).unwrap().insert_window(2, None);
        state.focus_window_id(w1);
        state.pending_split = Some(crate::layout::SplitDirection::Right);

        state.focus_window_id(w2);

        assert_eq!(state.pending_split, None);
        assert_eq!(state.focused_window, Some(w2));
    }

    #[test]
    fn closing_focused_window_clears_pending_split() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        state.outputs.insert(o1, Output::new());
        state.focused_output = Some(o1);
        let a = WindowId(1);
        let b = WindowId(2);
        state.windows.insert(a, Window::new(a, o1));
        state.windows.insert(b, Window::new(b, o1));
        state.tree_for_output(o1).unwrap().insert_window(a.0, None);
        state.tree_for_output(o1).unwrap().insert_window(b.0, None);
        state.focus_window_id(a);
        state.pending_split = Some(crate::layout::SplitDirection::Down);

        state.close_window_focus_reconcile(a);

        assert_eq!(state.pending_split, None);
        assert_eq!(state.focused_window, Some(b));
    }
}
