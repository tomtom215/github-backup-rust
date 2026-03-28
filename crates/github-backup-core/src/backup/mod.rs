// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Per-category backup helpers invoked by the [`crate::engine::BackupEngine`].
//!
//! Each submodule handles one data category (repositories, issues, …) and
//! exposes a single async function that writes artefacts via the
//! [`crate::storage::Storage`] and [`crate::git::GitRunner`] traits.

pub mod gist;
pub mod issue;
pub mod pull_request;
pub mod release;
pub mod repository;
pub mod user_data;
pub mod wiki;

#[cfg(test)]
pub(crate) mod mock_client;
