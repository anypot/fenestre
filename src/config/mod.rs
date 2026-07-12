//! Configuration model for Fenestre.
//!
//! The config layer is format-aware at the loader boundary but format-neutral
//! internally. Lua-specific parsing lives in `lua.rs` and TOML-specific parsing
//! in `toml.rs`, while this module owns the shared `Config`,
//! `KeyBindingConfig`, and merge behavior. TOML takes precedence over Lua:
//! when both `fenestre.toml` and `fenestre.lua` exist, only the TOML file is
//! loaded. If the TOML file fails to parse, Fenestre falls back to built-in
//! defaults; it does not retry the Lua file.
pub mod defaults;
mod lua;
pub mod parser;
mod toml;

use crate::command::Command;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Precomputed RGBA color components in 0-255 range.
pub(super) type Rgba = (u32, u32, u32, u32);

/// Result type for configuration loading and parsing.
pub type Result<T> = std::result::Result<T, ConfigError>;

/// Configuration loading error.
#[derive(Debug)]
pub enum ConfigError {
    /// Filesystem error while reading a config file.
    Io(std::io::Error),

    /// Lua interpreter or Lua config conversion error.
    Lua(mlua::Error),

    /// TOML deserialization error.
    Toml(::toml::de::Error),

    /// Config value was syntactically or semantically invalid.
    InvalidConfig(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "I/O error while loading config: {err}"),
            ConfigError::Lua(err) => write!(f, "Lua config error: {err}"),
            ConfigError::Toml(err) => write!(f, "TOML config error: {err}"),
            ConfigError::InvalidConfig(message) => write!(f, "Invalid config: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(err) => Some(err),
            ConfigError::Lua(err) => Some(err),
            ConfigError::Toml(err) => Some(err),
            ConfigError::InvalidConfig(_) => None,
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<mlua::Error> for ConfigError {
    fn from(value: mlua::Error) -> Self {
        Self::Lua(value)
    }
}

impl From<::toml::de::Error> for ConfigError {
    fn from(value: ::toml::de::Error) -> Self {
        Self::Toml(value)
    }
}

/// Target seats for a keybinding.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum KeyBindingTarget {
    /// Apply the binding to the primary seat.
    Primary,

    /// Apply the binding to every known seat.
    All,
}

/// Declarative keybinding from configuration.
#[derive(Debug, Clone)]
pub struct KeyBindingConfig {
    /// Seat target for this binding.
    pub target: KeyBindingTarget,

    /// XKB keysym value.
    pub keysym: u32,

    /// River modifier bitmask.
    pub modifiers: u32,

    /// Command to run when the binding is pressed.
    pub command: Command,
}

impl KeyBindingConfig {
    fn identity(&self) -> KeyBindingIdentity {
        KeyBindingIdentity {
            target: self.target,
            keysym: self.keysym,
            modifiers: self.modifiers,
        }
    }
}

/// Identity used to match keybindings during config merging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct KeyBindingIdentity {
    target: KeyBindingTarget,
    keysym: u32,
    modifiers: u32,
}

/// Layout configuration for tiled windows.
///
/// All fields are `Option` so that `None` means "inherit the default" and
/// `Some(0)` explicitly disables the feature. This lets users opt back out
/// of a gap or margin without editing the default config.
#[derive(Debug, Clone, Default)]
pub struct LayoutConfig {
    /// Pixel gap between adjacent tiled windows.
    pub gap: Option<i32>,

    /// Top margin inset for the tiling area.
    pub margin_top: Option<i32>,

    /// Right margin inset for the tiling area.
    pub margin_right: Option<i32>,

    /// Bottom margin inset for the tiling area.
    pub margin_bottom: Option<i32>,

    /// Left margin inset for the tiling area.
    pub margin_left: Option<i32>,
}

/// Complete Fenêtre configuration.
///
/// Values use `Option` when the distinction between "unset" and "explicitly
/// disabled" matters (layout and border fields). Booleans and keybinding
/// lists are always explicit.
#[derive(Debug, Clone)]
pub struct Config {
    /// Layout configuration (gap and margins).
    pub layout: LayoutConfig,

    /// Whether client-side decorations are enabled.
    pub decorations: bool,

