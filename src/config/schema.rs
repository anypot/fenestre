//! Unified configuration schema.
//!
//! This module defines a single serde-backed intermediate representation for
//! Fenêtre's configuration. Both TOML and Lua loaders deserialize into these
//! schema types, then `build_config` validates and converts them into the
//! runtime `Config`. Adding a new config field means editing exactly one struct
//! here plus the conversion in `build_config`.

use super::*;
use crate::layout::Rect;
use serde::Deserialize;
use serde_json::Number;

/// Intermediate representation of a keybinding command.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawCommand {
    Simple(String),
    Multi(Vec<String>),
}

/// Intermediate representation of an `app_id`/`title` pattern matcher.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum UnresolvedPattern {
    Exact(String),
    WithMatch {
        value: String,
        #[serde(rename = "match")]
        match_type: Option<String>,
    },
}

/// Intermediate representation of layout configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RawLayout {
    #[serde(default)]
    pub gap: Option<i32>,
    #[serde(default)]
    pub margin_top: Option<i32>,
    #[serde(default)]
    pub margin_right: Option<i32>,
    #[serde(default)]
    pub margin_bottom: Option<i32>,
    #[serde(default)]
    pub margin_left: Option<i32>,
    #[serde(default)]
    pub margins: Option<parser::RawMargins>,
    #[serde(default)]
    pub default_float_ratio: Option<f32>,
    #[serde(default, deserialize_with = "de_preview_border_color")]
    pub preview_border_color: Option<u32>,
    #[serde(default, deserialize_with = "de_preview_border_width")]
    pub preview_border_width: Option<i32>,
}

/// Intermediate representation of a window rule.
#[derive(Debug, Clone, Deserialize)]
pub struct RawRule {
    #[serde(default, deserialize_with = "de_app_id")]
    pub app_id: Option<UnresolvedPattern>,

    #[serde(default, deserialize_with = "de_title")]
    pub title: Option<UnresolvedPattern>,

    #[serde(deserialize_with = "de_rule_mode")]
    pub mode: String,
    #[serde(default)]
    pub floating_rect: Option<RawRect>,
}

/// Intermediate representation of a rectangle.
#[derive(Debug, Clone, Deserialize)]
pub struct RawRect {
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
}

/// Intermediate representation of a keybinding.
#[derive(Debug, Clone, Deserialize)]
pub struct RawKeyBinding {
    pub target: Option<String>,
    pub keysym: String,
    pub modifiers: RawModifiers,
    pub command: RawCommand,
}

