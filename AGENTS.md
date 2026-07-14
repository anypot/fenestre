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
      wm.rs                - Pure core: WMState, handle_event(Event), desired_scene, scene reconciler (apply_manage/apply_render), focus/output/window/seat lookup helpers
      events.rs            - Event domain enum: pure events consumed by handle_event (translated from River by handlers.rs)
      effects.rs           - Effect domain enum + ALL_EDGES: deferred window-level River protocol calls
      adapter.rs           - Protocol adapter: only state/ module that issues River calls; applies deferred Effect's, proxy bookkeeping for windows/outputs/seats
      handlers.rs          - Thin Wayland Dispatch translator: River events -> Event, Effect -> adapter. No state mutation.
      commands.rs          - Command execution (focus, spawn, close, fullscreen, resize, etc.)
      config.rs            - Config loading into WMState, keybinding reconciliation
      keybindings.rs       - Runtime River xkb binding CRUD
      window.rs            - Window proxy + metadata tracking (no state cache; WindowState lives in the tree)
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

### Architecture (hexagonal)

The `state` module is split into three layers so the core logic is testable
without a live compositor. See `docs/refactor-plan.md` for the full design
rationale and sequencing:

- **Core (pure):** `WMState` and the layout engine. Knows nothing about River/Wayland.
  The single entry point is `WMState::handle_event(Event)`, a pure reducer over
  the domain `Event` enum (`src/state/events.rs`). It performs no protocol I/O.
- **Adapter (IO):** `src/state/adapter.rs` is the **only** `state/` module that
  imports `protocol::` and issues River calls. It applies deferred `Effect`s and
  owns proxy bookkeeping for windows/outputs/seats.
- **Runtime:** `main.rs` owns the `calloop` loop and wires adapter ↔ core.

The domain boundary types are:

- `Event` (`src/state/events.rs`) — pure inputs translated from River protocol
  events by `handlers.rs` (e.g. `AppIdUpdated`, `TitleUpdated`, `OutputDimensionsUpdated`).
- `Effect` (`src/state/effects.rs`) — deferred window-level River protocol calls
  (`ProposeDimensions`, `Fullscreen`, `ExitFullscreen`, `UseSsd`, `EnsureNode`,
  `SetBorders`, `SetPosition`, `PlaceTop`, `Close`, `FocusWindow`). `ALL_EDGES`
  is the bitmask requesting all four border edges.

`handlers.rs` is a thin translator only: River event → `Event` (fed to
`handle_event`), and `Effect` → adapter. **No state mutation happens in
`handlers.rs`.**

#### Declarative scene reconciler

`apply_manage` / `apply_render` no longer imperatively detect transitions from a
cached mode. Instead:

- `desired_scene(&self) -> SceneSnapshot` is a **pure, read-only** function of
  current state. It snapshots every window's intended `rect`, `state`, `z`
  priority, and `border` appearance as a `SceneEntry` (`src/state/wm.rs`).
- Each protocol phase keeps its **own** snapshot and diffs the fresh
  `desired_scene()` against it, emitting only the `Effect`s that changed:
  - `last_manage_scene` — diffed/updated by `apply_manage` (dimensions,
    fullscreen enter/leave, server-side decorations).
  - `last_render_scene` — diffed/updated by `apply_render` (position, z-order, borders).
- Both phases return `Vec<Effect>`; the caller (runtime) hands that vector to the
  adapter, which applies the River protocol calls.

**Do not collapse `last_manage_scene` and `last_render_scene` into one snapshot.**
River splits window management into `ManageStart` (dimensions/fullscreen/SSD) and
`RenderStart` (position/z-order/borders); a single shared snapshot would let
`apply_manage` overwrite the baseline before `apply_render` runs, silently
dropping render-phase effects for newly mapped windows. Each phase must
re-snapshot z-priority and border for **every** window so focus-only changes
between renders are still caught.

### River Protocol Flow

1. `main.rs` connects to Wayland and gets the registry.
2. `handlers.rs` binds `river_window_manager_v1` and `river_xkb_bindings_v1` globals,
   translating each River event into a domain `Event` and calling `state.handle_event(event)`.
3. River emits `ManageStart` / `RenderStart` sequences.
4. During `ManageStart`:
   - `handle_event` has already reconciled dirty state (BSP layout, focus, close)
     from the inbound `Event`s.
   - `apply_manage` computes `desired_scene()`, diffs it against `last_manage_scene`,
     and returns `Vec<Effect>` (dimensions/fullscreen/SSD). The runtime forwards
     these to the adapter, which issues the River calls.
   - Clears `render_order_cache` for stacking order rebuild on next render.
   - If `xkb_bindings_dirty`: destroys stale bindings via `destroy_pending_keybindings`, then
     creates/enables desired bindings via `configure_keybindings`.
   - Window rules are **not** re-applied here. They are evaluated event-driven on
     each window `AppId`/`Title` arrival (`WMState::evaluate_window_rules`, triggered
     by `Event::AppIdUpdated` / `Event::TitleUpdated`) and
     re-run for an output's windows when that output's geometry becomes known.
