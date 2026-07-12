//! River output tracking.
#![allow(dead_code)]

use crate::layout::Rect;
use crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1;

/// Stable identifier for an output owned by `WMState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OutputId(pub u32);

/// Runtime state for a River output.
pub(super) struct Output {
    /// Internal output identifier.
    pub(super) id: OutputId,

    /// River output protocol proxy.
    pub(super) river_output: Option<RiverOutputV1>,

    /// River wl_output global name.
    pub(super) wl_output_name: u32,

    /// Logical output position.
    pub(super) position: Option<(i32, i32)>,

    /// Logical output dimensions.
    pub(super) dimensions: Option<super::window::Dimensions>,
}

impl Output {
    /// Create a new output record.
    pub(super) fn new(id: OutputId) -> Self {
        Self {
            id,
            river_output: None,
            wl_output_name: 0,
            position: None,
            dimensions: None,
        }
    }

    /// Update this output's logical position.
    pub(super) fn set_position(&mut self, x: i32, y: i32) {
        self.position = Some((x, y));
    }

    /// Update this output's logical dimensions.
    pub(super) fn set_dimensions(&mut self, width: i32, height: i32) {
        self.dimensions = Some(super::window::Dimensions { width, height });
    }

    /// Return this output's rectangle, or `None` if dimensions are not yet known.
    pub(super) fn rect(&self) -> Option<Rect> {
        let (x, y) = self.position.unwrap_or((0, 0));
        self.dimensions
            .as_ref()
            .map(|d| Rect::new(x, y, d.width, d.height))
    }
}
