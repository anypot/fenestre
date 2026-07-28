//! Runtime state for Fenestre.
//!
//! This module owns all compositor-facing state for the window manager.
//! It contains River protocol proxies, windows, outputs, seats, keybindings,
//! focus state, configuration application, and Wayland dispatch handlers.

mod adapter;
mod commands;
mod config;
mod effects;
mod events;
mod focus;
mod handlers;
mod input;
mod keybindings;
pub(crate) mod output;
mod pointerbindings;
mod reassign;
pub mod rule;
mod scene;
mod seat;
pub(crate) mod window;
mod wm;

pub(crate) use wm::WMState;