/// Intermediate representation of a pointer binding.
#[derive(Debug, Clone, Deserialize)]
pub struct RawPointerBinding {
    #[serde(default)]
    pub target: Option<String>,
    /// Pointer button name (e.g. "BTN_LEFT", "BTN_RIGHT") or raw Linux code.
    pub button: String,
    pub modifiers: RawModifiers,
    /// Operation: "move" or "resize".
    pub op: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawAccelProfile {
    None,
    Flat,
    Adaptive,
    // `Custom` is intentionally excluded: River's custom accel profile requires
    // additional `set_points` calls (acceleration curve points) that aren't
    // exposed via the config interface yet.
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawTapButtonMap {
    LeftRightMiddle,
    LeftMiddleRight,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawScrollMethod {
    None,
    TwoFinger,
    Edge,
    OnButtonDown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawDragLockState {
    Disabled,
    EnabledTimeout,
    EnabledSticky,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawClickMethod {
    None,
    ButtonAreas,
    Clickfinger,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RawSendEventsMode {
    Enabled,
    Disabled,
    DisabledOnExternalMouse,
}

/// Intermediate representation of keyboard layout configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RawKeyboardLayout {
    pub rules: Option<String>,
    pub model: Option<String>,
    pub layout: String,
    pub variant: Option<String>,
    pub options: Option<String>,
}

/// Intermediate representation of an input device configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RawInputDevice {
    pub name: String,
    #[serde(default)]
    pub accel_profile: Option<RawAccelProfile>,
    #[serde(default)]
    pub accel_speed: Option<f64>,
    #[serde(default)]
    pub scroll_factor: Option<f64>,
    #[serde(default)]
    pub repeat_rate: Option<i32>,
    #[serde(default)]
    pub repeat_delay: Option<i32>,
    #[serde(default)]
    pub tap: Option<bool>,
    #[serde(default)]
    pub tap_button_map: Option<RawTapButtonMap>,
    #[serde(default)]
    pub natural_scroll: Option<bool>,
    #[serde(default)]
    pub left_handed: Option<bool>,
    #[serde(default)]
    pub scroll_method: Option<RawScrollMethod>,
    #[serde(default)]
    pub middle_emulation: Option<bool>,
    #[serde(default)]
    pub dwt: Option<bool>,
    #[serde(default)]
    pub send_events: Option<RawSendEventsMode>,
    #[serde(default)]
    pub drag: Option<bool>,
    #[serde(default)]
    pub drag_lock: Option<RawDragLockState>,
    #[serde(default)]
    pub click_method: Option<RawClickMethod>,
    #[serde(default)]
    pub rotation: Option<u32>,
}

/// Intermediate representation of keybinding modifiers: either a single string
/// or an array of strings.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawModifiers {
    Single(String),
    Multi(Vec<String>),
}

/// Intermediate representation of the top-level configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RawConfig {
    #[serde(default)]
    pub layout: Option<RawLayout>,
    #[serde(default, deserialize_with = "de_decorations")]
    pub decorations: Option<bool>,
    #[serde(default, deserialize_with = "de_border_width")]
    pub border_width: Option<i32>,
    #[serde(default, deserialize_with = "de_border_color_focused")]
    pub border_color_focused: Option<u32>,
    #[serde(default, deserialize_with = "de_border_color_unfocused")]
    pub border_color_unfocused: Option<u32>,
    #[serde(default)]
    pub keybindings: Option<Vec<RawKeyBinding>>,
    #[serde(default)]
    pub pointer_bindings: Option<Vec<RawPointerBinding>>,
    #[serde(default, deserialize_with = "de_resize_delta_ratio")]
    pub resize_delta_ratio: Option<f64>,
    #[serde(default, deserialize_with = "de_resize_delta_percent")]
    pub resize_delta_percent: Option<f32>,
    #[serde(default)]
    pub rules: Option<Vec<RawRule>>,
    #[serde(default)]
    pub keyboard_layout: Option<RawKeyboardLayout>,
    #[serde(default)]
    pub input_devices: Option<Vec<RawInputDevice>>,
}

/// Generate a `deserialize_with` helper that wraps serde's generic error with
/// the offending field name, so config-load failures stay diagnosable after
/// the loaders were unified behind a single serde schema.
macro_rules! named_opt_de {
    ($fn_name:ident, $field:literal, $ty:ty) => {
        fn $fn_name<'de, D>(deserializer: D) -> std::result::Result<Option<$ty>, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            Option::<$ty>::deserialize(deserializer).map_err(|e| {
                serde::de::Error::custom(format!(concat!("invalid ", $field, ": {}"), e))
            })
        }
    };
}

named_opt_de!(de_border_width, "border_width", i32);
named_opt_de!(de_border_color_focused, "border_color_focused", u32);
named_opt_de!(de_border_color_unfocused, "border_color_unfocused", u32);
named_opt_de!(de_decorations, "decorations", bool);
named_opt_de!(de_resize_delta_ratio, "resize_delta_ratio", f64);
named_opt_de!(de_resize_delta_percent, "resize_delta_percent", f32);
named_opt_de!(de_app_id, "app_id", UnresolvedPattern);
named_opt_de!(de_title, "title", UnresolvedPattern);
named_opt_de!(de_preview_border_color, "preview_border_color", u32);
fn de_preview_border_width<'de, D>(deserializer: D) -> std::result::Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<i32> = Option::deserialize(deserializer)
        .map_err(|e| serde::de::Error::custom(format!("invalid preview_border_width: {e}")))?;
    Ok(value.map(|v| v.clamp(0, 100)))
}

