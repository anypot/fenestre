//! Runtime state for Fenestre.
//!
//! This module owns all compositor-facing state for the window manager.
//! It contains River protocol proxies, windows, outputs, seats, keybindings,
//! focus state, configuration application, and Wayland dispatch handlers.
#![allow(dead_code)]

mod commands;
mod config;
mod handlers;
mod keybindings;
mod output;
pub mod rule;
mod seat;
mod window;
mod wm;

pub(crate) use output::OutputId;
pub(crate) use wm::WMState;
