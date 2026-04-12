// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`BackupStats`] — counters collected during a backup run.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Statistics gathered during a [`BackupEngine`] run.
///
/// All counters are incremented atomically so that concurrent repository tasks
/// can write to the same `BackupStats` without locking.
///
/// [`BackupEngine`]: crate::BackupEngine
#[derive(Debug)]
pub struct BackupStats {
    inner: Arc<StatsInner>,
}

#[derive(Debug)]
struct StatsInner {
    /// Wall-clock instant at which the [`BackupStats`] was created.
    started_at: Instant,
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
    /// Total issues fetched across all repositories.
    issues_fetched: AtomicU64,
    /// Total pull requests fetched across all repositories.
    prs_fetched: AtomicU64,
    /// Total GitHub Actions workflows fetched across all repositories.
    workflows_fetched: AtomicU64,
    /// Total GitHub Discussions fetched across all repositories.
    discussions_fetched: AtomicU64,
}

impl Default for StatsInner {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            repos_discovered: AtomicU64::new(0),
            repos_backed_up: AtomicU64::new(0),
            repos_skipped: AtomicU64::new(0),
            repos_errored: AtomicU64::new(0),
            gists_backed_up: AtomicU64::new(0),
            issues_fetched: AtomicU64::new(0),
            prs_fetched: AtomicU64::new(0),
            workflows_fetched: AtomicU64::new(0),
            discussions_fetched: AtomicU64::new(0),
        }
    }
}

impl Default for BackupStats {
    fn default() -> Self {
        Self {
            inner: Arc::new(StatsInner::default()),
        }
    }
}

impl BackupStats {
    /// Creates a new, zeroed [`BackupStats`] with the elapsed timer started
    /// at the moment of construction.
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

    /// Records that `n` gists were backed up (batch form of [`inc_gists`]).
    ///
    /// Prefer this over calling [`inc_gists`] in a loop for O(1) behaviour.
    ///
    /// [`inc_gists`]: BackupStats::inc_gists
    pub fn add_gists(&self, n: u64) {
        self.inner.gists_backed_up.fetch_add(n, Ordering::Relaxed);
    }

    /// Records that `n` issues were fetched for a repository.
    pub fn add_issues(&self, n: u64) {
        self.inner.issues_fetched.fetch_add(n, Ordering::Relaxed);
    }

    /// Records that `n` pull requests were fetched for a repository.
    pub fn add_prs(&self, n: u64) {
        self.inner.prs_fetched.fetch_add(n, Ordering::Relaxed);
    }

    /// Records that `n` Actions workflows were fetched for a repository.
    pub fn add_workflows(&self, n: u64) {
        self.inner.workflows_fetched.fetch_add(n, Ordering::Relaxed);
    }

    /// Records that `n` Discussions were fetched for a repository.
    pub fn add_discussions(&self, n: u64) {
        self.inner
            .discussions_fetched
            .fetch_add(n, Ordering::Relaxed);
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

    /// Total issues fetched across all repositories.
    #[must_use]
    pub fn issues_fetched(&self) -> u64 {
        self.inner.issues_fetched.load(Ordering::Relaxed)
    }

    /// Total pull requests fetched across all repositories.
    #[must_use]
    pub fn prs_fetched(&self) -> u64 {
        self.inner.prs_fetched.load(Ordering::Relaxed)
    }

    /// Total GitHub Actions workflows fetched across all repositories.
    #[must_use]
    pub fn workflows_fetched(&self) -> u64 {
        self.inner.workflows_fetched.load(Ordering::Relaxed)
    }

    /// Elapsed seconds since this [`BackupStats`] was constructed.
    ///
    /// Because every handle shares the same [`Arc`], this returns the time
    /// since the *original* stats object was created — i.e. the total elapsed
    /// time for the backup run regardless of which handle is queried.
    #[must_use]
    pub fn elapsed_secs(&self) -> f64 {
        self.inner.started_at.elapsed().as_secs_f64()
    }
}

impl fmt::Display for BackupStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "repos: {}/{} backed up, {} skipped, {} errored; \
             gists: {} backed up; \
             issues: {} fetched; PRs: {} fetched; \
             workflows: {} fetched \
             ({:.1}s elapsed)",
            self.repos_backed_up(),
            self.repos_discovered(),
            self.repos_skipped(),
            self.repos_errored(),
            self.gists_backed_up(),
            self.issues_fetched(),
            self.prs_fetched(),
            self.workflows_fetched(),
            self.elapsed_secs(),
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
    fn backup_stats_elapsed_secs_is_non_negative() {
        let s = BackupStats::new();
        assert!(s.elapsed_secs() >= 0.0);
    }

