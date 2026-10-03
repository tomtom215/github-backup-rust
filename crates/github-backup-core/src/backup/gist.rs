// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Gist metadata and git clone backup.

use std::path::Path;

use tracing::{info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{
    backup::repository::rewrite_host,
    error::CoreError,
    git::{CloneOptions, GitRunner},
    storage::Storage,
};

/// What [`backup_gists`] accomplished.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GistOutcome {
    /// Gists backed up (owned and starred).
    pub count: u64,
    /// Owned gists whose git data could not be backed up, as
    /// `(gist id, error)`.  The remaining gists were still processed.
    pub failed: Vec<(String, String)>,
}

/// Backs up gists owned by `username` and optionally starred gists.
///
/// For each owned gist:
/// - Writes `gists_meta_dir/<id>.json` with gist metadata.
/// - Clones `gists_git_dir/<id>.git` as a bare mirror.
///
/// One gist that cannot be cloned (deleted since listing, blocked, a transient
/// network error) is reported in [`GistOutcome::failed`] and does not stop the
/// others.
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls and storage writes, and any *fatal*
/// git error ([`CoreError::is_fatal`]: cancellation, full disk).
pub async fn backup_gists(
    client: &impl BackupClient,
    username: &str,
    opts: &BackupOptions,
    gists_git_dir: &Path,
    gists_meta_dir: &Path,
    storage: &impl Storage,
    git: &impl GitRunner,
    clone_opts: &CloneOptions,
) -> Result<GistOutcome, CoreError> {
    let mut outcome = GistOutcome::default();
    if !opts.gists && !opts.starred_gists {
        return Ok(outcome);
    }

    if opts.dry_run {
        info!(username, "dry-run: skipping gist backup");
        return Ok(outcome);
    }

    if opts.gists {
        info!(username, "fetching gists");
        let gists = client.list_gists(username).await?;
        for gist in &gists {
            let meta_path = gists_meta_dir.join(format!("{}.json", gist.id));
            storage.write_json(&meta_path, gist)?;

            let dest = gists_git_dir.join(format!("{}.git", gist.id));
            let rewritten;
            let gist_url: &str = if let Some(ref host) = opts.clone_host {
                rewritten = rewrite_host(&gist.git_pull_url, host);
                &rewritten
            } else {
                &gist.git_pull_url
            };
            match git.mirror_clone(gist_url, &dest, clone_opts).await {
                Ok(()) => outcome.count += 1,
                Err(e) if e.is_fatal() => return Err(e),
                Err(e) => {
                    warn!(gist = %gist.id, error = %e, "gist clone failed; continuing with the others");
                    outcome.failed.push((gist.id.clone(), e.to_string()));
                }
            }
        }
        storage.write_json(&gists_meta_dir.join("index.json"), &gists)?;
    }

    if opts.starred_gists {
        // NOTE: /gists/starred returns gists starred by the *authenticated user*,
        // not the `username` argument being backed up. This is correct behaviour
        // for a backup tool but differs from other user-scoped calls.
        info!("fetching starred gists for authenticated user");
        let starred = client.list_starred_gists().await?;
        for gist in &starred {
            let meta_path = gists_meta_dir.join(format!("{}.starred.json", gist.id));
            storage.write_json(&meta_path, gist)?;
            outcome.count += 1;
        }
        storage.write_json(&gists_meta_dir.join("starred_index.json"), &starred)?;
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::git::test_support::SpyGitRunner;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use std::path::PathBuf;

    const GIST_DIR: &str = "/git/gists";
    const META_DIR: &str = "/json/gists";

    #[tokio::test]
    async fn backup_gists_disabled_returns_zero_and_no_io() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions::default(); // gists = false, starred_gists = false

        let outcome = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert_eq!(outcome.count, 0);
        assert_eq!(git.recorded_calls().len(), 0, "no git calls expected");
        assert_eq!(storage.len(), 0, "no storage writes expected");
    }

    #[tokio::test]
    async fn backup_gists_empty_list_writes_index_only() {
        // MockBackupClient returns empty gists by default
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: true,
            ..Default::default()
        };

        let outcome = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert_eq!(outcome.count, 0);
        assert!(
            storage
                .get(&PathBuf::from(format!("{META_DIR}/index.json")))
                .is_some(),
            "index.json should be written even for empty list"
        );
        assert_eq!(git.recorded_calls().len(), 0);
    }

    #[tokio::test]
    async fn backup_starred_gists_disabled_skips() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: false,
            starred_gists: false,
            ..Default::default()
        };

        let outcome = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert_eq!(outcome.count, 0);
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_gists_dry_run_returns_zero_and_no_io() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: true,
            starred_gists: true,
            dry_run: true,
            ..Default::default()
        };

        let outcome = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists dry_run");

        assert_eq!(outcome.count, 0, "dry-run must return 0");
        assert_eq!(
            git.recorded_calls().len(),
            0,
            "dry-run must make no git calls"
        );
        assert_eq!(storage.len(), 0, "dry-run must write nothing");
    }

    #[tokio::test]
    async fn backup_starred_gists_only_writes_starred_index() {
        let client = MockBackupClient::new(); // starred_gists = empty
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: false,
            starred_gists: true,
            ..Default::default()
        };

        backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert!(
            storage
                .get(&PathBuf::from(format!("{META_DIR}/starred_index.json")))
                .is_some(),
            "starred_index.json should be written"
        );
        // No index.json when only starred_gists is enabled
        assert!(
            storage
                .get(&PathBuf::from(format!("{META_DIR}/index.json")))
                .is_none(),
            "index.json should not be written when only starred_gists is enabled"
        );
    }

    fn gist(id: &str) -> github_backup_types::Gist {
        github_backup_types::Gist {
            id: id.to_string(),
            description: None,
            public: true,
            owner: None,
            files: Default::default(),
            git_pull_url: format!("https://gist.github.com/{id}.git"),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: format!("https://gist.github.com/{id}"),
        }
    }

    async fn run(
        client: &MockBackupClient,
        git: &SpyGitRunner,
        storage: &MemStorage,
    ) -> Result<GistOutcome, CoreError> {
        let opts = BackupOptions {
            gists: true,
            ..Default::default()
        };
        backup_gists(
            client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            storage,
            git,
            &CloneOptions::unauthenticated(),
        )
        .await
    }

    /// One bad gist used to abort the loop, losing every gist after it.
    #[tokio::test]
    async fn one_failing_gist_does_not_stop_the_others() {
        use crate::git::spy::SpyFailure;
        let client =
            MockBackupClient::new().with_gists(vec![gist("aaa"), gist("bad"), gist("ccc")]);
        let git = SpyGitRunner::default().failing_when_url_contains(
            "/bad.git",
            SpyFailure::Git {
                code: 128,
                stderr: "fatal: repository not found".into(),
            },
        );
        let storage = MemStorage::default();

        let outcome = run(&client, &git, &storage).await.expect("not fatal");

        assert_eq!(outcome.count, 2, "aaa and ccc were backed up");
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].0, "bad");
        assert!(outcome.failed[0].1.contains("repository not found"));
        let cloned: Vec<String> = git.recorded_calls().into_iter().map(|c| c.url).collect();
        assert!(
            cloned.iter().any(|u| u.ends_with("/ccc.git")),
            "the gist after the bad one must still be cloned: {cloned:?}"
        );
        assert!(
            storage
                .get(&PathBuf::from(format!("{META_DIR}/index.json")))
                .is_some(),
            "the index is still written"
        );
    }

    #[tokio::test]
    async fn a_fatal_git_error_still_aborts_the_gist_backup() {
        use crate::git::spy::SpyFailure;
        let client = MockBackupClient::new().with_gists(vec![gist("aaa"), gist("bbb")]);
        let git = SpyGitRunner::default().failing_when_url_contains("", SpyFailure::Interrupted);
        let err = run(&client, &git, &MemStorage::default())
            .await
            .expect_err("cancellation must propagate");
        assert!(matches!(err, CoreError::Interrupted), "{err:?}");
        assert_eq!(
            git.recorded_calls().len(),
            1,
            "no further gist is attempted"
        );
    }
}
