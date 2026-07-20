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
mod rule_types;
pub mod schema;
mod toml;

pub(crate) use rule_types::{RulePattern, WindowRule};

use crate::command::Command;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Precomputed RGBA color components in 0-255 range.
pub(crate) type Rgba = (u32, u32, u32, u32);

/// Convert an ARGB hex value (0xAARRGGBB) to an RGBA tuple with each
/// component scaled from 8-bit to the full 32-bit range (0–0xFFFFFFFF),
/// matching River's `set_borders` protocol expectations.
pub(crate) fn argb_to_rgba(argb: u32) -> Rgba {
    let scale = |v: u32| (v as u64 * u64::from(u32::MAX) / 255) as u32;
    (
        scale((argb >> 16) & 0xff),
        scale((argb >> 8) & 0xff),
        scale(argb & 0xff),
        scale((argb >> 24) & 0xff),
    )
}

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

/// Validate that a ratio parameter lies in the `[0.0, 1.0]` range.
///
/// `None` is treated as unset and passes through unchanged. Returns
/// `Err(ConfigError::InvalidConfig)` when the value is present but outside
/// the valid range.
pub(crate) fn validate_ratio(name: &str, value: Option<f32>) -> Result<Option<f32>> {
    match value {
        Some(v) if (0.0..=1.0).contains(&v) => Ok(Some(v)),
        Some(v) => Err(ConfigError::InvalidConfig(format!(
            "{name} must be between 0.0 and 1.0, got {v}"
        ))),
        None => Ok(None),
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

/// Interactive pointer operation triggered by a pointer binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerOp {
    /// Interactive move of the focused window.
    Move,
    /// Interactive resize of the focused window.
    Resize,
}

/// Declarative pointer binding from configuration.
///
/// Binds a pointer button plus keyboard modifiers to an interactive operation
/// (move/resize) on the focused window. On press, Fenestre starts a River
/// pointer operation (`op_start_pointer`) and drives the window geometry from
/// the cumulative `op_delta` events.
#[derive(Debug, Clone)]
pub struct PointerBindingConfig {
    /// Seat target for this binding.
    pub target: KeyBindingTarget,

    /// Linux input event code for the pointer button (e.g. `BTN_LEFT`).
    pub button: u32,

    /// River modifier bitmask.
    pub modifiers: u32,

    /// Interactive operation to perform on press.
    pub op: PointerOp,
}

impl PointerBindingConfig {
    /// Identity used to match pointer bindings during config merging.
    fn identity(&self) -> PointerBindingIdentity {
        PointerBindingIdentity {
            target: self.target,
            button: self.button,
            modifiers: self.modifiers,
        }
    }
}

/// Identity used to match pointer bindings during config merging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PointerBindingIdentity {
    target: KeyBindingTarget,
    button: u32,
    modifiers: u32,
}

/// Input device acceleration profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelProfile {
    None,
    Flat,
    Adaptive,
    Custom,
}

/// Tap-to-click button mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapButtonMap {
    Lrm,
    Lmr,
}

/// Scroll method for pointer devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollMethod {
    None,
    TwoFinger,
    Edge,
    OnButtonDown,
}

/// Drag lock state for touchpad devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragLockState {
    Disabled,
    EnabledTimeout,
    EnabledSticky,
}

/// Click method for touchpad devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickMethod {
    None,
    ButtonAreas,
    Clickfinger,
}

/// Send events mode for input devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendEventsMode {
    Enabled,
    Disabled,
    DisabledOnExternalMouse,
}

/// Keyboard layout configuration.
#[derive(Debug, Clone)]
pub struct KeyboardLayoutConfig {
    pub rules: Option<String>,
    pub model: Option<String>,
    pub layout: String,
    pub variant: Option<String>,
    pub options: Option<String>,
}

/// Per-input-device configuration.
#[derive(Debug, Clone)]
pub struct InputDeviceConfig {
    pub name: String,
    pub accel_profile: Option<AccelProfile>,
    pub accel_speed: Option<f64>,
    pub scroll_factor: Option<f64>,
    pub repeat_rate: Option<i32>,
    pub repeat_delay: Option<i32>,
    pub tap: Option<bool>,
    pub tap_button_map: Option<TapButtonMap>,
    pub natural_scroll: Option<bool>,
    pub left_handed: Option<bool>,
    pub scroll_method: Option<ScrollMethod>,
    pub middle_emulation: Option<bool>,
    pub dwt: Option<bool>,
    pub send_events: Option<SendEventsMode>,
    pub drag: Option<bool>,
    pub drag_lock: Option<DragLockState>,
    pub click_method: Option<ClickMethod>,
    pub rotation: Option<u32>,
}

