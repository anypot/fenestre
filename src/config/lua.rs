//! Lua configuration loader.
//!
//! This module is intentionally Lua-specific. It converts Lua tables
//! into the shared config types defined in `config::mod`.
use super::{Config, ConfigError, KeyBindingConfig, LayoutConfig, Result, parser};
use mlua::{Lua, Table, Value};
use std::path::Path;

/// Load and parse a Lua configuration file.
pub(crate) fn load_from_lua(path: &Path) -> Result<Config> {
    let lua = Lua::new();
    let source = std::fs::read_to_string(path).map_err(mlua::Error::external)?;
    let table: Table = lua.load(source).call(())?;
    parse_config_table(table)
}

/// Parse the top-level Lua config table.
///
/// Missing `keybindings` is valid and means "no user keybindings".
/// Non-table `keybindings` is invalid.
fn parse_config_table(table: Table) -> Result<Config> {
    let keybindings = parse_keybindings_table(table.get("keybindings")?)?;
    let rules = parse_rules_table(table.get("rules")?)?;

    let layout = match table.get::<Option<Value>>("layout")? {
        Some(Value::Table(layout_table)) => parse_layout_table(layout_table)?,
        Some(_) => {
            return Err(ConfigError::InvalidConfig(
                "Expected layout to be a table".to_string(),
            ));
        }
        None => LayoutConfig::default(),
    };

    let decorations = table.get::<Option<bool>>("decorations")?.unwrap_or(true);

    let border_width = table.get::<Option<i32>>("border_width")?;
    let border_color_focused = table.get::<Option<u32>>("border_color_focused")?;
    let border_color_unfocused = table.get::<Option<u32>>("border_color_unfocused")?;
    let resize_delta_ratio = table.get::<Option<f64>>("resize_delta_ratio")?;
    let resize_delta_percent = table.get::<Option<f32>>("resize_delta_percent")?;

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

fn parse_lua_table_list<T, F>(value: Option<Value>, name: &str, parse_item: F) -> Result<Vec<T>>
where
    F: Fn(Table) -> Result<T>,
{
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    let Value::Table(table) = value else {
        return Err(ConfigError::InvalidConfig(format!(
            "Expected {} to be a table",
            name
        )));
    };

    let mut items = Vec::new();
    for pair in table.pairs::<Value, Table>() {
        let (_, item_table) = pair?;
        items.push(parse_item(item_table)?);
    }
    Ok(items)
}

fn parse_keybindings_table(keybindings_value: Option<Value>) -> Result<Vec<KeyBindingConfig>> {
    parse_lua_table_list(keybindings_value, "keybindings", parse_keybinding_table)
}

/// Parse a Lua layout table into `LayoutConfig`.
///
/// Supports both flat keys (`gap`, `margin_top`, ...) and a nested `margins`
/// table with `top`/`right`/`bottom`/`left` keys. The nested table takes
/// precedence for individual edges it defines.
fn parse_layout_table(table: Table) -> Result<LayoutConfig> {
    let gap = table.get::<Option<i32>>("gap")?;
    let margin_top = table.get::<Option<i32>>("margin_top")?;
    let margin_right = table.get::<Option<i32>>("margin_right")?;
    let margin_bottom = table.get::<Option<i32>>("margin_bottom")?;
    let margin_left = table.get::<Option<i32>>("margin_left")?;

    let margins = match table.get::<Option<Table>>("margins")? {
        Some(margins_table) => Some(parser::RawMargins {
            top: margins_table.get::<Option<i32>>("top")?,
            right: margins_table.get::<Option<i32>>("right")?,
            bottom: margins_table.get::<Option<i32>>("bottom")?,
            left: margins_table.get::<Option<i32>>("left")?,
        }),
        None => None,
    };

    Ok(parser::build_layout(
        gap,
        margin_top,
        margin_right,
        margin_bottom,
        margin_left,
        margins,
    ))
}

/// Parse one Lua keybinding table into a shared `KeyBindingConfig`.
fn parse_keybinding_table(binding: Table) -> Result<KeyBindingConfig> {
    let target_name: Option<String> = binding.get("target")?;
    let keysym_name: String = binding.get("keysym")?;
    let modifier_names = match binding.get::<Option<Value>>("modifiers")? {
        Some(value) => parse_string_list(value)?,
        None => {
            return Err(ConfigError::InvalidConfig(
                "Missing keybinding modifiers".to_string(),
            ));
        }
    };
    let command_tokens = match binding.get::<Option<Value>>("command")? {
        Some(value) => parse_string_list(value)?,
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

/// Parse a Lua string or array of strings.
///
/// This supports both:
///
/// ```lua
/// command = "close"
/// ```
///
/// and:
///
/// ```lua
/// command = { "spawn", "foot" }
/// ```
fn parse_string_list(value: mlua::Value) -> Result<Vec<String>> {
    let strings = match value {
        mlua::Value::String(string) => vec![string.to_str()?.to_string()],
        mlua::Value::Table(table) => {
            let mut values = Vec::new();
            for value in table.sequence_values::<String>() {
                values.push(value?);
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

fn parse_rules_table(rules_value: Option<Value>) -> Result<Vec<crate::state::rule::WindowRule>> {
    parse_lua_table_list(rules_value, "rules", parse_rule_table)
}

fn parse_rule_table(rule: Table) -> Result<crate::state::rule::WindowRule> {
    // `app_id` / `title` accept either a plain string (exact match) or a table
    // `{ value = "...", match = "exact" | "prefix" | "regex" }`. At least one
    // matcher must be present.
    let app_id = parse_pattern_field(&rule, "app_id")?;
    let title = parse_pattern_field(&rule, "title")?;

    let mode_str: String = rule.get("mode")?;

    let floating_rect = match rule.get::<Option<Value>>("floating_rect")? {
        Some(Value::Table(rect_table)) => {
            let x = rect_table.get::<Option<i32>>("x")?.unwrap_or(0);
            let y = rect_table.get::<Option<i32>>("y")?.unwrap_or(0);
            let width = rect_table.get::<Option<i32>>("width")?.unwrap_or(0);
            let height = rect_table.get::<Option<i32>>("height")?.unwrap_or(0);
            Some(crate::layout::Rect::new(x, y, width, height))
        }
        Some(_) => {
            return Err(ConfigError::InvalidConfig(
                "Expected floating_rect to be a table".to_string(),
            ));
        }
        None => None,
    };

    parser::build_rule(app_id, title, &mode_str, floating_rect)
}

/// Parse an `app_id`/`title` matcher: a plain string is an exact match; a table
/// `{ value, match }` selects `exact` (default), `prefix`, or `regex`.
fn parse_pattern_field(rule: &Table, name: &str) -> Result<Option<parser::RawPattern>> {
    match rule.get::<Option<Value>>(name)? {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(parser::RawPattern::Exact(s.to_str()?.to_string()))),
        Some(Value::Table(pattern)) => {
            let value: String = pattern.get("value")?;
            let mode = match pattern.get::<Option<Value>>("match")? {
                Some(Value::String(s)) => Some(s.to_str()?.to_string()),
                Some(_) => {
                    return Err(ConfigError::InvalidConfig(
                        "Expected match to be a string".to_string(),
                    ));
                }
                None => None,
            };
            Ok(Some(parser::build_pattern_field(name, value, mode)?))
        }
        Some(_) => Err(ConfigError::InvalidConfig(format!(
            "Expected {name} to be a string or table"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConfigError, KeyBindingConfig, load_from_lua, parse_config_table, parse_string_list,
    };
    use crate::command::Command;
    use crate::config::KeyBindingTarget;
    use mlua::{Lua, Value};
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

    fn temp_lua_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after UNIX_EPOCH")
            .as_nanos();

        std::env::temp_dir().join(format!("fenestre-config-{nanos}.lua"))
    }

    #[test]
    fn parse_config_table_accepts_empty_rules_table() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    rules = {},
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let config = parse_config_table(table).expect("config should parse");

        assert!(config.rules.is_empty());
    }

    #[test]
    fn parse_config_table_parses_valid_rules() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    rules = {
                        {
                            app_id = "foot",
                            mode = "floating",
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let config = parse_config_table(table).expect("config should parse");

        assert_eq!(config.rules.len(), 1);
        assert!(matches!(
            config.rules[0].app_id,
            Some(crate::state::rule::RulePattern::Exact(ref s)) if s == "foot"
        ));
        assert_eq!(config.rules[0].target, crate::layout::WindowState::Floating);
    }

    #[test]
    fn parse_config_table_parses_rule_prefix() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    rules = {
                        {
                            app_id = { value = "mate-", match = "prefix" },
                            mode = "floating",
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let config = parse_config_table(table).expect("config should parse");

        assert!(matches!(
            config.rules[0].app_id,
            Some(crate::state::rule::RulePattern::Prefix(ref s)) if s == "mate-"
        ));
    }

    #[test]
    fn parse_config_table_rejects_invalid_rule_mode() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    rules = {
                        {
                            app_id = "foot",
                            mode = "invalid",
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let error = parse_config_table(table).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Invalid rule mode"
        ));
    }

    #[test]
    fn parse_config_table_rejects_empty_rule() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    rules = {
                        {
                            mode = "floating",
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let error = parse_config_table(table).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Rule must match at least one of app_id or title"
        ));
    }

    #[test]
    fn parse_config_table_rejects_non_string_match_mode() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    rules = {
                        {
                            app_id = { value = "foot", match = 123 },
                            mode = "floating",
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let error = parse_config_table(table).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected match to be a string"
        ));
    }

    #[test]
    fn parse_config_table_parses_rule_regex_and_rect() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    rules = {
                        {
                            app_id = { value = "^foot.*", match = "regex" },
                            title = { value = "terminal$", match = "regex" },
                            mode = "fullscreen",
                            floating_rect = { x = 10, y = 20, width = 800, height = 600 },
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let config = parse_config_table(table).expect("config should parse");

        let rule = &config.rules[0];
        assert!(matches!(
            rule.app_id,
            Some(crate::state::rule::RulePattern::Regex(_))
        ));
        assert!(matches!(
            rule.title,
            Some(crate::state::rule::RulePattern::Regex(_))
        ));
        assert_eq!(rule.target, crate::layout::WindowState::Fullscreen);
        assert_eq!(
            rule.floating_rect,
            Some(crate::layout::Rect::new(10, 20, 800, 600))
        );
    }

    #[test]
    fn parse_config_table_missing_keybindings_returns_empty_config() {
        let lua = Lua::new();
        let table = lua.create_table().expect("table should be created");

        let config = parse_config_table(table).expect("config should parse");

        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn parse_config_table_accepts_empty_keybindings_table() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    keybindings = {},
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let config = parse_config_table(table).expect("config should parse");

        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn parse_config_table_rejects_non_table_keybindings() {
        let lua = Lua::new();
        let table = lua.create_table().expect("table should be created");
        table
            .set("keybindings", false)
            .expect("keybindings value should be set");

        let error = parse_config_table(table).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected keybindings to be a table"
        ));
    }

    #[test]
    fn parse_config_table_parses_valid_keybindings() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    keybindings = {
                        {
                            keysym = "q",
                            modifiers = "super",
                            command = "close",
                        },
                        {
                            target = "all",
                            keysym = "Return",
                            modifiers = { "super" },
                            command = { "spawn", "foot" },
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let config = parse_config_table(table).expect("config should parse");

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
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    keybindings = {
                        {
                            keysym = "h",
                            modifiers = { "super", "shift" },
                            command = "move_left",
                        },
                        {
                            keysym = "l",
                            modifiers = { "super", "shift" },
                            command = "move_right",
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let config = parse_config_table(table).expect("config should parse");

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
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(
                r#"
                return {
                    keybindings = {
                        {
                            keysym = "Return",
                            modifiers = { "super" },
                            command = { "invalid" },
                        },
                    },
                }
                "#,
            )
            .call(())
            .expect("lua chunk should return a table");

        let error = parse_config_table(table).expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Invalid keybinding"
        ));
    }

    #[test]
    fn parse_string_list_accepts_string() {
        let lua = Lua::new();
        let value = Value::String(
            lua.create_string("close")
                .expect("string should be created"),
        );

        let values = parse_string_list(value).expect("string list should parse");

        assert_eq!(values, vec!["close"]);
    }

    #[test]
    fn parse_string_list_accepts_string_array() {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(r#"return { "spawn", "foot" }"#)
            .call(())
            .expect("lua chunk should return a table");

        let values = parse_string_list(Value::Table(table)).expect("string list should parse");

        assert_eq!(values, vec!["spawn", "foot"]);
    }

    #[test]
    fn parse_string_list_rejects_non_string_values() {
        let error = parse_string_list(Value::Nil).expect_err("string list should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Expected string or array of strings"
        ));
    }

    #[test]
    fn load_from_lua_reads_file() {
        let path = temp_lua_path();
        fs::write(
            &path,
            r#"
            return {
                keybindings = {
                    {
                        keysym = "Return",
                        modifiers = { "super" },
                        command = { "spawn", "foot" },
                    },
                },
            }
            "#,
        )
        .expect("temp config should be written");

        let config = super::load_from_lua(&path).expect("lua config should load");

        let _ = fs::remove_file(&path);

        assert_eq!(config.keybindings.len(), 1);
        assert_binding(
            &config.keybindings[0],
            &binding(
                KeyBindingTarget::Primary,
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
    fn load_from_lua_reads_full_example() {
        let config =
            load_from_lua(Path::new("examples/fenestre.lua")).expect("example should load");

        assert_eq!(config.keybindings.len(), 26);
        assert_eq!(config.rules.len(), 4);
        assert_eq!(config.layout.gap, Some(10));
        assert_eq!(config.border_width, Some(2));
    }

    #[test]
    fn load_from_lua_reads_advanced_example() {
        let config =
            load_from_lua(Path::new("examples/advanced.lua")).expect("example should load");

        // 2 base bindings + 4 directions * 4 (focus/move/resize-expand/shrink).
        assert_eq!(config.keybindings.len(), 18);
        assert_eq!(config.rules.len(), 3);
        assert_eq!(config.layout.gap, Some(10));
        assert_eq!(config.border_width, Some(2));
    }
}
