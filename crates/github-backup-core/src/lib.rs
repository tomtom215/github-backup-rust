// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Core backup engine: orchestration, storage, and git operations.
//!
//! # Architecture
//!
//! ```text
//! BackupEngine
//!   ├── GitHubClient  (API calls, pagination, rate limiting)
//!   ├── Storage       (filesystem write abstraction)
//!   └── GitRunner     (subprocess-based git clone/fetch)
//! ```
//!
//! Three key traits enable testability and loose coupling:
//!
//! | Trait | Purpose |
//! |-------|---------|
//! [`Storage`] | Read/write JSON artefacts and binary asset files |
//! [`GitRunner`] | Execute git clone/fetch as a subprocess |
//! [`BackupEngine`] | Orchestrate a full backup of an owner |
//!
//! The production implementations ([`FsStorage`], [`ProcessGitRunner`]) use
//! the real filesystem and real `git` binary. Tests substitute lightweight
//! in-memory or no-op replacements.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod backup;
pub mod engine;
pub mod error;
pub mod git;
pub mod storage;

pub use engine::BackupEngine;
pub use error::CoreError;
pub use git::{GitRunner, ProcessGitRunner};
pub use storage::{FsStorage, Storage};
