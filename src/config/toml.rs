//! TOML configuration loader.
//!
//! This module is intentionally TOML-specific. It converts TOML values into
//! the shared config types defined in `config::mod`, mirroring the Lua loader
//! in `lua.rs` and reusing the format-neutral helpers in `parser`.
use super::{Config, ConfigError, KeyBindingConfig, LayoutConfig, Result, parser};
use crate::layout::Rect;
use crate::state::rule::WindowRule;
use std::path::Path;

/// Load and parse a TOML configuration file.
pub(crate) fn load_from_toml(path: &Path) -> Result<Config> {
    let source = std::fs::read_to_string(path)?;
    let value: ::toml::Value = ::toml::from_str(&source)?;
    parse_config_table(value)
}

/// Parse the top-level TOML config value.
///
/// Missing `keybindings` is valid and means "no user keybindings".
/// Non-array `keybindings` is invalid.
fn parse_config_table(value: ::toml::Value) -> Result<Config> {
    let table = as_table(&value, "config")?;

    let keybindings = parse_keybindings(table.get("keybindings"))?;
    let rules = parse_rules(table.get("rules"))?;

    let layout = match table.get("layout") {
        Some(value) => parse_layout(value)?,
        None => LayoutConfig::default(),
    };

    let decorations = match table.get("decorations") {
        None => true,
        Some(::toml::Value::Boolean(b)) => *b,
        Some(_) => {
            return Err(ConfigError::InvalidConfig(
                "Expected decorations to be a boolean".to_string(),
            ));
        }
    };

    let border_width = opt_i32(table, "border_width")?;
    let border_color_focused = opt_u32(table, "border_color_focused")?;
    let border_color_unfocused = opt_u32(table, "border_color_unfocused")?;
    let resize_delta_ratio = opt_f64(table, "resize_delta_ratio")?;
    let resize_delta_percent = opt_f32(table, "resize_delta_percent")?;

    Ok(Config {
        layout,
        decorations,
        border_width,
        border_color_focused,
        border_color_unfocused,
        resize_delta_ratio,
        resize_delta_percent,
        keybindings,
        rules,
    })
}

fn as_table<'a>(value: &'a ::toml::Value, name: &str) -> Result<&'a ::toml::Table> {
    match value {
        ::toml::Value::Table(table) => Ok(table),
        _ => Err(ConfigError::InvalidConfig(format!(
            "Expected {name} to be a table"
        ))),
    }
}

fn as_string_list(value: &::toml::Value) -> Result<Vec<String>> {
    let strings = match value {
        ::toml::Value::String(string) => vec![string.clone()],
        ::toml::Value::Array(array) => {
            let mut values = Vec::with_capacity(array.len());
            for item in array {
                match item {
                    ::toml::Value::String(string) => values.push(string.clone()),
                    _ => {
                        return Err(ConfigError::InvalidConfig(
                            "Expected string or array of strings".to_string(),
                        ));
                    }
                }
            }
            values
        }
        _ => {
            return Err(ConfigError::InvalidConfig(
                "Expected string or array of strings".to_string(),
            ));
        }
    };
    parser::validate_string_list(&strings)
}

fn parse_keybindings(value: Option<&::toml::Value>) -> Result<Vec<KeyBindingConfig>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    let ::toml::Value::Array(items) = value else {
        return Err(ConfigError::InvalidConfig(
            "Expected keybindings to be an array".to_string(),
        ));
    };

    let mut keybindings = Vec::with_capacity(items.len());
    for item in items {
        let ::toml::Value::Table(table) = item else {
            return Err(ConfigError::InvalidConfig(
                "Expected keybinding to be a table".to_string(),
            ));
        };
        keybindings.push(parse_keybinding(table)?);
    }
    Ok(keybindings)
}

/// Parse one TOML keybinding table into a shared `KeyBindingConfig`.
fn parse_keybinding(binding: &::toml::Table) -> Result<KeyBindingConfig> {
    let target_name: Option<String> = string_opt(binding, "target")?;
    let keysym_name: String = string_req(binding, "keysym")?;
    let modifier_names = match binding.get("modifiers") {
        Some(value) => as_string_list(value)?,
        None => {
            return Err(ConfigError::InvalidConfig(
                "Missing keybinding modifiers".to_string(),
            ));
        }
    };
    let command_tokens = match binding.get("command") {
        Some(value) => as_string_list(value)?,
        None => {
            return Err(ConfigError::InvalidConfig(
                "Missing keybinding command".to_string(),
            ));
        }
    };

    parser::build_keybinding(
        target_name.as_deref(),
        &keysym_name,
        &modifier_names,
        &command_tokens,
    )
}

