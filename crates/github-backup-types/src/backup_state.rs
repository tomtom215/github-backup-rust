// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Persistent backup state: last-success timestamp and per-run checkpoint.
//!
//! Three independent files are managed:
//!
//! ## `backup_state.json`
//!
//! Written after every *successful* backup run.  Contains the ISO 8601
//! timestamp at which the run started, which the next invocation can use as
//! the `--since` filter automatically — enabling true incremental backups
//! without the user having to track timestamps manually.
//!
//! ## `backup_checkpoint.json`
//!
//! Written *during* a backup run.  Records which repositories have been fully
//! processed so far.  If the process is interrupted (OOM kill, SIGTERM, power
//! loss) a subsequent run can load the checkpoint and skip already-completed
//! repositories rather than restarting from scratch.
//!
//! ## `backup_history.json`
//!
//! A rolling log of the last [`BackupRunHistory::MAX_ENTRIES`] backup runs.
//! Used by the TUI dashboard to display a run history table.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ── Backup state (last successful run) ───────────────────────────────────────

/// Persistent record written after every successful backup run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupState {
    /// ISO 8601 timestamp at which the last successful run *started*.
    ///
    /// Used as the automatic `--since` value on the next run to implement
    /// incremental backups without manual timestamp tracking.
    pub last_successful_run: String,

    /// Human-readable description of the tool version that wrote this file.
    pub tool_version: String,

    /// Number of repositories that were backed up in the last successful run.
    pub repos_backed_up: u64,
}

impl BackupState {
    /// Writes the state to `path`, creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an error string on I/O or serialisation failure.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create state directory: {e}"))?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("serialise state: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("write state file: {e}"))
    }

    /// Loads the state from `path`.
    ///
    /// Returns `None` if the file does not exist (first run) rather than an
    /// error, so callers can treat a missing state file as "no prior run".
    ///
    /// # Errors
    ///
    /// Returns an error string if the file exists but cannot be read or
    /// deserialised (corrupted state file).
    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path).map_err(|e| format!("read state file: {e}"))?;
        let state: Self =
            serde_json::from_str(&content).map_err(|e| format!("parse state file: {e}"))?;
        Ok(Some(state))
    }
}

// ── Backup run history ────────────────────────────────────────────────────────

/// A single entry in the backup run history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRunEntry {
    /// ISO 8601 timestamp of when this run started.
    pub timestamp: String,
    /// Number of repositories backed up during this run.
    pub repos_backed_up: u64,
    /// Elapsed wall-clock time in seconds.
    pub elapsed_secs: f64,
    /// `true` if the run completed without a fatal error.
    pub success: bool,
    /// Tool version that produced this entry.
    pub tool_version: String,
}

/// Rolling history of the last [`BackupRunHistory::MAX_ENTRIES`] backup runs.
///
/// Stored in `backup_history.json` alongside `backup_state.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupRunHistory {
    /// Most recent runs, newest first.
    pub entries: Vec<BackupRunEntry>,
}

impl BackupRunHistory {
    /// Maximum number of history entries to retain.
    pub const MAX_ENTRIES: usize = 20;

    /// Appends a new entry and trims the list to `MAX_ENTRIES`.
    pub fn push(&mut self, entry: BackupRunEntry) {
        self.entries.insert(0, entry);
        self.entries.truncate(Self::MAX_ENTRIES);
    }

    /// Loads the history from `path`.
    ///
    /// Returns an empty history if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error string if the file exists but cannot be parsed.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("read history file: {e}"))?;
        serde_json::from_str(&content).map_err(|e| format!("parse history file: {e}"))
    }

    /// Saves the history to `path`, creating parent directories as needed.
    ///
    /// Writes are atomic (write-then-rename) to prevent corrupt files.
    ///
    /// # Errors
    ///
    /// Returns an error string on I/O or serialisation failure.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create history directory: {e}"))?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("serialise history: {e}"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("write history tmp: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("rename history file: {e}"))
    }
}

