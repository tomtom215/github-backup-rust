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
//! | [`glob`] | Glob pattern matching for `--include-repos` / `--exclude-repos` |
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
//! | [`deploy_key`] | Repository deploy keys |
//! | [`collaborator`] | Repository collaborators with permission levels |
//! | [`team`] | GitHub organisation teams |
//! | [`starred_queue`] | Durable queue types for starred-repo clone progress |
//! | [`workflow`] | GitHub Actions workflow and workflow-run metadata |
//! | [`environment`] | Repository deployment environments and protection rules |

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod backup_state;
pub mod branch;
pub mod collaborator;
pub mod config;
pub mod discussion;
pub mod package;
pub mod project;
pub mod deploy_key;
pub mod environment;
pub mod gist;
pub mod glob;
pub mod hook;
pub mod issue;
pub mod label;
pub mod milestone;
pub mod pull_request;
pub mod release;
pub mod repository;
pub mod security_advisory;
pub mod starred_queue;
pub mod team;
pub mod user;
pub mod workflow;

// Convenience re-exports for the most commonly used types.
pub use backup_state::{BackupCheckpoint, BackupState};
pub use branch::{Branch, BranchCommit};
pub use discussion::{Discussion, DiscussionCategory, DiscussionComment};
pub use package::{Package, PackageVersion};
pub use project::{ClassicProject, ProjectCard, ProjectColumn};
pub use collaborator::{Collaborator, CollaboratorPermissions};
pub use config::{BackupOptions, ConfigFile, OutputConfig};
pub use deploy_key::DeployKey;
pub use environment::{DeploymentBranchPolicy, Environment, EnvironmentProtectionRule};
pub use gist::Gist;
pub use hook::Hook;
pub use issue::{Issue, IssueComment, IssueEvent};
pub use label::Label;
pub use milestone::Milestone;
pub use pull_request::{PullRequest, PullRequestComment, PullRequestCommit, PullRequestReview};
pub use release::{Release, ReleaseAsset};
pub use repository::Repository;
pub use security_advisory::SecurityAdvisory;
pub use team::{Team, TeamParent};
pub use user::User;
pub use workflow::{Workflow, WorkflowRun};