/// `deserialize_with` helper that names the rule `mode` field in errors.
fn de_rule_mode<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer)
        .map_err(|e| serde::de::Error::custom(format!("invalid rule mode: {e}")))
}

/// Convert a validated `RawConfig` into the runtime `Config`.
pub fn build_config(raw: RawConfig) -> Result<Config> {
    let layout = match raw.layout {
        Some(l) => build_raw_layout(l)?,
        None => LayoutConfig::default(),
    };

    let decorations = raw.decorations.unwrap_or(true);

    let keybindings = raw
        .keybindings
        .unwrap_or_default()
        .into_iter()
        .map(build_raw_keybinding)
        .collect::<Result<Vec<_>>>()?;

    let rules = raw
        .rules
        .unwrap_or_default()
        .into_iter()
        .map(build_raw_rule)
        .collect::<Result<Vec<_>>>()?;

    let keyboard_layout = raw.keyboard_layout.map(|raw| KeyboardLayoutConfig {
        rules: raw.rules,
        model: raw.model,
        layout: raw.layout,
        variant: raw.variant,
        options: raw.options,
    });

    let input_devices = raw
        .input_devices
        .unwrap_or_default()
        .into_iter()
        .map(build_raw_input_device)
        .collect::<Result<Vec<_>>>()?;

    Ok(Config {
        layout,
        decorations,
        border_width: raw.border_width,
        border_color_focused: raw.border_color_focused,
        border_color_unfocused: raw.border_color_unfocused,
        resize_delta_ratio: raw.resize_delta_ratio,
        resize_delta_percent: raw.resize_delta_percent,
        keybindings,
        pointer_bindings: raw
            .pointer_bindings
            .unwrap_or_default()
            .into_iter()
            .map(build_raw_pointer_binding)
            .collect::<Result<Vec<_>>>()?,
        rules,
        keyboard_layout,
        input_devices,
    })
}

fn build_raw_layout(raw: RawLayout) -> Result<LayoutConfig> {
    let default_float_ratio = validate_ratio("default_float_ratio", raw.default_float_ratio)?;

    let nested = raw.margins.unwrap_or_default();

    Ok(parser::build_layout(
        raw.gap,
        parser::RawMargins {
            top: nested.top.or(raw.margin_top),
            right: nested.right.or(raw.margin_right),
            bottom: nested.bottom.or(raw.margin_bottom),
            left: nested.left.or(raw.margin_left),
        },
        default_float_ratio,
        raw.preview_border_color,
        raw.preview_border_width,
    ))
}

fn build_raw_pointer_binding(raw: RawPointerBinding) -> Result<PointerBindingConfig> {
    let modifiers = match raw.modifiers {
        RawModifiers::Single(s) => vec![s],
        RawModifiers::Multi(v) => v,
    };

    parser::build_pointer_binding(raw.target.as_deref(), &raw.button, &modifiers, &raw.op)
}

fn build_raw_keybinding(raw: RawKeyBinding) -> Result<KeyBindingConfig> {
    let command_tokens = match raw.command {
        RawCommand::Simple(s) => vec![s],
        RawCommand::Multi(v) => v,
    };

    let modifiers = match raw.modifiers {
        RawModifiers::Single(s) => vec![s],
        RawModifiers::Multi(v) => v,
    };

    parser::build_keybinding(
        raw.target.as_deref(),
        &raw.keysym,
        &modifiers,
        &command_tokens,
    )
}

fn build_raw_pattern(raw: UnresolvedPattern) -> Result<parser::RawPattern> {
    match raw {
        UnresolvedPattern::Exact(s) => Ok(parser::RawPattern::Exact(s)),
        UnresolvedPattern::WithMatch { value, match_type } => {
            let mode = match_type.unwrap_or_else(|| "exact".to_string());
            parser::build_raw_pattern("pattern", value, &mode)
        }
    }
}