    /// Border width in pixels. 0 disables compositor borders.
    pub border_width: Option<i32>,

    /// Focused window border color in 0xAARRGGBB format.
    pub border_color_focused: Option<u32>,

    /// Unfocused window border color in 0xAARRGGBB format.
    pub border_color_unfocused: Option<u32>,

    /// Configured keybindings.
    pub keybindings: Vec<KeyBindingConfig>,

    /// Tiling resize ratio delta per resize command press.
    pub resize_delta_ratio: Option<f64>,

    /// Floating resize dimension delta as percentage of output size.
    pub resize_delta_percent: Option<f32>,

    /// Per-window rules applied on metadata arrival.
    pub rules: Vec<crate::state::rule::WindowRule>,
}

impl Config {
    /// Precompute RGBA values from ARGB border colors for efficient rendering.
    ///
    /// Returns `(focused_rgba, unfocused_rgba)` where each RGBA tuple is
    /// `(red, green, blue, alpha)` in 0-255 range suitable for passing
    /// directly to `set_borders`.
    pub(super) fn border_rgba(&self) -> (Rgba, Rgba) {
        let to_rgba = |argb: u32| -> Rgba {
            let to_u32 = |v: u32| (v as u64 * 0xffffffff / 255) as u32;
            (
                to_u32((argb >> 16) & 0xff),
                to_u32((argb >> 8) & 0xff),
                to_u32(argb & 0xff),
                to_u32((argb >> 24) & 0xff),
            )
        };

        let focused = self.border_color_focused.unwrap_or(0xffffffff);
        let unfocused = self.border_color_unfocused.unwrap_or(0xffffffff);
        (to_rgba(focused), to_rgba(unfocused))
    }

    /// Load the built-in default configuration.
    pub fn load() -> Self {
        defaults::defaults()
    }

    /// Load a Lua config file and merge it over the defaults.
    fn load_lua(path: &Path) -> Result<Self> {
        let mut config = defaults::defaults();
        let user_config = lua::load_from_lua(path)?;
        config.merge(user_config);
        Ok(config)
    }

    /// Load a TOML config file and merge it over the defaults.
    fn load_toml(path: &Path) -> Result<Self> {
        let mut config = defaults::defaults();
        let user_config = toml::load_from_toml(path)?;
        config.merge(user_config);
        Ok(config)
    }

    /// Return the default user config path, if an existing config file is found.
    ///
    /// TOML is preferred over Lua. The search order is:
    ///
    /// 1. `$XDG_CONFIG_HOME/fenestre/fenestre.toml`
    /// 2. `~/.config/fenestre/fenestre.toml`
    /// 3. `$XDG_CONFIG_HOME/fenestre/fenestre.lua`
    /// 4. `~/.config/fenestre/fenestre.lua`
    ///
    /// If a TOML file exists but fails to parse, Fenestre logs a warning and
    /// uses built-in defaults; it does not fall back to a coexisting Lua file.
    pub(crate) fn default_path() -> Option<PathBuf> {
        Self::xdg_toml_path()
            .or_else(Self::home_toml_path)
            .or_else(Self::xdg_config_path)
            .or_else(Self::home_config_path)
    }

    fn resolve_config_path(env_var: &str, components: &[&str]) -> Option<PathBuf> {
        let value = std::env::var_os(env_var)?;
        if value.is_empty() {
            return None;
        }
        let path = components
            .iter()
            .fold(PathBuf::from(value), |path, c| path.join(c));
        path.exists().then_some(path)
    }

    fn xdg_config_path() -> Option<PathBuf> {
        Self::resolve_config_path("XDG_CONFIG_HOME", &["fenestre", "fenestre.lua"])
    }

    fn home_config_path() -> Option<PathBuf> {
        Self::resolve_config_path("HOME", &[".config", "fenestre", "fenestre.lua"])
    }

    fn xdg_toml_path() -> Option<PathBuf> {
        Self::resolve_config_path("XDG_CONFIG_HOME", &["fenestre", "fenestre.toml"])
    }

    fn home_toml_path() -> Option<PathBuf> {
        Self::resolve_config_path("HOME", &[".config", "fenestre", "fenestre.toml"])
    }

