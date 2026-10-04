// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Incremental state: which repositories already have their per-item files
//! captured, and since when.
//!
//! The rules (see also [`crate::backup::merge`]):
//!
//! * Issue and pull request **lists are always fetched in full** and merged
//!   into what is stored; the watermark only lets the engine skip the
//!   per-item comment / event / commit / review requests for items that have
//!   not changed since.
//! * A repository's watermark is per repository and only advances when every
//!   step for it succeeded in a run.
//! * A watermark is trusted only for the categories it was recorded with.
//! * `--full` ignores watermarks; an explicit `--since` replaces them and is
//!   never written back (a mistyped date must not poison later runs).

use std::path::Path;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use tracing::{info, warn};

use github_backup_types::backup_state::{BackupState, RunRecord};
use github_backup_types::config::BackupOptions;

/// How far before the start of a run the stored watermark is set.
///
/// An item is judged "unchanged" when its `updated_at` is *earlier* than the
/// watermark, and `updated_at` comes from GitHub's clock while the run start
/// comes from ours.  If this machine's clock runs ahead of GitHub's, an item
/// changed while the run was in flight could carry an `updated_at` earlier
/// than our start time and be skipped for good.  Backing the watermark off by
/// a quarter of an hour covers any plausible clock error and the lag of
/// GitHub's read replicas, at the price of re-fetching the few items changed
/// in that window.
pub(crate) const WATERMARK_MARGIN: Duration = Duration::minutes(15);

/// The incremental state a run starts from.
#[derive(Debug)]
pub(crate) struct Incremental {
    state: BackupState,
    explicit_since: Option<String>,
    full: bool,
    categories: Vec<&'static str>,
}

/// What the run produced, for [`Incremental::finish`].
#[derive(Debug)]
pub(crate) struct RunSummary<'a> {
    pub started_at: DateTime<Utc>,
    pub clean_repos: &'a [String],
    pub repos_backed_up: u64,
    pub fully_successful: bool,
}

