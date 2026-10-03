// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Persistent backup state: last-success timestamp and per-run checkpoint.
//!
//! Three independent files are managed:
//!
//! ## `backup_state.json`
//!
//! Written at the end of every backup run.  Holds one **watermark per
//! repository**: the instant before which everything that repository's
//! incremental categories (issue comments and events, pull request comments,
//! commits and reviews) contain has already been captured.  The next run skips
//! re-fetching those per-item files for issues and pull requests that have not
//! changed since — the lists themselves are always fetched in full.
//!
//! A repository's watermark only advances when *every* step for it succeeded,
//! so a failure is retried by the next run instead of being skipped over.
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

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

// ── Backup state ─────────────────────────────────────────────────────────────

/// Incremental watermark for one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoWatermark {
    /// RFC 3339 instant: items of this repository last updated before it have
    /// had their per-item files captured by a run that fully succeeded.
    pub at: String,
    /// The incremental categories that run captured (for example
    /// `"issue_comments"`).  A watermark vouches for these only: enabling
    /// another category later means the first run for it fetches everything.
    pub covers: Vec<String>,
}

/// What a finished run contributes to the persisted state.
#[derive(Debug, Clone)]
pub struct RunRecord<'a> {
    /// RFC 3339 start of the run (reported as `last_successful_run`).
    pub started_at: &'a str,
    /// RFC 3339 instant to store as the watermark of every clean repository.
    ///
    /// Earlier than `started_at` by a safety margin that absorbs clock skew
    /// between this machine and GitHub and replication lag in GitHub's API.
    pub watermark: &'a str,
    /// Incremental categories that were enabled in the run.
    pub categories: Vec<&'static str>,
    /// Full names of repositories for which every step succeeded.
    pub clean_repos: &'a [String],
    /// Repositories backed up (all steps succeeded) in the run.
    pub repos_backed_up: u64,
    /// `true` if nothing failed anywhere in the run.
    pub fully_successful: bool,
    /// Version of the tool that performed the run.
    pub tool_version: &'a str,
}

/// Persistent record of the incremental state, written by every run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupState {
    /// ISO 8601 start of the last run in which nothing failed, or `None` if
    /// no run has ever finished without failures.
    ///
    /// Informational (shown by the TUI dashboard); incremental decisions use
    /// the per-repository watermarks in [`BackupState::repos`].
    #[serde(default)]
    pub last_successful_run: Option<String>,

    /// Human-readable description of the tool version that wrote this file.
    #[serde(default)]
    pub tool_version: String,

    /// Number of repositories that were backed up in the most recent run.
    #[serde(default)]
    pub repos_backed_up: u64,

    /// Incremental watermark per repository, keyed by `owner/repo`.
    ///
    /// Absent in files written by older versions, which simply means the
    /// first run after upgrading fetches everything once.
    #[serde(default)]
    pub repos: BTreeMap<String, RepoWatermark>,
}

impl BackupState {
    /// The watermark for `full_name`, if it covers every one of `categories`.
    ///
    /// `None` means "fetch everything": the repository has never completed a
    /// clean run, or the run that did not capture one of the categories now in
    /// use.
    #[must_use]
    pub fn watermark_for(&self, full_name: &str, categories: &[&str]) -> Option<&str> {
        let w = self.repos.get(full_name)?;
        categories
            .iter()
            .all(|c| w.covers.iter().any(|covered| covered == c))
            .then_some(w.at.as_str())
    }

    /// Folds the outcome of a finished run into the state.
    ///
    /// * Every clean repository gets a fresh watermark.
    /// * A repository that was not clean keeps its **old** watermark (or none),
    ///   so whatever failed is fetched again next time.
    /// * `last_successful_run` only moves when the whole run succeeded.
    pub fn record_run(&mut self, run: &RunRecord<'_>) {
        for name in run.clean_repos {
            self.repos.insert(
                name.clone(),
                RepoWatermark {
                    at: run.watermark.to_string(),
                    covers: run.categories.iter().map(|c| (*c).to_string()).collect(),
                },
            );
        }
        if run.fully_successful {
            self.last_successful_run = Some(run.started_at.to_string());
        }
        self.tool_version = run.tool_version.to_string();
        self.repos_backed_up = run.repos_backed_up;
    }

