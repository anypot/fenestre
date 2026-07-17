//! River seat tracking.

use crate::protocol::river::river_layer_shell_v1::client::river_layer_shell_seat_v1::RiverLayerShellSeatV1;
use crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1;

/// Stable identifier for a seat owned by `WMState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct SeatId(pub u32);

/// Which kind of layer-shell keyboard focus is active for this seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LayerShellFocus {
    /// No layer surface has focus; the WM controls focus normally.
    #[default]
    None,
    /// A layer surface has non-exclusive focus; WM may still move focus.
    NonExclusive,
    /// A layer surface has exclusive focus; WM focus changes are ignored.
    Exclusive,
}

/// Runtime state for a River seat.
pub(super) struct Seat {
    /// River seat protocol proxy.
    pub(super) river_seat: Option<RiverSeatV1>,

    /// Which kind of layer-shell keyboard focus is active for this seat.
    pub(super) layer_shell_focus: LayerShellFocus,

    /// River layer shell seat protocol proxy.
    pub(super) river_layer_shell_seat: Option<RiverLayerShellSeatV1>,

    /// River wl_seat global name.
    pub(super) wl_seat_name: u32,

    /// Last known pointer position for this seat. Reserved for the planned
    /// pointer-driven move/resize feature (not yet implemented); currently
    /// written on every `SeatPointerPositionUpdated` but never read.
    #[allow(dead_code)]
    pub(super) pointer_position: Option<(i32, i32)>,
}

impl Seat {
    /// Create a new seat record.
    pub(super) fn new() -> Self {
        Self {
            river_seat: None,
            layer_shell_focus: LayerShellFocus::None,
            river_layer_shell_seat: None,
            wl_seat_name: 0,
            pointer_position: None,
        }
    }
}
