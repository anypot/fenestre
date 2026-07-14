//! Window-level River protocol effects collected during manage/render cycles.
//!
//! Each variant carries the data needed to apply the corresponding protocol
//! call after state mutation is complete. Child-object creation (`get_node`)
//! and `event_created_child` proxy lifecycle remain inline.
use super::output::OutputId;
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
}