fn build_raw_rule(raw: RawRule) -> Result<WindowRule> {
    let app_id = match raw.app_id {
        Some(p) => Some(build_raw_pattern(p)?),
        None => None,
    };
    let title = match raw.title {
        Some(p) => Some(build_raw_pattern(p)?),
        None => None,
    };

    let floating_rect = raw.floating_rect.map(|r| {
        Rect::new(
            r.x.unwrap_or(0),
            r.y.unwrap_or(0),
            r.width.unwrap_or(0),
            r.height.unwrap_or(0),
        )
    });

    parser::build_rule(app_id, title, &raw.mode, floating_rect)
}

impl From<RawAccelProfile> for AccelProfile {
    fn from(raw: RawAccelProfile) -> Self {
        match raw {
            RawAccelProfile::None => AccelProfile::None,
            RawAccelProfile::Flat => AccelProfile::Flat,
            RawAccelProfile::Adaptive => AccelProfile::Adaptive,
        }
    }
}

impl From<RawTapButtonMap> for TapButtonMap {
    fn from(raw: RawTapButtonMap) -> Self {
        match raw {
            RawTapButtonMap::LeftRightMiddle => TapButtonMap::Lrm,
            RawTapButtonMap::LeftMiddleRight => TapButtonMap::Lmr,
        }
    }
}

impl From<RawScrollMethod> for ScrollMethod {
    fn from(raw: RawScrollMethod) -> Self {
        match raw {
            RawScrollMethod::None => ScrollMethod::None,
            RawScrollMethod::TwoFinger => ScrollMethod::TwoFinger,
            RawScrollMethod::Edge => ScrollMethod::Edge,
            RawScrollMethod::OnButtonDown => ScrollMethod::OnButtonDown,
        }
    }
}

impl From<RawDragLockState> for DragLockState {
    fn from(raw: RawDragLockState) -> Self {
        match raw {
            RawDragLockState::Disabled => DragLockState::Disabled,
            RawDragLockState::EnabledTimeout => DragLockState::EnabledTimeout,
            RawDragLockState::EnabledSticky => DragLockState::EnabledSticky,
        }
    }
}

impl From<RawClickMethod> for ClickMethod {
    fn from(raw: RawClickMethod) -> Self {
        match raw {
            RawClickMethod::None => ClickMethod::None,
            RawClickMethod::ButtonAreas => ClickMethod::ButtonAreas,
            RawClickMethod::Clickfinger => ClickMethod::Clickfinger,
        }
    }
}

impl From<RawSendEventsMode> for SendEventsMode {
    fn from(raw: RawSendEventsMode) -> Self {
        match raw {
            RawSendEventsMode::Enabled => SendEventsMode::Enabled,
            RawSendEventsMode::Disabled => SendEventsMode::Disabled,
            RawSendEventsMode::DisabledOnExternalMouse => SendEventsMode::DisabledOnExternalMouse,
        }
    }
}

