//! River output tracking.
#![allow(dead_code)]

use crate::layout::Rect;
use crate::protocol::river::river_layer_shell_v1::client::river_layer_shell_output_v1::RiverLayerShellOutputV1;
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

    /// River layer shell output protocol proxy.
    pub(super) river_layer_shell_output: Option<RiverLayerShellOutputV1>,

    /// River wl_output global name.
    pub(super) wl_output_name: u32,

    /// Logical output position.
    pub(super) position: Option<(i32, i32)>,

    /// Logical output dimensions.
    pub(super) dimensions: Option<super::window::Dimensions>,

    /// Usable tiling area after subtracting layer-shell exclusive zones
    /// (panels/bars), in global coordinates. `None` until River reports it via
    /// `river_layer_shell_output_v1.non_exclusive_area`.
    pub(super) non_exclusive_area: Option<Rect>,
}

impl Output {
    /// Create a new output record.
    pub(super) fn new(id: OutputId) -> Self {
        Self {
            id,
            river_output: None,
            river_layer_shell_output: None,
            wl_output_name: 0,
            position: None,
            dimensions: None,
            non_exclusive_area: None,
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

    /// Update this output's layer-shell non-exclusive area: the region left
    /// after subtracting panel/bar exclusive zones. Coordinates are global,
    /// matching [`Output::rect`] and the layout tree's coordinate space.
    pub(super) fn set_non_exclusive_area(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.non_exclusive_area = Some(Rect::new(x, y, width, height));
    }

    /// Rectangle to use for tiling: the layer-shell non-exclusive area if known
    /// (so tiled windows don't overlap panels), otherwise the full output rect.
    /// `None` until geometry is known.
    ///
    /// Fullscreen windows are unaffected: they are driven by
    /// `river_window_v1.fullscreen` against the output proxy, not by the tree's
    /// output rect, so they still cover the whole physical output.
    pub(super) fn tiling_rect(&self) -> Option<Rect> {
        self.non_exclusive_area.or_else(|| self.rect())
    }

    /// Return this output's rectangle, or `None` if dimensions are not yet known.
    pub(super) fn rect(&self) -> Option<Rect> {
        let (x, y) = self.position.unwrap_or((0, 0));
        self.dimensions
            .as_ref()
            .map(|d| Rect::new(x, y, d.width, d.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiling_rect_is_none_without_geometry() {
        let output = Output::new(OutputId(0));
        assert_eq!(output.tiling_rect(), None);
    }

    #[test]
    fn tiling_rect_falls_back_to_full_output_rect() {
        let mut output = Output::new(OutputId(0));
        output.set_position(10, 20);
        output.set_dimensions(1920, 1080);
        assert_eq!(output.tiling_rect(), Some(Rect::new(10, 20, 1920, 1080)));
    }

    #[test]
    fn tiling_rect_prefers_non_exclusive_area_over_full_rect() {
        let mut output = Output::new(OutputId(0));
        output.set_position(0, 0);
        output.set_dimensions(1920, 1080);
        // A 40px top bar: usable area starts lower and is shorter.
        output.set_non_exclusive_area(0, 40, 1920, 1040);
        assert_eq!(output.tiling_rect(), Some(Rect::new(0, 40, 1920, 1040)));
    }

    #[test]
    fn tiling_rect_uses_non_exclusive_area_even_before_dimensions() {
        let mut output = Output::new(OutputId(0));
        output.set_non_exclusive_area(0, 40, 1920, 1040);
        assert_eq!(output.tiling_rect(), Some(Rect::new(0, 40, 1920, 1040)));
    }
}