/// Output-relative window sizing and positioning defaults.
///
/// Covers the tiling area (gap, margins) and the default size used for
/// floating / pseudo-tiled windows that have not reported their own
/// dimensions.
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

    /// Default floating / pseudo-tiled size as a fraction of the destination
    /// output, used when a window has not reported its own dimensions.
    pub default_float_ratio: Option<f32>,

    /// Border color for the pending split preview in 0xAARRGGBB format.
    pub preview_border_color: Option<u32>,

    /// Border width in pixels for the pending split preview.
    pub preview_border_width: Option<i32>,
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

    /// Configured pointer bindings (Super+drag to move/resize).
    pub pointer_bindings: Vec<PointerBindingConfig>,

    /// Tiling resize ratio delta per resize command press.
    pub resize_delta_ratio: Option<f64>,

    /// Floating resize dimension delta as percentage of output size.
    pub resize_delta_percent: Option<f32>,

    /// Per-window rules applied on metadata arrival.
    pub rules: Vec<WindowRule>,

    /// Keyboard layout configuration.
    pub keyboard_layout: Option<KeyboardLayoutConfig>,

    /// Per-input-device configuration matched by exact device name.
    pub input_devices: Vec<InputDeviceConfig>,
}

/// Override an `Option` field when the source value is `Some`.
pub(crate) fn apply_if_some<T>(field: &mut Option<T>, override_: Option<T>) {
    if let Some(val) = override_ {
        *field = Some(val);
    }
}