fn build_raw_input_device(raw: RawInputDevice) -> Result<InputDeviceConfig> {
    if let Some(factor) = raw.scroll_factor
        && factor < 0.0
    {
        return Err(ConfigError::InvalidConfig(format!(
            "input_devices[{}].scroll_factor must be >= 0",
            raw.name
        )));
    }
    if let Some(rate) = raw.repeat_rate
        && rate < 0
    {
        return Err(ConfigError::InvalidConfig(format!(
            "input_devices[{}].repeat_rate must be >= 0",
            raw.name
        )));
    }
    if let Some(delay) = raw.repeat_delay
        && delay < 0
    {
        return Err(ConfigError::InvalidConfig(format!(
            "input_devices[{}].repeat_delay must be >= 0",
            raw.name
        )));
    }
    if let Some(rotation) = raw.rotation
        && rotation >= 360
    {
        return Err(ConfigError::InvalidConfig(format!(
            "input_devices[{}].rotation must be in range [0, 360)",
            raw.name
        )));
    }
    Ok(InputDeviceConfig {
        name: raw.name,
        accel_profile: raw.accel_profile.map(AccelProfile::from),
        accel_speed: raw.accel_speed,
        scroll_factor: raw.scroll_factor,
        repeat_rate: raw.repeat_rate,
        repeat_delay: raw.repeat_delay,
        tap: raw.tap,
        tap_button_map: raw.tap_button_map.map(TapButtonMap::from),
        natural_scroll: raw.natural_scroll,
        left_handed: raw.left_handed,
        scroll_method: raw.scroll_method.map(ScrollMethod::from),
        middle_emulation: raw.middle_emulation,
        dwt: raw.dwt,
        send_events: raw.send_events.map(SendEventsMode::from),
        drag: raw.drag,
        drag_lock: raw.drag_lock.map(DragLockState::from),
        click_method: raw.click_method.map(ClickMethod::from),
        rotation: raw.rotation,
    })
}

/// Convert an `mlua::Value` into a `serde_json::Value` so Lua tables can be
/// deserialized through the same schema as TOML.
pub fn mlua_value_to_json_value(value: mlua::Value) -> Result<serde_json::Value> {
    mlua_value_to_json_value_inner(value, 0)
}

fn mlua_value_to_json_value_inner(value: mlua::Value, depth: usize) -> Result<serde_json::Value> {
    match value {
        mlua::Value::Nil => Ok(serde_json::Value::Null),
        mlua::Value::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        mlua::Value::Integer(i) => Ok(serde_json::Value::Number(i.into())),
        mlua::Value::Number(n) => {
            if n.is_finite() {
                Ok(serde_json::Value::Number(
                    Number::from_f64(n).expect("finite float"),
                ))
            } else {
                Err(ConfigError::InvalidConfig(format!("Invalid float: {n}")))
            }
        }
        mlua::Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        mlua::Value::Table(table) => mlua_table_to_json_value(&table, depth),
        mlua::Value::Function(_) => Err(ConfigError::InvalidConfig(
            "Cannot convert Lua function to config value".to_string(),
        )),
        mlua::Value::Thread(_) => Err(ConfigError::InvalidConfig(
            "Cannot convert Lua thread to config value".to_string(),
        )),
        mlua::Value::LightUserData(_) | mlua::Value::UserData(_) => Err(
            ConfigError::InvalidConfig("Cannot convert Lua userdata to config value".to_string()),
        ),
        mlua::Value::Error(err) => Err(ConfigError::Lua(*err)),
        mlua::Value::Other(_) => Err(ConfigError::InvalidConfig(
            "Cannot convert unsupported Lua value to config".to_string(),
        )),
    }
}

