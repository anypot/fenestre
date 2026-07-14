//! Internal command execution for `WMState`.
//!
//! Commands are dispatched from keybindings and eventually from other command sources.
use super::output::OutputId;
use super::window::WindowId;
use super::wm::WMState;
use crate::command::Command;
use crate::layout::{FocusDirection, SplitDirection, WindowState};
use log::{debug, error};
use std::process::Command as ProcessCommand;
use wayland_client::QueueHandle;

impl WMState {
    /// Execute an internal command.
    pub(super) fn run_command(&mut self, command: Command, _qh: &QueueHandle<Self>) {
        debug!(target: "fenestre::state::commands", "Running command: {command:?}");

        match command {
            Command::FocusNext => self.focus_next(),
            Command::FocusPrevious => self.focus_previous(),
            Command::FocusUp => self.focus_up(),
            Command::FocusDown => self.focus_down(),
            Command::FocusLeft => self.focus_left(),
            Command::FocusRight => self.focus_right(),
            Command::SplitVertical => self.split_vertical(),
            Command::SplitHorizontal => self.split_horizontal(),
            Command::ToggleFullscreen => self.toggle_focused_fullscreen(),
            Command::ToggleFloating => self.toggle_focused_floating(),
            Command::TogglePseudoTiled => self.toggle_focused_pseudo_tiled(),
            Command::SetTiled => self.set_focused_state(WindowState::Tiled),
            Command::Spawn { program, args } => self.spawn(&program, &args),
            Command::ExitRiver => self.exit_river(),
            Command::ReloadConfig => self.reload_config(),
            Command::CloseFocused => self.close_focused(),
            Command::FocusOutput { output_id } => self.focus_output(output_id),
            Command::MoveLeft => self.move_window(FocusDirection::Left),
            Command::MoveRight => self.move_window(FocusDirection::Right),
            Command::MoveUp => self.move_window(FocusDirection::Up),
            Command::MoveDown => self.move_window(FocusDirection::Down),
            Command::ResizeExpand { direction } => self.resize_expand(direction),
            Command::ResizeShrink { direction } => self.resize_shrink(direction),
        }
    }

    fn focus_current_layout_window(&mut self) {
        let Some(id) = self.focused_tree().and_then(|t| t.focused_window()) else {
            return;
        };
        self.focus_window_id(WindowId(id));
    }

    /// Move focus to the next window in the focused output's tree.
    ///
    /// Delegates to the layout tree's `focus_next` and syncs global focus via
    /// `focus_current_layout_window`. No-op when there is no focused tree or the
    /// tree reports no next window.
    pub(super) fn focus_next(&mut self) {
        if self.focused_tree().is_some_and(|tree| tree.focus_next()) {
            self.focus_current_layout_window();
        }
    }

    fn focus_previous(&mut self) {
        if self
            .focused_tree()
            .is_some_and(|tree| tree.focus_previous())
        {
            self.focus_current_layout_window();
        }
    }

    fn focus_up(&mut self) {
        if self
            .focused_tree()
            .is_some_and(|tree| tree.focus_direction(FocusDirection::Up))
        {
            self.focus_current_layout_window();
        }
    }

    fn focus_down(&mut self) {
        if self
            .focused_tree()
            .is_some_and(|tree| tree.focus_direction(FocusDirection::Down))
        {
            self.focus_current_layout_window();
        }
    }

    fn focus_left(&mut self) {
        if self
            .focused_tree()
            .is_some_and(|tree| tree.focus_direction(FocusDirection::Left))
        {
            self.focus_current_layout_window();
        }
    }

    fn focus_right(&mut self) {
        if self
            .focused_tree()
            .is_some_and(|tree| tree.focus_direction(FocusDirection::Right))
        {
            self.focus_current_layout_window();
        }
    }

    fn split_vertical(&mut self) {
        if self
            .focused_tree()
            .is_some_and(|tree| tree.split_focused(SplitDirection::Vertical).is_some())
        {
            self.request_manage_dirty();
        }
    }

