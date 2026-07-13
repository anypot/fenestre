//! Format-neutral parser for shared keybinding config.
//!
//! This module does not know about Lua, TOML, YAML, or any file format.
//! It only converts parsed strings into Fenestre's shared config types.
use super::{ConfigError, KeyBindingConfig, KeyBindingTarget, LayoutConfig, Result};
use crate::command::Command;
use crate::layout::Rect;
use crate::state::rule::{RulePattern, WindowRule};
use xkbcommon::xkb;

/// Parse an optional target string into `KeyBindingTarget`.
fn parse_target(target: Option<&str>) -> Option<KeyBindingTarget> {
    match target.unwrap_or("primary") {
        "primary" => Some(KeyBindingTarget::Primary),
        "all" => Some(KeyBindingTarget::All),
        _ => None,
    }
}

/// Parse an XKB keysym name into a numeric keysym.
///
/// Supports XKB names such as `Return`, `q`, `h`, `j`, `k`, `l`, and `Escape`.
/// Also supports convenience aliases: `enter`/`esc` for the obvious keys,
/// abbreviated names like `del`/`pgup`/`caps`/`prtsc`, literal special
/// characters such as `[`/`.`/`,`/`#`/`+`, and media keys like `volup`/`mute`.
/// NUL bytes are rejected to avoid panics inside XKB string conversion.
fn parse_keysym(name: &str) -> Option<u32> {
    // xkb::keysym_from_name() can panic on NUL bytes
    if name.as_bytes().contains(&0) {
        return None;
    }

    // Aliases support. Note: xkb::keysym_from_name is already case-insensitive,
    // so canonical names (e.g. "Return", "Page_Up", "XF86AudioMute") resolve on
    // their own. These aliases only cover non-canonical abbreviations and the
    // literal special characters a user might type in a config.
    let name = match name {
        // Empty input is invalid.
        "" => return None,

        // Whitespace characters.
        " " => "Space",
        "\t" => "Tab",

        // Common key-name abbreviations.
        "enter" => "Return",
        "esc" => "Escape",
        "del" => "Delete",
        "ins" => "Insert",
        "pgup" | "pageup" => "Page_Up",
        "pgdn" | "pagedown" => "Page_Down",
        "caps" => "Caps_Lock",
        "prtsc" => "Print",
        "sysrq" => "Sys_Req",
        "scrolllock" => "Scroll_Lock",
        "numlock" => "Num_Lock",
        "menu" => "Menu",

        // Literal special characters.
        "/" => "slash",
        "\\" => "backslash",
        "'" => "apostrophe",
        "\"" => "quotedbl",
        "`" => "grave",
        "~" => "asciitilde",
        "-" => "minus",
        "=" => "equal",
        "+" => "plus",
        ";" => "semicolon",
        ":" => "colon",
        "*" => "asterisk",
        "&" => "ampersand",
        "#" => "numbersign",
        "@" => "at",
        "$" => "dollar",
        "^" => "asciicircum",
        "|" => "bar",
        "_" => "underscore",
        "!" => "exclam",
        "?" => "question",
        "[" => "bracketleft",
        "]" => "bracketright",
        "." => "period",
        "," => "comma",

        // Media / XF86 keys (no short canonical form).
        "volup" | "volumeup" => "XF86AudioRaiseVolume",
        "voldown" | "volumedown" => "XF86AudioLowerVolume",
        "mute" => "XF86AudioMute",
        "playpause" => "XF86AudioPlay",
        "brightup" | "brightnessup" => "XF86MonBrightnessUp",
        "brightdown" | "brightnessdown" => "XF86MonBrightnessDown",
        "touchpad" => "XF86TouchpadToggle",

        other => other,
    };

    let keysym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE).into();

    if keysym == xkb::keysyms::KEY_NoSymbol {
        None
    } else {
        Some(keysym)
    }
}

/// Parse a window state string into `WindowState`.
pub(super) fn parse_mode(name: &str) -> Option<crate::layout::WindowState> {
    match name {
        "tiled" => Some(crate::layout::WindowState::Tiled),
        "floating" => Some(crate::layout::WindowState::Floating),
        "pseudo_tiled" => Some(crate::layout::WindowState::PseudoTiled),
        "fullscreen" => Some(crate::layout::WindowState::Fullscreen),
        _ => None,
    }
}