    #[test]
    fn backup_stats_handle_shares_timer() {
        let s = BackupStats::new();
        let h = s.handle();
        // Both the original and the handle report the same elapsed time.
        let elapsed_s = s.elapsed_secs();
        let elapsed_h = h.elapsed_secs();
        // They should be very close (within 1 second of each other).
        assert!((elapsed_s - elapsed_h).abs() < 1.0);
    }

    #[test]
    fn backup_stats_display_shows_all_fields() {
        let s = BackupStats::new();
        s.add_discovered(5);
        s.inc_backed_up();
        s.inc_skipped();
        s.inc_errored();
        s.inc_gists();
        let display = format!("{s}");
        assert!(display.contains("1/5"), "should show backed_up/discovered");
        assert!(display.contains("backed up"));
        assert!(display.contains("skipped"));
        assert!(display.contains("errored"));
        assert!(display.contains("elapsed"));
    }

    #[test]
    fn backup_stats_add_gists_batch() {
        let s = BackupStats::new();
        s.add_gists(7);
        assert_eq!(s.gists_backed_up(), 7);
    }

    // ── Issue / PR / workflow / discussion counters ───────────────────────
    //
    // These tests pin down each (incrementor, accessor) pair so that
    // mutation tests cannot replace either side with a constant or a
    // no-op without observable failure.

    #[test]
    fn backup_stats_add_issues_is_observed_via_accessor() {
        let s = BackupStats::new();
        assert_eq!(s.issues_fetched(), 0, "starts at zero");
        s.add_issues(13);
        assert_eq!(s.issues_fetched(), 13);
        s.add_issues(2);
        assert_eq!(s.issues_fetched(), 15);
    }

    #[test]
    fn backup_stats_add_prs_is_observed_via_accessor() {
        let s = BackupStats::new();
        assert_eq!(s.prs_fetched(), 0, "starts at zero");
        s.add_prs(8);
        assert_eq!(s.prs_fetched(), 8);
        s.add_prs(4);
        assert_eq!(s.prs_fetched(), 12);
    }

    #[test]
    fn backup_stats_add_workflows_is_observed_via_accessor() {
        let s = BackupStats::new();
        assert_eq!(s.workflows_fetched(), 0, "starts at zero");
        s.add_workflows(5);
        assert_eq!(s.workflows_fetched(), 5);
        s.add_workflows(1);
        assert_eq!(s.workflows_fetched(), 6);
    }

    #[test]
    fn backup_stats_add_discussions_increments_internal_counter() {
        // No public accessor for discussions yet, so verify the increment
        // through the `Display` impl which renders the same atomic.
        let s = BackupStats::new();
        s.add_discussions(3);
        // The Display impl doesn't print discussions yet; round-trip via
        // a second add to ensure it's *not* a no-op (mutation `with ()`).
        s.add_discussions(2);
        assert_eq!(
            s.inner.discussions_fetched.load(Ordering::Relaxed),
            5,
            "add_discussions must accumulate"
        );
    }

    #[test]
    fn backup_stats_counters_are_independent() {
        // Cross-counter sanity check: incrementing one counter must not
        // bleed into a sibling counter (defends against accidental
        // copy/paste of the wrong AtomicU64 in the impl).
        let s = BackupStats::new();
        s.add_issues(100);
        s.add_prs(200);
        s.add_workflows(300);
        s.add_discussions(400);

        assert_eq!(s.issues_fetched(), 100);
        assert_eq!(s.prs_fetched(), 200);
        assert_eq!(s.workflows_fetched(), 300);
        assert_eq!(s.inner.discussions_fetched.load(Ordering::Relaxed), 400);
    }

    #[test]
    fn backup_stats_elapsed_secs_increases_monotonically() {
        // Pins down `elapsed_secs` so it cannot be replaced with the
        // constant `0.0` or `1.0` mutants.
        let s = BackupStats::new();
        let t0 = s.elapsed_secs();
        // Brief, non-flaky busy-wait — 5ms is enough for `Instant`.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let t1 = s.elapsed_secs();
        assert!(t1 > t0, "elapsed must strictly grow ({t0} -> {t1})");
        // And it must be a real, small number — not the constant 1.0
        // a mutant might substitute.
        assert!(t1 < 1.0, "5ms cannot exceed 1s ({t1})");
    }
}
