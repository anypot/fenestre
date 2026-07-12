# Fenestre AGENTS.md

## Project Overview

Fenestre is an experimental Wayland window manager for the River compositor.  
It uses a binary space partitioning (BSP) tree as the core tiling layout model.

- Language: Rust (edition 2024)
- Build system: Cargo
- Wayland stack: `wayland-client`, `wayland-scanner`, `calloop`
- Config: Lua via `mlua`; TOML via `toml` crate
- Input: `xkbcommon` for keysym parsing
- Logging: `env_logger` / `log`

## Quick Start

```sh
cargo build
```

Run with debug logging:

```sh
RUST_LOG=fenestre=debug cargo run
```

Run with a config file:

```sh
RUST_LOG=fenestre=debug cargo run -- examples/fenestre.lua
```

Run tests:

```sh
cargo test
```

No `rustfmt.toml` or `clippy.toml` are present yet; rely on Cargo defaults.

## Repository Structure

```
fenestre/
  Cargo.toml               - Package manifest (single crate, edition 2024)
  build.rs                 - wayland-scanner build hook (rerun-if-changed on XML protocols)
  protocol/
    river-window-management-v1.xml
    river-xkb-bindings-v1.xml
  src/
    main.rs                - Bootstrap: env_logger, WMState, Wayland connection, calloop loop
    command/
      mod.rs               - Module declaration, re-exports
      internal.rs          - Internal Command enum (NOT a public IPC API)
    config/
      mod.rs               - Config, KeyBindingConfig, KeyBindingTarget, ConfigError, merge logic
      defaults.rs          - Built-in default keybindings
      lua.rs               - Lua config loader
      toml.rs              - TOML config loader
      parser.rs            - Keysym / modifier / target / command parsing
    layout/
      mod.rs               - Module declaration, re-exports
      tree.rs              - BSP tree (LayoutTree, LayoutNode, Rect, capped_rect, split/focus/arrange, resize logic)
    protocol/
      mod.rs               - Module declaration, re-exports
      river.rs             - wayland-scanner generated River protocol bindings
    state/
      mod.rs               - Module declaration, re-exports (WMState, OutputId)
      wm.rs                - WMState definition, focus/output/window/seat lookup helpers
      handlers.rs          - Wayland Dispatch impls for all River + wl protocols
      commands.rs          - Command execution (focus, spawn, close, fullscreen, resize, etc.)
      config.rs            - Config loading into WMState, keybinding reconciliation
      keybindings.rs       - Runtime River xkb binding CRUD
      window.rs            - Window proxy + metadata tracking
      output.rs            - Output proxy + metadata tracking
      seat.rs              - Seat proxy + metadata tracking
      rule.rs              - Window rule matching/evaluation (exact/prefix/regex matchers, later-wins apply)
  examples/
    fenestre.toml          - Full TOML config example (canonical, validated by tests)
    fenestre.lua           - Full Lua config example (validated by tests)
    advanced.lua           - Small Lua example: variables, helpers, loops (validated by tests)
    minimal.toml           - Minimal copy-and-edit starter (validated by tests)
  README.md                - User-facing docs: install/build, configuration, default keybindings, status
```

## Core Concepts

### WMState

- Owns **all** mutable compositor-facing state: River proxies, windows, outputs, seats,
  keybindings, focus, config, layout, and pending request queues.
- Defined in `src/state/wm.rs`.
- Public crate surface is intentionally tiny: re-exported from `src/state/mod.rs`.
- Most fields are `pub(super)` to keep the `state` module boundary strict.
- Maintains three `HashMap` proxy indexes (`windows_by_proxy`, `outputs_by_proxy`, `seats_by_proxy`) for O(1) lookup of Wayland objects, plus a per-output window grouping index `windows_by_output` (`HashMap<OutputId, HashSet<WindowId>>`) for O(1) lookup of which windows belong to an output.

### River Protocol Flow

1. `main.rs` connects to Wayland and gets the registry.
2. `handlers.rs` binds `river_window_manager_v1` and `river_xkb_bindings_v1` globals.
3. River emits `ManageStart` / `RenderStart` sequences.
4. During `ManageStart`:
   - `apply_manage` reconciles dirty state (BSP layout, focus, close).
   - Clears `render_order_cache` for stacking order rebuild on next render.
   - If `xkb_bindings_dirty`: destroys stale bindings via `destroy_pending_keybindings`, then
     creates/enables desired bindings via `configure_keybindings`.
   - Window rules are **not** re-applied here. They are evaluated event-driven on
     each window `AppId`/`Title` arrival (`WMState::evaluate_window_rules`) and
     re-run for an output's windows when that output's geometry becomes known.