// ── Backup checkpoint (in-progress run) ──────────────────────────────────────

/// In-progress checkpoint recording which repositories have been completed.
///
/// Loaded at the start of a run; updated after each repository completes.
/// Deleted on successful completion of the full run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupCheckpoint {
    /// Full names (`owner/repo`) of repositories that have been fully backed up.
    pub completed_repos: HashSet<String>,

    /// ISO 8601 timestamp when this checkpoint was first created (= run start).
    pub run_started_at: String,
}

impl BackupCheckpoint {
    /// Returns `true` if `full_name` has already been completed.
    #[must_use]
    pub fn is_complete(&self, full_name: &str) -> bool {
        self.completed_repos.contains(full_name)
    }

    /// Marks `full_name` as completed and persists the checkpoint to `path`.
    ///
    /// Writes are atomic at the file-system level (write-then-rename) to
    /// ensure the checkpoint is never left in a half-written state.
    ///
    /// # Errors
    ///
    /// Returns an error string on I/O or serialisation failure.
    pub fn mark_complete_and_save(&mut self, full_name: &str, path: &Path) -> Result<(), String> {
        self.completed_repos.insert(full_name.to_string());
        self.save(path)
    }

    /// Loads a checkpoint from `path`.
    ///
    /// Returns an empty checkpoint if the file does not exist (no prior
    /// interrupted run) rather than an error.
    ///
    /// # Errors
    ///
    /// Returns an error string if the file exists but cannot be parsed.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path).map_err(|e| format!("read checkpoint: {e}"))?;
        serde_json::from_str(&content).map_err(|e| format!("parse checkpoint: {e}"))
    }

    /// Saves the checkpoint to `path`, creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an error string on I/O or serialisation failure.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create checkpoint dir: {e}"))?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("serialise checkpoint: {e}"))?;
        // Write to a temp file then rename for atomicity.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("write checkpoint tmp: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("rename checkpoint: {e}"))
    }

    /// Removes the checkpoint file after a successful run.
    ///
    /// A missing file is treated as success (already cleaned up).
    ///
    /// # Errors
    ///
    /// Returns an error string if the file exists but cannot be deleted.
    pub fn delete(path: &Path) -> Result<(), String> {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("delete checkpoint: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn backup_state_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.json");

        let state = BackupState {
            last_successful_run: "2026-01-01T00:00:00Z".to_string(),
            tool_version: "0.3.0".to_string(),
            repos_backed_up: 42,
        };

        state.save(&path).expect("save");
        let loaded = BackupState::load(&path).expect("load").expect("present");
        assert_eq!(loaded.last_successful_run, "2026-01-01T00:00:00Z");
        assert_eq!(loaded.repos_backed_up, 42);
    }

    #[test]
    fn backup_state_load_missing_returns_none() {
        let dir = tempdir().expect("tempdir");
        let result = BackupState::load(&dir.path().join("nonexistent.json")).expect("no error");
        assert!(result.is_none());
    }

    #[test]
    fn backup_checkpoint_mark_and_resume() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("checkpoint.json");

        let mut cp = BackupCheckpoint {
            completed_repos: HashSet::new(),
            run_started_at: "2026-01-01T00:00:00Z".to_string(),
        };
        cp.mark_complete_and_save("owner/repo-a", &path)
            .expect("save");

        let loaded = BackupCheckpoint::load(&path).expect("load");
        assert!(loaded.is_complete("owner/repo-a"));
        assert!(!loaded.is_complete("owner/repo-b"));
    }

    #[test]
    fn backup_checkpoint_load_missing_returns_default() {
        let dir = tempdir().expect("tempdir");
        let cp = BackupCheckpoint::load(&dir.path().join("none.json")).expect("no error");
        assert!(cp.completed_repos.is_empty());
    }

    #[test]
    fn backup_checkpoint_delete_removes_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("cp.json");
        std::fs::write(&path, b"{}").expect("create");
        BackupCheckpoint::delete(&path).expect("delete");
        assert!(!path.exists());
    }
}
