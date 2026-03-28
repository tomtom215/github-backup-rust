// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub API response types and backup configuration.
//!
//! This crate provides strongly-typed structures that model the GitHub REST API
//! v3 responses used during backup operations. All types implement [`serde::Deserialize`]
//! so they can be deserialised directly from API JSON payloads, and
//! [`serde::Serialize`] so they can be written to disk as backup artefacts.
//!
//! # Organisation
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`config`] | Backup options and output-path configuration |
//! | [`repository`] | Repository metadata |
//! | [`branch`] | Branch list and commit tips |
//! | [`issue`] | Issues, comments, and events |
//! | [`pull_request`] | Pull requests, review comments, commits, and reviews |
//! | [`release`] | Releases and binary release assets |
//! | [`gist`] | Gist metadata |
//! | [`label`] | Repository labels |
//! | [`milestone`] | Repository milestones |
//! | [`hook`] | Repository webhooks |
//! | [`security_advisory`] | Published security advisories |
//! | [`user`] | User / actor summaries |

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod branch;
pub mod config;
pub mod gist;
pub mod hook;
pub mod issue;
pub mod label;
pub mod milestone;
pub mod pull_request;
pub mod release;
pub mod repository;
pub mod security_advisory;
pub mod user;

// Convenience re-exports for the most commonly used types.
pub use branch::{Branch, BranchCommit};
pub use config::{BackupOptions, ConfigFile, OutputConfig};
pub use gist::Gist;
pub use hook::Hook;
pub use issue::{Issue, IssueComment, IssueEvent};
pub use label::Label;
pub use milestone::Milestone;
pub use pull_request::{PullRequest, PullRequestComment, PullRequestCommit, PullRequestReview};
pub use release::{Release, ReleaseAsset};
pub use repository::Repository;
pub use security_advisory::SecurityAdvisory;
pub use user::User;