    fn split_horizontal(&mut self) {
        if self
            .focused_tree()
            .is_some_and(|tree| tree.split_focused(SplitDirection::Horizontal).is_some())
        {
            self.request_manage_dirty();
        }
    }

    /// Toggle fullscreen state for the focused window.
    fn toggle_focused_fullscreen(&mut self) {
        if let Some(window_id) = self.focused_window
            && self
                .focused_tree()
                .is_some_and(|tree| tree.toggle_fullscreen(window_id.0))
        {
            self.request_manage_dirty();
            self.render_order_cache.clear();
        }
    }

    /// Toggle floating state for the focused window.
    pub(super) fn toggle_focused_floating(&mut self) {
        let Some(window_id) = self.focused_window else {
            return;
        };
        let Some(rect) = self.resolve_toggle_rect(window_id) else {
            return;
        };
        let Some(tree) = self.focused_tree() else {
            return;
        };

        if tree.toggle_floating(window_id.0, rect) {
            self.request_manage_dirty();
            self.render_order_cache.clear();
        }
    }

    /// Toggle pseudo-tiled state for the focused window.
    pub(super) fn toggle_focused_pseudo_tiled(&mut self) {
        let Some(window_id) = self.focused_window else {
            return;
        };
        let Some(rect) = self.resolve_toggle_rect(window_id) else {
            return;
        };
        let Some(tree) = self.focused_tree() else {
            return;
        };

        if tree.toggle_pseudo_tiled(window_id.0, rect) {
            self.request_manage_dirty();
            self.render_order_cache.clear();
        }
    }

    /// Set the focused window to a specific state.
    fn set_focused_state(&mut self, target: WindowState) {
        let Some(window_id) = self.focused_window else {
            return;
        };
        let Some(rect) = self.resolve_toggle_rect(window_id) else {
            return;
        };
        let Some(tree) = self.focused_tree() else {
            return;
        };

        let state = match target {
            WindowState::Floating { .. } => WindowState::Floating { rect },
            WindowState::PseudoTiled { .. } => WindowState::PseudoTiled { rect },
            other => other,
        };

        if tree.set_window_state(window_id.0, state) {
            self.request_manage_dirty();
            self.render_order_cache.clear();
        }
    }

    /// Resolve the geometry rectangle to use when toggling window state.
    fn resolve_toggle_rect(&mut self, window_id: WindowId) -> Option<crate::layout::Rect> {
        let float_ratio = self.default_float_ratio();
        let window = self.windows.get(&window_id)?;
        let output_rect = self
            .outputs
            .get(&window.output_id)
            .and_then(|o| o.rect())
            .unwrap_or(crate::layout::Rect::new(0, 0, 0, 0));
        let pseudo_rect = window.pseudo_tiled_rect(output_rect, float_ratio);

        let state = match self.focused_tree() {
            Some(tree) => tree.window_state(window_id.0),
            None => None,
        };

        match state {
            Some(crate::layout::WindowState::Floating { rect }) => Some(*rect),
            Some(crate::layout::WindowState::Fullscreen { restore }) => match restore.as_ref() {
                crate::layout::WindowState::Floating { rect } => {
                    if rect.width > 0 && rect.height > 0 {
                        Some(*rect)
                    } else {
                        Some(pseudo_rect)
                    }
                }
                crate::layout::WindowState::PseudoTiled { rect } => Some(*rect),
                _ => Some(pseudo_rect),
            },
            _ => Some(pseudo_rect),
        }
    }

    fn spawn(&mut self, program: &str, args: &[String]) {
        debug!(target: "fenestre::state::commands", "Spawning program: {program} args: {args:?}");

        match ProcessCommand::new(program).args(args).spawn() {
            Ok(child) => {
                debug!(
                    target: "fenestre::state::commands",
                    "Spawned program `{program}` with pid={}",
                    child.id()
                );
            }
            Err(err) => {
                error!(
                    target: "fenestre::state::commands",
                    "Failed to spawn program `{program}`: {err}"
                );
            }
        }
    }

