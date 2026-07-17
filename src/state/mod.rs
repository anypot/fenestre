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
mod keybindings;
mod output;
mod reassign;
pub mod rule;
mod scene;
mod seat;
mod window;
mod wm;

pub(crate) use wm::WMState;