5. During `RenderStart`:
   - `apply_render` positions `RiverNodeV1` objects and updates stacking order.
   - `reconcile_keybindings` is called when seats appear/disappear to update the binding set.

### BSP Layout Engine (`layout/tree.rs`)

- `LayoutTree` is a binary tree of `LayoutNode`s.
- Nodes are either splits or leaves (windows).
- Fenestre keeps one `LayoutTree` per output in `WMState::output_trees`
  (`HashMap<OutputId, LayoutTree>`); `tree_for_output` / `ensure_tree_for_output`
  resolve the tree for an output, and `focused_output` selects which tree
  focus/move/resize commands target.
- `insert_window` splits the currently focused window along its longest side.
- `remove_window` collapses empty splits; distinguishes `LeafRemoved`, `Replaced`, `Modified`, `NotFound`.
- Window states: `Tiled`, `Floating`, `PseudoTiled`, `Fullscreen`.
- Non-tiling windows (floating/fullscreen) receive zero-area rects so they do not consume split space.
- `arranged_windows` returns `(window_id, rect, WindowState)` for both `apply_manage` and `apply_render`.
- Resize navigation: `focus_to_resize_target` finds ancestor splits supporting the requested resize direction when the immediate split doesn't support it.

### Focus Model

- Focus is coordinated between `WMState` and `LayoutTree`.
- `state.focused_window` holds the semantic focus; `layout.focused` holds the tree focus.
- `push_focus` updates `focus_stack` and `focused_window` only (low-level helper).
- `focus_window_id` updates all three plus `pending_focus` and clears `render_order_cache`.
  Callers should use `focus_window_id` rather than mutating `state.focused_window` directly.
- On window close, `WMState::close_window_focus_reconcile` routes focus only when
  the closed window was the globally focused one. In that case post-close focus goes
  to the layout tree's preferred next window on that output, or — if that output just
  emptied — falls back to the global `focus_stack`. Closing a non-focused window (on
  any output) leaves global focus unchanged, so a background window closing never
  steals focus to another output. The tree (and, as a fallback, the global focus
  stack) is the source of truth for where focus goes after a close.
- `ensure_focused_output` self-heals a stale `focused_output` (e.g. an output
  that was removed) by falling back to the first remaining output before focus
  commands resolve their target tree.

### Output Hotplug & Reassignment

- Each output owns a `LayoutTree`; the global `focused_window` is the semantic
  focus while each output tree tracks its own `focused` window.
- On output removal or hotplug, `reassign_output(from, to)` moves every window
   from `from`'s tree into `to`'s tree, preserving each window's mode
   (tiled / floating / pseudo-tiled / fullscreen) and focus, and rebuilding
   split directions from the destination output's real geometry, so windows
   survive output changes without being destroyed.
- Windows created before any real output exists (or left behind when the last
  output is removed) live in an orphan tree. `Event::Output` drains orphan trees
  into the first real output via `reassign_output`, so no window is lost.

### Keybinding Model

- Config identity: `target + keysym + modifiers`.
- User config is merged over defaults by identity.
- `KeyBindingConfig` (declarative) vs `KeyBinding` (runtime, has `RiverXkbBindingV1` proxy).
- Keybindings are created as River protocol objects during `ManageStart`.
- On `XkbBindingEvent::Pressed`, the command is cloned and dispatched via `run_command`.

### Window Rules

- Config key: `rules` — a list of rule tables; defaults ship none.
- A rule matches a window when its `app_id` AND `title` criteria both match (a
  field you omit is a wildcard). At least one matcher must be present.
- Matcher forms for `app_id` / `title`:
  - a plain string → **exact** match (`==`);
  - a table `{ value = "...", match = "exact" | "prefix" | "regex" }`
    (defaults to `exact`):
    - `exact` → `==`
    - `prefix` → `starts_with` (safe `*`-style wildcard; prefer this over regex)
    - `regex` → compiled with a `size_limit` guard (match-time backtracking is
      still possible, so avoid complex patterns)
- Action fields: `mode` (`tiled` / `floating` / `pseudo_tiled` / `fullscreen`,
  required) and optional `floating_rect = { x, y, width, height }`
  (missing `x`/`y` default to `0`; use `mode = "floating"` to float at that size).
- Semantics mirror former River's `rule-add`: **all matching rules apply, later wins**.
  The list is evaluated on each `AppId`/`Title` event; evaluation re-runs until
  every metadata field any rule references is known (so a general rule can apply
  immediately and a more specific, later-listed rule can override once its field
  arrives), after which the window is finalized and never re-evaluated.
- Rules are applied once per window. Reloading config does **not** re-apply rules
  to already-on-screen windows (by design).
- Implementation: `state/rule.rs` (`WindowRule`, `RulePattern`, `WindowRules`);
  evaluation is triggered from `state/handlers.rs` `AppId`/`Title` events via
  `WMState::evaluate_window_rules`, and re-run for an output's windows when its
  geometry becomes known (to catch up windows deferred for a missing output rect).

