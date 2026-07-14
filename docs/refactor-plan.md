# Fenêtre — Big Refactor Plan

Design doc / migration outline for the larger architectural work. This is a
**planning document**, not a commitment — each initiative is independent and can
be scheduled, deferred, or dropped on its own.

## Goals

The north star is **understandable, readable, maintainable code**:

- One obvious source of truth for every piece of state.
- Core logic that can be tested without a live compositor.
- Illegal states that are hard or impossible to represent.
- New features (layouts, config fields) that touch as few places as possible.

## Guiding principles

1. **Test-first.** The trickiest behaviour (toggle / reassign / fullscreen
   round-trips, multi-output) can only be fully validated by running the
   compositor. Add characterization tests that pin current behaviour *before*
   each structural change so refactors stay behaviour-preserving.
2. **Small, reversible commits.** Each initiative lands as a series of pure,
   green-at-every-step commits. Prefer "extract, then rewire, then delete".
3. **No behaviour change inside a refactor commit.** Behaviour changes get their
   own commits with their own tests.

## Phase 0 — Characterization tests (prerequisite)

Audit current coverage, then grow it for the remaining under-tested paths.
Note: several of these paths **already have characterization tests** in
`src/state/wm.rs` — `toggle_from_fullscreen_to_float_keeps_float_size`,
`toggle_from_fullscreen_to_pseudo_keeps_pseudo_size`,
`pseudo_tiled_after_fullscreen_keeps_pseudo_size`, and the `reassign_output_*`
family. Re-run the coverage audit before committing to the estimate below; the
risky surface is already better covered than the original call-graph pass
suggested, so re-baseline toward the low end.

Paths still worth pinning down:

- `apply_manage` — state → protocol calls, including fullscreen enter/leave, for
  windows that are *not* freshly toggled (the toggle round-trips are covered;
  the bare manage-cycle transitions are the gap).
- `reassign_output` / `reassign_with_rebuild` — edge cases beyond the existing
  happy paths (mixed floating/pseudo/fullscreen sets, split topology on
  dimension-less destinations).
- Focus reconciliation after removal / reassignment in multi-output setups.

Estimated effort: **0.5–1 day**, but re-baseline after the audit — likely
toward the low end given existing coverage. Everything downstream depends on it.

### Phase 0 — Coverage audit (baseline)

Measured with `cargo llvm-cov` against the full test suite (172 tests, all
passing) on 2026-07-14. Per-file line coverage of the risky paths:

| File | Coverage |
|------|----------|
| `src/state/output.rs` | 100.0% |
| `src/state/rule.rs` | 99.1% |
| `src/layout/tree.rs` | 95.5% |
| `src/state/window.rs` | 92.6% |
| `src/state/wm.rs` | 73.4% |
| `src/state/commands.rs` | 18.0% |
| `src/state/handlers.rs` | 0.0% |
| **all instrumented files** | **78.8%** |

Function-level coverage of the named risky functions (before this phase's
characterization tests):

| Function | Coverage | Notes |
|----------|----------|-------|
| `apply_manage` | 0/94 | **Blocked**: gated on live `river_output` / `river_window` proxies (`wm.rs:544`, `wm.rs:558`). Cannot be exercised without a running compositor or the Initiative 1 `Effect` collector. |
| `apply_render` | 0/97 | Same proxy gate. |
| `reassign_output` | 96/102 | Happy paths covered. |
| `reassign_with_rebuild` | 49/99 | **Gap**: mixed-mode `ReassignPlan` branches (Floating / PseudoTiled / FullscreenPseudo / FullscreenFloating) untested. |
| `reassign_clone_topology` | 17/20 | Floating-rect translation untested. |
| `close_window_focus_reconcile` | 41/45 | **Gap**: empty-output fallback branches untested. |
| `remove_output_by_proxy` / `remove_seat_by_proxy` | 0 | No tests (minor; low-risk bookkeeping). |

`handlers.rs` at 0% is expected — it requires a live Wayland connection and is
out of scope for unit characterization (Initiative 1's adapter split is the
enabler there too).

#### New characterization tests added (this phase)

Six tests pinned the under-covered behaviour, all passing (suite 166 → 172):

