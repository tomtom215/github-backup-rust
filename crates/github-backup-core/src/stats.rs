// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`BackupStats`] — counters collected during a backup run.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Statistics gathered during a [`BackupEngine`] run.
///
/// All counters are incremented atomically so that concurrent repository tasks
/// can write to the same `BackupStats` without locking.
///
/// [`BackupEngine`]: crate::BackupEngine
#[derive(Debug, Default)]
pub struct BackupStats {
    inner: Arc<StatsInner>,
}

#[derive(Debug, Default)]
struct StatsInner {
    /// Total repositories discovered for the owner (user or org listing).
    repos_discovered: AtomicU64,
    /// Repositories successfully mirrored or updated.
    repos_backed_up: AtomicU64,
    /// Repositories skipped (fork/private filters, dry-run mode).
    repos_skipped: AtomicU64,
    /// Repositories where a non-fatal error was encountered.
    repos_errored: AtomicU64,
    /// Gists backed up (cloned or updated).
    gists_backed_up: AtomicU64,
}

impl BackupStats {
    /// Creates a new, zeroed [`BackupStats`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a cheaply cloneable handle to the same counters.
    ///
    /// The engine passes cloned handles to spawned tasks so they can update
    /// the shared counters concurrently.
    #[must_use]
    pub fn handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }

    // ── Incrementors ──────────────────────────────────────────────────────

    /// Records that `n` repositories were discovered.
    pub fn add_discovered(&self, n: u64) {
        self.inner.repos_discovered.fetch_add(n, Ordering::Relaxed);
    }

    /// Records that one repository was successfully backed up.
    pub fn inc_backed_up(&self) {
        self.inner.repos_backed_up.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that one repository was skipped.
    pub fn inc_skipped(&self) {
        self.inner.repos_skipped.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that one repository encountered a non-fatal error.
    pub fn inc_errored(&self) {
        self.inner.repos_errored.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that one gist was backed up.
    pub fn inc_gists(&self) {
        self.inner.gists_backed_up.fetch_add(1, Ordering::Relaxed);
    }

    // ── Accessors ─────────────────────────────────────────────────────────

    /// Repositories discovered in the owner listing.
    #[must_use]
    pub fn repos_discovered(&self) -> u64 {
        self.inner.repos_discovered.load(Ordering::Relaxed)
    }

    /// Repositories successfully backed up.
    #[must_use]
    pub fn repos_backed_up(&self) -> u64 {
        self.inner.repos_backed_up.load(Ordering::Relaxed)
    }

    /// Repositories skipped due to filters or dry-run mode.
    #[must_use]
    pub fn repos_skipped(&self) -> u64 {
        self.inner.repos_skipped.load(Ordering::Relaxed)
    }

    /// Repositories that encountered a non-fatal error.
    #[must_use]
    pub fn repos_errored(&self) -> u64 {
        self.inner.repos_errored.load(Ordering::Relaxed)
    }

    /// Gists backed up (cloned or updated).
    #[must_use]
    pub fn gists_backed_up(&self) -> u64 {
        self.inner.gists_backed_up.load(Ordering::Relaxed)
    }
}

impl fmt::Display for BackupStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "repos: {} backed up, {} skipped, {} errored; gists: {} backed up",
            self.repos_backed_up(),
            self.repos_skipped(),
            self.repos_errored(),
            self.gists_backed_up(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_stats_default_all_zero() {
        let s = BackupStats::new();
        assert_eq!(s.repos_discovered(), 0);
        assert_eq!(s.repos_backed_up(), 0);
        assert_eq!(s.repos_skipped(), 0);
        assert_eq!(s.repos_errored(), 0);
        assert_eq!(s.gists_backed_up(), 0);
    }

    #[test]
    fn backup_stats_increments_correctly() {
        let s = BackupStats::new();
        s.add_discovered(5);
        s.inc_backed_up();
        s.inc_backed_up();
        s.inc_skipped();
        s.inc_errored();
        s.inc_gists();
        s.inc_gists();
        s.inc_gists();

        assert_eq!(s.repos_discovered(), 5);
        assert_eq!(s.repos_backed_up(), 2);
        assert_eq!(s.repos_skipped(), 1);
        assert_eq!(s.repos_errored(), 1);
        assert_eq!(s.gists_backed_up(), 3);
    }

    #[test]
    fn backup_stats_handle_shares_counters() {
        let s = BackupStats::new();
        let h = s.handle();
        h.inc_backed_up();
        assert_eq!(s.repos_backed_up(), 1);
    }

    #[test]
    fn backup_stats_display_shows_all_fields() {
        let s = BackupStats::new();
        s.inc_backed_up();
        s.inc_skipped();
        s.inc_errored();
        s.inc_gists();
        let display = format!("{s}");
        assert!(display.contains("backed up"));
        assert!(display.contains("skipped"));
        assert!(display.contains("errored"));
    }
}