impl Config {
    /// Precompute RGBA values from ARGB border colors for efficient rendering.
    ///
    /// Returns `(focused_rgba, unfocused_rgba)` where each RGBA tuple is
    /// `(red, green, blue, alpha)` in 0-255 range suitable for passing
    /// directly to `set_borders`.
    pub(super) fn border_rgba(&self) -> (Rgba, Rgba) {
        let focused = self.border_color_focused.unwrap_or(0xffffffff);
        let unfocused = self.border_color_unfocused.unwrap_or(0xffffffff);
        (argb_to_rgba(focused), argb_to_rgba(unfocused))
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
        use crate::config::apply_if_some;
        apply_if_some(&mut self.layout.gap, other.layout.gap);
        apply_if_some(&mut self.layout.margin_top, other.layout.margin_top);
        apply_if_some(&mut self.layout.margin_right, other.layout.margin_right);
        apply_if_some(&mut self.layout.margin_bottom, other.layout.margin_bottom);
        apply_if_some(&mut self.layout.margin_left, other.layout.margin_left);
        apply_if_some(
            &mut self.layout.default_float_ratio,
            other.layout.default_float_ratio,
        );
        apply_if_some(
            &mut self.layout.preview_border_color,
            other.layout.preview_border_color,
        );
        apply_if_some(
            &mut self.layout.preview_border_width,
            other.layout.preview_border_width,
        );
        self.decorations = other.decorations;
        apply_if_some(&mut self.border_width, other.border_width);
        apply_if_some(&mut self.border_color_focused, other.border_color_focused);
        apply_if_some(
            &mut self.border_color_unfocused,
            other.border_color_unfocused,
        );
        apply_if_some(&mut self.resize_delta_ratio, other.resize_delta_ratio);
        apply_if_some(&mut self.resize_delta_percent, other.resize_delta_percent);
        // Replaced, not identity-merged: defaults ship no rules by design.
        self.rules = other.rules;
        // Replaced: defaults ship no keyboard_layout or input_devices by design.
        if other.keyboard_layout.is_some() {
            self.keyboard_layout = other.keyboard_layout;
        }
        if !other.input_devices.is_empty() {
            self.input_devices = other.input_devices;
        }
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
        // Merge pointer bindings by identity, mirroring keybindings: a user
        // binding whose identity matches a default overrides it, otherwise it
        // is appended. Without this, user-supplied `pointer_bindings` were
        // silently dropped and the built-in defaults used instead.
        let default_pointer_indices: HashMap<PointerBindingIdentity, usize> = self
            .pointer_bindings
            .iter()
            .enumerate()
            .map(|(i, b)| (b.identity(), i))
            .collect();
        for binding in other.pointer_bindings {
            let identity = binding.identity();
            if let Some(&index) = default_pointer_indices.get(&identity) {
                self.pointer_bindings[index] = binding;
            } else {
                self.pointer_bindings.push(binding);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Config, KeyBindingConfig, KeyBindingTarget, LayoutConfig, PointerBindingConfig, PointerOp,
    };
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

    fn pointer_binding(
        target: KeyBindingTarget,
        button: u32,
        modifiers: u32,
        op: PointerOp,
    ) -> PointerBindingConfig {
        PointerBindingConfig {
            target,
            button,
            modifiers,
            op,
        }
    }

    fn layout_config() -> LayoutConfig {
        LayoutConfig {
            gap: None,
            margin_top: None,
            margin_right: None,
            margin_bottom: None,
            margin_left: None,
            default_float_ratio: None,
            preview_border_color: None,
            preview_border_width: None,
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
        make_config_full(keybindings, Vec::new())
    }

    fn make_config_with_pointer(pointer_bindings: Vec<PointerBindingConfig>) -> Config {
        make_config_full(Vec::new(), pointer_bindings)
    }

    fn make_config_full(
        keybindings: Vec<KeyBindingConfig>,
        pointer_bindings: Vec<PointerBindingConfig>,
    ) -> Config {
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
            pointer_bindings,
            rules: Vec::new(),
            keyboard_layout: None,
            input_devices: Vec::new(),
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
            pointer_bindings: Vec::new(),
            rules: Vec::new(),
            keyboard_layout: None,
            input_devices: Vec::new(),
        };

        config.merge(override_config);

        assert!(!config.decorations);
        assert_eq!(config.border_width, Some(2));
        assert_eq!(config.border_color_focused, Some(0xff0000ff));
        assert_eq!(config.border_color_unfocused, Some(0x00ff00ff));
    }

    #[test]
    fn merge_keeps_user_pointer_bindings() {
        // Before the fix, `merge` never copied `pointer_bindings`, so a user
        // config's pointer bindings were silently dropped in favour of defaults.
        let default_move =
            pointer_binding(KeyBindingTarget::Primary, 0x110, SUPER, PointerOp::Move);
        let user_resize =
            pointer_binding(KeyBindingTarget::Primary, 0x111, SUPER, PointerOp::Resize);

        let mut config = make_config(Vec::new());
        config.pointer_bindings = vec![default_move.clone()];

        config.merge(make_config_with_pointer(vec![user_resize.clone()]));

        // The user binding is preserved (appended, distinct identity), and the
        // default is retained.
        assert_eq!(config.pointer_bindings.len(), 2);
        assert_eq!(config.pointer_bindings[0].op, PointerOp::Move);
        assert_eq!(config.pointer_bindings[1].op, PointerOp::Resize);
    }

    #[test]
    fn merge_overrides_matching_pointer_binding_identity() {
        let default_move =
            pointer_binding(KeyBindingTarget::Primary, 0x110, SUPER, PointerOp::Move);
        // Same trigger (target/button/modifiers) but a different op: the user
        // binding overrides the default in place rather than appending a
        // duplicate, matching keybinding merge semantics.
        let override_resize =
            pointer_binding(KeyBindingTarget::Primary, 0x110, SUPER, PointerOp::Resize);

        let mut config = make_config(Vec::new());
        config.pointer_bindings = vec![default_move];

        config.merge(make_config_with_pointer(vec![override_resize]));

        assert_eq!(
            config.pointer_bindings.len(),
            1,
            "matching pointer-binding trigger must override, not append"
        );
        assert_eq!(
            config.pointer_bindings[0].op,
            PointerOp::Resize,
            "user op must replace default op on override"
        );
    }

    #[test]
    fn merge_appends_distinct_pointer_binding_trigger() {
        let default_move =
            pointer_binding(KeyBindingTarget::Primary, 0x110, SUPER, PointerOp::Move);
        let user_resize =
            pointer_binding(KeyBindingTarget::Primary, 0x111, SUPER, PointerOp::Resize);

        let mut config = make_config(Vec::new());
        config.pointer_bindings = vec![default_move];

        config.merge(make_config_with_pointer(vec![user_resize]));

        assert_eq!(
            config.pointer_bindings.len(),
            2,
            "distinct trigger must append"
        );
        assert_eq!(config.pointer_bindings[0].op, PointerOp::Move);
        assert_eq!(config.pointer_bindings[1].op, PointerOp::Resize);
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