/// Parse modifiers from any string-like iterator into River's modifier bitmask.
///
/// This function is generic over the iterator item so config loaders for Lua,
/// TOML, YAML, or other formats can pass their own representation without
/// converting everything to a single concrete type first.
///
/// Modifier names are matched case-insensitively (`Super` ≡ `super`). Unknown
/// names are rejected. Caps Lock (`lock`) and NumLock (`mod2`) are intentionally
/// not supported.
///
/// For example, all of these work:
///
/// ```ignore
/// parse_modifiers(["super", "shift"]);
/// parse_modifiers(vec!["super".to_string(), "ctrl".to_string()]);
/// parse_modifiers(modifiers.iter().map(String::as_str));
/// ```
fn parse_modifiers<I>(modifiers: I) -> Option<u32>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut result = 0;

    for modifier in modifiers {
        let name = modifier.as_ref().to_ascii_lowercase();
        let value = match name.as_str() {
            "shift" => 1,
            "ctrl" | "control" => 4,
            "alt" | "mod1" => 8,
            "mod3" => 32,
            "super" | "mod4" => 64,
            "mod5" => 128,
            _ => return None,
        };
        result |= value;
    }

    Some(result)
}

/// Parse command tokens into an internal `Command`.
fn parse_command(tokens: &[&str]) -> Option<Command> {
    match tokens.first().copied()? {
        "close" => Some(Command::CloseFocused),
        "focus_next" => Some(Command::FocusNext),
        "focus_previous" => Some(Command::FocusPrevious),
        "focus_left" => Some(Command::FocusLeft),
        "focus_right" => Some(Command::FocusRight),
        "focus_up" => Some(Command::FocusUp),
        "focus_down" => Some(Command::FocusDown),
        "spawn" => {
            let program = tokens.get(1)?;
            if program.is_empty() {
                None
            } else {
                Some(Command::Spawn {
                    program: program.to_string(),
                    args: tokens[2..].iter().map(|arg| arg.to_string()).collect(),
                })
            }
        }
        "reload_config" | "reload" => Some(Command::ReloadConfig),
        "exit_river" | "exit" => Some(Command::ExitRiver),
        "toggle_floating" | "floating" => Some(Command::ToggleFloating),
        "toggle_pseudo_tiled" | "pseudo_tiled" => Some(Command::TogglePseudoTiled),
        "toggle_fullscreen" | "fullscreen" => Some(Command::ToggleFullscreen),
        "tiled" => Some(Command::SetTiled),
        "move_left" => Some(Command::MoveLeft),
        "move_right" => Some(Command::MoveRight),
        "move_up" => Some(Command::MoveUp),
        "move_down" => Some(Command::MoveDown),
        "resize_expand_left" => Some(Command::ResizeExpand {
            direction: crate::layout::FocusDirection::Left,
        }),
        "resize_expand_right" => Some(Command::ResizeExpand {
            direction: crate::layout::FocusDirection::Right,
        }),
        "resize_expand_up" => Some(Command::ResizeExpand {
            direction: crate::layout::FocusDirection::Up,
        }),
        "resize_expand_down" => Some(Command::ResizeExpand {
            direction: crate::layout::FocusDirection::Down,
        }),
        "resize_shrink_left" => Some(Command::ResizeShrink {
            direction: crate::layout::FocusDirection::Left,
        }),
        "resize_shrink_right" => Some(Command::ResizeShrink {
            direction: crate::layout::FocusDirection::Right,
        }),
        "resize_shrink_up" => Some(Command::ResizeShrink {
            direction: crate::layout::FocusDirection::Up,
        }),
        "resize_shrink_down" => Some(Command::ResizeShrink {
            direction: crate::layout::FocusDirection::Down,
        }),
        _ => None,
    }
}

/// Parse a complete keybinding from normalized string inputs.
///
/// Returns `None` when any part of the keybinding is invalid.
pub fn parse_keybinding(
    target: Option<&str>,
    keysym: &str,
    modifiers: &[&str],
    command_tokens: &[&str],
) -> Option<KeyBindingConfig> {
    let target = parse_target(target)?;
    let keysym = parse_keysym(keysym)?;
    let modifiers = parse_modifiers(modifiers.iter().copied())?;
    let command = parse_command(command_tokens)?;

    Some(KeyBindingConfig {
        target,
        keysym,
        modifiers,
        command,
    })
}