/// Parse a TOML layout value into `LayoutConfig`.
///
/// Supports both flat keys (`gap`, `margin_top`, ...) and a nested `margins`
/// table with `top`/`right`/`bottom`/`left` keys. The nested table takes
/// precedence for individual edges it defines.
fn parse_layout(value: &::toml::Value) -> Result<LayoutConfig> {
    let table = as_table(value, "layout")?;

    let gap = opt_i32(table, "gap")?;
    let margin_top = opt_i32(table, "margin_top")?;
    let margin_right = opt_i32(table, "margin_right")?;
    let margin_bottom = opt_i32(table, "margin_bottom")?;
    let margin_left = opt_i32(table, "margin_left")?;

    let margins = match table.get("margins") {
        Some(::toml::Value::Table(margins)) => Ok(Some(parser::RawMargins {
            top: opt_i32(margins, "top")?,
            right: opt_i32(margins, "right")?,
            bottom: opt_i32(margins, "bottom")?,
            left: opt_i32(margins, "left")?,
        })),
        Some(_) => Err(ConfigError::InvalidConfig(
            "Expected margins to be a table".to_string(),
        )),
        None => Ok(None),
    }?;

    Ok(parser::build_layout(
        gap,
        margin_top,
        margin_right,
        margin_bottom,
        margin_left,
        margins,
    ))
}

fn parse_rules(value: Option<&::toml::Value>) -> Result<Vec<WindowRule>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    let ::toml::Value::Array(items) = value else {
        return Err(ConfigError::InvalidConfig(
            "Expected rules to be an array".to_string(),
        ));
    };

    let mut rules = Vec::with_capacity(items.len());
    for item in items {
        let ::toml::Value::Table(table) = item else {
            return Err(ConfigError::InvalidConfig(
                "Expected rule to be a table".to_string(),
            ));
        };
        rules.push(parse_rule(table)?);
    }
    Ok(rules)
}

fn parse_rule(rule: &::toml::Table) -> Result<WindowRule> {
    let app_id = parse_pattern_field(rule, "app_id")?;
    let title = parse_pattern_field(rule, "title")?;

    let mode_str: String = string_req(rule, "mode")?;

    let floating_rect = match rule.get("floating_rect") {
        None => None,
        Some(value) => {
            let rect_table = as_table(value, "floating_rect")?;
            let x = opt_i32(rect_table, "x")?.unwrap_or(0);
            let y = opt_i32(rect_table, "y")?.unwrap_or(0);
            let width = opt_i32(rect_table, "width")?.unwrap_or(0);
            let height = opt_i32(rect_table, "height")?.unwrap_or(0);
            Some(Rect::new(x, y, width, height))
        }
    };

    parser::build_rule(app_id, title, &mode_str, floating_rect)
}

/// Parse an `app_id`/`title` matcher: a plain string is an exact match; a table
/// `{ value, match }` selects `exact` (default), `prefix`, or `regex`.
fn parse_pattern_field(rule: &::toml::Table, name: &str) -> Result<Option<parser::RawPattern>> {
    match rule.get(name) {
        None => Ok(None),
        Some(::toml::Value::String(s)) => Ok(Some(parser::RawPattern::Exact(s.clone()))),
        Some(::toml::Value::Table(pattern)) => {
            let value: String = string_req(pattern, "value")?;
            let mode: Option<String> = string_opt(pattern, "match")?;
            Ok(Some(parser::build_pattern_field(name, value, mode)?))
        }
        Some(_) => Err(ConfigError::InvalidConfig(format!(
            "Expected {name} to be a string or table"
        ))),
    }
}

fn string_req(table: &::toml::Table, key: &str) -> Result<String> {
    match table.get(key) {
        Some(::toml::Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(ConfigError::InvalidConfig(format!(
            "Expected {key} to be a string"
        ))),
        None => Err(ConfigError::InvalidConfig(format!(
            "Missing required key: {key}"
        ))),
    }
}

fn string_opt(table: &::toml::Table, key: &str) -> Result<Option<String>> {
    match table.get(key) {
        None => Ok(None),
        Some(::toml::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ConfigError::InvalidConfig(format!(
            "Expected {key} to be a string"
        ))),
    }
}

fn opt_i32(table: &::toml::Table, key: &str) -> Result<Option<i32>> {
    match table.get(key) {
        None => Ok(None),
        Some(::toml::Value::Integer(v)) => {
            let iv = *v;
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&iv) {
                return Err(invalid_integer(key));
            }
            Ok(Some(iv as i32))
        }
        Some(::toml::Value::Float(v)) => {
            let iv = coerce_integer_float(v, key, |iv| {
                (i32::MIN as i64..=i32::MAX as i64).contains(&iv)
            })?;
            Ok(Some(iv as i32))
        }
        Some(_) => Err(invalid_integer(key)),
    }
}

