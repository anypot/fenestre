//! Binary space partitioning layout engine.
//!
//! This module owns Fenestre's pure BSP layout policy.
//! It exposes only the crate-internal API needed by runtime state and command handling.
//! The tree representation, split bookkeeping, and traversal helpers remain private.
#![allow(dead_code)]

mod tree;

pub(crate) use tree::{FocusDirection, LayoutTree, Rect, SplitDirection, WindowState, capped_rect};
