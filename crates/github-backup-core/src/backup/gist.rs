// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Gist metadata, comment and git clone backup.

use std::path::Path;

use serde_json::Value;
use tracing::{info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{
    backup::{
        all_json, describe_unparsed, json_str, json_u64, merge::merge_and_write,
        repository::rewrite_host, Outcome,
    },
    error::CoreError,
    git::{CloneOptions, GitRunner},
    storage::Storage,
};

/// Backs up gists owned by `username` and optionally starred gists.
///
/// For each owned gist:
/// - Writes `gists_meta_dir/<id>.json` with gist metadata.
/// - Writes `gists_meta_dir/<id>.comments.json` with its comments (merged by
///   `id`, so a deleted comment stays), when the gist has any.
/// - Clones `gists_git_dir/<id>.git` as a bare mirror.
///
/// `index.json` lists every owned gist and accumulates (merged by `id`, so a
/// deleted gist stays listed); `starred_index.json` is a snapshot of the gists
/// starred now.  For each starred gist `<id>.starred.json` is written.
///
/// One gist that cannot be cloned or whose comments cannot be fetched
/// (deleted since listing, blocked, a transient network error) is reported in
/// [`Outcome::failed`] and does not stop the others.  A gist the typed model
/// cannot read is still cloned from the `id` and `git_pull_url` of its JSON;
/// an id that is not a safe file name is never used as one.
///
/// # Errors
///
/// Propagates [`CoreError`] from the gist listings and storage writes, and
/// any *fatal* error ([`CoreError::is_fatal`]: cancellation, full disk).
#[allow(clippy::too_many_arguments)]
pub async fn backup_gists(
    client: &impl BackupClient,
    username: &str,
    opts: &BackupOptions,
    gists_git_dir: &Path,
    gists_meta_dir: &Path,
    storage: &impl Storage,
    git: &impl GitRunner,
    clone_opts: &CloneOptions,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
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
        for gist in all_json(&gists) {
            let Some(id) = safe_id(gist, &mut outcome) else {
                continue;
            };
            storage.write_json(&gists_meta_dir.join(format!("{id}.json")), gist)?;

            // The listing says how many comments a gist has; ask only then.
            if json_u64(gist, "comments") != Some(0) {
                match client.list_gist_comments(id).await {
                    Ok(comments) => merge_and_write(
                        storage,
                        &gists_meta_dir.join(format!("{id}.comments.json")),
                        &comments,
                        "id",
                    )?,
                    Err(e) => outcome.item_error(format!("{id} comments"), e)?,
                }
            }

            let Some(pull_url) = json_str(gist, "git_pull_url") else {
                outcome.fail(id, "the API object has no `git_pull_url`");
                continue;
            };
            let dest = gists_git_dir.join(format!("{id}.git"));
            let url = match opts.clone_host {
                Some(ref host) => rewrite_host(pull_url, host),
                None => pull_url.to_string(),
            };
            match git.mirror_clone(&url, &dest, clone_opts).await {
                Ok(()) => outcome.count += 1,
                Err(e) if e.is_fatal() => return Err(e),
                Err(e) => {
                    warn!(gist = %id, error = %e, "gist clone failed; continuing with the others");
                    outcome.fail(id, e.to_string());
                }
            }
        }
        merge_and_write(storage, &gists_meta_dir.join("index.json"), &gists, "id")?;
    }

    if opts.starred_gists {
        // NOTE: /gists/starred returns gists starred by the *authenticated user*,
        // not the `username` argument being backed up. This is correct behaviour
        // for a backup tool but differs from other user-scoped calls.
        info!("fetching starred gists for authenticated user");
        let starred = client.list_starred_gists().await?;
        for gist in all_json(&starred) {
            let Some(id) = safe_id(gist, &mut outcome) else {
                continue;
            };
            storage.write_json(&gists_meta_dir.join(format!("{id}.starred.json")), gist)?;
            outcome.count += 1;
        }
        storage.write_json(&gists_meta_dir.join("starred_index.json"), &starred)?;
    }

    Ok(outcome)
}

/// The `id` of a gist if it can be used as a file name; otherwise the gist
/// is recorded as a failure.
fn safe_id<'g>(gist: &'g Value, outcome: &mut Outcome) -> Option<&'g str> {
    match json_str(gist, "id") {
        Some(id) if crate::paths::is_safe_component(id) => Some(id),
        Some(id) => {
            warn!(gist = %id, "gist id is not a safe file name; skipping it");
            outcome.fail(id, "unsafe gist id");
            None
        }
        None => {
            outcome.fail(
                format!("({})", describe_unparsed(gist)),
                "the API object has no `id`",
            );
            None
        }
    }
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
    ) -> Result<Outcome, CoreError> {
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

    /// Starred gist ids were used as file names without the check owned
    /// gists get.
    #[tokio::test]
    async fn an_unsafe_starred_gist_id_is_never_a_file_name() {
        let client = MockBackupClient::new().with_raw(
            "starred_gists",
            vec![
                serde_json::json!({"id": "../../escape"}),
                serde_json::json!({"id": "fine"}),
            ],
        );
        let storage = MemStorage::default();
        let opts = BackupOptions {
            starred_gists: true,
            ..Default::default()
        };
        let outcome = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &SpyGitRunner::default(),
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("not fatal");

        assert_eq!(outcome.failed.len(), 1);
        for path in storage.written_paths() {
            assert!(path.starts_with(META_DIR), "{path:?}");
            assert!(!path.to_string_lossy().contains(".."), "{path:?}");
        }
        assert!(storage
            .get(&PathBuf::from(format!("{META_DIR}/fine.starred.json")))
            .is_some());
    }

    #[tokio::test]
    async fn gist_comments_are_backed_up_when_there_are_any() {
        let client = MockBackupClient::new()
            .with_raw(
                "gists",
                vec![
                    serde_json::json!({"id": "a1", "git_pull_url": "https://gist.github.com/a1.git", "comments": 2}),
                    serde_json::json!({"id": "b2", "git_pull_url": "https://gist.github.com/b2.git", "comments": 0}),
                ],
            )
            .with_raw("gist_comments", vec![serde_json::json!({"id": 7, "body": "hi"})]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            gists: true,
            ..Default::default()
        };
        backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &SpyGitRunner::default(),
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("ok");

        assert!(storage
            .get(&PathBuf::from(format!("{META_DIR}/a1.comments.json")))
            .is_some());
        assert!(
            !client.calls().contains(&"gist_comments:b2".to_string()),
            "a gist without comments costs no request"
        );
    }
}