    /// Load configuration from a path.
    ///
    /// Supports `.lua` and `.toml` files. Paths without an extension are treated
    /// as Lua for compatibility with explicit config-file usage.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref()
        {
            Some("lua") => Self::load_lua(path),
            Some("toml") => Self::load_toml(path),
            Some(ext) => Err(ConfigError::InvalidConfig(format!(
                "Unsupported config file extension: {ext}"
            ))),
            None => Self::load_lua(path),
        }
    }

    /// Merge `other` into this config.
    ///
    /// Bindings are matched by identity: `target + keysym + modifiers`.
    /// Matching user bindings override defaults. New user bindings are appended.
    /// Default bindings are indexed in a temporary HashMap so overrides are O(1)
    /// instead of O(n) per user binding. Duplicates are removed.
    ///
    /// Layout fields are overwritten only when present (Some). None means unset,
    /// so a user can explicitly set `gap = 0` to disable it.
    ///
    /// Decorations and border fields are always overwritten by the user config
    /// when present, and colors update independently of `border_width`.
    ///
    /// Window rules are fully replaced: the user's rules list replaces the
    /// defaults list in its entirety. This is safe because `merge` is only
    /// called on `Config::defaults()` (which ships no rules), so the replace
    /// behaves like an append. It is not intended for partial overlays or
    /// runtime rule additions.
    fn merge(&mut self, other: Self) {
        if let Some(gap) = other.layout.gap {
            self.layout.gap = Some(gap);
        }
        if let Some(margin_top) = other.layout.margin_top {
            self.layout.margin_top = Some(margin_top);
        }
        if let Some(margin_right) = other.layout.margin_right {
            self.layout.margin_right = Some(margin_right);
        }
        if let Some(margin_bottom) = other.layout.margin_bottom {
            self.layout.margin_bottom = Some(margin_bottom);
        }
        if let Some(margin_left) = other.layout.margin_left {
            self.layout.margin_left = Some(margin_left);
        }
        self.decorations = other.decorations;
        if let Some(border_width) = other.border_width {
            self.border_width = Some(border_width);
        }
        if let Some(border_color_focused) = other.border_color_focused {
            self.border_color_focused = Some(border_color_focused);
        }
        if let Some(border_color_unfocused) = other.border_color_unfocused {
            self.border_color_unfocused = Some(border_color_unfocused);
        }
        if let Some(resize_delta_ratio) = other.resize_delta_ratio {
            self.resize_delta_ratio = Some(resize_delta_ratio);
        }
        if let Some(resize_delta_percent) = other.resize_delta_percent {
            self.resize_delta_percent = Some(resize_delta_percent);
        }
        // Replaced, not identity-merged: defaults ship no rules by design.
        self.rules = other.rules;
        // Pre-index defaults for O(1) identity lookup instead of linear scan.
        let default_indices: HashMap<KeyBindingIdentity, usize> = self
            .keybindings
            .iter()
            .enumerate()
            .map(|(i, b)| (b.identity(), i))
            .collect();

        for binding in other.keybindings {
            let identity = binding.identity();
            if let Some(&index) = default_indices.get(&identity) {
                self.keybindings[index] = binding;
            } else {
                self.keybindings.push(binding);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, KeyBindingConfig, KeyBindingTarget, LayoutConfig};
    use crate::command::Command;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };
    use xkbcommon::xkb::keysyms;

    const SHIFT: u32 = 1;
    const SUPER: u32 = 64;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after UNIX_EPOCH")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("fenestre-config-test-{name}-{nanos}"));
        fs::create_dir_all(&root).expect("temp root should be created");
        root
    }

    fn write_config(path: &Path) {
        fs::create_dir_all(path.parent().expect("config path should have parent"))
            .expect("config parent should be created");
        fs::write(path, "keybindings = {}\n").expect("config should be written");
    }

    fn with_config_env<R>(
        xdg_config_home: Option<&Path>,
        home: Option<&Path>,
        f: impl FnOnce() -> R,
    ) -> R {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        let old_home = std::env::var_os("HOME");

        // `std::env::{set_var,remove_var}` are declared `unsafe fn` in Rust's
        // standard library because they mutate the global process environment,
        // which is a shared, unsynchronised resource in multi-threaded programs.
        // This guard is single-threaded and the lock below serialises test access,
        // so the calls are safe in this context — the `unsafe` block acknowledges
        // the contract rather than adding any additional protection.
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HOME");
        }
        if let Some(path) = xdg_config_home {
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", path);
            }
        }
        if let Some(path) = home {
            unsafe {
                std::env::set_var("HOME", path);
            }
        }

        let result = f();

        unsafe {
            if let Some(value) = old_xdg_config_home {
                std::env::set_var("XDG_CONFIG_HOME", value);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
            if let Some(value) = old_home {
                std::env::set_var("HOME", value);
            } else {
                std::env::remove_var("HOME");
            }
        }

        result
    }

    #[test]
    fn default_path_uses_existing_xdg_config_home_file() {
        let root = temp_root("xdg");
        let xdg_config = root.join("xdg").join("fenestre").join("fenestre.lua");
        let home_config = root
            .join("home")
            .join(".config")
            .join("fenestre")
            .join("fenestre.lua");
        write_config(&xdg_config);
        write_config(&home_config);

        let result = with_config_env(
            Some(&root.join("xdg")),
            Some(&root.join("home")),
            Config::default_path,
        );

        assert_eq!(result, Some(xdg_config));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn default_path_falls_back_to_home_config_file() {
        let root = temp_root("home");
        let xdg_root = root.join("xdg");
        let home_config = root
            .join("home")
            .join(".config")
            .join("fenestre")
            .join("fenestre.lua");
        write_config(&home_config);

        let result = with_config_env(
            Some(&xdg_root),
            Some(&root.join("home")),
            Config::default_path,
        );

        assert_eq!(result, Some(home_config));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn default_path_is_none_when_no_existing_config_file_exists() {
        let root = temp_root("none");

        let result = with_config_env(
            Some(&root.join("xdg")),
            Some(&root.join("home")),
            Config::default_path,
        );

        assert_eq!(result, None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn default_path_prefers_toml_over_lua() {
        let root = temp_root("prefer-toml");
        let xdg_toml = root.join("xdg").join("fenestre").join("fenestre.toml");
        let xdg_lua = root.join("xdg").join("fenestre").join("fenestre.lua");
        write_config(&xdg_toml);
        write_config(&xdg_lua);

        let result = with_config_env(
            Some(&root.join("xdg")),
            Some(&root.join("home")),
            Config::default_path,
        );

        assert_eq!(result, Some(xdg_toml));
        let _ = fs::remove_dir_all(root);
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

    fn assert_binding(binding: &KeyBindingConfig, expected: &KeyBindingConfig) {
        assert_eq!(binding.target, expected.target);
        assert_eq!(binding.keysym, expected.keysym);
        assert_eq!(binding.modifiers, expected.modifiers);
        assert_eq!(binding.command, expected.command);
    }

    fn layout_config() -> LayoutConfig {
        LayoutConfig {
            gap: None,
            margin_top: None,
            margin_right: None,
            margin_bottom: None,
            margin_left: None,
        }
    }

    type DecorationDefaults = (
        bool,
        Option<i32>,
        Option<u32>,
        Option<u32>,
        Option<f64>,
        Option<f32>,
    );

    fn decoration_defaults() -> DecorationDefaults {
        (true, None, Some(0xffffffff), Some(0xffffffff), None, None)
    }

    fn make_config(keybindings: Vec<KeyBindingConfig>) -> Config {
        let (
            decorations,
            border_width,
            border_color_focused,
            border_color_unfocused,
            resize_delta_ratio,
            resize_delta_percent,
        ) = decoration_defaults();
        Config {
            layout: layout_config(),
            decorations,
            border_width,
            border_color_focused,
            border_color_unfocused,
            resize_delta_ratio,
            resize_delta_percent,
            keybindings,
            rules: Vec::new(),
        }
    }

    #[test]
    fn load_returns_default_config() {
        let config = Config::load();

        assert!(!config.keybindings.is_empty());
    }

    #[test]
    fn merge_overrides_matching_identity_and_preserves_order() {
        let default_focus = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::CloseFocused,
        );
        let default_tab = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_Tab,
            SUPER,
            Command::FocusNext,
        );
        let override_focus = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::FocusNext,
        );

        let mut config = make_config(vec![default_focus, default_tab.clone()]);

        config.merge(make_config(vec![override_focus.clone()]));

        assert_eq!(config.keybindings.len(), 2);
        assert_binding(&config.keybindings[0], &override_focus);
        assert_binding(&config.keybindings[1], &default_tab);
    }

    #[test]
    fn merge_appends_new_identity() {
        let default_binding = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::CloseFocused,
        );
        let new_binding = binding(
            KeyBindingTarget::All,
            keysyms::KEY_Return,
            SUPER,
            Command::Spawn {
                program: "foot".to_string(),
                args: Vec::new(),
            },
        );

        let mut config = make_config(vec![default_binding.clone()]);

        config.merge(make_config(vec![new_binding.clone()]));

        assert_eq!(config.keybindings.len(), 2);
        assert_binding(&config.keybindings[0], &default_binding);
        assert_binding(&config.keybindings[1], &new_binding);
    }

    #[test]
    fn merge_last_duplicate_user_binding_wins() {
        let default_binding = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::CloseFocused,
        );
        let first_override = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::FocusNext,
        );
        let final_override = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::FocusPrevious,
        );

        let mut config = make_config(vec![default_binding]);

        config.merge(make_config(vec![first_override, final_override.clone()]));

        assert_eq!(config.keybindings.len(), 1);
        assert_binding(&config.keybindings[0], &final_override);
    }

    #[test]
    fn merge_empty_user_config_preserves_defaults() {
        let default_binding = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::CloseFocused,
        );

        let mut config = make_config(vec![default_binding.clone()]);

        config.merge(make_config(Vec::new()));

        assert_eq!(config.keybindings.len(), 1);
        assert_binding(&config.keybindings[0], &default_binding);
    }

    #[test]
    fn merge_does_not_override_different_modifiers() {
        let default_binding = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::CloseFocused,
        );
        let shifted_binding = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SHIFT | SUPER,
            Command::FocusNext,
        );

        let mut config = make_config(vec![default_binding.clone()]);

        config.merge(make_config(vec![shifted_binding.clone()]));

        assert_eq!(config.keybindings.len(), 2);
        assert_binding(&config.keybindings[0], &default_binding);
        assert_binding(&config.keybindings[1], &shifted_binding);
    }

    #[test]
    fn merge_does_not_override_different_target() {
        let primary_binding = binding(
            KeyBindingTarget::Primary,
            keysyms::KEY_q,
            SUPER,
            Command::CloseFocused,
        );
        let all_binding = binding(
            KeyBindingTarget::All,
            keysyms::KEY_q,
            SUPER,
            Command::FocusNext,
        );

        let mut config = make_config(vec![primary_binding.clone()]);

        config.merge(make_config(vec![all_binding.clone()]));

        assert_eq!(config.keybindings.len(), 2);
        assert_binding(&config.keybindings[0], &primary_binding);
        assert_binding(&config.keybindings[1], &all_binding);
    }

    #[test]
    fn merge_overrides_decorations() {
        let mut config = make_config(Vec::new());

        let override_config = Config {
            layout: layout_config(),
            decorations: false,
            border_width: Some(2),
            border_color_focused: Some(0xff0000ff),
            border_color_unfocused: Some(0x00ff00ff),
            resize_delta_ratio: None,
            resize_delta_percent: None,
            keybindings: Vec::new(),
            rules: Vec::new(),
        };

        config.merge(override_config);

        assert!(!config.decorations);
        assert_eq!(config.border_width, Some(2));
        assert_eq!(config.border_color_focused, Some(0xff0000ff));
        assert_eq!(config.border_color_unfocused, Some(0x00ff00ff));
    }

    #[test]
    fn load_from_path_rejects_unsupported_extension() {
        let error =
            Config::load_from_path(Path::new("config.yaml")).expect_err("config should fail");

        assert!(matches!(
            error,
            super::ConfigError::InvalidConfig(message) if message.contains("Unsupported config file extension")
        ));
    }
}
