# Fenestre

Fenestre is an experimental Wayland window manager for [River](https://codeberg.org/river/river). It uses a binary space partitioning (BSP) tree as its core tiling model and supports Lua and TOML configuration.

> Status: early-stage. Core tiling, keybindings, and config loading work; some commands are still placeholders. See [Status](#status).

## Install / Build

Requires Rust 1.88+ (edition 2024) and the Wayland, xkbcommon, and Lua 5.4 development libraries. On Debian/Ubuntu:

```sh
sudo apt install libwayland-dev libxkbcommon-dev liblua5.4-dev
```

Build from source:

```sh
cargo build --release
```

Or install directly from the git repository (still requires the system libraries above). Pin to a release tag to avoid pulling unreleased commits:

```sh
cargo install --git https://github.com/anypot/fenestre --tag v0.1.0
```

Run (with debug logging):

```sh
RUST_LOG=fenestre=debug cargo run
```

Run with a config file:

```sh
RUST_LOG=fenestre=debug cargo run -- examples/fenestre.toml
```

Reference configs live in `examples/`:

- `examples/fenestre.toml` — full TOML example
- `examples/fenestre.lua` — full Lua example (scripting/DRY support)
- `examples/advanced.lua` — small Lua example using variables, helpers, and loops
- `examples/minimal.toml` — smallest viable config to copy and edit

When no path is given, Fenestre searches `$XDG_CONFIG_HOME/fenestre/` and `~/.config/fenestre/` for `fenestre.toml` (preferred) or `fenestre.lua`.

## Runtime requirements

Fenestre runs as a River window manager client. It requires a River that supports:

- `river_window_management_v1`
- `river_xkb_bindings_v1`

## Configuration

Fenestre supports two config formats. **TOML** is preferred and takes precedence: if `fenestre.toml` exists it is used and any coexisting `fenestre.lua` is ignored. If the TOML file fails to parse, Fenestre logs a warning and falls back to built-in defaults (it does **not** fall back to the Lua file). **Lua** adds variables, conditionals, and DRY support.

Canonical, validated copies of both formats are in `examples/` (see [Install / Build](#install--build)).

> Note: in TOML, a `[layout]` table stays "open" until the next `[section]` header, so top-level keys (`decorations`, `border_*`, `resize_*`) must appear **before** `[layout]` (or under their own `[section]`), or they are silently absorbed into `layout` and ignored.

### Keybindings

A keybinding is identified by `target + keysym + modifiers`. User keybindings with the same identity override defaults; new ones are appended.

Supported targets:

- `primary`
- `all`

Supported modifiers (matched case-insensitively, e.g. `Super` ≡ `super`):

- `shift`
- `ctrl` / `control`
- `alt` / `mod1`
- `mod3`
- `super` / `mod4`
- `mod5`

Keysyms are parsed with `xkbcommon`, so names such as `Return`, `q`, `h`, `j`, `k`, `l`, `Tab`, and `Escape` can be used.

### Window rules

Rules match a window by `app_id` and/or `title` and set its mode (like River's `riverctl rule-add`). Declared as a `rules` table (Lua) or `[[rules]]` array (TOML):

```toml
[[rules]]
app_id = "mpv"
mode = "fullscreen"

[[rules]]
app_id = { value = "libreoffice-", match = "prefix" }
mode = "floating"
```

Each rule has:

- `app_id` / `title` — matchers. A plain string is an **exact** match; a table `{ value = "...", match = "exact" | "prefix" | "regex" }` selects the mode (`exact` default). `prefix` is a safe `*`-style wildcard; `regex` is compiled with a size limit (prefer `prefix`). At least one matcher is required.
- `mode` — required; one of `tiled`, `floating`, `pseudo_tiled`, `fullscreen`.
- `floating_rect` — optional `{ x, y, width, height }`; `x`/`y` default to `0`.

All matching rules apply, later wins. Rules are evaluated once per window as its `app_id`/`title` arrive; reloading config does not re-apply them to existing windows. Fenestre ships no default rules.

## Default keybindings

- `Super+Return`: spawn `foot`
- `Super+q`: close focused window
- `Super+h`: focus left
- `Super+j`: focus down
- `Super+k`: focus up
- `Super+l`: focus right
- `Super+Tab`: focus next
- `Shift+Super+Tab`: focus previous
- `Super+s`: toggle floating
- `Super+t`: set tiled
- `Super+Shift+t`: toggle pseudo-tiled
- `Super+f`: toggle fullscreen
- `Shift+Super+r`: reload config
- `Shift+Super+e`: exit River
- `Shift+Super+q`: close focused window
- `Shift+Super+h`: move window left
- `Shift+Super+j`: move window down
- `Shift+Super+k`: move window up
- `Shift+Super+l`: move window right
- `Super+Alt+h`: resize expand left
- `Super+Alt+j`: resize expand down
- `Super+Alt+k`: resize expand up
- `Super+Alt+l`: resize expand right
- `Shift+Super+Alt+h`: resize shrink left
- `Shift+Super+Alt+j`: resize shrink down
- `Shift+Super+Alt+k`: resize shrink up
- `Shift+Super+Alt+l`: resize shrink right

## Status

Working: River integration, Lua + TOML config loading (TOML preferred), per-output BSP layout with hotplug reassignment, compositor-side borders, window rules, and keyboard-driven focus/move/resize.

Not yet implemented: pointer-driven move/resize, IPC, robust config validation, and some commands (rotate, cycle).
