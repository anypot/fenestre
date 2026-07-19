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

This repo is indexed by CodeGraph (a `.codegraph/` directory exists at the root).
Before grepping, finding, or reading files to locate or understand code, prefer
CodeGraph: the `codegraph_explore` MCP tool (or `codegraph explore "<query>"` in
the shell) returns the relevant symbols' source plus call paths in one call. If
`.codegraph/` is absent, skip it and use normal grep/Read.

```
fenestre/
  Cargo.toml               - Package manifest (single crate, edition 2024)
  build.rs                 - wayland-scanner build hook (rerun-if-changed on XML protocols)
  protocol/
    river-window-management-v1.xml
    river-xkb-bindings-v1.xml
  src/
    main.rs                - Bootstrap: env_logger, WMState, Wayland connection, calloop loop
    command/               - Internal Command enum (NOT a public IPC API)
    config/                - Config, KeyBindingConfig, KeyBindingTarget, ConfigError, loaders (lua/toml), schema, parser, rule_types (RulePattern/WindowRule data types)
    layout/                - BSP tree (LayoutTree, LayoutNode, Rect, split/focus/arrange, resize logic)
    protocol/              - wayland-scanner generated River protocol bindings
    state/                 - WMState, Event/Effect domains, adapter, handlers, commands, config, keybindings, window/output/seat proxies, rule, reassign, focus (focus stack / close reconciliation)
  examples/
    fenestre.toml          - Full TOML config example (canonical, validated by tests)
    fenestre.lua           - Full Lua config example (validated by tests)
    advanced.lua           - Small Lua example: variables, helpers, loops (validated by tests)
    minimal.toml           - Minimal copy-and-edit starter (validated by tests)
  README.md                - User-facing docs: install/build, configuration, default keybindings, status
```

For the conceptual model, protocol flow, layout engine, focus model, and the
invariants the implementation must respect, see [`docs/architecture.md`](docs/architecture.md).
Architecture decision records live in [`docs/adr/`](docs/adr/README.md).

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

## Documentation Sync

After any change that affects behaviour, public API, layout, focus, config, or
keybindings, update the docs to match before finishing:

- **Rust docstrings** (`src/**`): keep module/struct/function docs accurate; the
  pure core (`state/wm.rs`, `layout/`) must stay protocol-free and documented as such.
- **`README.md`**: user-facing install/build, configuration, and default keybindings.
- **`AGENTS.md`**: this file — agent instructions, conventions, and pointers only
  (no code encyclopedia; that lives in `docs/architecture.md`).
- **`docs/`**: `docs/architecture.md` for the conceptual model/invariants and
  `docs/adr/` for accepted architecture decisions (index in `docs/adr/README.md`); `docs/refactor-plan.md` is the
  roadmap and must stay untouched unless the user asks.

Run `cargo fmt` after editing any Rust file. `AGENTS.md` should stay short and
scannable — move detailed explanations into `docs/`, not into this file.

## Work in Progress / Gotchas

- A layout `rotate` command does not exist (no `Command::Rotate` variant). Focus
  cycling (`FocusNext` / `FocusPrevious`) is wired.
- Pointer-driven move/resize is implemented (see `InteractiveOp` on `Seat` and the `StartPointerOp` / `EndPointerOp` / `SetCursor` effects); cumulative `op_delta` events drive a window's floating rect, clamped to its `DimensionsHint`.
- IPC is not implemented.
- `wayland-client` event_created_child is used for River child objects;
  manual child data hashing is commented out in `handlers.rs`.
- `render_order_cache` must be cleared on any structural change to windows or focus.
- River's drawBorders uses a zero-size workaround for disabled edges — Zig 0.16.0
  ReleaseSafe elides wlr_scene_node_setEnabled(false) extern calls when the struct
  field binding appears dead after store.
- `ensure_focused_output` falls back to the first output if none is focused yet.

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

## Planned work

Scope and sequencing for further improvements live in [`docs/refactor-plan.md`](docs/refactor-plan.md).
Note that several of its initiatives (pure core + adapter split, declarative
scene reconciler, unified TOML/Lua config schema, typed `WindowState`) are
**already merged** — treat the plan as historical context, not a pending backlog.

## Docker / Environment Notes

CI runs in `.github/workflows/ci.yml` (fmt + clippy `-D warnings` + tests on
push/PR). No Dockerfile is present. The project targets a Linux desktop environment
with a River compositor that supports:

- `river_window_management_v1` (version 5)
- `river_xkb_bindings_v1` (version 3)
- `river_layer_shell_v1` (layer-shell exclusive zones and keyboard focus)

Building requires Rust 1.88+ (edition 2024) and Wayland development libraries.