    fn exit_river(&self) {
        if let Some(wm) = self.wm.as_ref() {
            debug!(target: "fenestre::state::commands", "Exiting River session");
            wm.exit_session();
        } else {
            error!(
                target: "fenestre::state::commands",
                "Cannot exit River: river_window_manager_v1 is not bound"
            );
        }
    }

    fn close_focused(&mut self) {
        if let Some(window_id) = self.focused_window {
            self.pending_closes.push(window_id);
            self.request_manage_dirty();
        }
    }

    fn focus_output(&mut self, output_id: OutputId) {
        if self.outputs.contains_key(&output_id) && self.focused_output != Some(output_id) {
            self.focused_output = Some(output_id);
            if let Some(tree) = self.focused_tree()
                && let Some(window_id) = tree
                    .focused_window()
                    .or_else(|| tree.first_window())
                    .map(WindowId)
            {
                self.focus_window_id(window_id);
            }
        }
    }

    fn move_window(&mut self, direction: FocusDirection) {
        let Some(window_id) = self.focused_window else {
            return;
        };
        let Some(output_rect) = self.active_output_rect() else {
            return;
        };
        let delta_x = (output_rect.width as f64 * 0.1).round() as i32;
        let delta_y = (output_rect.height as f64 * 0.1).round() as i32;
        let Some(tree) = self.focused_tree() else {
            return;
        };
        let success = if tree.window_is_floating(window_id.0) {
            tree.move_floating_window(window_id.0, direction, delta_x, delta_y)
        } else if tree.window_is_fullscreen(window_id.0) {
            false
        } else {
            tree.swap_windows(direction)
        };

        if success {
            self.render_order_cache.clear();
            self.request_manage_dirty();
        }
    }

    fn resize_expand(&mut self, direction: FocusDirection) {
        self.resize_adjust(direction, true);
    }

    fn resize_shrink(&mut self, direction: FocusDirection) {
        self.resize_adjust(direction, false);
    }

    /// Resize the focused tiled window in the given direction.
    /// For floating windows, resizes the floating dimensions.
    /// For tiled windows, attempts direct resize first; if the immediate split
    /// doesn't support the direction (e.g., Up on a vertical split), navigates
    /// to an ancestor split that does support it, performs the resize, and restores
    /// focus to the original window.
    fn resize_adjust(&mut self, direction: FocusDirection, is_expand: bool) {
        let Some(window_id) = self.focused_window else {
            return;
        };

        let delta_ratio = self
            .config
            .as_ref()
            .and_then(|c| c.resize_delta_ratio)
            .unwrap_or(0.05);
        let delta_percent = self
            .config
            .as_ref()
            .and_then(|c| c.resize_delta_percent)
            .unwrap_or(5.0);

        let Some(tree) = self.focused_tree() else {
            return;
        };
        let success = if tree.window_is_floating(window_id.0) {
            tree.resize_floating_window(window_id.0, direction, delta_percent, is_expand)
        } else if tree.window_is_fullscreen(window_id.0) {
            false
        } else {
            let delta = if is_expand { delta_ratio } else { -delta_ratio };
            tree.resize_ratio(direction, delta)
        };

        if success {
            self.render_order_cache.clear();
            self.request_manage_dirty();
            return;
        }

        let original_focus = window_id;
        let Some(target_id) = tree.focus_to_resize_target(direction) else {
            return;
        };
        tree.focus_window(target_id);
        self.focus_current_layout_window();
        let delta = if is_expand { delta_ratio } else { -delta_ratio };
        let Some(tree) = self.focused_tree() else {
            return;
        };
        let resized = tree.resize_ratio(direction, delta);
        tree.focus_window(original_focus.0);
        self.focus_current_layout_window();
        if resized {
            self.render_order_cache.clear();
            self.request_manage_dirty();
        }
    }
}
