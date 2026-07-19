//! Domain events emitted by the adapter layer.
//!
//! These are the pure events that `WMState::handle_event` consumes. They are
//! translated from River protocol events by `handlers.rs`.
use super::output::OutputId;
use super::seat::SeatId;
use super::window::WindowId;

/// A domain event for `WMState` to process.
pub(crate) enum Event {
    WindowCreated {
        window_id: WindowId,
        target_output: OutputId,
    },
    OutputCreated {
        output_id: OutputId,
    },
    SeatCreated {
        seat_id: SeatId,
    },
    WindowClosed {
        window_id: WindowId,
    },
    WindowInteraction {
        window_id: WindowId,
    },
    DimensionsHint {
        window_id: WindowId,
        min_w: i32,
        min_h: i32,
        max_w: i32,
        max_h: i32,
    },
    AppIdUpdated {
        window_id: WindowId,
        app_id: Option<String>,
    },
    TitleUpdated {
        window_id: WindowId,
        title: Option<String>,
    },
    ParentUpdated {
        window_id: WindowId,
        parent_id: Option<WindowId>,
    },
    DecorationHintUpdated {
        window_id: WindowId,
        hint: u32,
    },
    PidUpdated {
        window_id: WindowId,
        pid: u32,
    },
    FullscreenRequested {
        window_id: WindowId,
    },
    ExitFullscreenRequested {
        window_id: WindowId,
    },
    OutputRemoved {
        output_id: OutputId,
    },
    OutputNameUpdated {
        output_id: OutputId,
        name: u32,
    },
    OutputPositionUpdated {
        output_id: OutputId,
        x: i32,
        y: i32,
    },
    OutputDimensionsUpdated {
        output_id: OutputId,
        w: i32,
        h: i32,
    },
    SeatRemoved {
        seat_id: SeatId,
    },
    SeatNameUpdated {
        seat_id: SeatId,
        name: u32,
    },
    SeatPointerPositionUpdated {
        seat_id: SeatId,
        x: i32,
        y: i32,
    },
    SeatLayerShellFocus {
        seat_id: SeatId,
        mode: super::seat::LayerShellFocus,
    },
    /// River requested an interactive pointer move of a window. The window
    /// manager should start an `op_start_pointer` operation and track deltas.
    PointerMoveRequested {
        window_id: WindowId,
        seat_id: SeatId,
    },
    /// River requested an interactive pointer resize of a window on the given
    /// edges. The window manager should start an `op_start_pointer` operation
    /// and transform deltas into a new size.
    PointerResizeRequested {
        window_id: WindowId,
        seat_id: SeatId,
        edges: u32,
    },
    /// Cumulative pointer displacement since the start of the active operation
    /// for this seat. `dx`/`dy` are totals, not increments.
    OpDelta {
        seat_id: SeatId,
        dx: i32,
        dy: i32,
    },
    /// The input driving the active operation was released (all buttons up).
    /// The operation itself is ended explicitly with `EndPointerOp`.
    OpRelease {
        seat_id: SeatId,
    },
}
