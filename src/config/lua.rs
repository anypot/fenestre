//! Lua configuration loader.
//!
//! This module is intentionally Lua-specific. It converts Lua tables
//! into the shared schema types defined in `schema`, then validates and
//! converts them into the runtime `Config` via `build_config`.
use super::schema;
use super::{Config, ConfigError, Result};
use mlua::{Lua, Table, Value};
use std::path::Path;

/// Load and parse a Lua configuration file.
pub(crate) fn load_from_lua(path: &Path) -> Result<Config> {
    let lua = Lua::new();
    let source = std::fs::read_to_string(path).map_err(mlua::Error::external)?;
    let table: Table = lua.load(source).call(())?;
    let json = schema::mlua_value_to_json_value(Value::Table(table))?;
    let raw: schema::RawConfig = serde_json::from_value(json).map_err(|e| {
        let msg = e.to_string();
        // Strip the serde_json " at line X column Y" suffix, which refers to
        // the intermediate JSON representation, not the Lua source file.
        let msg = msg.split(" at line ").next().unwrap_or(&msg).trim_end();
        ConfigError::InvalidConfig(msg.to_string())
    })?;
    schema::build_config(raw)
}

#[cfg(test)]
mod tests {
    use super::load_from_lua;
    use crate::command::Command;
    use crate::config::ConfigError;
    use crate::config::KeyBindingTarget;
    use crate::config::schema;
    use mlua::Lua;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };
    use xkbcommon::xkb::keysyms;

    const SUPER: u32 = 64;
    const SHIFT: u32 = 1;

    fn assert_binding(
        binding: &crate::config::KeyBindingConfig,
        expected: &crate::config::KeyBindingConfig,
    ) {
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
    ) -> crate::config::KeyBindingConfig {
        crate::config::KeyBindingConfig {
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

    fn parse_lua_table(lua_source: &str) -> Result<crate::config::Config, ConfigError> {
        let lua = Lua::new();
        let table: mlua::Table = lua
            .load(lua_source)
            .call(())
            .expect("lua chunk should return a table");
        let json = schema::mlua_value_to_json_value(mlua::Value::Table(table))?;
        let raw: schema::RawConfig = serde_json::from_value(json).map_err(|e| {
            let msg = e.to_string();
            let msg = msg.split(" at line ").next().unwrap_or(&msg).trim_end();
            ConfigError::InvalidConfig(msg.to_string())
        })?;
        schema::build_config(raw)
    }

    #[test]
    fn parse_config_table_accepts_empty_rules_table() {
        let config = parse_lua_table(
            r#"
            return {
                rules = {},
            }
            "#,
        )
        .expect("config should parse");

        assert!(config.rules.is_empty());
    }

    #[test]
    fn parse_config_table_parses_valid_rules() {
        let config = parse_lua_table(
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
        .expect("config should parse");

        assert_eq!(config.rules.len(), 1);
        assert!(matches!(
            config.rules[0].app_id,
            Some(crate::state::rule::RulePattern::Exact(ref s)) if s == "foot"
        ));
        assert_eq!(
            config.rules[0].target,
            crate::layout::WindowState::Floating {
                rect: crate::layout::Rect::new(0, 0, 0, 0)
            }
        );
    }

    #[test]
    fn parse_config_table_parses_rule_prefix() {
        let config = parse_lua_table(
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
        .expect("config should parse");

        assert!(matches!(
            config.rules[0].app_id,
            Some(crate::state::rule::RulePattern::Prefix(ref s)) if s == "mate-"
        ));
    }

    #[test]
    fn parse_config_table_parses_rule_regex_and_rect() {
        let config = parse_lua_table(
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
        .expect("config should parse");

        let rule = &config.rules[0];
        assert!(matches!(
            rule.app_id,
            Some(crate::state::rule::RulePattern::Regex(_))
        ));
        assert!(matches!(
            rule.title,
            Some(crate::state::rule::RulePattern::Regex(_))
        ));
        assert_eq!(
            rule.target,
            crate::layout::WindowState::Fullscreen {
                restore: Box::new(crate::layout::WindowState::Tiled)
            }
        );
        assert_eq!(
            rule.floating_rect,
            Some(crate::layout::Rect::new(10, 20, 800, 600))
        );
    }

    #[test]
    fn parse_config_table_accepts_empty_keybindings_table() {
        let config = parse_lua_table(
            r#"
            return {
                keybindings = {},
            }
            "#,
        )
        .expect("config should parse");

        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn parse_config_table_parses_valid_keybindings() {
        let config = parse_lua_table(
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
        let config = parse_lua_table(
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
        let error = parse_lua_table(
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
        .expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Invalid keybinding"
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

        let config = load_from_lua(&path).expect("lua config should load");

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