/// RFC 3339 with a `Z` suffix and whole seconds, the form the state file uses.
fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl Incremental {
    /// Loads the previous state from `path`.
    ///
    /// A missing file is a first run; an unreadable one is reported and
    /// treated the same way, which only costs a full fetch.
    pub(crate) fn load(path: &Path, opts: &BackupOptions) -> Self {
        let state = match BackupState::load(path) {
            Ok(Some(state)) => state,
            Ok(None) => {
                info!("no earlier backup state: every item is fetched in full this run");
                BackupState::default()
            }
            Err(e) => {
                warn!(error = %e, "backup state file is unreadable; every item is fetched in full this run");
                BackupState::default()
            }
        };
        Self {
            state,
            explicit_since: opts.since.clone(),
            full: opts.full,
            categories: opts.incremental_categories(),
        }
    }

    /// The instant before which `full_name`'s items count as already captured,
    /// or `None` to fetch everything for it.
    pub(crate) fn since_for(&self, full_name: &str) -> Option<String> {
        if self.full {
            return None;
        }
        if let Some(since) = &self.explicit_since {
            return Some(since.clone());
        }
        self.state
            .watermark_for(full_name, &self.categories)
            .map(str::to_owned)
    }

    /// Folds the finished run into the state and writes it to `path`.
    ///
    /// With an explicit `--since` no watermark is recorded: the user *asserted*
    /// that older items are captured, the engine did not verify it, and
    /// recording it would carry the assertion into every later run.
    pub(crate) fn finish(mut self, path: &Path, run: &RunSummary<'_>) -> Result<(), String> {
        let started_at = rfc3339(run.started_at);
        let watermark = rfc3339(run.started_at - WATERMARK_MARGIN);
        let trusted: &[String] = if self.explicit_since.is_some() {
            &[]
        } else {
            run.clean_repos
        };
        self.state.record_run(&RunRecord {
            started_at: &started_at,
            watermark: &watermark,
            categories: self.categories.clone(),
            clean_repos: trusted,
            repos_backed_up: run.repos_backed_up,
            fully_successful: run.fully_successful,
            tool_version: env!("CARGO_PKG_VERSION"),
        });
        self.state.save(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn opts() -> BackupOptions {
        BackupOptions {
            issue_comments: true,
            ..Default::default()
        }
    }

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, h, m, 0)
            .single()
            .expect("valid time")
    }

    fn run<'a>(clean: &'a [String], ok: bool) -> RunSummary<'a> {
        RunSummary {
            started_at: at(12, 0),
            clean_repos: clean,
            repos_backed_up: clean.len() as u64,
            fully_successful: ok,
        }
    }

    #[test]
    fn first_run_has_no_watermark() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inc = Incremental::load(&dir.path().join("state.json"), &opts());
        assert_eq!(inc.since_for("o/a"), None);
    }

    #[test]
    fn a_clean_run_records_a_watermark_before_the_start_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("json").join("state.json");
        let clean = vec!["o/a".to_string()];
        Incremental::load(&path, &opts())
            .finish(&path, &run(&clean, true))
            .expect("save");

        let next = Incremental::load(&path, &opts());
        assert_eq!(
            next.since_for("o/a").as_deref(),
            Some("2026-03-01T11:45:00Z"),
            "start 12:00 minus the 15-minute margin"
        );
        assert_eq!(next.since_for("o/other"), None);
    }

    #[test]
    fn a_failed_repository_is_not_given_a_watermark() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        // o/a was clean, o/b failed (so it is not in the clean list).
        let clean = vec!["o/a".to_string()];
        Incremental::load(&path, &opts())
            .finish(&path, &run(&clean, false))
            .expect("save");
        let next = Incremental::load(&path, &opts());
        assert!(next.since_for("o/a").is_some());
        assert_eq!(next.since_for("o/b"), None);
    }

    #[test]
    fn enabling_another_category_forces_a_full_fetch_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let clean = vec!["o/a".to_string()];
        Incremental::load(&path, &opts())
            .finish(&path, &run(&clean, true))
            .expect("save");

        let more = BackupOptions {
            issue_comments: true,
            pull_reviews: true,
            ..Default::default()
        };
        assert_eq!(
            Incremental::load(&path, &more).since_for("o/a"),
            None,
            "pull_reviews was never captured under this watermark"
        );
    }

    #[test]
    fn full_ignores_every_watermark() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let clean = vec!["o/a".to_string()];
        Incremental::load(&path, &opts())
            .finish(&path, &run(&clean, true))
            .expect("save");
        let full = BackupOptions {
            full: true,
            ..opts()
        };
        assert_eq!(Incremental::load(&path, &full).since_for("o/a"), None);
    }

    #[test]
    fn an_explicit_since_applies_to_every_repository_and_is_never_recorded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let with_since = BackupOptions {
            since: Some("2030-01-01T00:00:00Z".into()),
            ..opts()
        };
        let inc = Incremental::load(&path, &with_since);
        assert_eq!(
            inc.since_for("anything/at-all").as_deref(),
            Some("2030-01-01T00:00:00Z")
        );

        let clean = vec!["o/a".to_string()];
        inc.finish(&path, &run(&clean, true)).expect("save");
        assert_eq!(
            Incremental::load(&path, &opts()).since_for("o/a"),
            None,
            "an asserted --since must not become a stored watermark"
        );
    }

    #[test]
    fn a_corrupt_state_file_means_a_full_fetch_not_a_crash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"{ not json").expect("write");
        assert_eq!(Incremental::load(&path, &opts()).since_for("o/a"), None);
    }

    #[test]
    fn margin_is_applied_across_a_day_boundary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let clean = vec!["o/a".to_string()];
        Incremental::load(&path, &opts())
            .finish(
                &path,
                &RunSummary {
                    started_at: Utc.with_ymd_and_hms(2026, 3, 1, 0, 5, 0).single().unwrap(),
                    clean_repos: &clean,
                    repos_backed_up: 1,
                    fully_successful: true,
                },
            )
            .expect("save");
        assert_eq!(
            Incremental::load(&path, &opts())
                .since_for("o/a")
                .as_deref(),
            Some("2026-02-28T23:50:00Z")
        );
    }
}
