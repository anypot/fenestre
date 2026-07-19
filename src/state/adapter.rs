//! Protocol adapter for `WMState`.
//!
//! This module owns the boundary between pure core state and River protocol
//! calls. It applies deferred effects and performs proxy bookkeeping for
//! newly created windows, outputs, and seats.
use super::effects::Effect;
use super::output::OutputId;
use super::seat::SeatId;
use super::window::WindowId;
use super::wm::WMState;
use wayland_client::QueueHandle;

impl WMState {
    /// Set the River window proxy on an already-created window entry.
    pub(super) fn set_window_proxy(
        &mut self,
        window_id: WindowId,
        river_window: crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1,
    ) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.river_window = Some(river_window);
        }
    }

    /// Set the River output proxy on an already-created output entry.
    pub(super) fn set_output_proxy(
        &mut self,
        output_id: OutputId,
        river_output: crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1,
    ) {
        if let Some(output) = self.outputs.get_mut(&output_id) {
            output.river_output = Some(river_output);
        }
    }

    /// Set the River seat proxy on an already-created seat entry.
    pub(super) fn set_seat_proxy(
        &mut self,
        seat_id: SeatId,
        river_seat: crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1,
    ) {
        if let Some(seat) = self.seats.get_mut(&seat_id) {
            seat.river_seat = Some(river_seat);
        }
    }

    /// Drain all pending effects and apply their protocol calls.
    pub(super) fn apply_effects(&mut self, _qh: &QueueHandle<Self>, effects: Vec<Effect>) {
        for effect in effects {
            self.apply_effect(_qh, effect);
        }
    }

    /// Remove an output: dispatch core event and clean the proxy index.
    ///
    /// Destroys the `river_layer_shell_output_v1` child proxy before dispatching
    /// the core removal, as required by the river_layer_shell_v1 protocol: the
    /// child is made inert by the `river_output_v1.removed` event and must be
    /// destroyed to complete destruction of the output (and before the owner
    /// `river_output` proxy itself is dropped by the caller). The child proxy is
    /// `take`n out of the entry first because the core event removes the entry
    /// from `self.outputs`.
    pub(super) fn remove_output(
        &mut self,
        output_id: OutputId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1,
    ) {
        if let Some(output) = self.outputs.get_mut(&output_id)
            && let Some(layer_output) = output.river_layer_shell_output.take()
        {
            layer_output.destroy();
        }
        self.handle_event(super::events::Event::OutputRemoved { output_id });
        self.outputs_by_proxy.remove(proxy);
    }

    /// Remove a seat: dispatch core event and clean the proxy index.
    ///
    /// Mirrors [`remove_output`]: destroys the `river_layer_shell_seat_v1` child
    /// proxy before dispatching the core removal, since the child is made inert
    /// by the `river_seat_v1.removed` event and must be destroyed before the
    /// owner `river_seat` proxy is dropped by the caller.
    pub(super) fn remove_seat(
        &mut self,
        seat_id: SeatId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1,
    ) {
        if let Some(seat) = self.seats.get_mut(&seat_id)
            && let Some(layer_seat) = seat.river_layer_shell_seat.take()
        {
            layer_seat.destroy();
        }
        self.handle_event(super::events::Event::SeatRemoved { seat_id });
        self.seats_by_proxy.remove(proxy);
    }

    /// Remove a window: dispatch core event and clean the proxy index.
    pub(super) fn remove_window(
        &mut self,
        window_id: WindowId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1,
    ) {
        self.handle_event(super::events::Event::WindowClosed { window_id });
        self.windows_by_proxy.remove(proxy);
    }

    fn apply_effect(&mut self, qh: &QueueHandle<Self>, effect: Effect) {
        match effect {
            Effect::ProposeDimensions {
                window_id,
                width,
                height,
            } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.propose_dimensions(width, height);
                }
            }
            Effect::Fullscreen {
                window_id,
                output_id,
            } => {
                if let Some(output) = self.outputs.get(&output_id)
                    && let Some(river_output) = output.river_output.as_ref()
                    && let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.fullscreen(river_output);
                }
            }
            Effect::ExitFullscreen { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.exit_fullscreen();
                }
            }
            Effect::UseSsd { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.use_ssd();
                }
            }
            Effect::EnsureNode { window_id } => {
                if let Some(window) = self.windows.get_mut(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    window.node = Some(river_window.get_node(qh, ()));
                }
            }
            Effect::SetBorders {
                window_id,
                edges,
                width,
                r,
                g,
                b,
                a,
            } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    // NOTE: River's drawBorders in Window.zig uses a zero-size
                    // workaround for disabled edges instead of calling
                    // wlr_scene_node_setEnabled(false) directly. This avoids a
                    // Zig 0.16.0 ReleaseSafe optimizer bug where extern C fn
                    // calls to wlroots are elided when the field appears to be
                    // dead (the zig-pkg struct binding reads @field(node,"enabled")
                    // is never read through the Zig type before the next write).
                    let edges = crate::protocol::river::river_window_management_v1::client::river_window_v1::Edges::from_bits(edges).unwrap_or(crate::protocol::river::river_window_management_v1::client::river_window_v1::Edges::empty());
                    river_window.set_borders(edges, width, r, g, b, a);
                }
            }
            Effect::SetPosition { window_id, x, y } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(node) = window.node.as_ref()
                {
                    node.set_position(x, y);
                }
            }
            Effect::PlaceTop { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(node) = window.node.as_ref()
                {
                    node.place_top();
                }
            }
            Effect::Close { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.close();
                }
            }
            Effect::FocusWindow { window_id } => {
                // While a layer surface holds exclusive focus, River ignores WMState
                // focus changes, so don't emit them.
                if let Some(seat_id) = self.current_seat
                    && let Some(seat) = self.seats.get(&seat_id)
                    && seat.layer_shell_focus == super::seat::LayerShellFocus::Exclusive
                {
                    return;
                }
                if let Some(seat_id) = self.current_seat
                    && let Some(seat) = self.seats.get(&seat_id)
                    && let Some(window) = self.windows.get(&window_id)
                {
                    // Focus is a River call; make it here in the adapter
                    // rather than delegating to `Seat::focus_window`, so
                    // the core `Seat` stays free of protocol calls.
                    if let (Some(river_seat), Some(river_window)) =
                        (&seat.river_seat, &window.river_window)
                    {
                        river_seat.focus_window(river_window);
                    }
                }
            }
            Effect::SetLayerShellDefault { output_id } => {
                // Must be called during a manage sequence; `apply_manage`
                // produces this effect and the runtime applies it within
                // `ManageStart`. No-op if the output's layer-shell proxy is
                // not yet created (the effect is re-emitted on the next change).
                if let Some(output) = self.outputs.get(&output_id)
                    && let Some(layer_output) = output.river_layer_shell_output.as_ref()
                {
                    layer_output.set_default();
                }
            }
            Effect::StartPointerOp { seat_id } => {
                // Begin an interactive pointer operation. Sent from
                // `apply_manage`; River ignores it unless it is made inside a
                // manage sequence, which is exactly where this effect is applied.
                if let Some(seat) = self.seats.get(&seat_id)
                    && let Some(river_seat) = seat.river_seat.as_ref()
                {
                    river_seat.op_start_pointer();
                }
            }
            Effect::EndPointerOp { seat_id } => {
                // End the interactive pointer operation started by
                // `StartPointerOp`. River keeps the op alive until this is
                // processed, so it is emitted (with `StartPointerOp`) during a
                // manage sequence.
                if let Some(seat) = self.seats.get(&seat_id)
                    && let Some(river_seat) = seat.river_seat.as_ref()
                {
                    river_seat.op_end();
                }
            }
            Effect::InformResizeStart { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.inform_resize_start();
                }
            }
            Effect::InformResizeEnd { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.inform_resize_end();
                }
            }
        }
    }

    /// Ensure a `river_layer_shell_output_v1` child proxy exists for the given
    /// output, creating it on demand.
    ///
    /// This is the adapter-side bookkeeping that issues the River protocol call:
    /// it binds the layer-shell output proxy via `RiverLayerShellV1::get_output`
    /// against the output's underlying `river_output` proxy. It is a no-op when
    /// the layer-shell global is not yet bound, when the output has no underlying
    /// `river_output`, or when a layer-shell output proxy was already created, so
    /// it is safe to call repeatedly (e.g. once per output on global arrival and
    /// once per newly created output).
    pub(super) fn ensure_layer_shell_output(
        &mut self,
        output_id: OutputId,
        qh: &QueueHandle<Self>,
    ) {
        let (Some(shell), Some(output)) =
            (self.layer_shell.as_ref(), self.outputs.get_mut(&output_id))
        else {
            return;
        };
        if output.river_layer_shell_output.is_some() {
            return;
        }
        // Guard against double get_output
        let Some(river_output) = output.river_output.as_ref() else {
            return;
        };
        // Carry OutputId as dispatch user-data so `non_exclusive_area` events
        // can be routed back to the owning output.
        output.river_layer_shell_output = Some(shell.get_output(river_output, qh, output_id));
        // The proxy just became available: if it belongs to the currently
        // focused output, flag the default for re-emission on the next manage
        // (it may now be routable where it previously was not).
        if self.focused_output == Some(output_id) {
            self.layer_shell_default_dirty = true;
        }
    }

    /// Ensure a `river_layer_shell_seat_v1` child proxy exists for the given
    /// seat, creating it on demand.
    ///
    /// Mirrors [`ensure_layer_shell_output`]: issues the River protocol call
    /// `RiverLayerShellV1::get_seat` against the seat's underlying `river_seat`
    /// proxy. It is a no-op when the layer-shell global is not yet bound, when
    /// the seat has no underlying `river_seat`, or when a layer-shell seat proxy
    /// was already created, so it is safe to call repeatedly (e.g. once per seat
    /// on global arrival and once per newly created seat).
    pub(super) fn ensure_layer_shell_seat(&mut self, seat_id: SeatId, qh: &QueueHandle<Self>) {
        let (Some(shell), Some(seat)) = (self.layer_shell.as_ref(), self.seats.get_mut(&seat_id))
        else {
            return;
        };
        if seat.river_layer_shell_seat.is_some() {
            return;
        }
        // Guard against double get_seat.
        let Some(river_seat) = seat.river_seat.as_ref() else {
            return;
        };
        // Carry SeatId as dispatch user-data so `focus_exclusive` events
        // can be routed back to the owning seat.
        seat.river_layer_shell_seat = Some(shell.get_seat(river_seat, qh, seat_id));
    }
}
