// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Incremental state: which repositories already have their lists, per-item
//! files and git data captured, and since when.
//!
//! The rules (see also [`crate::backup::merge`]):
//!
//! * A repository's watermark is per repository and only advances when every
//!   step for it succeeded in a run.
//! * A watermark is trusted only for the categories it was recorded with
//!   (`covers`): the per-item categories, the `issues` and `pulls` lists, and
//!   the git update mode.
//! * Issue and pull request **lists** are asked only for what changed since
//!   the watermark and merged into the stored list, when that list is intact
//!   ([`crate::backup::merge::delta_base`]); otherwise in full.
//! * Per-item comment / event / commit / review requests are skipped for
//!   items that have not changed since and whose files are on disk.
//! * The `git fetch` of a repository is skipped when nothing was pushed to it
//!   since the watermark (`pushed_at`), the clone is on disk, and the last
//!   fetch is less than [`REFETCH_AFTER`] old (see [`RepoSince::git`]).
//! * `--full` ignores watermarks; an explicit `--since` replaces them for the
//!   lists and per-item files (never for git) and is never written back (a
//!   mistyped date must not poison later runs).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use jiff::{SignedDuration, Timestamp};

use crate::backup::merge::unchanged_since;
use crate::time::{format_utc, minus, parse_rfc3339};
use tracing::{info, warn};

use github_backup_types::backup_state::{BackupState, RunRecord};
use github_backup_types::config::{BackupOptions, CloneType};
use github_backup_types::Repository;

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
pub(crate) const WATERMARK_MARGIN: SignedDuration = SignedDuration::from_mins(15);

/// The longest a repository's clone goes without a `git fetch`, however
/// little `pushed_at` says has changed.
///
/// A safety valve: GitHub does not document every kind of ref update that
/// moves `pushed_at` (a pushed tag, a deleted branch), so a clone is
/// refreshed at least weekly whatever it says.
pub(crate) const REFETCH_AFTER: SignedDuration = SignedDuration::from_hours(7 * 24);

/// The `covers` key of the issue list (`issues.json`).
const ISSUE_LIST: &str = "issues";
/// The `covers` key of the pull request list (`pulls.json`).
const PULL_LIST: &str = "pulls";
/// The `covers` key recorded when deleted refs were pruned.
const GIT_PRUNE: &str = "git:prune";

/// What one repository's backup may take as already captured.
///
/// Every field is an RFC 3339 instant; `None` means "fetch everything".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RepoSince {
    /// Per-item files (comments, events, commits, reviews) of items not
    /// updated since are kept.
    pub items: Option<String>,
    /// The issue list is asked only for issues updated since.
    pub issue_list: Option<String>,
    /// The pull request list is asked only for pull requests updated since.
    pub pull_list: Option<String>,
    /// Nothing was pushed to the repository since this instant, a clean run
    /// fetched it with the current git mode after it, and that fetch is
    /// recent: the `git fetch` may be skipped if the clone is on disk (and,
    /// for clones that hold `refs/pull/*`, no pull request changed since).
    pub git: Option<String>,
}

/// The incremental state a run starts from.
#[derive(Debug)]
pub(crate) struct Incremental {
    state: BackupState,
    explicit_since: Option<String>,
    full: bool,
    /// Per-item categories enabled in this run.
    categories: Vec<&'static str>,
    /// Every `covers` key this run captures: `categories`, the lists, the
    /// git mode.
    covers: Vec<&'static str>,
    /// The git `covers` keys of this run (empty without `--repositories`).
    git: Vec<&'static str>,
    /// For each repository [`for_repo`](Self::for_repo) was asked about with
    /// git enabled: whether it allowed skipping the fetch.
    git_skippable: Mutex<BTreeMap<String, bool>>,
}