- `reassign_with_rebuild_preserves_mixed_mode_set` — a tiled + floating +
  pseudo-tiled + fullscreen(source) window set reassigned into a real-geometry
  destination keeps every mode and floating rect. Drives the `Tiled` / `Floating`
  / `PseudoTiled` / `FullscreenTiled` plan branches through the **rebuild** path.
- `reassign_with_rebuild_preserves_fullscreen_floating_base` — fullscreen-over-
  floating reassigned via rebuild keeps `Floating` base state and exact float
  rect on un-fullscreen. Covers `FullscreenFloating`.
- `reassign_with_rebuild_preserves_fullscreen_pseudo_base` — same for
  fullscreen-over-pseudo via rebuild (the existing `…preserves_fullscreen_base_state`
  only exercised the dimension-less clone path). Covers `FullscreenPseudo`.
- `reassign_into_dimensionless_output_translates_floating_rects` — a floating
  window reassigned into a dimension-less recreated output has its rect shifted
  by the output position delta. Covers the floating-translation branch of
  `reassign_clone_topology`.
- `closing_focused_window_when_output_empties_falls_back_to_other_output` —
  closing the globally focused window when its output empties falls back to
  another output's window via the focus stack (multi-output reconciliation).
- `closing_last_remaining_window_clears_focus` — closing the last window clears
  `focused_window` / `pending_focus` cleanly. Covers the `next == None` branch.

After these tests: `reassign_with_rebuild` 49→**97/99**, `reassign_clone_topology`
17→**20/20**, `close_window_focus_reconcile` 41→**42/45**. The 3 remaining
uncovered regions in `close_window_focus_reconcile` are defensive branches (a
window removed from the map, or whose output tree is already gone) — reachable
only via contrived setups and not part of the named risk surface.

#### Residual gap (deferred to Initiative 1)

`apply_manage` / `apply_render` remain 0% unit coverage because both are gated
on live River proxies. This is exactly the surface Initiative 1's `Effect`
collector is meant to unlock: once the per-window protocol decisions are
extracted into a pure function, the fullscreen enter/leave transition logic
(using the cached `window.mode`) becomes directly testable without a compositor.
The toggle round-trip tests already cover the layout-tree side of those
transitions; only the `window.mode = state` cache assignment and the protocol
emission in `apply_manage` (wm.rs:562–586) remain an integration-only path.

---

## Initiative 1 — Pure core + IO adapter (hexagonal) ★ top priority

### Problem
`handlers.rs` interleaves Wayland/River protocol dispatch with state mutation.
Testing means dealing with real protocol proxies, so the interesting logic is
hard to exercise in isolation.

### Target design
Split into three layers:

- **Core (pure):** `fn reduce(state, Event) -> Vec<Effect>` (or
  `(State, Vec<Effect>)`). Knows nothing about River/Wayland. `Event` is a
  domain enum (`WindowMapped`, `OutputResized`, `ToggleFloating`, …). `Effect`
  is a domain enum (`ProposeDimensions`, `Fullscreen`, `SetBorders`, …).
- **Adapter (IO):** translates River events → core `Event`, and core `Effect` →
  River protocol calls. This is the only layer that touches `protocol::`.
- **Runtime:** owns the event loop and wires adapter ↔ core.

### Migration outline
1. Introduce `Effect` and make the current imperative protocol calls go through
   an `effects: Vec<Effect>` collected during a manage cycle, then applied at the
   end. (No behaviour change; just centralizes IO.) This imperative collector is
   a **stepping stone** — Initiative #2 replaces it with a pure `desired_scene`
   diff, so do not over-invest in it. Also confirm River tolerates
   **deferred/batched** protocol calls (e.g. `propose_dimensions`, `fullscreen`,
   `exit_fullscreen`): today they are applied inline, and some transitions may
   rely on immediate application before the next River event.
2. Introduce a domain `Event` enum; have `handlers.rs` translate into it and call
   a single `state.handle(event)`.
3. Move state mutation out of `handlers.rs` into the core; leave `handlers.rs` as
   a thin translator.
4. Make the core module free of `protocol::` imports (compiler-enforced).