fn opt_u32(table: &::toml::Table, key: &str) -> Result<Option<u32>> {
    match table.get(key) {
        None => Ok(None),
        Some(::toml::Value::Integer(v)) => {
            let iv = *v;
            if !(0..=u32::MAX as i64).contains(&iv) {
                return Err(invalid_integer(key));
            }
            Ok(Some(iv as u32))
        }
        Some(::toml::Value::Float(v)) => {
            let iv = coerce_integer_float(v, key, |iv| (0..=u32::MAX as i64).contains(&iv))?;
            Ok(Some(iv as u32))
        }
        Some(_) => Err(invalid_integer(key)),
    }
}

fn invalid_integer(key: &str) -> ConfigError {
    ConfigError::InvalidConfig(format!("Expected {key} to be an integer"))
}

fn coerce_integer_float(v: &f64, key: &str, range_check: impl FnOnce(i64) -> bool) -> Result<i64> {
    if v.fract() != 0.0 {
        return Err(invalid_integer(key));
    }
    let iv = v.trunc() as i64;
    if !range_check(iv) {
        return Err(invalid_integer(key));
    }
    Ok(iv)
}

fn opt_f64(table: &::toml::Table, key: &str) -> Result<Option<f64>> {
    match table.get(key) {
        None => Ok(None),
        Some(::toml::Value::Float(v)) => Ok(Some(*v)),
        Some(::toml::Value::Integer(v)) => Ok(Some(*v as f64)),
        Some(_) => Err(ConfigError::InvalidConfig(format!(
            "Expected {key} to be a number"
        ))),
    }
}

