//! Internal command dispatch types.
//!
//! Commands are used to route keybinding activations and future IPC/config
//! actions into `WMState` behavior. They are intentionally crate-internal.

mod internal;

pub(crate) use internal::Command;
