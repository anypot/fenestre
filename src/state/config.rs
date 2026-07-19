//! Configuration application for `WMState`.
//!
//! This module bridges declarative configuration and runtime River xkb bindings.
//! It resolves abstract targets like `Primary` and `All` into concrete
//! seat-specific runtime bindings.
use std::path::Path;

use super::seat::SeatId;
use super::wm::WMState;
use crate::config::{Config, KeyBindingTarget, Result};
use log::{debug, error, warn};

impl WMState {
    /// Load a config file from disk and apply it to this state.
    pub(crate) fn load_config_file(&mut self, path: &Path) -> Result<()> {
        let config = Config::load_from_path(path)?;
        self.config_path = Some(path.to_path_buf());
        self.load_config(config);
        Ok(())
    }

    /// Load the built-in default configuration.
    pub(super) fn load_default_config(&mut self) {
        let config = Config::load();
        self.load_config(config);
    }

    /// Replace the active configuration and reconcile runtime bindings and window rules.
    pub(super) fn load_config(&mut self, config: Config) {
        debug!(target: "fenestre::state::config", "Loaded final config: {config:#?}");
        self.config = Some(config);

        if let Some(cfg) = self.config.as_ref() {
            for tree in self.output_trees.values_mut() {
                tree.set_layout_config(cfg.layout.clone());
            }
        }

        self.reconcile_keybindings();
        self.reconcile_pointer_bindings();
        self.reconcile_window_rules();
        self.request_manage_dirty();
    }

    /// Reload the active configuration file, if one was loaded.
    pub(super) fn reload_config(&mut self) {
        let Some(path) = self.config_path.as_deref() else {
            warn!(
                target: "fenestre::state::config",
                "Cannot reload config: no config path is loaded"
            );
            return;
        };

        match Config::load_from_path(path) {
            Ok(config) => {
                debug!(
                    target: "fenestre::state::config",
                    "Reloaded config from {}",
                    path.display()
                );
                self.load_config(config);
            }
            Err(err) => {
                error!(
                    target: "fenestre::state::config",
                    "Failed to reload config from {}: {err}",
                    path.display()
                );
            }
        }
    }

    /// Rebuild runtime River xkb bindings from the active config.
    ///
    /// Existing runtime binding entries are removed, their River protocol objects
    /// are queued for destruction during the next River manage sequence,
    /// and the resolved bindings are recreated from the active config.
    pub(super) fn reconcile_keybindings(&mut self) {
        let Some(keybindings) = self.config.as_ref().map(|c| c.keybindings.clone()) else {
            return;
        };

        self.delete_keybindings();

        debug!(
            target: "fenestre::state::config",
            "Resolved {} keybindings: {keybindings:#?}",
            keybindings.len()
        );

        for binding in keybindings {
            for seat_id in self.resolve_seat_targets(&binding.target) {
                self.add_keybinding(
                    seat_id,
                    binding.keysym,
                    binding.modifiers,
                    binding.command.clone(),
                );
            }
        }

        self.xkb_bindings_dirty = true;
    }

    /// Rebuild runtime River pointer bindings from the active config.
    ///
    /// Mirrors `reconcile_keybindings` for pointer bindings: existing runtime
    /// entries are removed (their River protocol objects queued for destruction
    /// during the next manage sequence) and the resolved bindings recreated.
    pub(super) fn reconcile_pointer_bindings(&mut self) {
        let Some(bindings) = self.config.as_ref().map(|c| c.pointer_bindings.clone()) else {
            return;
        };

        self.delete_pointer_bindings();

        debug!(
            target: "fenestre::state::config",
            "Resolved {} pointer bindings", bindings.len()
        );

        for binding in bindings {
            for seat_id in self.resolve_seat_targets(&binding.target) {
                self.add_pointer_binding(seat_id, binding.button, binding.modifiers, binding.op);
            }
        }

        self.pointer_bindings_dirty = true;
    }

    /// Resolve a `KeyBindingTarget` into concrete seat IDs.
    ///
    /// `Primary` resolves to the lowest `SeatId`. `All` resolves to every known seat.
    fn resolve_seat_targets(&self, target: &KeyBindingTarget) -> Vec<SeatId> {
        match target {
            KeyBindingTarget::Primary => self.seats.keys().next().copied().into_iter().collect(),
            KeyBindingTarget::All => self.seats.keys().copied().collect(),
        }
    }

    /// Load desired window rules from the active config into runtime state.
    ///
    /// Rules are evaluated per-window when their metadata (app_id/title) arrives
    /// via `WMState::evaluate_window_rules`, and each window is matched only
    /// once (`Window::rules_applied`). We intentionally do NOT re-evaluate
    /// already-on-screen windows here on (re)load: existing windows keep the
    /// state they were assigned when their metadata first arrived.
    pub(super) fn reconcile_window_rules(&mut self) {
        let rules = self
            .config
            .as_ref()
            .map(|c| c.rules.clone())
            .map(super::rule::WindowRules::new);
        self.window_rules = rules;
    }
}
