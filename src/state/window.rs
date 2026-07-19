//! River window tracking.
use crate::layout::Rect;
use crate::protocol::river::river_window_management_v1::client::river_node_v1::RiverNodeV1;
use crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1;

use super::output::OutputId;

/// Stable identifier for a window owned by `WMState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct WindowId(pub u32);

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

    /// Current size hints.
    pub(super) dimensions_hint: DimensionsHint,

    /// Last decoration hint from the protocol.
    ///
    /// `0` = server-side decorations (compositor borders)
    /// `1` = client-side decorations
    /// `2` or `None` = no preference, fall back to global `decorations` config
    pub(super) decoration_hint: Option<u32>,
}

/// Minimum size (logical pixels) for a floating/pseudo-tiled window when the
/// window has not reported its own dimensions, so a tiny output does not
/// produce an unusably small window. Capped to the output size so it never
/// overflows a tiny output.
const MIN_FLOAT_SIZE: i32 = 320;

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
            dimensions_hint: DimensionsHint::default(),
            decoration_hint: None,
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

    /// Record the app-provided size constraints reported by the compositor.
    ///
    /// These are the window's genuine min/max preferences and are used to clamp
    /// the ratio-based default size in `preferred_dimensions`.
    pub(super) fn set_dimensions_hint(
        &mut self,
        min_width: i32,
        min_height: i32,
        max_width: i32,
        max_height: i32,
    ) {
        self.dimensions_hint = DimensionsHint {
            min_width,
            min_height,
            max_width,
            max_height,
        };
    }

    /// Compute preferred window dimensions for a floating / pseudo-tiled window.
    ///
    /// The base size is always `ratio` (the `default_float_ratio` config field)
    /// of `fallback`, with a minimum floor to keep windows usable on tiny
    /// outputs. The size the compositor imposes on a window via tiling/fullscreen
    /// is deliberately ignored, so it cannot leak a tiling-slot or fullscreen
    /// size into the default. The app's genuine request comes only through its
    /// size hints (`dimensions_hint`), which clamp the ratio-based base below.
    pub(super) fn preferred_dimensions(&self, fallback: Rect, ratio: f32) -> (i32, i32) {
        let size_for = |fallback_dim: i32| -> i32 {
            let raw = (fallback_dim as f32 * ratio).round() as i32;
            // At least MIN_FLOAT_SIZE, but never larger than the output itself
            // so a window cannot overflow a tiny output.
            raw.max(MIN_FLOAT_SIZE).min(fallback_dim.max(1))
        };

        let mut width = size_for(fallback.width).max(1);
        let mut height = size_for(fallback.height).max(1);

        (width, height) = self.apply_dimensions_hint(width, height);

        (width.max(1), height.max(1))
    }

    /// Compute the rectangle for a pseudo-tiled window based on its preferred dimensions.
    ///
    /// The window is centered within `self.layout_rect` when it has already been
    /// arranged (i.e. `layout_rect` is set); otherwise it is centered within
    /// `fallback_rect`. `layout_rect` takes precedence, so `fallback_rect` only
    /// applies to windows that have not yet been arranged. `ratio` is the
    /// default-size fraction used when the window has not reported its dimensions.
    pub(super) fn pseudo_tiled_rect(&self, fallback_rect: Rect, ratio: f32) -> Rect {
        // Size is a fraction of the destination output (`fallback_rect`) so the
        // ratio is not re-applied to an already-arranged `layout_rect`, which
        // would shrink a previously sized pseudo/floating window on every toggle
        // or output reassignment. `layout_rect` is used only for centering.
        let (width, height) = self.preferred_dimensions(fallback_rect, ratio);
        let size = Rect::new(0, 0, width, height);
        let center_in = self.layout_rect.unwrap_or(fallback_rect);
        crate::layout::capped_rect(center_in, size)
    }

    /// Clamp a proposed width/height to the window's size constraints.
    ///
    /// Used during an interactive resize to keep the window within its
    /// app-reported `dimensions_hint` (`[min, max]` on each axis). A zero hint
    /// on an axis means "no constraint" on that axis, matching the wire protocol
    /// semantics used by River. The result is always at least 1 on each axis.
    pub(super) fn clamp_dimensions(&self, width: i32, height: i32) -> (i32, i32) {
        let (w, h) = self.apply_dimensions_hint(width, height);
        (w.max(1), h.max(1))
    }

    /// Apply the window's `dimensions_hint` (`[min, max]` per axis) to a proposed
    /// size. A zero hint on an axis means "no constraint" there, matching River's
    /// wire protocol semantics. Each axis is floored to at least 1 by callers via
    /// a final `.max(1)`. Shared by `preferred_dimensions` and `clamp_dimensions`
    /// so the hint rules live in exactly one place.
    fn apply_dimensions_hint(&self, width: i32, height: i32) -> (i32, i32) {
        let hint = &self.dimensions_hint;
        let mut w = width.max(1);
        let mut h = height.max(1);
        if hint.min_width > 0 {
            w = w.max(hint.min_width);
        }
        if hint.min_height > 0 {
            h = h.max(hint.min_height);
        }
        if hint.max_width > 0 {
            w = w.min(hint.max_width);
        }
        if hint.max_height > 0 {
            h = h.min(hint.max_height);
        }
        (w, h)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_dimensions_ratio_scales_fallback() {
        let window = Window::new(WindowId(1), OutputId(1));
        let rect = Rect::new(0, 0, 1000, 800);
        let (w, h) = window.preferred_dimensions(rect, 0.5);
        assert_eq!((w, h), (500, 400));
    }

    #[test]
    fn preferred_dimensions_min_floor_on_tiny_output() {
        let window = Window::new(WindowId(1), OutputId(1));
        // 200x200 fallback with 0.5 ratio would yield 100x100, but MIN_FLOAT_SIZE=320
        // floors it. The cap also limits to the fallback size.
        let rect = Rect::new(0, 0, 200, 200);
        let (w, h) = window.preferred_dimensions(rect, 0.5);
        assert_eq!((w, h), (200, 200));
    }

    #[test]
    fn preferred_dimensions_min_width_hint_overrides_ratio() {
        let mut window = Window::new(WindowId(1), OutputId(1));
        window.dimensions_hint.min_width = 500;
        // 0.1 ratio on 1000-wide fallback gives 100, but min_width forces 500.
        let rect = Rect::new(0, 0, 1000, 800);
        let (w, _h) = window.preferred_dimensions(rect, 0.1);
        assert_eq!(w, 500);
    }

    #[test]
    fn preferred_dimensions_max_width_hint_caps_ratio() {
        let mut window = Window::new(WindowId(1), OutputId(1));
        window.dimensions_hint.max_width = 200;
        // 0.5 ratio on 1000-wide fallback gives 500, but max_width caps at 200.
        let rect = Rect::new(0, 0, 1000, 800);
        let (w, _h) = window.preferred_dimensions(rect, 0.5);
        assert_eq!(w, 200);
    }

    #[test]
    fn preferred_dimensions_ratio_one_uses_full_fallback() {
        let window = Window::new(WindowId(1), OutputId(1));
        let rect = Rect::new(0, 0, 800, 600);
        let (w, h) = window.preferred_dimensions(rect, 1.0);
        assert_eq!((w, h), (800, 600));
    }

    #[test]
    fn preferred_dimensions_ratio_zero_hits_min_floor() {
        let window = Window::new(WindowId(1), OutputId(1));
        let rect = Rect::new(0, 0, 1000, 800);
        let (w, h) = window.preferred_dimensions(rect, 0.0);
        assert_eq!((w, h), (320, 320));
    }

    #[test]
    fn pseudo_tiled_rect_centers_in_fallback() {
        let window = Window::new(WindowId(1), OutputId(1));
        let rect = Rect::new(0, 0, 1000, 800);
        let r = window.pseudo_tiled_rect(rect, 0.5);
        // 500x400 window centered in 1000x800 slot.
        assert_eq!(r, Rect::new(250, 200, 500, 400));
    }

    #[test]
    fn pseudo_tiled_rect_centers_in_layout_rect() {
        let mut window = Window::new(WindowId(1), OutputId(1));
        window.set_layout_rect(Rect::new(100, 100, 800, 600));
        let fallback = Rect::new(0, 0, 1000, 800);
        let r = window.pseudo_tiled_rect(fallback, 0.5);
        // Ratio-sized 500x400 (from the output fallback), centered inside
        // layout_rect (which is used only for positioning).
        assert_eq!(r, Rect::new(250, 200, 500, 400));
    }

    #[test]
    fn preferred_dimensions_hints_clamp_ratio_base() {
        // A window that reports genuine size hints has the ratio-based base
        // clamped into [min, max] on each axis.
        let mut window = Window::new(WindowId(1), OutputId(1));
        window.set_dimensions_hint(600, 0, 0, 300);
        let rect = Rect::new(0, 0, 1000, 800);
        // width: base 500 -> min 600 lifts it to 600.
        // height: base 400 -> max 300 caps it to 300.
        let (w, h) = window.preferred_dimensions(rect, 0.5);
        assert_eq!((w, h), (600, 300));
    }

    #[test]
    fn pseudo_tiled_rect_caps_size_to_slot() {
        let window = Window::new(WindowId(1), OutputId(1));
        // ratio=1.0 on 300x200 fallback gives 300x200, capped_rect keeps it.
        let rect = Rect::new(0, 0, 300, 200);
        let r = window.pseudo_tiled_rect(rect, 1.0);
        assert_eq!(r, Rect::new(0, 0, 300, 200));
    }

    #[test]
    fn pseudo_tiled_rect_does_not_rescale_arranged_window() {
        // A window that has already been arranged (layout_rect set from a
        // previous manage cycle) must keep its established size when toggled
        // or reassigned; the ratio must size from the OUTPUT rect, not be
        // re-applied to the already-ratio-scaled layout_rect.
        let mut window = Window::new(WindowId(1), OutputId(1));
        // 500x400 is exactly ratio(0.5) * output(1000x800) — the expected size.
        window.set_layout_rect(Rect::new(100, 100, 500, 400));
        let output = Rect::new(0, 0, 1000, 800);
        let r = window.pseudo_tiled_rect(output, 0.5);
        // Size must remain 500x400 (not shrink to the MIN_FLOAT_SIZE floor).
        assert_eq!((r.width, r.height), (500, 400));
    }
}
