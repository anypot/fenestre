//! River seat tracking.
#![allow(dead_code)]

use crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1;

/// Stable identifier for a seat owned by `WMState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct SeatId(pub u32);

/// Runtime state for a River seat.
pub(super) struct Seat {
    /// Internal seat identifier.
    pub(super) id: SeatId,

    /// River seat protocol proxy.
    pub(super) river_seat: Option<RiverSeatV1>,

    /// River wl_seat global name.
    pub(super) wl_seat_name: u32,

    /// Last known pointer position for this seat.
    pub(super) pointer_position: Option<(i32, i32)>,
}

impl Seat {
    /// Create a new seat record.
    pub(super) fn new(id: SeatId) -> Self {
        Self {
            id,
            river_seat: None,
            wl_seat_name: 0,
            pointer_position: None,
        }
    }

    /// Focus a window, returning `true` only if the seat actually issued the
    /// River focus request (i.e. both the seat proxy and the window proxy are
    /// present). Callers use this to know whether the focus was applied.
    pub(super) fn focus_window(&self, window: &super::window::Window) -> bool {
        if let (Some(river_seat), Some(river_window)) = (&self.river_seat, &window.river_window) {
            river_seat.focus_window(river_window);
            true
        } else {
            false
        }
    }
}
