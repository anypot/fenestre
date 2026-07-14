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
    pub(super) fn apply_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            self.apply_effect(effect);
        }
    }

    /// Remove an output: dispatch core event and clean the proxy index.
    pub(super) fn remove_output(
        &mut self,
        output_id: OutputId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1,
    ) {
        self.handle_event(super::events::Event::OutputRemoved { output_id });
        self.outputs_by_proxy.remove(proxy);
    }

    /// Remove a seat: dispatch core event and clean the proxy index.
    pub(super) fn remove_seat(
        &mut self,
        seat_id: SeatId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1,
    ) {
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

    fn apply_effect(&self, effect: Effect) {
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
                if let Some(seat_id) = self.current_seat
                    && let Some(seat) = self.seats.get(&seat_id)
                    && let Some(window) = self.windows.get(&window_id)
                {
                    seat.focus_window(window);
                }
            }
        }
    }
}
