//! Built-in default configuration.
//!
//! Defaults provide a usable baseline when no user config is supplied.
//! User config is merged over these defaults by keybinding identity.
use super::KeyBindingTarget;
use super::{Config, KeyBindingConfig, LayoutConfig};
use crate::command::Command;
use xkbcommon::xkb::keysyms::*;

const SUPER: u32 = 64;
const SHIFT: u32 = 1;
const CTRL: u32 = 4;
const ALT: u32 = 8;

fn binding(
    target: KeyBindingTarget,
    keysym: u32,
    modifiers: u32,
    command: Command,
) -> KeyBindingConfig {
    KeyBindingConfig {
        target,
        keysym,
        modifiers,
        command,
    }
}

/// Return Fenestre's built-in default configuration.
pub fn defaults() -> Config {
    let decorations = true;
    Config {
        layout: LayoutConfig {
            preview_border_color: Some(0xff00ff00),
            preview_border_width: Some(2),
            ..LayoutConfig::default()
        },
        decorations,
        border_width: None,
        border_color_focused: Some(0xffffffff),
        border_color_unfocused: Some(0xffffffff),
        resize_delta_ratio: None,
        resize_delta_percent: None,
        keybindings: vec![
            binding(
                KeyBindingTarget::Primary,
                KEY_Return,
                SUPER,
                Command::Spawn {
                    program: "foot".to_string(),
                    args: Vec::new(),
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_q,
                SUPER,
                Command::CloseFocused,
            ),
            binding(KeyBindingTarget::Primary, KEY_h, SUPER, Command::FocusLeft),
            binding(KeyBindingTarget::Primary, KEY_j, SUPER, Command::FocusDown),
            binding(KeyBindingTarget::Primary, KEY_k, SUPER, Command::FocusUp),
            binding(KeyBindingTarget::Primary, KEY_l, SUPER, Command::FocusRight),
            binding(KeyBindingTarget::Primary, KEY_t, SUPER, Command::SetTiled),
            binding(
                KeyBindingTarget::Primary,
                KEY_Tab,
                SUPER,
                Command::FocusNext,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_Tab,
                SUPER | SHIFT,
                Command::FocusPrevious,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_s,
                SUPER,
                Command::ToggleFloating,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_f,
                SUPER,
                Command::ToggleFullscreen,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_t,
                SUPER | SHIFT,
                Command::TogglePseudoTiled,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_r,
                SUPER | SHIFT,
                Command::ReloadConfig,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_e,
                SUPER | SHIFT,
                Command::ExitRiver,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_q,
                SUPER | SHIFT,
                Command::CloseFocused,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_h,
                SUPER | SHIFT,
                Command::MoveLeft,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_j,
                SUPER | SHIFT,
                Command::MoveDown,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_k,
                SUPER | SHIFT,
                Command::MoveUp,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_l,
                SUPER | SHIFT,
                Command::MoveRight,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_h,
                SUPER | ALT,
                Command::ResizeExpand {
                    direction: crate::layout::FocusDirection::Left,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_j,
                SUPER | ALT,
                Command::ResizeExpand {
                    direction: crate::layout::FocusDirection::Down,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_k,
                SUPER | ALT,
                Command::ResizeExpand {
                    direction: crate::layout::FocusDirection::Up,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_l,
                SUPER | ALT,
                Command::ResizeExpand {
                    direction: crate::layout::FocusDirection::Right,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_h,
                SUPER | SHIFT | ALT,
                Command::ResizeShrink {
                    direction: crate::layout::FocusDirection::Left,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_j,
                SUPER | SHIFT | ALT,
                Command::ResizeShrink {
                    direction: crate::layout::FocusDirection::Down,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_k,
                SUPER | SHIFT | ALT,
                Command::ResizeShrink {
                    direction: crate::layout::FocusDirection::Up,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_l,
                SUPER | SHIFT | ALT,
                Command::ResizeShrink {
                    direction: crate::layout::FocusDirection::Right,
                },
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_h,
                SUPER | CTRL,
                Command::FocusOutputLeft,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_j,
                SUPER | CTRL,
                Command::FocusOutputDown,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_k,
                SUPER | CTRL,
                Command::FocusOutputUp,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_l,
                SUPER | CTRL,
                Command::FocusOutputRight,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_v,
                SUPER,
                Command::TogglePendingSplitVertical,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_v,
                SUPER | SHIFT,
                Command::TogglePendingSplitHorizontal,
            ),
            binding(
                KeyBindingTarget::Primary,
                KEY_Escape,
                SUPER,
                Command::CancelPendingSplit,
            ),
        ],
        rules: Vec::new(),
    }
}