/// Intermediate, format-neutral pattern emitted by a loader before regex
/// compilation. Centralizing this means the `exact`/`prefix`/`regex` mode
/// mapping and regex compilation live in exactly one place.
pub(super) enum RawPattern {
    Exact(String),
    Prefix(String),
    Regex(String),
}

/// Map a `value`/`match` pair to a `RawPattern`, preserving the field name for
/// error messages. Shared by every loader so pattern-mode handling is not
/// duplicated across Lua and TOML.
pub(super) fn build_raw_pattern(name: &str, value: String, mode: &str) -> Result<RawPattern> {
    match mode {
        "exact" => Ok(RawPattern::Exact(value)),
        "prefix" => Ok(RawPattern::Prefix(value)),
        "regex" => Ok(RawPattern::Regex(value)),
        other => Err(ConfigError::InvalidConfig(format!(
            "Invalid {name} match mode: {other}"
        ))),
    }
}

/// Build a `RawPattern` from already-extracted `value` and optional `match`
/// mode, applying the `exact` default in one shared spot.
pub(super) fn build_pattern_field(
    name: &str,
    value: String,
    mode: Option<String>,
) -> Result<RawPattern> {
    let mode = mode.unwrap_or_else(|| "exact".to_string());
    build_raw_pattern(name, value, &mode)
}

/// Compile a `RawPattern` into a `RulePattern`, performing regex compilation
/// (with the `size_limit` guard from `RulePattern::regex`) in one shared spot.
pub(super) fn build_pattern(name: &str, pattern: RawPattern) -> Result<RulePattern> {
    match pattern {
        RawPattern::Exact(s) => Ok(RulePattern::exact(s)),
        RawPattern::Prefix(s) => Ok(RulePattern::prefix(s)),
        RawPattern::Regex(s) => RulePattern::regex(&s)
            .ok_or_else(|| ConfigError::InvalidConfig(format!("Invalid {name} regex: {s}"))),
    }
}

/// Format-neutral margins table consumed by `build_layout`.
pub(super) struct RawMargins {
    pub top: Option<i32>,
    pub right: Option<i32>,
    pub bottom: Option<i32>,
    pub left: Option<i32>,
}

/// Build a `LayoutConfig`, applying the nested-`margins`-overrides-flat-edge
/// precedence exactly once.
pub(super) fn build_layout(
    gap: Option<i32>,
    margin_top: Option<i32>,
    margin_right: Option<i32>,
    margin_bottom: Option<i32>,
    margin_left: Option<i32>,
    margins: Option<RawMargins>,
    default_float_ratio: Option<f32>,
) -> LayoutConfig {
    match margins {
        Some(m) => LayoutConfig {
            gap,
            margin_top: m.top.or(margin_top),
            margin_right: m.right.or(margin_right),
            margin_bottom: m.bottom.or(margin_bottom),
            margin_left: m.left.or(margin_left),
            default_float_ratio,
        },
        None => LayoutConfig {
            gap,
            margin_top,
            margin_right,
            margin_bottom,
            margin_left,
            default_float_ratio,
        },
    }
}

/// Build a `WindowRule` from already-extracted matchers and mode, centralizing
/// the "at least one matcher" check, mode parsing, and regex compilation.
pub(super) fn build_rule(
    app_id: Option<RawPattern>,
    title: Option<RawPattern>,
    mode: &str,
    floating_rect: Option<Rect>,
) -> Result<WindowRule> {
    if app_id.is_none() && title.is_none() {
        return Err(ConfigError::InvalidConfig(
            "Rule must match at least one of app_id or title".to_string(),
        ));
    }

    let target = parse_mode(mode)
        .ok_or_else(|| ConfigError::InvalidConfig("Invalid rule mode".to_string()))?;

    let app_id = app_id.map(|p| build_pattern("app_id", p)).transpose()?;
    let title = title.map(|p| build_pattern("title", p)).transpose()?;

    Ok(WindowRule {
        app_id,
        title,
        target,
        floating_rect,
    })
}

