//! River seat tracking.

use crate::layout::Rect;
use crate::protocol::river::river_layer_shell_v1::client::river_layer_shell_seat_v1::RiverLayerShellSeatV1;
use crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1;
use crate::state::window::WindowId;

/// Stable identifier for a seat owned by `WMState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct SeatId(pub u32);

/// State of an interactive pointer operation driven by River's
/// `op_start_pointer` / `op_delta` / `op_release` / `op_end` protocol flow.
///
/// The operation is started in response to a `PointerMoveRequested` /
/// `PointerResizeRequested` River event and remains `Inactive` until then. While
/// active, River sends cumulative `op_delta` events (`dx`, `dy` are totals since
/// the op started, not increments) that the WM uses to reposition or resize the
/// window. The operation is ended explicitly via `op_end` (sent after the
/// `OpRelease` event and any follow-up snapping).
pub(super) enum InteractiveOp {
    /// No interactive pointer operation is in progress for this seat.
    Inactive,
    /// The window is being moved; `initial_rect` is its geometry when the op
    /// started and `dx`/`dy` are applied to it as a translation.
    Move {
        window_id: WindowId,
        initial_rect: Rect,
    },
    /// The window is being resized; `edges` is the River `edges` bitmask
    /// describing which borders are grabbed, and `initial_rect` is the geometry
    /// when the op started.
    Resize {
        window_id: WindowId,
        edges: u32,
        initial_rect: Rect,
    },
}

impl InteractiveOp {
    /// The window this op targets, if any.
    pub(super) fn window_id(&self) -> Option<WindowId> {
        match self {
            InteractiveOp::Inactive => None,
            InteractiveOp::Move { window_id, .. } | InteractiveOp::Resize { window_id, .. } => {
                Some(*window_id)
            }
        }
    }

    /// Whether an operation of any kind is active.
    pub(super) fn is_active(&self) -> bool {
        !matches!(self, InteractiveOp::Inactive)
    }
}

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

    /// Last known pointer position for this seat. Written on every
    /// `SeatPointerPositionUpdated` and read at the start of an interactive
    /// move/resize to record the window's initial geometry.
    pub(super) pointer_position: Option<(i32, i32)>,

    /// Active interactive pointer operation for this seat, if any.
    pub(super) op: InteractiveOp,
    /// Whether `op_start_pointer` has been sent for the current operation. The
    /// op is recorded on `PointerMoveRequested` / `PointerResizeRequested`, but
    /// `op_start_pointer` may only be called during a manage sequence, so this
    /// flag defers the protocol call to the next `apply_manage`.
    pub(super) op_started: bool,
    /// Whether `op_end` should be sent on the next manage sequence. Set by the
    /// `OpRelease` event; the operation stays active until `op_end` is processed
    /// so that any trailing `op_delta` events are still applied.
    pub(super) op_ending: bool,

    /// Floating rect pending application to the layout tree. Written by the
    /// `OpDelta` event handler (which may fire many times per drag) and applied
    /// once inside `apply_manage`, so the tree is only touched once per manage
    /// sequence rather than on every pointer-motion delta.
    pub(super) pending_float: Option<(WindowId, Rect)>,
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
            op: InteractiveOp::Inactive,
            op_started: false,
            op_ending: false,
            pending_float: None,
        }
    }
}