### Configuration

- Both Lua (`.lua`) and TOML (`.toml`) config formats are supported. TOML is
  preferred: when both `fenestre.toml` and `fenestre.lua` exist, only the TOML
  file is loaded and the Lua file is ignored.
- Canonical, test-validated example configs live in `examples/`
  (`fenestre.toml`, `fenestre.lua`, `advanced.lua`, `minimal.toml`).
- Default search paths (TOML wins at the file level):
  1. `$XDG_CONFIG_HOME/fenestre/fenestre.toml`
  2. `~/.config/fenestre/fenestre.toml`
  3. `$XDG_CONFIG_HOME/fenestre/fenestre.lua`
  4. `~/.config/fenestre/fenestre.lua`
- An explicit CLI path with no extension falls back to the Lua loader (backward
  compatibility); only the default-search precedence prefers TOML.
- `Config::merge` (pub(crate)) overrides matching identities and appends new bindings.
- `Config::border_rgba()` precomputes RGBA color components from ARGB border colors for efficient border rendering.
- Layout and border fields use `Option` types: `None` means unset (inherit default), `Some(v)` means explicit.
  - `LayoutConfig`: `gap`, `margin_top/right/bottom/left`
  - `Config`: `border_width`, `border_color_focused`, `border_color_unfocused`, `resize_delta_ratio`, `resize_delta_percent`
- Border colors update independently of `border_width` during merge.
- Per-window `decoration_hint` (from River protocol) overrides global `decorations`:
  - `0` = server-side decorations (compositor borders)
  - `1` = client-side decorations
  - `2` or absent = fall back to global `decorations` config
- Modifier constants: Shift=1, Super=64 (matches `xkbcommon` modifier bits used by River).
- Modifier names are matched case-insensitively (`Super` ≡ `super`).
- **WindowMode vs WindowState**: `WindowMode` is stored on `Window` and represents persistent window state; `WindowState` is used internally by the layout engine for BSP arrangement decisions.

## Coding Conventions

- **Formatting is mandatory and must be run before finishing any change.** Always
  run `cargo fmt` (or `cargo fmt --check` to verify) on every Rust file you
  create or edit — including newly added files. Do not hand-format and assume
  it matches `rustfmt`; the saved code must already be `cargo fmt` clean.
  The repo relies on Cargo's default `rustfmt` (no `rustfmt.toml` present).
- Visibility: Prefer the most restrictive visibility that works.
  - `pub(crate)` for crate-internal types.
  - `pub(super)` for state-internal fields accessed across `state/` submodules.
- Error handling: Use `Result<T, E>` with `Box<dyn std::error::Error>` at the top level
  (`main`). Configuration uses the dedicated `ConfigError` enum.
- Logging targets: Prefix with `fenestre::` (e.g., `fenestre::state::handlers`).
- `unsafe` is used only in `layout/tree.rs` (`swap_windows`), where raw pointers are
  employed to swap two disjoint leaf nodes that the borrow checker otherwise cannot
  prove non-aliasing; the invariants making it sound are documented inline in that function.
- `#[allow(dead_code)]` is used at module level where appropriate.
- No `println!` / `eprintln!` in library code; use `log` macros.

## Important Invariants

1. **State module boundary**: River protocol event handlers live in `state/handlers.rs`.
   Layout policy lives in `layout/`. Command dispatch lives in `state/commands.rs`.
2. **Internal commands only**: `Command` is `pub(crate)` and is not an IPC surface.
3. **Manage/render sequence discipline**: Window geometry changes must happen inside
   `apply_manage`; node positioning must happen inside `apply_render`.
4. **Pending queues**: Window closes, xkb binding destroys, and focus changes use
   pending queues applied during the appropriate sequence to avoid mid-event mutation.
5. **Config reconciliation**: When seats/outputs appear late, `reconcile_keybindings`
   is called so bindings are created for the current seat set.

## Work in Progress / Gotchas

- Some command implementations are placeholders: rotate and cycle are not wired.
- Pointer-driven move/resize is not implemented.
- IPC is not implemented.
- `wayland-client` event_created_child is used for River child objects;
  manual child data hashing is commented out in `handlers.rs`.
- `render_order_cache` must be cleared on any structural change to windows or focus.
- `ensure_focused_output` falls back to the first output if none is focused yet.

## Docker / Environment Notes

No Dockerfile or CI workflow is present. The project targets a Linux desktop environment
with a River compositor that supports:

- `river_window_management_v1` (version 5)
- `river_xkb_bindings_v1` (version 3)

Building requires Rust 1.84+ (edition 2024) and Wayland development libraries.