/// What the run produced, for [`Incremental::finish`].
#[derive(Debug)]
pub(crate) struct RunSummary<'a> {
    pub started_at: Timestamp,
    pub clean_repos: &'a [String],
    pub repos_backed_up: u64,
    pub fully_successful: bool,
}

/// The git `covers` keys for `opts`: how the clone is updated.  A change of
/// mode (say from bare to mirror, or pruning switched on) means the clone
/// on disk was not made the way this run would make it, so it is fetched.
fn git_keys(opts: &BackupOptions) -> Vec<&'static str> {
    if !opts.repositories {
        return Vec::new();
    }
    let mode = if opts.lfs {
        "git:lfs"
    } else {
        match opts.clone_type {
            CloneType::Mirror => "git:mirror",
            CloneType::Bare => "git:bare",
            CloneType::Full => "git:full",
            CloneType::Shallow(_) => "git:shallow",
        }
    };
    let mut keys = vec![mode];
    if opts.prune {
        keys.push(GIT_PRUNE);
    }
    keys
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
        let categories = opts.incremental_categories();
        let git = git_keys(opts);
        let mut covers = categories.clone();
        covers.extend(
            [(opts.issues, ISSUE_LIST), (opts.pulls, PULL_LIST)]
                .into_iter()
                .filter_map(|(enabled, key)| enabled.then_some(key)),
        );
        covers.extend(&git);
        Self {
            state,
            explicit_since: opts.since.clone(),
            full: opts.full,
            categories,
            covers,
            git,
            git_skippable: Mutex::new(BTreeMap::new()),
        }
    }

    /// What `repo`'s backup may take as already captured.  Call it once per
    /// repository that is backed up: the git answer is remembered for
    /// [`finish`](Self::finish).
    pub(crate) fn for_repo(&self, repo: &Repository) -> RepoSince {
        let name = repo.full_name.as_str();
        if self.full {
            return self.remember_git(name, RepoSince::default());
        }
        if let Some(since) = &self.explicit_since {
            let since = Some(since.clone());
            return self.remember_git(
                name,
                RepoSince {
                    items: since.clone(),
                    issue_list: since.clone(),
                    pull_list: since,
                    git: None,
                },
            );
        }
        let watermark = |keys: &[&str]| self.state.watermark_for(name, keys).map(str::to_owned);
        let since = RepoSince {
            items: watermark(&self.categories),
            issue_list: watermark(&[ISSUE_LIST]),
            pull_list: watermark(&[PULL_LIST]),
            git: self.git_since(repo),
        };
        self.remember_git(name, since)
    }

    /// See [`RepoSince::git`].
    fn git_since(&self, repo: &Repository) -> Option<String> {
        if self.git.is_empty() {
            return None;
        }
        let at = self.state.watermark_for(&repo.full_name, &self.git)?;
        let fetched_at = self
            .state
            .repos
            .get(&repo.full_name)?
            .fetched_at
            .as_deref()?;
        let recent =
            parse_rfc3339(fetched_at).is_some_and(|t| t > minus(Timestamp::now(), REFETCH_AFTER));
        (recent && unchanged_since(repo.pushed_at.as_deref()?, Some(at))).then(|| at.to_owned())
    }

    fn remember_git(&self, name: &str, since: RepoSince) -> RepoSince {
        if !self.git.is_empty() {
            self.git_skippable
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(name.to_owned(), since.git.is_some());
        }
        since
    }

    /// Folds the finished run into the state and writes it to `path`.
    ///
    /// With an explicit `--since` no watermark is recorded: the user *asserted*
    /// that older items are captured, the engine did not verify it, and
    /// recording it would carry the assertion into every later run.
    ///
    /// The last fetch time of a repository whose fetch was not skippable
    /// becomes this run's watermark if it is clean, and is cleared if it is
    /// not, so that its next run fetches.  A repository whose fetch was
    /// skippable keeps the time of the fetch that made it so.
    pub(crate) fn finish(self, path: &Path, run: &RunSummary<'_>) -> Result<(), String> {
        let Self {
            mut state,
            explicit_since,
            covers,
            git_skippable,
            ..
        } = self;
        let started_at = format_utc(run.started_at);
        let watermark = format_utc(minus(run.started_at, WATERMARK_MARGIN));
        let trusted: &[String] = if explicit_since.is_some() {
            &[]
        } else {
            run.clean_repos
        };
        state.record_run(&RunRecord {
            started_at: &started_at,
            watermark: &watermark,
            categories: covers,
            clean_repos: trusted,
            repos_backed_up: run.repos_backed_up,
            fully_successful: run.fully_successful,
            tool_version: env!("CARGO_PKG_VERSION"),
        });
        let git_skippable = git_skippable
            .into_inner()
            .unwrap_or_else(|p| p.into_inner());
        for (name, skippable) in git_skippable {
            if skippable {
                continue;
            }
            if let Some(w) = state.repos.get_mut(&name) {
                w.fetched_at = trusted.contains(&name).then(|| watermark.clone());
            }
        }
        state.save(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> BackupOptions {
        BackupOptions {
            issue_comments: true,
            ..Default::default()
        }
    }

    fn repository(name: &str, pushed_at: Option<&str>) -> Repository {
        serde_json::from_value(serde_json::json!({
            "id": 1, "name": name, "full_name": name,
            "owner": {"id": 1, "login": "o", "type": "User", "avatar_url": "", "html_url": ""},
            "private": false, "fork": false,
            "clone_url": "", "ssh_url": "", "html_url": "",
            "pushed_at": pushed_at,
        }))
        .expect("repository")
    }

    impl Incremental {
        /// The per-item watermark of `name`.
        fn since_for(&self, name: &str) -> Option<String> {
            self.for_repo(&repository(name, None)).items
        }
    }

    /// Repositories and issues, as a nightly run would back them up.
    fn git_opts() -> BackupOptions {
        BackupOptions {
            repositories: true,
            issues: true,
            pulls: true,
            ..Default::default()
        }
    }

    const PUSHED_LONG_AGO: &str = "2020-01-01T00:00:00Z";

    /// Runs `opts` for `o/a` (clean or not), starting `hours_ago`, asking
    /// about it with `pushed_at`; returns what was answered.
    fn one_run(
        path: &Path,
        opts: &BackupOptions,
        hours_ago: i64,
        pushed_at: &str,
        clean: bool,
    ) -> RepoSince {
        let inc = Incremental::load(path, opts);
        let since = inc.for_repo(&repository("o/a", Some(pushed_at)));
        let clean: Vec<String> = if clean { vec!["o/a".into()] } else { vec![] };
        inc.finish(
            path,
            &RunSummary {
                started_at: minus(Timestamp::now(), SignedDuration::from_hours(hours_ago)),
                clean_repos: &clean,
                repos_backed_up: clean.len() as u64,
                fully_successful: !clean.is_empty(),
            },
        )
        .expect("save");
        since
    }

    #[test]
    fn an_unchanged_repository_after_a_clean_fetch_may_skip_its_fetch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let first = one_run(&path, &git_opts(), 2, PUSHED_LONG_AGO, true);
        assert_eq!(first, RepoSince::default(), "nothing is known yet");

        let second = one_run(&path, &git_opts(), 1, PUSHED_LONG_AGO, true);
        assert!(second.git.is_some(), "{second:?}");
        assert!(second.issue_list.is_some() && second.pull_list.is_some());
        assert_eq!(second.git, second.issue_list, "both are the watermark");

        // The skipped fetch keeps the time of the fetch before it.
        let state = BackupState::load(&path).expect("load").expect("state");
        let w = &state.repos["o/a"];
        assert_ne!(w.fetched_at.as_deref(), Some(w.at.as_str()));
        assert!(one_run(&path, &git_opts(), 0, PUSHED_LONG_AGO, true)
            .git
            .is_some());
    }

    #[test]
    fn a_push_since_the_watermark_means_a_fetch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        one_run(&path, &git_opts(), 2, PUSHED_LONG_AGO, true);
        let just_now = format_utc(Timestamp::now());
        assert_eq!(one_run(&path, &git_opts(), 1, &just_now, true).git, None);
        // Without a `pushed_at` (an empty repository) nothing is assumed.
        let inc = Incremental::load(&path, &git_opts());
        assert_eq!(inc.for_repo(&repository("o/a", None)).git, None);
    }

    #[test]
    fn full_or_an_explicit_since_never_skips_a_fetch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        one_run(&path, &git_opts(), 2, PUSHED_LONG_AGO, true);
        let full = BackupOptions {
            full: true,
            ..git_opts()
        };
        assert_eq!(
            Incremental::load(&path, &full).for_repo(&repository("o/a", Some(PUSHED_LONG_AGO))),
            RepoSince::default()
        );
        let since = BackupOptions {
            since: Some("2026-01-01T00:00:00Z".into()),
            ..git_opts()
        };
        let answer =
            Incremental::load(&path, &since).for_repo(&repository("o/a", Some(PUSHED_LONG_AGO)));
        assert_eq!(answer.git, None, "--since asserts nothing about git");
        assert_eq!(answer.issue_list.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(answer.pull_list.as_deref(), Some("2026-01-01T00:00:00Z"));
    }

    /// A run that decided to fetch and then failed for the repository leaves
    /// it without a fetch time, so the next run fetches whatever `pushed_at`
    /// says.
    #[test]
    fn a_failed_run_for_the_repository_means_a_fetch_next_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        one_run(&path, &git_opts(), 3, PUSHED_LONG_AGO, true);
        let full = BackupOptions {
            full: true,
            ..git_opts()
        };
        one_run(&path, &full, 2, PUSHED_LONG_AGO, false);
        assert_eq!(
            one_run(&path, &git_opts(), 1, PUSHED_LONG_AGO, true).git,
            None
        );
        assert!(one_run(&path, &git_opts(), 0, PUSHED_LONG_AGO, true)
            .git
            .is_some());
    }

    #[test]
    fn a_week_without_a_fetch_means_a_fetch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        one_run(&path, &git_opts(), 8 * 24, PUSHED_LONG_AGO, true);
        assert_eq!(
            one_run(&path, &git_opts(), 1, PUSHED_LONG_AGO, true).git,
            None
        );
    }

    #[test]
    fn another_git_mode_means_a_fetch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        one_run(&path, &git_opts(), 2, PUSHED_LONG_AGO, true);
        for other in [
            BackupOptions {
                clone_type: CloneType::Bare,
                ..git_opts()
            },
            BackupOptions {
                prune: true,
                ..git_opts()
            },
            BackupOptions {
                lfs: true,
                ..git_opts()
            },
        ] {
            let inc = Incremental::load(&path, &other);
            assert_eq!(
                inc.for_repo(&repository("o/a", Some(PUSHED_LONG_AGO))).git,
                None,
                "{other:?}"
            );
        }
    }

    /// A list is vouched for only by a watermark of a run that stored it.
    #[test]
    fn a_list_the_last_run_did_not_store_is_fetched_in_full() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let without_lists = BackupOptions {
            issues: false,
            pulls: false,
            ..git_opts()
        };
        one_run(&path, &without_lists, 2, PUSHED_LONG_AGO, true);
        let next = one_run(&path, &git_opts(), 1, PUSHED_LONG_AGO, true);
        assert_eq!((next.issue_list, next.pull_list), (None, None));
        assert!(next.git.is_some(), "git was captured");
    }

    fn at(h: u32, m: u32) -> Timestamp {
        format!("2026-03-01T{h:02}:{m:02}:00Z")
            .parse()
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
                    started_at: at(0, 5),
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
