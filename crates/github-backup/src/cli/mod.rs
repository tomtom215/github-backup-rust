// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Command-line argument parsing.
//!
//! The module is split into focused sub-modules:
//!
//! - [`args`] — the top-level [`Args`] struct
//! - [`args_impl`] — `merge_config` and `into_backup_options` implementations
//! - [`clone_type`] — the `--clone-type` flag parser (`CliCloneType`)

pub mod args;
mod args_impl;
pub mod clone_type;

pub use args::Args;
