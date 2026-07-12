//! Runtime River xkb binding state.
//!
//! `KeyBindingConfig` is declarative config. `KeyBinding` is the runtime representation
//! of one configured binding attached to one seat.
use super::seat::SeatId;
use super::wm::WMState;
use crate::command::Command;
use crate::protocol::river::river_window_management_v1::client::river_seat_v1::Modifiers;
use crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_binding_v1::RiverXkbBindingV1;
use wayland_client::QueueHandle;

/// Identifier for a runtime River xkb binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct XkbBindingId(pub u32);

/// Runtime River xkb binding attached to a specific seat.
pub(super) struct KeyBinding {
    /// Runtime binding identifier.
    pub(super) id: XkbBindingId,

    /// River protocol binding object, if currently configured.
    pub(super) river_binding: Option<RiverXkbBindingV1>,

    /// Seat this binding belongs to.
    pub(super) seat_id: SeatId,

    /// XKB keysym for this binding.
    pub(super) keysym: u32,

    /// River modifier bitmask for this binding.
    pub(super) modifiers: u32,

    /// Command to run when the binding is pressed.
    pub(super) command: Command,
}

impl WMState {
    /// Create a runtime binding entry.
    ///
    /// This does not create a River protocol object. Protocol creation happens later
    /// in `configure_keybindings` during a River manage sequence.
    pub(super) fn add_keybinding(
        &mut self,
        seat_id: SeatId,
        keysym: u32,
        modifiers: u32,
        command: Command,
    ) -> XkbBindingId {
        let id = self.next_xkb_binding_id();

        self.keybindings.insert(
            id,
            KeyBinding {
                id,
                seat_id,
                keysym,
                modifiers,
                command,
                river_binding: None,
            },
        );

        id
    }

    /// Remove runtime binding entries and queue their River protocol objects for destruction.
    ///
    /// This does not send River destroy requests immediately. Protocol deletion happens later
    /// in `destroy_pending_keybindings` during a River manage sequence.
    pub(super) fn delete_keybindings(&mut self) {
        for binding in self.keybindings.values_mut() {
            if let Some(proxy) = binding.river_binding.take() {
                self.pending_xkb_binding_destroys.push(proxy);
            }
        }

        self.keybindings.clear();
    }

    /// Create and enable River xkb binding objects during a manage sequence.
    ///
    /// Returns `true` when the River xkb bindings global is available
    /// and binding configuration was attempted for known seats.
    /// Returns `false` if the xkb bindings global is not available yet.
    /// Existing runtime bindings are enabled again. Bindings without a River protocol object
    /// are created using the associated `RiverSeatV1`.
    pub(super) fn configure_keybindings(&mut self, qh: &QueueHandle<Self>) -> bool {
        let Some(xkb) = self.xkb_bindings.as_ref() else {
            return false;
        };

        for binding in self.keybindings.values_mut() {
            if binding.river_binding.is_none() {
                let Some(seat) = self.seats.get(&binding.seat_id) else {
                    continue;
                };

                let Some(seat_proxy) = seat.river_seat.as_ref() else {
                    continue;
                };

                let proxy = xkb.get_xkb_binding(
                    seat_proxy,
                    binding.keysym,
                    Modifiers::from_bits_truncate(binding.modifiers),
                    qh,
                    binding.id,
                );

                binding.river_binding = Some(proxy);
            }

            if let Some(proxy) = binding.river_binding.as_ref() {
                proxy.enable();
            }
        }

        true
    }

    /// Destroy River xkb binding protocol objects queued by `delete_keybindings`.
    ///
    /// This must run during a River manage sequence.
    pub(super) fn destroy_pending_keybindings(&mut self) {
        for proxy in self.pending_xkb_binding_destroys.drain(..) {
            proxy.destroy();
        }
    }
}
