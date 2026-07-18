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
    #[serde(default, deserialize_with = "de_resize_delta_ratio")]
    pub resize_delta_ratio: Option<f64>,
    #[serde(default, deserialize_with = "de_resize_delta_percent")]
    pub resize_delta_percent: Option<f32>,
    #[serde(default)]
    pub rules: Option<Vec<RawRule>>,
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

    Ok(Config {
        layout,
        decorations,
        border_width: raw.border_width,
        border_color_focused: raw.border_color_focused,
        border_color_unfocused: raw.border_color_unfocused,
        resize_delta_ratio: raw.resize_delta_ratio,
        resize_delta_percent: raw.resize_delta_percent,
        keybindings,
        rules,
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
            rules: None,
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
            rules: None,
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
}