fn mlua_table_to_json_value(table: &mlua::Table, depth: usize) -> Result<serde_json::Value> {
    const MAX_LUA_TABLE_DEPTH: usize = 64;

    if depth > MAX_LUA_TABLE_DEPTH {
        return Err(ConfigError::InvalidConfig(
            "Config table exceeds maximum nesting depth".to_string(),
        ));
    }

    let mut pairs: Vec<(mlua::Value, mlua::Value)> = Vec::new();
    let mut all_integer_keys = true;

    for pair in table.pairs::<mlua::Value, mlua::Value>() {
        let (k, v) = pair?;
        match &k {
            mlua::Value::Integer(_) => {}
            mlua::Value::String(_) => all_integer_keys = false,
            _ => all_integer_keys = false,
        }
        pairs.push((k, v));
    }

    if all_integer_keys && !pairs.is_empty() || pairs.is_empty() {
        let mut sorted: Vec<_> = pairs
            .into_iter()
            .filter(|(k, _)| matches!(k, mlua::Value::Integer(_)))
            .collect();
        sorted.sort_by_key(|(k, _)| match k {
            mlua::Value::Integer(i) => *i,
            _ => 0,
        });

        let mut array = Vec::with_capacity(sorted.len());
        for (_, v) in sorted {
            array.push(mlua_value_to_json_value_inner(v, depth + 1)?);
        }
        Ok(serde_json::Value::Array(array))
    } else {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            match k {
                mlua::Value::String(s) => {
                    map.insert(
                        s.to_str()?.to_string(),
                        mlua_value_to_json_value_inner(v, depth + 1)?,
                    );
                }
                other => {
                    return Err(ConfigError::InvalidConfig(format!(
                        "Config object keys must be strings, found {other:?}"
                    )));
                }
            }
        }
        Ok(serde_json::Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use mlua::Lua;

    #[test]
    fn build_config_from_raw() {
        let raw = RawConfig {
            layout: Some(RawLayout {
                gap: Some(10),
                margin_top: None,
                margin_right: None,
                margin_bottom: None,
                margin_left: None,
                margins: None,
                default_float_ratio: None,
                preview_border_color: None,
                preview_border_width: None,
            }),
            decorations: Some(true),
            border_width: Some(2),
            border_color_focused: Some(0xffffffff),
            border_color_unfocused: Some(0xffffffff),
            resize_delta_ratio: None,
            resize_delta_percent: None,
            keybindings: Some(vec![RawKeyBinding {
                target: None,
                keysym: "q".to_string(),
                modifiers: RawModifiers::Multi(vec!["super".to_string()]),
                command: RawCommand::Simple("close".to_string()),
            }]),
            pointer_bindings: None,
            rules: None,
            keyboard_layout: None,
            input_devices: None,
        };

        let config = build_config(raw).expect("raw config should build");

        assert_eq!(config.layout.gap, Some(10));
        assert_eq!(config.border_width, Some(2));
        assert_eq!(config.keybindings.len(), 1);
        assert_eq!(config.keybindings[0].command, Command::CloseFocused);
    }

    #[test]
    fn build_config_rejects_invalid_ratio() {
        let raw = RawConfig {
            layout: Some(RawLayout {
                gap: None,
                margin_top: None,
                margin_right: None,
                margin_bottom: None,
                margin_left: None,
                margins: None,
                default_float_ratio: Some(2.0),
                preview_border_color: None,
                preview_border_width: None,
            }),
            decorations: None,
            border_width: None,
            border_color_focused: None,
            border_color_unfocused: None,
            resize_delta_ratio: None,
            resize_delta_percent: None,
            keybindings: None,
            pointer_bindings: None,
            rules: None,
            keyboard_layout: None,
            input_devices: None,
        };

        let error = build_config(raw).expect_err("invalid ratio should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message.contains("must be between 0.0 and 1.0")
        ));
    }

    #[test]
    fn mlua_table_to_json_value_handles_array() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(r#"return { "a", "b", "c" }"#)
            .call(())
            .expect("lua chunk should return a table");

        let json =
            mlua_value_to_json_value(mlua::Value::Table(table)).expect("table should convert");

        assert_eq!(json, serde_json::json!(["a", "b", "c"]));
    }

    #[test]
    fn mlua_table_to_json_value_handles_sparse_array() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(r#"return { [1] = "a", [1000] = "b" }"#)
            .call(())
            .expect("lua chunk should return a table");

        let json =
            mlua_value_to_json_value(mlua::Value::Table(table)).expect("table should convert");

        let arr = match json {
            serde_json::Value::Array(a) => a,
            _ => panic!("expected array"),
        };
        assert_eq!(
            arr,
            vec![
                serde_json::Value::String("a".to_string()),
                serde_json::Value::String("b".to_string()),
            ]
        );
    }

    #[test]
    fn mlua_table_to_json_value_handles_object() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(r#"return { key = "value", num = 42 }"#)
            .call(())
            .expect("lua chunk should return a table");

        let json =
            mlua_value_to_json_value(mlua::Value::Table(table)).expect("table should convert");

        assert_eq!(json, serde_json::json!({ "key": "value", "num": 42 }));
    }

    #[test]
    fn mlua_table_to_json_value_handles_nested() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(r#"return { outer = { inner = true } }"#)
            .call(())
            .expect("lua chunk should return a table");

        let json =
            mlua_value_to_json_value(mlua::Value::Table(table)).expect("table should convert");

        assert_eq!(json, serde_json::json!({ "outer": { "inner": true } }));
    }

    #[test]
    fn mlua_table_to_json_value_handles_empty_table() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(r#"return {}"#)
            .call(())
            .expect("lua chunk should return a table");

        let json =
            mlua_value_to_json_value(mlua::Value::Table(table)).expect("table should convert");

        // An empty table used as a list is an empty array, so empty
        // `keybindings = {}` / `rules = {}` deserialize as empty lists.
        assert_eq!(json, serde_json::json!([]));
    }

    #[test]
    fn mlua_table_to_json_value_rejects_cyclic_table() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(r#"local t = {}; t[1] = t; return t"#)
            .call(())
            .expect("lua chunk should return a table");

        let error =
            mlua_value_to_json_value(mlua::Value::Table(table)).expect_err("cycle should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn raw_accel_profile_converts_to_all_variants() {
        assert_eq!(
            AccelProfile::from(RawAccelProfile::None),
            AccelProfile::None
        );
        assert_eq!(
            AccelProfile::from(RawAccelProfile::Flat),
            AccelProfile::Flat
        );
        assert_eq!(
            AccelProfile::from(RawAccelProfile::Adaptive),
            AccelProfile::Adaptive
        );
    }

    #[test]
    fn raw_tap_button_map_converts_to_all_variants() {
        assert_eq!(
            TapButtonMap::from(RawTapButtonMap::LeftRightMiddle),
            TapButtonMap::Lrm
        );
        assert_eq!(
            TapButtonMap::from(RawTapButtonMap::LeftMiddleRight),
            TapButtonMap::Lmr
        );
    }

    #[test]
    fn raw_scroll_method_converts_to_all_variants() {
        assert_eq!(
            ScrollMethod::from(RawScrollMethod::None),
            ScrollMethod::None
        );
        assert_eq!(
            ScrollMethod::from(RawScrollMethod::TwoFinger),
            ScrollMethod::TwoFinger
        );
        assert_eq!(
            ScrollMethod::from(RawScrollMethod::Edge),
            ScrollMethod::Edge
        );
        assert_eq!(
            ScrollMethod::from(RawScrollMethod::OnButtonDown),
            ScrollMethod::OnButtonDown
        );
    }

    #[test]
    fn raw_drag_lock_state_converts_to_all_variants() {
        assert_eq!(
            DragLockState::from(RawDragLockState::Disabled),
            DragLockState::Disabled
        );
        assert_eq!(
            DragLockState::from(RawDragLockState::EnabledTimeout),
            DragLockState::EnabledTimeout
        );
        assert_eq!(
            DragLockState::from(RawDragLockState::EnabledSticky),
            DragLockState::EnabledSticky
        );
    }

    #[test]
    fn raw_click_method_converts_to_all_variants() {
        assert_eq!(ClickMethod::from(RawClickMethod::None), ClickMethod::None);
        assert_eq!(
            ClickMethod::from(RawClickMethod::ButtonAreas),
            ClickMethod::ButtonAreas
        );
        assert_eq!(
            ClickMethod::from(RawClickMethod::Clickfinger),
            ClickMethod::Clickfinger
        );
    }

    #[test]
    fn raw_send_events_mode_converts_to_all_variants() {
        assert_eq!(
            SendEventsMode::from(RawSendEventsMode::Enabled),
            SendEventsMode::Enabled
        );
        assert_eq!(
            SendEventsMode::from(RawSendEventsMode::Disabled),
            SendEventsMode::Disabled
        );
        assert_eq!(
            SendEventsMode::from(RawSendEventsMode::DisabledOnExternalMouse),
            SendEventsMode::DisabledOnExternalMouse
        );
    }
}