5. During `RenderStart`:
   - `apply_render` computes `desired_scene()`, diffs it against `last_render_scene`,
     and returns `Vec<Effect>` (position/z-order/borders); the adapter applies them.
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
- Window states are an **enum-with-data** living on the tree node (`src/layout/tree.rs`)
  so illegal states are unrepresentable:
  - `Tiled`
  - `Floating { rect: Rect }`
  - `PseudoTiled { rect: Rect }`
  - `Fullscreen { restore: Box<WindowState> }`
  The tree node owns this **single** typed state; `Window` no longer caches state
  (that role moved to the reconciler's scene snapshots). `Floating`/`PseudoTiled`
  carry their own rect, so a tiled window *cannot* hold a floating rect, and
  `Fullscreen` carries the pre-fullscreen state in `restore` (replacing the old
  `base_state` field). Toggle transitions (`toggle_fullscreen` / `toggle_floating`
  / `toggle_pseudo_tiled`) are total, derived from the variant via `mem::replace`.
- Non-tiling windows (floating/fullscreen) receive zero-area rects so they do not consume split space.
- `arranged_windows` / `arranged_windows_readonly` return `(window_id, rect, WindowState)`
  for the reconciler (`apply_manage` / `apply_render` read from the latter).
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
  output is removed) live in an orphan tree. `Event::OutputCreated` drains orphan
  trees into the first real output via `reassign_output`, so no window is lost.

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
  evaluation is triggered by `Event::AppIdUpdated` / `Event::TitleUpdated` via
  `WMState::evaluate_window_rules` (handlers.rs translates the River events into
  those `Event`s), and re-run for an output's windows when its geometry becomes
  known (to catch up windows deferred for a missing output rect).

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
- **WindowState is the single window-state vocabulary.** The BSP layout engine
  uses an enum-with-data (`Tiled` / `Floating { rect }` / `PseudoTiled { rect }` /
  `Fullscreen { restore }`, see `layout/tree.rs`). `Window` no longer caches state
  — the authoritative state lives on the layout tree node and is mirrored into the
  reconciler's scene snapshots (`last_manage_scene` / `last_render_scene`).

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

1. **Hexagonal boundary**: `state/wm.rs` + `layout/` are the pure core — no
   `protocol::` imports, no River calls (compiler-enforced). `state/adapter.rs` is
   the **only** `state/` module that issues River protocol calls; it applies
   `Effect`s returned by the core. `state/handlers.rs` is a thin translator
   (River event → `Event`, `Effect` → adapter) and performs **no** state mutation.
2. **Internal commands only**: `Command` is `pub(crate)` and is not an IPC surface.
3. **Manage/render sequence discipline**: Window geometry changes must happen inside
   `apply_manage`; node positioning must happen inside `apply_render`.
4. **Pending queues**: Window closes, xkb binding destroys, and focus changes use
   pending queues applied during the appropriate sequence to avoid mid-event mutation.
5. **Config reconciliation**: When seats/outputs appear late, `reconcile_keybindings`
   is called so bindings are created for the current seat set.
6. **Scene snapshot discipline**: `apply_manage` and `apply_render` each keep their
   **own** snapshot (`last_manage_scene` / `last_render_scene`) and diff the fresh
   `desired_scene()` against it. Never collapse the two snapshots into one, and each
   phase must re-snapshot z-priority and border for **every** window so focus-only
   changes between renders are still caught.

## Work in Progress / Gotchas

- Some command implementations are placeholders: rotate and cycle are not wired.
- Pointer-driven move/resize is not implemented.
- IPC is not implemented.
- `wayland-client` event_created_child is used for River child objects;
  manual child data hashing is commented out in `handlers.rs`.
- `render_order_cache` must be cleared on any structural change to windows or focus.
- `ensure_focused_output` falls back to the first output if none is focused yet.

## Planned work

Two larger improvements are scoped but not yet started. See `docs/refactor-plan.md`
for the full design, sequencing, and risk notes.

### Layout as a pluggable trait

BSP tiling is hard-wired into `layout/tree.rs`. Adding alternative layouts
(master-stack, grid, tabbed, monocle) currently means surgery in core.

- Define a `Layout` trait against the minimal `LayoutTree` API core actually
  needs (arrange windows into rects; insert / remove / focus-navigation hooks).
- Implement it for the existing BSP tree first (no behaviour change).
- Route core through the trait (one `Box<dyn Layout>` / enum per output).
  Floating and fullscreen handling stay outside the tiling layout.

### Unify the config schema

`config/lua.rs` and `config/toml.rs` are parallel loaders feeding a shared
`parser`. Adding one field (as with `default_float_ratio`) means edits in three
files, and the two paths can drift.

- Define the config shape once as serde structs with validation.
- Have TOML deserialize directly into those structs.
- Bridge the Lua table → serde (e.g. an `mlua` value → `serde` deserializer) so
  both formats share one validation / build path.
- Collapse `parser::build_layout` and the per-format duplication; adding a field
  becomes a one-line struct change.

## Docker / Environment Notes

No Dockerfile or CI workflow is present. The project targets a Linux desktop environment
with a River compositor that supports:

- `river_window_management_v1` (version 5)
- `river_xkb_bindings_v1` (version 3)

Building requires Rust 1.88+ (edition 2024) and Wayland development libraries.
