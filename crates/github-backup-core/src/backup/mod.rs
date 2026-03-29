// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Per-category backup helpers invoked by the [`crate::engine::BackupEngine`].
//!
//! Each submodule handles one data category (repositories, issues, …) and
//! exposes a single async function that writes artefacts via the
//! [`crate::storage::Storage`] and [`crate::git::GitRunner`] traits.

pub mod actions;
pub mod branches;
pub mod collaborators;
pub mod deploy_keys;
pub mod environments;
pub mod gist;
pub mod hooks;
pub mod issue;
pub mod labels;
pub mod milestones;
pub mod pull_request;
pub mod release;
pub mod repository;
pub mod security_advisories;
pub mod starred_repos;
pub mod topics;
pub mod user_data;
pub mod wiki;

#[cfg(test)]
pub(crate) mod mock_client;
