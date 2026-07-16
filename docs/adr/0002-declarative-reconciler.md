# ADR 0002: Declarative Scene Reconciler

## Status

Accepted.

## Context

River drives window management through `ManageStart` and `RenderStart` sequences,
and Fenestre must only emit the River protocol calls that actually changed between
frames. A naive approach mutating windows directly during events risks dropping
effects or issuing redundant calls.

## Decision

`apply_manage` / `apply_render` use declarative scene snapshots:

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

## Consequences

- **Do not collapse `last_manage_scene` and `last_render_scene` into one snapshot.**
  River splits window management into `ManageStart` (dimensions/fullscreen/SSD) and
  `RenderStart` (position/z-order/borders); a single shared snapshot would let
  `apply_manage` overwrite the baseline before `apply_render` runs, silently
  dropping render-phase effects for newly mapped windows.
- Each phase must re-snapshot z-priority and border for **every** window so
  focus-only changes between renders are still caught.
- This is captured as invariant 6 in `architecture.md` (Scene snapshot discipline)
  and discipline 3 (Manage/render sequence discipline).
