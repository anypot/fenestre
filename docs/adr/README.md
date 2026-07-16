# Architecture Decision Records

Accepted decisions for Fenestre, in numeric order. Each record captures the
context, the decision, and its consequences. The conceptual model and the
invariants these decisions enforce live in [`../architecture.md`](../architecture.md).

- [0001 — Hexagonal Architecture](0001-hexagonal-architecture.md)
  The `state` module is split into a pure core, an IO-only adapter, and a runtime
  wiring layer; protocol I/O is confined to the adapter.
- [0002 — Declarative Scene Reconciler](0002-declarative-reconciler.md)
  `apply_manage` / `apply_render` each keep their own scene snapshot and diff the
  fresh `desired_scene()` against it; the two snapshots are never collapsed.
- [0003 — WindowState as Enum-with-Data](0003-windowstate-enum.md)
  Window state is a single enum-with-data on the layout tree node so illegal states
  are unrepresentable.
