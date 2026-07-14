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
}
