//! Window-level River protocol effects collected during manage/render cycles.
//!
//! Each variant carries the data needed to apply the corresponding protocol
//! call after state mutation is complete. Child-object creation (`get_node`)
//! is routed through [`Effect::EnsureNode`] and applied by the adapter
//! (it needs the `QueueHandle`); `event_created_child` proxy lifecycle
//! remains inline at the handler/adapter boundary.
use super::output::OutputId;
use super::seat::SeatId;
use super::window::WindowId;

/// Bitmask requesting all four border edges (top | bottom | left | right).
pub(crate) const ALL_EDGES: u32 = 0b1111;

/// A deferred window-level River protocol call.
pub(crate) enum Effect {
    ProposeDimensions {
        window_id: WindowId,
        width: i32,
        height: i32,
    },
    Fullscreen {
        window_id: WindowId,
        output_id: OutputId,
    },
    ExitFullscreen {
        window_id: WindowId,
    },
    UseSsd {
        window_id: WindowId,
    },
    /// Ensure the River node child object exists for the window.
    ///
    /// Routes the `get_node` child-object creation (which requires the
    /// `QueueHandle`) through the adapter instead of being called
    /// inline in the core. The adapter stores the resulting proxy on
    /// `Window::node` for the later `SetPosition` / `PlaceTop` effects.
    EnsureNode {
        window_id: WindowId,
    },
    SetBorders {
        window_id: WindowId,
        edges: u32,
        width: i32,
        r: u32,
        g: u32,
        b: u32,
        a: u32,
    },
    SetPosition {
        window_id: WindowId,
        x: i32,
        y: i32,
    },
    PlaceTop {
        window_id: WindowId,
    },
    Close {
        window_id: WindowId,
    },
    FocusWindow {
        window_id: WindowId,
    },
    /// Mark an output as the layer-shell default for new layer surfaces that do
    /// not request a specific output (e.g. launchers). Overrides any previous
    /// `set_default` on any `river_layer_shell_output_v1` object, so it is
    /// emitted only when the chosen default output changes. Must be issued
    /// during a manage sequence, so it is produced by `apply_manage`.
    SetLayerShellDefault {
        output_id: OutputId,
    },
    /// Start an interactive pointer operation for the seat. Must be issued
    /// during a manage sequence; `apply_manage` produces it when a seat's
    /// pending operation is first activated.
    StartPointerOp {
        seat_id: SeatId,
    },
    /// End the interactive pointer operation for the seat. Must be issued during
    /// a manage sequence (River keeps the op alive until `op_end` is processed).
    EndPointerOp {
        seat_id: SeatId,
    },
    /// Inform a window that an interactive resize has begun. Must be issued
    /// during a manage sequence; `apply_manage` produces it when a resize op
    /// transitions from pending to active.
    InformResizeStart {
        window_id: WindowId,
    },
    /// Inform a window that an interactive resize has ended. Must be issued
    /// during a manage sequence; `apply_manage` produces it when a resize op
    /// is ended via `op_end`.
    InformResizeEnd {
        window_id: WindowId,
    },
}