    /// Writes the state to `path`, creating parent directories as needed.
    ///
    /// The write is atomic (temporary file, then rename) so a crash cannot
    /// leave a truncated file that would be discarded as corrupt.
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
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("write state file: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("replace state file: {e}")
        })
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
    /// `true` if the run backed up everything it was asked to, with no failures.
    pub success: bool,
    /// Number of failures recorded in the run (0 in entries written by older
    /// versions, which did not record them).
    #[serde(default)]
    pub failures: u64,
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
    /// Default maximum number of history entries to retain when the caller
    /// does not provide a custom limit.
    pub const MAX_ENTRIES: usize = 20;

    /// Appends a new entry and trims the list to `max_entries`.
    ///
    /// Pass [`Self::MAX_ENTRIES`] to use the default limit.
    pub fn push(&mut self, entry: BackupRunEntry, max_entries: usize) {
        self.entries.insert(0, entry);
        self.entries.truncate(max_entries);
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

    fn record<'a>(
        clean: &'a [String],
        categories: Vec<&'static str>,
        fully_successful: bool,
    ) -> RunRecord<'a> {
        RunRecord {
            started_at: "2026-01-02T00:00:00Z",
            watermark: "2026-01-01T23:45:00Z",
            categories,
            clean_repos: clean,
            repos_backed_up: clean.len() as u64,
            fully_successful,
            tool_version: "0.4.0",
        }
    }

    #[test]
    fn backup_state_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.json");

        let mut state = BackupState::default();
        let clean = vec!["o/a".to_string()];
        state.record_run(&record(&clean, vec!["issue_comments"], true));

        state.save(&path).expect("save");
        let loaded = BackupState::load(&path).expect("load").expect("present");
        assert_eq!(
            loaded.last_successful_run.as_deref(),
            Some("2026-01-02T00:00:00Z")
        );
        assert_eq!(loaded.repos_backed_up, 1);
        assert_eq!(loaded.repos["o/a"].at, "2026-01-01T23:45:00Z");
        assert!(
            !dir.path().join("state.json.tmp").exists(),
            "no temporary file may be left behind"
        );
    }

    /// A state file written by 0.3.x has no `repos` map and a plain string
    /// `last_successful_run`; it must still load, as "no watermarks".
    #[test]
    fn legacy_state_file_still_loads() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"last_successful_run":"2025-12-31T00:00:00Z","tool_version":"0.3.2","repos_backed_up":7}"#,
        )
        .expect("write");
        let loaded = BackupState::load(&path).expect("load").expect("present");
        assert_eq!(
            loaded.last_successful_run.as_deref(),
            Some("2025-12-31T00:00:00Z")
        );
        assert_eq!(loaded.repos_backed_up, 7);
        assert!(loaded.repos.is_empty());
        assert_eq!(loaded.watermark_for("o/a", &["issue_comments"]), None);
    }

    #[test]
    fn watermark_requires_every_current_category_to_be_covered() {
        let mut state = BackupState::default();
        let clean = vec!["o/a".to_string()];
        state.record_run(&record(
            &clean,
            vec!["issue_comments", "pull_reviews"],
            true,
        ));

        let at = Some("2026-01-01T23:45:00Z");
        assert_eq!(state.watermark_for("o/a", &["issue_comments"]), at);
        assert_eq!(
            state.watermark_for("o/a", &["issue_comments", "pull_reviews"]),
            at
        );
        assert_eq!(state.watermark_for("o/a", &[]), at);
        assert_eq!(
            state.watermark_for("o/a", &["issue_comments", "issue_events"]),
            None,
            "a category the watermark never covered forces a full fetch"
        );
        assert_eq!(state.watermark_for("o/other", &["issue_comments"]), None);
    }

    #[test]
    fn a_failed_repository_keeps_its_old_watermark() {
        let mut state = BackupState::default();
        let first = vec!["o/a".to_string(), "o/b".to_string()];
        state.record_run(&RunRecord {
            watermark: "2026-01-01T00:00:00Z",
            ..record(&first, vec![], true)
        });

        // Second run: o/b failed, so only o/a is clean.
        let second = vec!["o/a".to_string()];
        state.record_run(&RunRecord {
            watermark: "2026-02-01T00:00:00Z",
            ..record(&second, vec![], false)
        });

        assert_eq!(state.repos["o/a"].at, "2026-02-01T00:00:00Z");
        assert_eq!(
            state.repos["o/b"].at, "2026-01-01T00:00:00Z",
            "the failed repository must be re-examined from its old watermark"
        );
    }

    #[test]
    fn last_successful_run_only_moves_when_nothing_failed() {
        let mut state = BackupState::default();
        let clean = vec!["o/a".to_string()];
        state.record_run(&record(&clean, vec![], false));
        assert_eq!(state.last_successful_run, None, "never succeeded yet");

        state.record_run(&record(&clean, vec![], true));
        let first = state.last_successful_run.clone();
        assert_eq!(first.as_deref(), Some("2026-01-02T00:00:00Z"));

        state.record_run(&RunRecord {
            started_at: "2026-03-01T00:00:00Z",
            ..record(&clean, vec![], false)
        });
        assert_eq!(
            state.last_successful_run, first,
            "a run with failures must not advance the last-success time"
        );
    }

    #[test]
    fn dropping_a_category_drops_it_from_the_watermark() {
        let mut state = BackupState::default();
        let clean = vec!["o/a".to_string()];
        state.record_run(&record(
            &clean,
            vec!["issue_comments", "issue_events"],
            true,
        ));
        // A later run without issue events: the watermark advances for the
        // comments only, so events are no longer vouched for.
        state.record_run(&RunRecord {
            watermark: "2026-02-01T00:00:00Z",
            ..record(&clean, vec!["issue_comments"], true)
        });
        assert!(state.watermark_for("o/a", &["issue_comments"]).is_some());
        assert_eq!(
            state.watermark_for("o/a", &["issue_events"]),
            None,
            "events were not captured by the newer watermark"
        );
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

    fn make_entry(ts: &str) -> BackupRunEntry {
        BackupRunEntry {
            timestamp: ts.to_string(),
            repos_backed_up: 1,
            elapsed_secs: 1.0,
            success: true,
            failures: 0,
            tool_version: "0.1.0".to_string(),
        }
    }

    #[test]
    fn history_push_prepends_newest_first() {
        let mut h = BackupRunHistory::default();
        h.push(make_entry("2026-01-01T00:00:00Z"), 10);
        h.push(make_entry("2026-01-02T00:00:00Z"), 10);
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].timestamp, "2026-01-02T00:00:00Z");
        assert_eq!(h.entries[1].timestamp, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn history_push_truncates_at_max_entries() {
        let mut h = BackupRunHistory::default();
        for i in 0..5 {
            h.push(make_entry(&format!("2026-01-0{}T00:00:00Z", i + 1)), 3);
        }
        assert_eq!(h.entries.len(), 3, "must not exceed max_entries");
        assert_eq!(h.entries[0].timestamp, "2026-01-05T00:00:00Z");
    }

    #[test]
    fn history_roundtrip_save_load() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("history.json");
        let mut h = BackupRunHistory::default();
        h.push(make_entry("2026-01-01T00:00:00Z"), 20);
        h.save(&path).expect("save");
        let loaded = BackupRunHistory::load(&path).expect("load");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].timestamp, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn history_load_missing_returns_empty() {
        let dir = tempdir().expect("tempdir");
        let h = BackupRunHistory::load(&dir.path().join("none.json")).expect("no error");
        assert!(h.entries.is_empty());
    }

    #[test]
    fn checkpoint_is_complete_false_for_unknown_repo() {
        // Pin down `is_complete` so a mutant returning the constant `true`
        // is observable.
        let cp = BackupCheckpoint::default();
        assert!(!cp.is_complete("owner/never-seen"));
    }

    #[test]
    fn checkpoint_is_complete_true_only_for_inserted_repo() {
        let mut cp = BackupCheckpoint::default();
        cp.completed_repos.insert("owner/a".into());
        cp.completed_repos.insert("owner/b".into());
        assert!(cp.is_complete("owner/a"));
        assert!(cp.is_complete("owner/b"));
        assert!(!cp.is_complete("owner/c"));
        assert!(!cp.is_complete("owner/A")); // case-sensitive
    }

    #[test]
    fn checkpoint_delete_missing_is_ok() {
        let dir = tempdir().expect("tempdir");
        // Delete on a path that does not exist must not error.
        BackupCheckpoint::delete(&dir.path().join("nope.json")).expect("noop ok");
    }

    #[test]
    fn history_push_does_not_alter_other_entries() {
        // Pin down the `truncate(max_entries)` mutation so the test
        // observes the boundary precisely.
        let mut h = BackupRunHistory::default();
        for i in 0..3 {
            h.push(make_entry(&format!("2026-01-0{}T00:00:00Z", i + 1)), 5);
        }
        assert_eq!(h.entries.len(), 3);
        assert_eq!(h.entries[0].timestamp, "2026-01-03T00:00:00Z");
        assert_eq!(h.entries[1].timestamp, "2026-01-02T00:00:00Z");
        assert_eq!(h.entries[2].timestamp, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn history_push_truncates_at_exactly_max_not_max_minus_one() {
        // Pin down the off-by-one in `truncate(max_entries)` — pushing
        // exactly `max` items must keep all of them.
        let mut h = BackupRunHistory::default();
        for i in 0..3 {
            h.push(make_entry(&format!("2026-01-0{}T00:00:00Z", i + 1)), 3);
        }
        assert_eq!(h.entries.len(), 3, "must keep all when count == max");
    }
}