### Payoff
Fully unit-testable core; the biggest enabler for #2 and #3; removes the current
"can only test by running the compositor" risk.

### Risk / effort
Medium-high risk, but incremental. **~1–2 weeks.**

Risk not to underestimate: River **child objects**. `handlers.rs` creates child
protocol objects (`RiverNodeV1` from `RiverWindowV1`) via `event_created_child`,
and child-data hashing is a known gotcha (see AGENTS.md). Inbound event
translation is straightforward, but outbound **child-object creation** is
triggered by River events and the adapter must own that protocol-object
lifecycle. Design the adapter's ownership model for child objects explicitly
before splitting `handlers.rs`, not after.

---

## Initiative 2 — Declarative scene reconciler

### Problem
`apply_manage` imperatively issues `propose_dimensions` / `exit_fullscreen` /
`fullscreen` / `use_ssd` and relies on the cached `Window.mode` to detect
transitions. This is the class of "did we forget to reset X on this transition?"
bugs, and it duplicates state (see #3).

### Target design
Compute a **desired scene** as a pure function of state:

```
fn desired_scene(state) -> Vec<WindowRender { id, rect, state, decorations, z }>
```

Then **diff** it against the last applied scene and emit only the `Effect`s that
changed. Think Elm/React reconciliation for windows.

### Migration outline
1. Extract `desired_scene` from `apply_manage` (pure; unit-test it directly).
2. Keep a `last_scene` snapshot; compute the diff → `Vec<Effect>`.
3. Route effects through the adapter from #1.
4. Delete the ad-hoc transition checks; the diff subsumes them.

### Payoff
One readable "what should the screen look like" function; transition bugs become
structurally impossible; dedupes protocol chatter. Naturally retires the
last-applied role of `Window.mode`.

Note: `Window.mode` currently serves **two** roles — (a) fullscreen
enter/leave transition detection in `apply_manage`, and (b) focus/z priority in
`render_stack_priority` / `apply_render`'s `place_top` (wm.rs ~657, ~961). So
`last_scene` must snapshot prior state **and** recomputed priority for **every**
window, including ones that did not pass through a manage cycle (e.g. focus-only
changes between renders), or transition detection and z-ordering will regress.

Note (implementation): River splits window management into **two** protocol
phases — `ManageStart` (dimensions / fullscreen / server-side-decoration
effects) and `RenderStart` (position / z-order / border effects). The
reconciler therefore keeps **two independent scene snapshots** rather than one:
`last_manage_scene` (diffed and updated by `apply_manage`) and
`last_render_scene` (diffed and updated by `apply_render`). Each phase diffs the
fresh `desired_scene()` against *its own* prior snapshot and emits only the
effects that belong to that phase.

This split is not optional: a single shared snapshot would let `apply_manage`
overwrite the baseline before `apply_render` runs, so a newly mapped window
would receive its manage-phase effects but never its render-phase effects (it would
already be present in the snapshot with an equal scene and diff to zero). The two
snapshots also preserve the "every window, every cycle" requirement from the note
above — each phase re-snapshots z-priority and border for *all* windows, so a
focus-only change that arrives between renders is still caught by
`last_render_scene`. Keep both snapshots in mind whenever touching the reconciler;
collapsing them back into one silently drops render-phase effects for new windows.

### Risk / effort
Medium. Best done right after #1. **~3–5 days.**

### Dependency
Pairs with #1 (diff produces `Effect`s). Retires the remaining need for
`Window.mode`.

---

## Initiative 3 — Make illegal states unrepresentable

### Problem
Even after collapsing `WindowMode` into `WindowState` (already done on the
feature branch), state still lives in two places: the authoritative tree node
and the `Window.mode` cache. And geometry/state relationships are only enforced
by convention (e.g. "tiled windows never set `floating_rect`").

### Target design
A single typed state model with explicit, total transitions. Options:

- **Enum-with-data** where each variant carries exactly the data valid for it
  (`Floating { rect }`, `PseudoTiled { rect }`, `Fullscreen { restore: BaseState }`,
  `Tiled`), so a tiled window *cannot* hold a floating rect and `PseudoTiled`
  carries its own (capped) rect rather than sharing `Floating`'s.
- **Typestate** if we want compile-time transition enforcement (heavier).

The tree node owns this; `Window` no longer caches state (that role moved to the
reconciler's `last_scene` in #2).

### Migration outline
1. Land #2 first so `Window.mode` has no readers left.
2. Delete `Window.mode`.
3. Fold `floating_rect` + `base_state` into the node's state enum so invalid
   combinations are unrepresentable.
4. Replace the hand-maintained invariants/comments with types.

### Payoff
The compiler enforces what comments currently promise; the "two state machines"
smell is gone for good.

### Risk / effort
Medium. **~2–4 days**, once #2 has removed the `Window.mode` readers.

### Dependency
Do after #2.

---

## Initiative 4 — Layout as a pluggable trait

### Problem
BSP tiling is hard-wired. Adding alternative layouts (master-stack, grid,
tabbed, monocle) would mean surgery in core.

### Target design
```
trait Layout {
    fn arrange(&self, windows: &[WindowId], area: Rect) -> Vec<(WindowId, Rect)>;
    // + insert/remove/focus-navigation hooks as needed
}
```
BSP becomes one implementation; the WM holds a `Box<dyn Layout>` (or enum) per
output. Floating/fullscreen handling stays outside the tiling layout.

### Migration outline
1. Define the trait against the current `LayoutTree` API (find the minimal set of
   methods actually needed by core).
2. Implement it for the existing BSP tree (no behaviour change).
3. Route core through the trait.
4. Add a second layout (e.g. monocle) as validation.

### Payoff
Each layout is small and independently testable; strong feature runway.

### Risk / effort
Medium. **~1 week.** The hard part is a trait that fits both tiling and the
floating/fullscreen escape hatches.

### Dependency
Cleaner after #1/#3, but largely independent.

---

## Initiative 5 — Unify the config schema

### Problem
`config/lua.rs` and `config/toml.rs` are parallel loaders feeding a shared
`parser`. Adding one field (as with `default_float_ratio`) means edits in three
files, and the two paths can drift.

### Target design
Define the schema **once** (serde structs with validation), and bridge Lua →
serde so both formats share one validation/build path. Helpers like
`validate_ratio` live in exactly one place.

### Migration outline
1. Define serde structs for the config shape (single source).
2. TOML → serde directly.
3. Lua table → serde via a bridge (e.g. `mlua` value → `serde` deserializer).
4. Collapse `parser::build_layout` and the per-format duplication.
5. Adding a field becomes a one-line struct change.

### Payoff
Removes duplicated parse/build logic that is already drifting; makes config
changes trivial and consistent.

### Risk / effort
Low-medium. **~2–4 days.** Mostly independent of the others.

---

## Recommended sequencing

```
Phase 0  Characterization tests            (prerequisite)
   │
   ├─ 1  Pure core + IO adapter            (foundation)
   │      │
   │      └─ 2  Declarative reconciler      (retires Window.mode's last-applied role)
   │             │
   │             └─ 3  Illegal states unrepresentable
   │
   ├─ 4  Layout trait                        (independent; schedule anytime after 1)
   └─ 5  Config schema unify                 (independent; schedule anytime)
```

**If only one:** do #1 — it makes the whole codebase testable without a live
compositor and unlocks #2/#3.

## Rough total

| Item | Effort |
|------|--------|
| Phase 0 tests | 0.5–1 day |
| 1. Pure core + adapter | 1–2 weeks |
| 2. Reconciler | 3–5 days |
| 3. Illegal states | 2–4 days |
| 4. Layout trait | ~1 week |
| 5. Config schema | 2–4 days |

## Already done (feature branch, groundwork)

These smaller cleanups on `feat-default-float-ratio` reduce the surface the big
refactor has to touch:

- Sizing bug fixed: floating/pseudo windows use `default_float_ratio` instead of
  the compositor-imposed size; `DimensionsHint` wired up.
- `pseudo_tiled_rect` returns `Rect` (no more `Option` / `.expect`).
- Removed the dead write-only `Window.dimensions` field.
- Collapsed `WindowMode` into `WindowState`: one vocabulary, tree is
  authoritative, `Window.mode` is now an explicit cache (its full removal is
  Initiative #3, gated on #2).
