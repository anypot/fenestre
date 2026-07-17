//! Internal command representation for Fenestre.
//!
//! A `Command` is not a public IPC API. It is the internal dispatch enum used
//! by config-loaded keybindings and `WMState` command handling.
use crate::layout::FocusDirection;

/// Internal action triggered by a keybinding or future command source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// Move focus to the next window.
    FocusNext,

    /// Move focus to the previous window.
    FocusPrevious,

    /// Move focus upward.
    FocusUp,

    /// Move focus downward.
    FocusDown,

    /// Move focus left.
    FocusLeft,

    /// Move focus right.
    FocusRight,

    /// Choose a split direction is planned.
    #[allow(dead_code)]
    /// Split the focused container vertically.
    SplitVertical,

    /// Choose a split direction is planned.
    #[allow(dead_code)]
    /// Split the focused container horizontally.
    SplitHorizontal,

    /// Toggle fullscreen for the focused window.
    ToggleFullscreen,

    /// Toggle floating state for the focused window.
    ToggleFloating,

    /// Toggle pseudo-tiled state for the focused window.
    TogglePseudoTiled,

    /// Set the focused window to tiled state.
    SetTiled,

    /// Spawn a external program with optional arguments.
    Spawn { program: String, args: Vec<String> },

    /// Exit the River Wayland session.
    ExitRiver,

    /// Reload the active configuration file.
    ReloadConfig,

    /// Close the currently focused window.
    CloseFocused,

    /// Move focus to the output to the left.
    FocusOutputLeft,

    /// Move focus to the output to the right.
    FocusOutputRight,

    /// Move focus to the output above.
    FocusOutputUp,

    /// Move focus to the output below.
    FocusOutputDown,

    /// Move the focused window left.
    MoveLeft,

    /// Move the focused window right.
    MoveRight,

    /// Move the focused window up.
    MoveUp,

    /// Move the focused window down.
    MoveDown,

    /// Expand the focused window's size in the given direction.
    ResizeExpand { direction: FocusDirection },

    /// Shrink the focused window's size in the given direction.
    ResizeShrink { direction: FocusDirection },
}
