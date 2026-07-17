//! TOML configuration loader.
//!
//! This module is intentionally TOML-specific. It deserializes TOML into the
//! shared schema types defined in `schema`, then converts them into the runtime
//! `Config` via `build_config`.
use super::schema;
use super::{Config, Result};
use std::path::Path;

/// Load and parse a TOML configuration file.
pub(crate) fn load_from_toml(path: &Path) -> Result<Config> {
    let source = std::fs::read_to_string(path)?;
    let raw: schema::RawConfig = ::toml::from_str(&source)?;
    schema::build_config(raw)
}

#[cfg(test)]
mod tests {
    use super::{Config, Result, load_from_toml};
    use crate::command::Command;
    use crate::config::ConfigError;
    use crate::config::KeyBindingConfig;
    use crate::config::KeyBindingTarget;
    use crate::config::RulePattern;
    use crate::config::schema;
    use crate::layout::{Rect, WindowState};
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };
    use xkbcommon::xkb::keysyms;

    const SUPER: u32 = 64;
    const SHIFT: u32 = 1;

    fn parse_config_table(source: &str) -> Result<Config> {
        let raw: schema::RawConfig =
            ::toml::from_str(source).map_err(|e| ConfigError::InvalidConfig(e.to_string()))?;
        schema::build_config(raw)
    }

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

    fn parse(source: &str) -> Result<Config> {
        parse_config_table(source)
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
        assert_eq!(
            config.rules[0].target,
            WindowState::Floating {
                rect: Rect::new(0, 0, 0, 0)
            }
        );
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
        let error = parse_config_table(
            r#"
            rules = [
                { app_id = "foot", mode = "invalid" },
            ]
            "#,
        )
        .expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Invalid rule mode"
        ));
    }

    #[test]
    fn parse_config_table_rejects_empty_rule() {
        let error = parse_config_table(
            r#"
            rules = [
                { mode = "floating" },
            ]
            "#,
        )
        .expect_err("config should fail");

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
        assert_eq!(
            rule.target,
            WindowState::Fullscreen {
                restore: Box::new(WindowState::Tiled)
            }
        );
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
        let error = parse_config_table("keybindings = false\n").expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
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
        let error = parse_config_table(
            r#"
            keybindings = [
                { keysym = "Return", modifiers = ["super"], command = ["invalid"] },
            ]
            "#,
        )
        .expect_err("config should fail");

        assert!(matches!(
            error,
            ConfigError::InvalidConfig(message) if message == "Invalid keybinding"
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
    fn parse_config_table_rejects_whole_number_float_border_width() {
        let error = parse_config_table(
            r#"
            border_width = 2.0
            "#,
        )
        .expect_err("whole-number float border_width should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_whole_number_float_gap() {
        let error = parse_config_table(
            r#"
            layout = { gap = 4.0 }
            "#,
        )
        .expect_err("whole-number float gap should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_whole_number_float_border_color_focused() {
        let error = parse_config_table(
            r#"
            border_color_focused = 1.0
            "#,
        )
        .expect_err("whole-number float border_color_focused should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_fractional_float_border_width() {
        let error = parse_config_table(
            r#"
            border_width = 2.5
            "#,
        )
        .expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_non_table_top_level() {
        let error = parse_config_table("true").expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_non_integer_border_width() {
        let error = parse_config_table(r#"border_width = "bad""#).expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_non_boolean_decorations() {
        let error = parse_config_table(r#"decorations = 1"#).expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_non_number_resize_delta_ratio() {
        let error =
            parse_config_table(r#"resize_delta_ratio = false"#).expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_non_number_resize_delta_percent() {
        let error =
            parse_config_table(r#"resize_delta_percent = false"#).expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_non_integer_border_color_focused() {
        let error =
            parse_config_table(r#"border_color_focused = "bad""#).expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn parse_config_table_rejects_non_integer_border_color_unfocused() {
        let error = parse_config_table(r#"border_color_unfocused = "bad""#)
            .expect_err("config should fail");

        assert!(matches!(error, ConfigError::InvalidConfig(_)));
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