fn opt_f32(table: &::toml::Table, key: &str) -> Result<Option<f32>> {
    match table.get(key) {
        None => Ok(None),
        Some(::toml::Value::Float(v)) => Ok(Some(*v as f32)),
        Some(::toml::Value::Integer(v)) => Ok(Some(*v as f32)),
        Some(_) => Err(ConfigError::InvalidConfig(format!(
            "Expected {key} to be a number"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConfigError, KeyBindingConfig, as_string_list, load_from_toml, parse_config_table,
    };
    use crate::command::Command;
    use crate::config::KeyBindingTarget;
    use crate::layout::{Rect, WindowState};
    use crate::state::rule::RulePattern;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };
    use xkbcommon::xkb::keysyms;

    const SUPER: u32 = 64;
    const SHIFT: u32 = 1;

    fn assert_binding(binding: &KeyBindingConfig, expected: &KeyBindingConfig) {
        assert_eq!(binding.target, expected.target);
        assert_eq!(binding.keysym, expected.keysym);
        assert_eq!(binding.modifiers, expected.modifiers);
        assert_eq!(binding.command, expected.command);
    }

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

    fn temp_toml_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after UNIX_EPOCH")
            .as_nanos();

        std::env::temp_dir().join(format!("fenestre-config-{nanos}.toml"))
    }

    fn parse(source: &str) -> Result<super::Config, ::toml::de::Error> {
        let value: ::toml::Value = ::toml::from_str(source)?;
        Ok(parse_config_table(value).expect("config should parse"))
    }

    #[test]
    fn parse_config_table_accepts_empty_rules_table() {
        let config = parse("rules = []\n").expect("config should parse");

        assert!(config.rules.is_empty());
    }

    #[test]
    fn parse_config_table_parses_valid_rules() {
        let config = parse(
            r#"
            rules = [
                { app_id = "foot", mode = "floating" },
            ]
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.rules.len(), 1);
        assert!(matches!(
            config.rules[0].app_id,
            Some(RulePattern::Exact(ref s)) if s == "foot"
        ));
        assert_eq!(config.rules[0].target, WindowState::Floating);
    }

    #[test]
    fn parse_config_table_parses_rule_prefix() {
        let config = parse(
            r#"
            rules = [
                { app_id = { value = "mate-", match = "prefix" }, mode = "floating" },
            ]
            "#,
        )
        .expect("config should parse");

        assert!(matches!(
            config.rules[0].app_id,
            Some(RulePattern::Prefix(ref s)) if s == "mate-"
        ));
    }

    #[test]
    fn parse_config_table_rejects_invalid_rule_mode() {
        let value: ::toml::Value = ::toml::from_str(
            r#"
            rules = [
                { app_id = "foot", mode = "invalid" },
            ]
            "#,
        )
        .expect("toml should parse");
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Invalid rule mode"
        ));
    }

    #[test]
    fn parse_config_table_rejects_empty_rule() {
        let value: ::toml::Value = ::toml::from_str(
            r#"
            rules = [
                { mode = "floating" },
            ]
            "#,
        )
        .expect("toml should parse");
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Rule must match at least one of app_id or title"
        ));
    }

    #[test]
    fn parse_config_table_parses_rule_regex_and_rect() {
        let config = parse(
            r#"
            rules = [
                { app_id = { value = "^foot.*", match = "regex" }, title = { value = "terminal$", match = "regex" }, mode = "fullscreen", floating_rect = { x = 10, y = 20, width = 800, height = 600 } },
            ]
            "#,
        )
        .expect("config should parse");

        let rule = &config.rules[0];
        assert!(matches!(rule.app_id, Some(RulePattern::Regex(_))));
        assert!(matches!(rule.title, Some(RulePattern::Regex(_))));
        assert_eq!(rule.target, WindowState::Fullscreen);
        assert_eq!(rule.floating_rect, Some(Rect::new(10, 20, 800, 600)));
    }

    #[test]
    fn parse_config_table_missing_keybindings_returns_empty_config() {
        let config = parse("").expect("config should parse");

        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn parse_config_table_accepts_empty_keybindings_table() {
        let config = parse("keybindings = []\n").expect("config should parse");

        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn parse_config_table_rejects_non_table_keybindings() {
        let value: ::toml::Value =
            ::toml::from_str("keybindings = false\n").expect("toml should parse");
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected keybindings to be an array"
        ));
    }

    #[test]
    fn parse_config_table_parses_valid_keybindings() {
        let config = parse(
            r#"
            keybindings = [
                { keysym = "q", modifiers = ["super"], command = "close" },
                { target = "all", keysym = "Return", modifiers = ["super"], command = ["spawn", "foot"] },
            ]
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.keybindings.len(), 2);
        assert_binding(
            &config.keybindings[0],
            &binding(
                KeyBindingTarget::Primary,
                keysyms::KEY_q,
                SUPER,
                Command::CloseFocused,
            ),
        );
        assert_binding(
            &config.keybindings[1],
            &binding(
                KeyBindingTarget::All,
                keysyms::KEY_Return,
                SUPER,
                Command::Spawn {
                    program: "foot".to_string(),
                    args: Vec::new(),
                },
            ),
        );
    }

    #[test]
    fn parse_config_table_parses_move_commands() {
        let config = parse(
            r#"
            keybindings = [
                { keysym = "h", modifiers = ["super", "shift"], command = "move_left" },
                { keysym = "l", modifiers = ["super", "shift"], command = "move_right" },
            ]
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.keybindings.len(), 2);
        assert_binding(
            &config.keybindings[0],
            &binding(
                KeyBindingTarget::Primary,
                keysyms::KEY_h,
                SUPER | SHIFT,
                Command::MoveLeft,
            ),
        );
        assert_binding(
            &config.keybindings[1],
            &binding(
                KeyBindingTarget::Primary,
                keysyms::KEY_l,
                SUPER | SHIFT,
                Command::MoveRight,
            ),
        );
    }

    #[test]
    fn parse_config_table_rejects_invalid_keybinding() {
        let value: ::toml::Value = ::toml::from_str(
            r#"
            keybindings = [
                { keysym = "Return", modifiers = ["super"], command = ["invalid"] },
            ]
            "#,
        )
        .expect("toml should parse");
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Invalid keybinding"
        ));
    }

    #[test]
    fn as_string_list_accepts_string() {
        let values = as_string_list(&::toml::Value::String("close".to_string()))
            .expect("string list should parse");

        assert_eq!(values, vec!["close"]);
    }

    #[test]
    fn as_string_list_accepts_string_array() {
        let array = ::toml::Value::Array(vec![
            ::toml::Value::String("spawn".to_string()),
            ::toml::Value::String("foot".to_string()),
        ]);
        let values = as_string_list(&array).expect("string list should parse");

        assert_eq!(values, vec!["spawn", "foot"]);
    }

    #[test]
    fn as_string_list_rejects_non_string_values() {
        let error =
            as_string_list(&::toml::Value::Boolean(false)).expect_err("string list should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected string or array of strings"
        ));
    }

    #[test]
    fn parse_config_table_accepts_integer_resize_delta_ratio() {
        let config = parse(
            r#"
            resize_delta_ratio = 5
            "#,
        )
        .expect("integer resize_delta_ratio should coerce to f64");

        assert_eq!(config.resize_delta_ratio, Some(5.0));
    }

    #[test]
    fn parse_config_table_accepts_integer_resize_delta_percent() {
        let config = parse(
            r#"
            resize_delta_percent = 10
            "#,
        )
        .expect("integer resize_delta_percent should coerce to f32");

        assert_eq!(config.resize_delta_percent, Some(10.0));
    }

    #[test]
    fn parse_config_table_accepts_whole_number_float_border_width() {
        let config = parse(
            r#"
            border_width = 2.0
            "#,
        )
        .expect("whole-number float border_width should coerce to i32");

        assert_eq!(config.border_width, Some(2));
    }

    #[test]
    fn parse_config_table_rejects_fractional_float_border_width() {
        let value: ::toml::Value = ::toml::from_str(
            r#"
            border_width = 2.5
            "#,
        )
        .expect("toml should parse");
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected border_width to be an integer"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_table_top_level() {
        let value = ::toml::Value::Boolean(true);
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected config to be a table"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_integer_border_width() {
        let mut table = ::toml::Table::new();
        table.insert(
            "border_width".to_string(),
            ::toml::Value::String("bad".to_string()),
        );
        let value = ::toml::Value::Table(table);
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected border_width to be an integer"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_boolean_decorations() {
        let mut table = ::toml::Table::new();
        table.insert("decorations".to_string(), ::toml::Value::Integer(1));
        let value = ::toml::Value::Table(table);
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected decorations to be a boolean"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_number_resize_delta_ratio() {
        let mut table = ::toml::Table::new();
        table.insert(
            "resize_delta_ratio".to_string(),
            ::toml::Value::Boolean(false),
        );
        let value = ::toml::Value::Table(table);
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected resize_delta_ratio to be a number"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_number_resize_delta_percent() {
        let mut table = ::toml::Table::new();
        table.insert(
            "resize_delta_percent".to_string(),
            ::toml::Value::Boolean(false),
        );
        let value = ::toml::Value::Table(table);
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected resize_delta_percent to be a number"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_integer_border_color_focused() {
        let mut table = ::toml::Table::new();
        table.insert(
            "border_color_focused".to_string(),
            ::toml::Value::String("bad".to_string()),
        );
        let value = ::toml::Value::Table(table);
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected border_color_focused to be an integer"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_integer_border_color_unfocused() {
        let mut table = ::toml::Table::new();
        table.insert(
            "border_color_unfocused".to_string(),
            ::toml::Value::String("bad".to_string()),
        );
        let value = ::toml::Value::Table(table);
        let error = parse_config_table(value).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected border_color_unfocused to be an integer"
        ));
    }

    #[test]
    fn load_from_toml_reads_file() {
        let path = temp_toml_path();
        fs::write(
            &path,
            r#"
            keybindings = [
                { target = "all", keysym = "Return", modifiers = ["super"], command = ["spawn", "foot"] },
            ]
            "#,
        )
        .expect("temp config should be written");

        let config = super::load_from_toml(&path).expect("toml config should load");

        let _ = fs::remove_file(&path);

        assert_eq!(config.keybindings.len(), 1);
        assert_binding(
            &config.keybindings[0],
            &binding(
                KeyBindingTarget::All,
                keysyms::KEY_Return,
                SUPER,
                Command::Spawn {
                    program: "foot".to_string(),
                    args: Vec::new(),
                },
            ),
        );
    }

    #[test]
    fn load_from_toml_reads_full_example() {
        let config =
            load_from_toml(Path::new("examples/fenestre.toml")).expect("example should load");

        assert_eq!(config.keybindings.len(), 26);
        assert_eq!(config.rules.len(), 4);
        assert_eq!(config.layout.gap, Some(10));
        assert_eq!(config.border_width, Some(2));
    }

    #[test]
    fn load_from_toml_reads_minimal_example() {
        let config =
            super::load_from_toml(Path::new("examples/minimal.toml")).expect("example should load");

        assert_eq!(config.keybindings.len(), 2);
        assert!(config.rules.is_empty());
        assert_eq!(config.layout.gap, Some(4));
    }
}