/// Build a `KeyBindingConfig` from already-extracted strings, centralizing the
/// call into the shared `parse_keybinding` and its error fallback.
pub(super) fn build_keybinding(
    target: Option<&str>,
    keysym: &str,
    modifiers: &[String],
    command: &[String],
) -> Result<KeyBindingConfig> {
    let modifier_refs: Vec<&str> = modifiers.iter().map(String::as_str).collect();
    let command_refs: Vec<&str> = command.iter().map(String::as_str).collect();

    parse_keybinding(target, keysym, &modifier_refs, &command_refs)
        .ok_or_else(|| ConfigError::InvalidConfig("Invalid keybinding".to_string()))
}

/// Shared validation hook for extracted string lists. Used by both the Lua
/// and TOML loaders so any list-level validation rules live in exactly one
/// place.
pub(super) fn validate_string_list(strings: &[String]) -> Result<Vec<String>> {
    Ok(strings.to_vec())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_command, parse_keybinding, parse_keysym, parse_mode, parse_modifiers, parse_target,
    };
    use crate::command::Command;
    use xkbcommon::xkb;

    const SHIFT: u32 = 1;
    const CTRL: u32 = 4;
    const ALT: u32 = 8;
    const MOD3: u32 = 32;
    const SUPER: u32 = 64;
    const MOD5: u32 = 128;

    #[test]
    fn parses_target_strings() {
        assert_eq!(
            parse_target(Some("primary")),
            Some(super::KeyBindingTarget::Primary)
        );
        assert_eq!(
            parse_target(Some("all")),
            Some(super::KeyBindingTarget::All)
        );
        assert_eq!(parse_target(None), Some(super::KeyBindingTarget::Primary));
        assert_eq!(parse_target(Some("invalid")), None);
        assert_eq!(parse_target(Some("Primary")), None);
    }

    #[test]
    fn parses_named_xkb_keysyms() {
        assert_eq!(parse_keysym("Return"), Some(xkb::keysyms::KEY_Return));
        assert_eq!(parse_keysym("Space"), Some(xkb::keysyms::KEY_space));
        assert_eq!(parse_keysym("KP_Enter"), Some(xkb::keysyms::KEY_KP_Enter));
        assert_eq!(parse_keysym("Escape"), Some(xkb::keysyms::KEY_Escape));
        assert_eq!(parse_keysym("Left"), Some(xkb::keysyms::KEY_Left));
        assert_eq!(parse_keysym("eacute"), Some(xkb::keysyms::KEY_eacute));
        assert_eq!(parse_keysym("Eacute"), Some(xkb::keysyms::KEY_eacute));
    }

    #[test]
    fn parses_single_character_xkb_keysyms() {
        assert_eq!(parse_keysym("j"), Some(xkb::keysyms::KEY_j));
        assert_eq!(parse_keysym("J"), Some(xkb::keysyms::KEY_j));
    }

    #[test]
    fn parses_alias_keysyms() {
        // Whitespace characters.
        assert_eq!(parse_keysym(" "), Some(xkb::keysyms::KEY_space));
        assert_eq!(parse_keysym("\t"), Some(xkb::keysyms::KEY_Tab));

        // Key-name abbreviations.
        assert_eq!(parse_keysym("enter"), Some(xkb::keysyms::KEY_Return));
        assert_eq!(parse_keysym("esc"), Some(xkb::keysyms::KEY_Escape));
        assert_eq!(parse_keysym("del"), Some(xkb::keysyms::KEY_Delete));
        assert_eq!(parse_keysym("ins"), Some(xkb::keysyms::KEY_Insert));
        assert_eq!(parse_keysym("pgup"), Some(xkb::keysyms::KEY_Page_Up));
        assert_eq!(parse_keysym("pageup"), Some(xkb::keysyms::KEY_Page_Up));
        assert_eq!(parse_keysym("pgdn"), Some(xkb::keysyms::KEY_Page_Down));
        assert_eq!(parse_keysym("pagedown"), Some(xkb::keysyms::KEY_Page_Down));
        assert_eq!(parse_keysym("caps"), Some(xkb::keysyms::KEY_Caps_Lock));
        assert_eq!(parse_keysym("prtsc"), Some(xkb::keysyms::KEY_Print));
        assert_eq!(parse_keysym("sysrq"), Some(xkb::keysyms::KEY_Sys_Req));
        assert_eq!(
            parse_keysym("scrolllock"),
            Some(xkb::keysyms::KEY_Scroll_Lock)
        );
        assert_eq!(parse_keysym("numlock"), Some(xkb::keysyms::KEY_Num_Lock));
        assert_eq!(parse_keysym("menu"), Some(xkb::keysyms::KEY_Menu));

        // Literal special characters.
        assert_eq!(parse_keysym("["), Some(xkb::keysyms::KEY_bracketleft));
        assert_eq!(parse_keysym("]"), Some(xkb::keysyms::KEY_bracketright));
        assert_eq!(parse_keysym("."), Some(xkb::keysyms::KEY_period));
        assert_eq!(parse_keysym(","), Some(xkb::keysyms::KEY_comma));
        assert_eq!(parse_keysym("/"), Some(xkb::keysyms::KEY_slash));
        assert_eq!(parse_keysym("\\"), Some(xkb::keysyms::KEY_backslash));
        assert_eq!(parse_keysym("'"), Some(xkb::keysyms::KEY_apostrophe));
        assert_eq!(parse_keysym("\""), Some(xkb::keysyms::KEY_quotedbl));
        assert_eq!(parse_keysym("`"), Some(xkb::keysyms::KEY_grave));
        assert_eq!(parse_keysym("~"), Some(xkb::keysyms::KEY_asciitilde));
        assert_eq!(parse_keysym("-"), Some(xkb::keysyms::KEY_minus));
        assert_eq!(parse_keysym("="), Some(xkb::keysyms::KEY_equal));
        assert_eq!(parse_keysym("+"), Some(xkb::keysyms::KEY_plus));
        assert_eq!(parse_keysym(";"), Some(xkb::keysyms::KEY_semicolon));
        assert_eq!(parse_keysym(":"), Some(xkb::keysyms::KEY_colon));
        assert_eq!(parse_keysym("*"), Some(xkb::keysyms::KEY_asterisk));
        assert_eq!(parse_keysym("&"), Some(xkb::keysyms::KEY_ampersand));
        assert_eq!(parse_keysym("#"), Some(xkb::keysyms::KEY_numbersign));
        assert_eq!(parse_keysym("@"), Some(xkb::keysyms::KEY_at));
        assert_eq!(parse_keysym("$"), Some(xkb::keysyms::KEY_dollar));
        assert_eq!(parse_keysym("^"), Some(xkb::keysyms::KEY_asciicircum));
        assert_eq!(parse_keysym("|"), Some(xkb::keysyms::KEY_bar));
        assert_eq!(parse_keysym("_"), Some(xkb::keysyms::KEY_underscore));
        assert_eq!(parse_keysym("!"), Some(xkb::keysyms::KEY_exclam));
        assert_eq!(parse_keysym("?"), Some(xkb::keysyms::KEY_question));

        // Media / XF86 keys.
        assert_eq!(
            parse_keysym("volup"),
            Some(xkb::keysyms::KEY_XF86AudioRaiseVolume)
        );
        assert_eq!(
            parse_keysym("volumeup"),
            Some(xkb::keysyms::KEY_XF86AudioRaiseVolume)
        );
        assert_eq!(
            parse_keysym("voldown"),
            Some(xkb::keysyms::KEY_XF86AudioLowerVolume)
        );
        assert_eq!(
            parse_keysym("volumedown"),
            Some(xkb::keysyms::KEY_XF86AudioLowerVolume)
        );
        assert_eq!(parse_keysym("mute"), Some(xkb::keysyms::KEY_XF86AudioMute));
        assert_eq!(
            parse_keysym("playpause"),
            Some(xkb::keysyms::KEY_XF86AudioPlay)
        );
        assert_eq!(
            parse_keysym("brightup"),
            Some(xkb::keysyms::KEY_XF86MonBrightnessUp)
        );
        assert_eq!(
            parse_keysym("brightnessup"),
            Some(xkb::keysyms::KEY_XF86MonBrightnessUp)
        );
        assert_eq!(
            parse_keysym("brightdown"),
            Some(xkb::keysyms::KEY_XF86MonBrightnessDown)
        );
        assert_eq!(
            parse_keysym("brightnessdown"),
            Some(xkb::keysyms::KEY_XF86MonBrightnessDown)
        );
        assert_eq!(
            parse_keysym("touchpad"),
            Some(xkb::keysyms::KEY_XF86TouchpadToggle)
        );
    }

    #[test]
    fn rejects_invalid_xkb_keysyms() {
        assert_eq!(parse_keysym(""), None);
        assert_eq!(parse_keysym("not-a-real-keysym"), None);
        assert_eq!(parse_keysym("Return\0"), None);
    }

    #[test]
    fn parses_modifier_masks() {
        assert_eq!(parse_modifiers(std::iter::empty::<&str>()), Some(0));
        assert_eq!(parse_modifiers(["shift"]), Some(SHIFT));
        assert_eq!(parse_modifiers(["ctrl"]), Some(CTRL));
        assert_eq!(parse_modifiers(["control"]), Some(CTRL));
        assert_eq!(parse_modifiers(["alt"]), Some(ALT));
        assert_eq!(parse_modifiers(["mod1"]), Some(ALT));
        assert_eq!(parse_modifiers(["mod3"]), Some(MOD3));
        assert_eq!(parse_modifiers(["super"]), Some(SUPER));
        assert_eq!(parse_modifiers(["mod4"]), Some(SUPER));
        assert_eq!(parse_modifiers(["mod5"]), Some(MOD5));
        assert_eq!(parse_modifiers(["shift", "super"]), Some(SHIFT | SUPER));
        assert_eq!(parse_modifiers(["super", "shift"]), Some(SUPER | SHIFT));

        // Modifier names are matched case-insensitively.
        assert_eq!(parse_modifiers(["Super"]), Some(SUPER));
        assert_eq!(parse_modifiers(["SHIFT", "Ctrl"]), Some(SHIFT | CTRL));
        assert_eq!(parse_modifiers(["Mod4"]), Some(SUPER));
    }

    #[test]
    fn parses_owned_modifier_strings() {
        let modifiers = vec!["ctrl".to_string(), "alt".to_string()];

        assert_eq!(parse_modifiers(modifiers), Some(CTRL | ALT));
    }

    #[test]
    fn rejects_invalid_modifiers() {
        assert_eq!(parse_modifiers(["invalid"]), None);
        assert_eq!(parse_modifiers(["super", "invalid"]), None);
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse_command(&["close"]), Some(Command::CloseFocused));
        assert_eq!(parse_command(&["focus_next"]), Some(Command::FocusNext));
        assert_eq!(
            parse_command(&["focus_previous"]),
            Some(Command::FocusPrevious)
        );
        assert_eq!(parse_command(&["focus_left"]), Some(Command::FocusLeft));
        assert_eq!(parse_command(&["focus_right"]), Some(Command::FocusRight));
        assert_eq!(parse_command(&["focus_up"]), Some(Command::FocusUp));
        assert_eq!(parse_command(&["focus_down"]), Some(Command::FocusDown));
        assert_eq!(
            parse_command(&["spawn", "foot"]),
            Some(Command::Spawn {
                program: "foot".to_string(),
                args: Vec::new(),
            })
        );
        assert_eq!(
            parse_command(&["spawn", "foot", "-e", "htop"]),
            Some(Command::Spawn {
                program: "foot".to_string(),
                args: vec!["-e".to_string(), "htop".to_string()],
            })
        );
        assert_eq!(
            parse_command(&["reload_config"]),
            Some(Command::ReloadConfig)
        );
        assert_eq!(parse_command(&["reload"]), Some(Command::ReloadConfig));
        assert_eq!(parse_command(&["exit_river"]), Some(Command::ExitRiver));
        assert_eq!(parse_command(&["exit"]), Some(Command::ExitRiver));
        assert_eq!(
            parse_command(&["toggle_floating"]),
            Some(Command::ToggleFloating)
        );
        assert_eq!(parse_command(&["floating"]), Some(Command::ToggleFloating));
        assert_eq!(
            parse_command(&["toggle_fullscreen"]),
            Some(Command::ToggleFullscreen)
        );
        assert_eq!(
            parse_command(&["fullscreen"]),
            Some(Command::ToggleFullscreen)
        );
        assert_eq!(
            parse_command(&["toggle_pseudo_tiled"]),
            Some(Command::TogglePseudoTiled)
        );
        assert_eq!(
            parse_command(&["pseudo_tiled"]),
            Some(Command::TogglePseudoTiled)
        );
        assert_eq!(parse_command(&["tiled"]), Some(Command::SetTiled));
        assert_eq!(parse_command(&["move_left"]), Some(Command::MoveLeft));
        assert_eq!(parse_command(&["move_right"]), Some(Command::MoveRight));
        assert_eq!(parse_command(&["move_up"]), Some(Command::MoveUp));
        assert_eq!(parse_command(&["move_down"]), Some(Command::MoveDown));
        assert_eq!(
            parse_command(&["resize_expand_left"]),
            Some(Command::ResizeExpand {
                direction: crate::layout::FocusDirection::Left,
            })
        );
        assert_eq!(
            parse_command(&["resize_expand_right"]),
            Some(Command::ResizeExpand {
                direction: crate::layout::FocusDirection::Right,
            })
        );
        assert_eq!(
            parse_command(&["resize_expand_up"]),
            Some(Command::ResizeExpand {
                direction: crate::layout::FocusDirection::Up,
            })
        );
        assert_eq!(
            parse_command(&["resize_expand_down"]),
            Some(Command::ResizeExpand {
                direction: crate::layout::FocusDirection::Down,
            })
        );
        assert_eq!(
            parse_command(&["resize_shrink_left"]),
            Some(Command::ResizeShrink {
                direction: crate::layout::FocusDirection::Left,
            })
        );
        assert_eq!(
            parse_command(&["resize_shrink_right"]),
            Some(Command::ResizeShrink {
                direction: crate::layout::FocusDirection::Right,
            })
        );
        assert_eq!(
            parse_command(&["resize_shrink_up"]),
            Some(Command::ResizeShrink {
                direction: crate::layout::FocusDirection::Up,
            })
        );
        assert_eq!(
            parse_command(&["resize_shrink_down"]),
            Some(Command::ResizeShrink {
                direction: crate::layout::FocusDirection::Down,
            })
        );
    }

    #[test]
    fn rejects_invalid_commands() {
        assert_eq!(parse_command(&[]), None);
        assert_eq!(parse_command(&["spawn"]), None);
        assert_eq!(parse_command(&["spawn", ""]), None);
        assert_eq!(parse_command(&["invalid"]), None);
    }

    #[test]
    fn parses_full_keybinding() {
        let binding = parse_keybinding(Some("all"), "Return", &["super"], &["spawn", "foot"])
            .expect("keybinding should parse");

        assert_eq!(binding.target, super::KeyBindingTarget::All);
        assert_eq!(binding.keysym, xkb::keysyms::KEY_Return);
        assert_eq!(binding.modifiers, SUPER);
        assert_eq!(
            binding.command,
            Command::Spawn {
                program: "foot".to_string(),
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn parses_full_keybinding_with_default_target_and_no_modifiers() {
        let binding =
            parse_keybinding(None, "q", &[], &["close"]).expect("keybinding should parse");

        assert_eq!(binding.target, super::KeyBindingTarget::Primary);
        assert_eq!(binding.keysym, xkb::keysyms::KEY_q);
        assert_eq!(binding.modifiers, 0);
        assert_eq!(binding.command, Command::CloseFocused);
    }

    #[test]
    fn rejects_invalid_full_keybindings() {
        assert!(parse_keybinding(Some("invalid"), "Return", &["super"], &["close"]).is_none());
        assert!(parse_keybinding(Some("all"), "invalid", &["super"], &["close"]).is_none());
        assert!(parse_keybinding(Some("all"), "Return", &["invalid"], &["close"]).is_none());
        assert!(parse_keybinding(Some("all"), "Return", &["super"], &["invalid"]).is_none());
    }

    #[test]
    fn parses_modes() {
        assert_eq!(parse_mode("tiled"), Some(crate::layout::WindowState::Tiled));
        assert_eq!(
            parse_mode("floating"),
            Some(crate::layout::WindowState::Floating)
        );
        assert_eq!(
            parse_mode("pseudo_tiled"),
            Some(crate::layout::WindowState::PseudoTiled)
        );
        assert_eq!(
            parse_mode("fullscreen"),
            Some(crate::layout::WindowState::Fullscreen)
        );
        assert_eq!(parse_mode("invalid"), None);
    }
}
