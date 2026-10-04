// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Event types flowing through the TUI event loop.

use github_backup_core::Failure;

/// An event posted by the background tasks to the TUI render loop.
#[derive(Debug)]
pub enum BackupEvent {
    /// A structured log line captured from the tracing subscriber.
    LogLine {
        timestamp: String,
        level: String,
        message: String,
    },
    /// A repository was picked up by a worker task.
    RepoStarted { name: String },
    /// A repository finished (success or failure).
    RepoCompleted {
        name: String,
        success: bool,
        /// Error description when `success` is `false`.
        error: Option<String>,
    },
    /// Total repository count became known after listing.
    ReposDiscovered { total: u64 },
    /// The engine has finished with `current` of `total` repositories
    /// (whatever the outcome, including skipped ones).
    Progress { current: u64, total: u64 },
    /// The backup ran to the end.  It is only a clean success when `failures`
    /// is empty; otherwise the backup is incomplete.
    BackupDone {
        repos_backed_up: u64,
        repos_discovered: u64,
        repos_skipped: u64,
        repos_errored: u64,
        gists_backed_up: u64,
        issues_fetched: u64,
        prs_fetched: u64,
        workflows_fetched: u64,
        discussions_fetched: u64,
        elapsed_secs: f64,
        failures: Vec<Failure>,
        dry_run: bool,
    },
    /// Backup run failed with a fatal error.
    BackupFailed { error: String },
    /// The backup stopped because the user cancelled it.
    BackupCancelled,
    /// A process signal (SIGTERM, SIGHUP, SIGINT) asked the TUI to exit.
    /// `code` is the conventional exit status (128 + signal number).
    Shutdown { code: u8 },
    /// A verify step completed.
    VerifyDone {
        ok: u64,
        tampered: Vec<String>,
        missing: Vec<String>,
        unexpected: Vec<String>,
    },
    /// Verify failed before it could produce a report.
    VerifyFailed { error: String },
}
