// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Durable queue types for the starred-repository clone feature.
//!
//! [`StarredCloneQueue`] is serialised as JSON to disk after every successfully
//! processed item, so a backup run can be interrupted and resumed without
//! re-cloning repositories that are already done.
//!
//! # Layout
//!
//! The queue file is stored at:
//!
//! ```text
//! <output>/<owner>/json/starred_clone_queue.json
//! ```
//!
//! # States
//!
//! Each item progresses through states as follows:
//!
//! ```text
//! Pending ──success──► Done
//!    │
//!    └──failure (retries < max)──► Pending  (retried with backoff)
//!    └──failure (retries ≥ max)──► Failed
//! ```

use serde::{Deserialize, Serialize};

/// Processing state of a single starred-repository clone task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneState {
    /// Not yet processed or pending retry.
    Pending,
    /// Successfully cloned or updated on disk.
    Done,
    /// All retry attempts exhausted; will not be retried automatically.
    Failed,
}

/// A single entry in the starred-repositories clone queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarredQueueItem {
    /// GitHub repository numeric ID (stable across renames and transfers).
    pub id: u64,
    /// `"owner/repo"` full name used for display and as the clone path.
    pub full_name: String,
    /// HTTPS clone URL as returned by the GitHub API.
    pub clone_url: String,
    /// SSH clone URL as returned by the GitHub API.
    pub ssh_url: String,
    /// Repository size in kibibytes as reported by GitHub (may be 0).
    pub size_kb: u64,
    /// Current processing state.
    pub state: CloneState,
    /// Total failed attempts so far (reset to 0 on success).
    pub retries: u32,
    /// Human-readable last error, if any.
    pub last_error: Option<String>,
    /// RFC-3339 UTC timestamp when this item was completed or permanently failed.
    pub finished_at: Option<String>,
}

impl StarredQueueItem {
    /// Returns the owner portion of [`full_name`](Self::full_name)
    /// (the segment before the first `/`).
    ///
    /// Returns an empty string if `full_name` contains no `/`.
    #[must_use]
    pub fn repo_owner(&self) -> &str {
        self.full_name.split('/').next().unwrap_or("")
    }

    /// Returns the repository-name portion of [`full_name`](Self::full_name)
    /// (the segment after the first `/`).
    ///
    /// Falls back to the full string if no `/` is present.
    #[must_use]
    pub fn repo_name(&self) -> &str {
        self.full_name
            .split_once('/')
            .map(|(_, r)| r)
            .unwrap_or(&self.full_name)
    }
}

/// On-disk durable queue tracking starred-repository clone progress.
///
/// Written atomically (via a temporary file + rename) after every item so
/// that a crash or Ctrl+C can never leave the file in a corrupted state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarredCloneQueue {
    /// Schema version for forward compatibility (currently `1`).
    pub version: u32,
    /// GitHub owner whose starred repositories this queue tracks.
    pub owner: String,
    /// RFC-3339 UTC timestamp when this queue was first created.
    pub created_at: String,
    /// RFC-3339 UTC timestamp of the last write.
    pub updated_at: String,
    /// All known starred repositories across all runs.
    ///
    /// New entries are appended in [`CloneState::Pending`]; existing entries
    /// (matched by repository ID) are never overwritten.
    pub items: Vec<StarredQueueItem>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> StarredQueueItem {
        StarredQueueItem {
            id: 1_296_269,
            full_name: "rust-lang/rust".to_string(),
            clone_url: "https://github.com/rust-lang/rust.git".to_string(),
            ssh_url: "git@github.com:rust-lang/rust.git".to_string(),
            size_kb: 512_000,
            state: CloneState::Pending,
            retries: 0,
            last_error: None,
            finished_at: None,
        }
    }

    fn sample_queue() -> StarredCloneQueue {
        StarredCloneQueue {
            version: 1,
            owner: "octocat".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            items: vec![sample_item()],
        }
    }

    #[test]
    fn repo_owner_splits_correctly() {
        assert_eq!(sample_item().repo_owner(), "rust-lang");
    }

    #[test]
    fn repo_name_splits_correctly() {
        assert_eq!(sample_item().repo_name(), "rust");
    }

    #[test]
    fn repo_owner_fallback_for_no_slash() {
        let mut item = sample_item();
        item.full_name = "no-slash".to_string();
        assert_eq!(item.repo_owner(), "no-slash");
        assert_eq!(item.repo_name(), "no-slash");
    }

    #[test]
    fn clone_state_serialises_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&CloneState::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&CloneState::Done).unwrap(),
            r#""done""#
        );
        assert_eq!(
            serde_json::to_string(&CloneState::Failed).unwrap(),
            r#""failed""#
        );
    }

    #[test]
    fn queue_item_roundtrip() {
        let item = sample_item();
        let json = serde_json::to_string(&item).unwrap();
        let decoded: StarredQueueItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, item.id);
        assert_eq!(decoded.full_name, item.full_name);
        assert_eq!(decoded.clone_url, item.clone_url);
        assert_eq!(decoded.ssh_url, item.ssh_url);
        assert_eq!(decoded.size_kb, item.size_kb);
        assert_eq!(decoded.state, item.state);
        assert_eq!(decoded.retries, item.retries);
        assert!(decoded.last_error.is_none());
        assert!(decoded.finished_at.is_none());
    }

    #[test]
    fn queue_roundtrip() {
        let queue = sample_queue();
        let json = serde_json::to_string(&queue).unwrap();
        let decoded: StarredCloneQueue = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.owner, queue.owner);
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].full_name, "rust-lang/rust");
    }

    #[test]
    fn failed_item_roundtrip() {
        let mut item = sample_item();
        item.state = CloneState::Failed;
        item.retries = 4;
        item.last_error = Some("connection refused".to_string());
        item.finished_at = Some("2026-03-01T12:00:00Z".to_string());

        let json = serde_json::to_string(&item).unwrap();
        let decoded: StarredQueueItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.state, CloneState::Failed);
        assert_eq!(decoded.retries, 4);
        assert_eq!(decoded.last_error.as_deref(), Some("connection refused"));
        assert!(decoded.finished_at.is_some());
    }
}
