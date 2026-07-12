//! River window tracking.
#![allow(dead_code)]

use crate::layout::Rect;
use crate::protocol::river::river_window_management_v1::client::river_node_v1::RiverNodeV1;
use crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1;

use super::output::OutputId;

/// Stable identifier for a window owned by `WMState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct WindowId(pub u32);

/// Logical window position.
#[derive(Default)]
pub(super) struct Position {
    /// X coordinate.
    pub(super) x: i32,

    /// Y coordinate.
    pub(super) y: i32,
}
/// Window dimensions.
#[derive(Default)]
pub(super) struct Dimensions {
    /// Width in logical pixels.
    pub(super) width: i32,

    /// Height in logical pixels.
    pub(super) height: i32,
}

/// River-provided size constraints.
#[derive(Default)]
pub(super) struct DimensionsHint {
    /// Minimum width.
    pub(super) min_width: i32,

    /// Minimum height.
    pub(super) min_height: i32,

    /// Maximum width.
    pub(super) max_width: i32,

    /// Maximum height.
    pub(super) max_height: i32,
}

/// Window layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowMode {
    /// Window participates in tiling.
    Tiled,

    /// Window is pseudo-tiled.
    PseudoTiled,

    /// Window is floating.
    Floating {
        /// X coordinate.
        x: i32,

        /// Y coordinate.
        y: i32,

        /// Width in logical pixels.
        width: i32,

        /// Height in logical pixels.
        height: i32,
    },

    /// Window is fullscreen.
    Fullscreen,
}

/// Runtime state for a River window.
pub(super) struct Window {
    /// Internal window identifier.
    pub(super) id: WindowId,

    /// Output on which this window lives.
    pub(super) output_id: OutputId,

    /// River window protocol proxy.
    pub(super) river_window: Option<RiverWindowV1>,

    /// Parent window, if any.
    pub(super) parent: Option<WindowId>,

    /// Application ID reported by River.
    pub(super) app_id: Option<String>,

    /// Window title reported by River.
    pub(super) title: Option<String>,

    /// Whether window rules have been applied to this window.
    pub(super) rules_applied: bool,

    /// Process ID reported by River.
    pub(super) pid: i32,

    /// Render-list node for this window.
    pub(super) node: Option<RiverNodeV1>,

    /// Desired layout rectangle.
    pub(super) layout_rect: Option<Rect>,

    /// Current layout mode.
    pub(super) mode: WindowMode,

    /// Current position.
    pub(super) position: Position,

    /// Current dimensions.
    pub(super) dimensions: Dimensions,

    /// Current size hints.
    pub(super) dimensions_hint: DimensionsHint,

    /// Last decoration hint from the protocol.
    ///
    /// `0` = server-side decorations (compositor borders)
    /// `1` = client-side decorations
    /// `2` or `None` = no preference, fall back to global `decorations` config
    pub(super) decoration_hint: Option<u32>,

    /// Last-sent border state for this window, used to skip redundant
    /// `set_borders` calls when nothing has changed.
    pub(super) last_border: Option<(i32, u32)>,
}

impl Window {
    /// Create a new window record.
    pub(super) fn new(id: WindowId, output_id: OutputId) -> Self {
        Self {
            id,
            output_id,
            river_window: None,
            node: None,
            layout_rect: None,
            parent: None,
            app_id: None,
            title: None,
            rules_applied: false,
            pid: 0,
            mode: WindowMode::Tiled,
            position: Position::default(),
            dimensions: Dimensions::default(),
            dimensions_hint: DimensionsHint::default(),
            decoration_hint: None,
            last_border: None,
        }
    }

    /// Update this window's desired layout rectangle.
    pub(super) fn set_layout_rect(&mut self, rect: Rect) {
        self.layout_rect = Some(rect);
    }

    /// Update this window's output id.
    pub(super) fn set_output_id(&mut self, output_id: OutputId) {
        self.output_id = output_id;
    }

    /// Update this window's position.
    pub(super) fn set_position(&mut self, x: i32, y: i32) {
        self.position.x = x;
        self.position.y = y;
    }

    /// Update this window's dimensions.
    pub(super) fn set_dimensions(&mut self, width: i32, height: i32) {
        self.dimensions.width = width;
        self.dimensions.height = height;
    }

    /// Compute preferred window dimensions based on explicit dimensions and hints.
    pub(super) fn preferred_dimensions(&self, fallback: Rect) -> (i32, i32) {
        let mut width = if self.dimensions.width > 0 {
            self.dimensions.width
        } else {
            fallback.width
        }
        .max(1);
        let mut height = if self.dimensions.height > 0 {
            self.dimensions.height
        } else {
            fallback.height
        }
        .max(1);

        if self.dimensions_hint.min_width > 0 {
            width = width.max(self.dimensions_hint.min_width);
        }
        if self.dimensions_hint.min_height > 0 {
            height = height.max(self.dimensions_hint.min_height);
        }
        if self.dimensions_hint.max_width > 0 {
            width = width.min(self.dimensions_hint.max_width);
        }
        if self.dimensions_hint.max_height > 0 {
            height = height.min(self.dimensions_hint.max_height);
        }

        (width.max(1), height.max(1))
    }

    /// Compute the rectangle for a pseudo-tiled window based on its preferred dimensions.
    ///
    /// The window is centered within `self.layout_rect` when it has already been
    /// arranged (i.e. `layout_rect` is set); otherwise it is centered within
    /// `fallback_rect`. `layout_rect` takes precedence, so `fallback_rect` only
    /// applies to windows that have not yet been arranged.
    pub(super) fn pseudo_tiled_rect(&self, fallback_rect: Rect) -> Option<Rect> {
        let fallback = self.layout_rect.unwrap_or(fallback_rect);
        let (width, height) = self.preferred_dimensions(fallback);
        let size = Rect::new(0, 0, width, height);
        Some(crate::layout::capped_rect(fallback, size))
    }

    /// Determine whether this window should use client-side decorations.
    ///
    /// The River protocol sends `DecorationHint` events where:
    /// - `0` means the window prefers server-side (compositor) decorations
    /// - `1` means the window prefers client-side decorations
    /// - `2` or missing means no preference, so we fall back to the global
    ///   `decorations` config value.
    pub(super) fn use_client_decorations(&self, fallback_decorations: bool) -> bool {
        match self.decoration_hint {
            Some(1) => true,
            Some(0) => false,
            _ => fallback_decorations,
        }
    }
}
