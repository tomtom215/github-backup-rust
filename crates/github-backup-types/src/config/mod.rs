// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Backup configuration types.
//!
//! The main types exported from this module are:
//!
//! - [`Credential`] — authentication credential (token or anonymous)
//! - [`OutputConfig`] — root output path and per-owner directory layout
//! - [`CloneType`] — repository clone strategy (mirror, bare, full, shallow)
//! - [`BackupTarget`] — user account vs. organisation
//! - [`BackupOptions`] — per-category enable flags and execution settings
//! - [`ConfigFile`] — TOML config file schema

mod clone_type;
mod credential;
mod file;
mod options;
mod output;

pub use clone_type::CloneType;
pub use credential::Credential;
pub use file::ConfigFile;
pub use options::{BackupOptions, BackupTarget};
pub use output::OutputConfig;

pub use crate::glob::glob_match;

#[cfg(test)]
mod tests;
