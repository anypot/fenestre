//! Runtime River pointer-binding state.
//!
//! Mirrors `keybindings.rs` but for `river_pointer_binding_v1` objects created
//! via `river_seat_v1.get_pointer_binding`. These bindings let the
//! WM intercept a pointer button (e.g. Super+Left) and start an interactive
//! move/resize operation on the focused window.
use super::seat::SeatId;
use super::wm::WMState;
use crate::config::PointerOp;
use crate::protocol::river::river_window_management_v1::client::river_pointer_binding_v1::RiverPointerBindingV1;
use crate::protocol::river::river_window_management_v1::client::river_seat_v1::Modifiers;
use wayland_client::QueueHandle;

/// Identifier for a runtime River pointer binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PointerBindingId(pub u32);

/// Runtime River pointer binding attached to a specific seat.
pub(super) struct PointerBinding {
    /// Runtime binding identifier.
    pub(super) id: PointerBindingId,

    /// River protocol binding object, if currently configured.
    pub(super) river_binding: Option<RiverPointerBindingV1>,

    /// Seat this binding belongs to.
    pub(super) seat_id: SeatId,

    /// Linux input event code for the bound button.
    pub(super) button: u32,

    /// River modifier bitmask for this binding.
    pub(super) modifiers: u32,

    /// Interactive operation performed on press.
    pub(super) op: PointerOp,
}

impl WMState {
    /// Create a runtime pointer-binding entry.
    ///
    /// This does not create a River protocol object. Protocol creation happens
    /// later in `configure_pointer_bindings` during a River manage sequence.
    pub(super) fn add_pointer_binding(
        &mut self,
        seat_id: SeatId,
        button: u32,
        modifiers: u32,
        op: PointerOp,
    ) -> PointerBindingId {
        let id = self.next_pointer_binding_id();

        self.pointer_bindings.insert(
            id,
            PointerBinding {
                id,
                seat_id,
                button,
                modifiers,
                op,
                river_binding: None,
            },
        );

        id
    }

    /// Remove runtime pointer-binding entries and queue their River protocol
    /// objects for destruction.
    pub(super) fn delete_pointer_bindings(&mut self) {
        for binding in self.pointer_bindings.values_mut() {
            if let Some(proxy) = binding.river_binding.take() {
                self.pending_pointer_binding_destroys.push(proxy);
            }
        }

        self.pointer_bindings.clear();
    }

    /// Create and enable River pointer-binding objects during a manage sequence.
    ///
    /// Returns `true` when at least one seat is available and binding
    /// configuration was attempted; `false` otherwise.
    /// Bindings without a River protocol object are created using the seat's
    /// `river_seat_v1` proxy, then enabled.
    pub(super) fn configure_pointer_bindings(&mut self, qh: &QueueHandle<Self>) -> bool {
        for binding in self.pointer_bindings.values_mut() {
            if binding.river_binding.is_none() {
                let Some(seat) = self.seats.get(&binding.seat_id) else {
                    continue;
                };
                let Some(seat_proxy) = seat.river_seat.as_ref() else {
                    continue;
                };

                let proxy = seat_proxy.get_pointer_binding(
                    binding.button,
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

    /// Destroy River pointer-binding protocol objects queued by `delete_pointer_bindings`.
    ///
    /// This must run during a River manage sequence.
    pub(super) fn destroy_pending_pointer_bindings(&mut self) {
        for proxy in self.pending_pointer_binding_destroys.drain(..) {
            proxy.destroy();
        }
    }
}
