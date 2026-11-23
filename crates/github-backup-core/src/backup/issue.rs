// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue, issue comment, and issue event backup.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use tracing::{info, warn};

use github_backup_client::{BackupClient, ClientError};
use github_backup_types::config::BackupOptions;

use crate::{
    backup::{
        all_json, describe_unparsed, json_str, json_u64,
        merge::{delta_base, merge_and_write, merge_values, reusable},
        number_after, Outcome,
    },
    error::CoreError,
    storage::Storage,
};

/// Backs up all issues (and optionally comments and events) for a repository.
///
/// Writes:
/// - `meta_dir/issues.json` – every issue, merged with what earlier runs
///   stored (see below)
/// - `meta_dir/issue_comments/<number>.json` – comments per issue
/// - `meta_dir/issue_events/<number>.json` – events per issue
///
/// The issue list is merged into the existing `issues.json`: an issue that
/// has since been deleted on GitHub stays in the backup, and an incremental
/// run can never shrink the file.  With `list_since` (the instant the stored
/// list is vouched for) and an intact `issues.json`, only the issues updated
/// since are asked for (`since=`), merged in, and the merged list stands for
/// every issue below; otherwise every issue is listed.  The comment and event
/// files are merged too (by comment / event `id`), so a comment deleted on
/// GitHub stays in the backup.
///
/// `since` is the repository's watermark for the per-item files; it decides
/// which issues get their comment and event files re-fetched — those are
/// skipped for issues not updated since the watermark and whose files are
/// already on disk.  Editing a comment does not change its issue's
/// `updated_at`, so the repository-wide list of comments created or edited
/// since the watermark is asked for once, and every issue it names is
/// re-fetched as well.  With `None` every issue is fetched.
///
/// Pull requests are issues too, so a pull request `N` gets its conversation
/// in `issue_comments/N.json` and its events in `issue_events/N.json` like any
/// issue does; its review comments, commits and reviews are in
/// `pull_comments/N.json`, `pull_commits/N.json` and `pull_reviews/N.json`.
/// When Issues are switched off for the repository (the issues API answers
/// `410 Gone`) the pull requests still have their conversation, so it is
/// fetched for every pull request the pulls API lists.
///
/// An issue the typed model cannot read is still written verbatim and gets
/// its comments and events by its `number`; one without a `number` is
/// recorded as a failure.  A comment or event list that cannot be fetched is
/// recorded in [`Outcome::failed`] and the remaining issues are still backed
/// up.
///
/// Returns the number of issues, **not** counting pull requests (those are
/// counted by the pull request backup): in the listing, or, after a delta
/// listing, in the merged list (which includes issues deleted on GitHub).
///
/// # Errors
///
/// Propagates [`CoreError`] from the issue listing or storage writes, and any
/// fatal error.
#[allow(clippy::too_many_arguments)]
pub async fn backup_issues(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    since: Option<&str>,
    list_since: Option<&str>,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.issues && !opts.issue_comments && !opts.issue_events {
        return Ok(outcome);
    }

    let path = meta_dir.join("issues.json");
    // A delta needs the stored list it is merged into.
    let base = if opts.issues {
        delta_base(storage, &path, list_since)?
    } else {
        None
    };
    let delta_since = base.as_ref().map(|(since, _)| since.as_str());
    info!(
        owner,
        repo = repo_name,
        since = delta_since,
        "fetching issues"
    );
    let items: Vec<Value> = match client.list_issues(owner, repo_name, delta_since).await {
        Ok(issues) => {
            let items = match base {
                Some((_, stored)) => {
                    let merged = merge_values(stored, &path, &issues, "number")?;
                    storage.write_json(&path, &merged)?;
                    merged
                }
                None => {
                    if opts.issues {
                        merge_and_write(storage, &path, &issues, "number")?;
                    }
                    all_json(&issues).cloned().collect()
                }
            };
            outcome.count = items
                .iter()
                .filter(|issue| issue.get("pull_request").is_none_or(Value::is_null))
                .count() as u64;
            items
        }
        // "410 Gone": Issues are switched off.  Pull requests keep their
        // conversation, which the same per-number endpoints serve.
        Err(ClientError::ApiError { status: 410, .. })
            if opts.issue_comments || opts.issue_events =>
        {
            info!(
                owner,
                repo = repo_name,
                "issues are disabled; fetching the conversation of the pull requests"
            );
            let pulls = client.list_pull_requests(owner, repo_name, None).await?;
            all_json(&pulls).cloned().collect()
        }
        Err(e) => return Err(e.into()),
    };

    if !opts.issue_comments && !opts.issue_events {
        return Ok(outcome);
    }

    let edited =
        edited_comment_threads(client, owner, repo_name, opts, since, &mut outcome).await?;
    // Without the list of edited comments, unchanged issues cannot be trusted.
    let since = if edited.is_some() { since } else { None };
    let edited = edited.unwrap_or_default();

    for (index, item) in items.iter().enumerate() {
        let Some(number) = json_u64(item, "number") else {
            outcome.fail(
                format!("issue at position {index} ({})", describe_unparsed(item)),
                "the API object has no `number`, so its comments and events cannot be fetched",
            );
            continue;
        };
        backup_issue_threads(
            client,
            owner,
            repo_name,
            opts,
            number,
            json_str(item, "updated_at").unwrap_or_default(),
            if edited.contains(&number) {
                None
            } else {
                since
            },
            meta_dir,
            storage,
            &mut outcome,
        )
        .await?;
    }

    Ok(outcome)
}

/// The numbers of the issues and pull requests with a conversation comment
/// created or edited since the watermark (`Some(empty)` without a watermark
/// or without `--issue-comments`).
///
/// `None` if the list could not be fetched: the failure is recorded and the
/// caller must then fetch every issue's files.
async fn edited_comment_threads(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    since: Option<&str>,
    outcome: &mut Outcome,
) -> Result<Option<BTreeSet<u64>>, CoreError> {
    let Some(since) = since.filter(|_| opts.issue_comments) else {
        return Ok(Some(BTreeSet::new()));
    };
    match client
        .list_repo_issue_comments_since(owner, repo_name, since)
        .await
    {
        Ok(comments) => Ok(Some(
            all_json(&comments)
                .filter_map(|c| json_str(c, "issue_url"))
                .filter_map(|url| number_after(url, "issues"))
                .collect(),
        )),
        Err(e) => {
            warn!(owner, repo = repo_name, error = %e, "cannot list recently edited comments; fetching every issue's comments");
            outcome.item_error("comments edited since the last run", e)?;
            Ok(None)
        }
    }
}

/// Fetches (or keeps) the comment and event files of issue `number`.
#[allow(clippy::too_many_arguments)]
async fn backup_issue_threads(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    number: u64,
    updated_at: &str,
    since: Option<&str>,
    meta_dir: &Path,
    storage: &impl Storage,
    outcome: &mut Outcome,
) -> Result<(), CoreError> {
    let comments_path = meta_dir
        .join("issue_comments")
        .join(format!("{number}.json"));
    let events_path = meta_dir.join("issue_events").join(format!("{number}.json"));

    // Unchanged since the last complete run and its files are still on disk:
    // what that run stored is still current.
    let mut wanted: Vec<&Path> = Vec::new();
    if opts.issue_comments {
        wanted.push(&comments_path);
    }
    if opts.issue_events {
        wanted.push(&events_path);
    }
    if reusable(storage, updated_at, since, &wanted) {
        return Ok(());
    }

    if opts.issue_comments {
        match client.list_issue_comments(owner, repo_name, number).await {
            Ok(comments) => merge_and_write(storage, &comments_path, &comments, "id")?,
            Err(e) => outcome.item_error(format!("issue #{number} comments"), e)?,
        }
    }

    if opts.issue_events {
        match client.list_issue_events(owner, repo_name, number).await {
            Ok(events) => merge_and_write(storage, &events_path, &events, "id")?,
            Err(e) => outcome.item_error(format!("issue #{number} events"), e)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::issue::{Issue, IssuePullRequestRef};
    use github_backup_types::user::User;
    use std::path::PathBuf;

    fn make_user() -> User {
        User {
            id: 1,
            login: "octocat".to_string(),
            user_type: "User".to_string(),
            avatar_url: String::new(),
            html_url: String::new(),
        }
    }

    fn make_issue(number: u64, is_pr: bool) -> Issue {
        Issue {
            id: number,
            number,
            title: format!("Issue #{number}"),
            body: None,
            state: "open".to_string(),
            user: Some(make_user()),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            pull_request: if is_pr {
                Some(IssuePullRequestRef {
                    url: Some(format!(
                        "https://api.github.com/repos/octocat/repo/pulls/{number}"
                    )),
                    html_url: Some(format!("https://github.com/octocat/repo/pull/{number}")),
                })
            } else {
                None
            },
            comments: 0,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            closed_at: None,
            html_url: format!("https://github.com/octocat/repo/issues/{number}"),
        }
    }

    #[tokio::test]
    async fn backup_issues_all_flags_false_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_issues_flag_true_writes_issues_json() {
        let issue = make_issue(1, false);
        let client = MockBackupClient::new().with_issues(vec![issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(storage.get(&PathBuf::from("/meta/issues.json")).is_some());
    }

    #[tokio::test]
    async fn backup_issues_comments_written_per_issue() {
        let issue = make_issue(42, false);
        let client = MockBackupClient::new().with_issues(vec![issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issue_comments: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(storage
            .get(&PathBuf::from("/meta/issue_comments/42.json"))
            .is_some());
        // issues.json not written since `issues` flag is false
        assert!(storage.get(&PathBuf::from("/meta/issues.json")).is_none());
    }

    #[tokio::test]
    async fn backup_issues_pull_requests_get_their_conversation_and_events() {
        // A pull request is an issue in the issues API and shares its number
        // space; its conversation thread lives in `issues/{n}/comments`.
        let pr_issue = make_issue(1, true);
        let real_issue = make_issue(2, false);
        let client = MockBackupClient::new().with_issues(vec![pr_issue, real_issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issue_comments: true,
            issue_events: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        for n in [1, 2] {
            for dir in ["issue_comments", "issue_events"] {
                assert!(
                    storage
                        .get(&PathBuf::from(format!("/meta/{dir}/{n}.json")))
                        .is_some(),
                    "{dir}/{n}.json must exist (#1 is a pull request, #2 an issue)"
                );
            }
        }
    }

    #[tokio::test]
    async fn backup_issues_events_flag_writes_events_json() {
        let issue = make_issue(7, false);
        let client = MockBackupClient::new().with_issues(vec![issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issue_events: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(storage
            .get(&PathBuf::from("/meta/issue_events/7.json"))
            .is_some());
    }

    fn issue_updated(number: u64, updated_at: &str) -> Issue {
        Issue {
            updated_at: updated_at.to_string(),
            ..make_issue(number, false)
        }
    }

    fn stored_numbers(storage: &MemStorage) -> Vec<u64> {
        let bytes = storage
            .get(&PathBuf::from("/meta/issues.json"))
            .expect("issues.json written");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
        value
            .as_array()
            .expect("array")
            .iter()
            .map(|i| i["number"].as_u64().expect("number"))
            .collect()
    }

    /// Regression for the data-loss bug: every run after the first used to
    /// overwrite `issues.json` with only the issues GitHub returned for the
    /// automatic `since` filter, so the file usually ended up as `[]`.
    #[tokio::test]
    async fn incremental_run_never_shrinks_issues_json() {
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };

        // Run 1: three issues.
        let first = MockBackupClient::new().with_issues(vec![
            issue_updated(1, "2024-01-01T00:00:00Z"),
            issue_updated(2, "2024-01-02T00:00:00Z"),
            issue_updated(3, "2024-01-03T00:00:00Z"),
        ]);
        backup_issues(
            &first,
            "o",
            "r",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("run 1");
        assert_eq!(stored_numbers(&storage), vec![1, 2, 3]);

        // Run 2 (incremental): only #2 changed.
        let second =
            MockBackupClient::new().with_issues(vec![issue_updated(2, "2024-06-01T00:00:00Z")]);
        backup_issues(
            &second,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("run 2");
        assert_eq!(
            stored_numbers(&storage),
            vec![1, 2, 3],
            "older issues must survive"
        );

        // Run 3: GitHub returns nothing at all (everything deleted / empty delta).
        let third = MockBackupClient::new();
        backup_issues(
            &third,
            "o",
            "r",
            &opts,
            Some("2024-07-01T00:00:00Z"),
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("run 3");
        assert_eq!(
            stored_numbers(&storage),
            vec![1, 2, 3],
            "an empty listing must not erase the backup"
        );
    }

    #[tokio::test]
    async fn changed_issue_replaces_its_stored_copy() {
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };
        let mut old = issue_updated(1, "2024-01-01T00:00:00Z");
        old.title = "before".into();
        backup_issues(
            &MockBackupClient::new().with_issues(vec![old]),
            "o",
            "r",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("first");

        let mut new = issue_updated(1, "2024-02-01T00:00:00Z");
        new.title = "after".into();
        backup_issues(
            &MockBackupClient::new().with_issues(vec![new]),
            "o",
            "r",
            &opts,
            Some("2024-01-15T00:00:00Z"),
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("second");

        let text =
            String::from_utf8(storage.get(&PathBuf::from("/meta/issues.json")).unwrap()).unwrap();
        assert!(text.contains("after") && !text.contains("before"), "{text}");
    }

    #[tokio::test]
    async fn watermark_skips_sub_resources_of_unchanged_issues_only() {
        let storage = MemStorage::default();
        // Files an earlier run left for issue 1 (sentinel content proves they
        // are neither refetched nor rewritten).
        for dir in ["issue_comments", "issue_events"] {
            storage
                .write_bytes(
                    &PathBuf::from(format!("/meta/{dir}/1.json")),
                    br#"["kept"]"#,
                )
                .expect("seed");
        }
        let client = MockBackupClient::new().with_issues(vec![
            issue_updated(1, "2024-01-01T00:00:00Z"), // before the watermark
            issue_updated(2, "2024-06-01T00:00:00Z"), // after the watermark
        ]);
        let opts = BackupOptions {
            issue_comments: true,
            issue_events: true,
            ..Default::default()
        };
        backup_issues(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");

        for dir in ["issue_comments", "issue_events"] {
            assert_eq!(
                storage
                    .get(&PathBuf::from(format!("/meta/{dir}/1.json")))
                    .as_deref(),
                Some(br#"["kept"]"#.as_slice()),
                "{dir}/1.json: unchanged issue must be left alone"
            );
            let two = storage
                .get(&PathBuf::from(format!("/meta/{dir}/2.json")))
                .unwrap_or_else(|| panic!("{dir}/2.json: changed issue must be fetched"));
            assert_ne!(two, br#"["kept"]"#);
        }
    }

    /// The shortcut must never leave a hole: an unchanged issue whose file is
    /// absent (hand-cleaned directory, category enabled later) is fetched.
    #[tokio::test]
    async fn unchanged_issue_with_a_missing_file_is_still_fetched() {
        let storage = MemStorage::default();
        // Comments exist from an earlier run; events were never fetched.
        storage
            .write_bytes(
                &PathBuf::from("/meta/issue_comments/1.json"),
                br#"["kept"]"#,
            )
            .expect("seed");
        let client =
            MockBackupClient::new().with_issues(vec![issue_updated(1, "2024-01-01T00:00:00Z")]);
        let opts = BackupOptions {
            issue_comments: true,
            issue_events: true,
            ..Default::default()
        };
        backup_issues(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");

        assert!(
            storage
                .get(&PathBuf::from("/meta/issue_events/1.json"))
                .is_some(),
            "the missing events file must be fetched"
        );
    }

    #[tokio::test]
    async fn without_a_watermark_every_issue_gets_its_sub_resources() {
        let storage = MemStorage::default();
        let client = MockBackupClient::new().with_issues(vec![
            issue_updated(1, "2020-01-01T00:00:00Z"),
            issue_updated(2, "2020-01-02T00:00:00Z"),
        ]);
        let opts = BackupOptions {
            issue_comments: true,
            ..Default::default()
        };
        backup_issues(
            &client,
            "o",
            "r",
            &opts,
            None,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");
        assert!(storage
            .get(&PathBuf::from("/meta/issue_comments/1.json"))
            .is_some());
        assert!(storage
            .get(&PathBuf::from("/meta/issue_comments/2.json"))
            .is_some());
    }

    fn all_threads() -> BackupOptions {
        BackupOptions {
            issues: true,
            issue_comments: true,
            issue_events: true,
            ..Default::default()
        }
    }

    fn meta() -> PathBuf {
        PathBuf::from("/meta")
    }

    /// With Issues switched off the issues API answers 410, but the pull
    /// requests still have their conversation; it used to be lost because the
    /// whole issue backup was skipped for such a repository.
    #[tokio::test]
    async fn issues_switched_off_still_back_up_pull_request_conversations() {
        let client = MockBackupClient::new().with_error("issues", 410).with_raw(
            "pull_requests",
            vec![serde_json::json!({"number": 5, "updated_at": "2024-01-01T00:00:00Z"})],
        );
        let storage = MemStorage::default();
        let outcome = backup_issues(
            &client,
            "o",
            "r",
            &all_threads(),
            None,
            None,
            &meta(),
            &storage,
        )
        .await
        .expect("410 is not a failure");
        assert!(outcome.failed.is_empty());
        assert_eq!(outcome.count, 0, "no issues");
        assert!(storage.get(&meta().join("issue_comments/5.json")).is_some());
        assert!(storage.get(&meta().join("issue_events/5.json")).is_some());
        assert!(storage.get(&meta().join("issues.json")).is_none());
    }

    #[tokio::test]
    async fn issues_switched_off_without_conversation_options_is_the_categorys_410() {
        let client = MockBackupClient::new().with_error("issues", 410);
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };
        let err = backup_issues(
            &client,
            "o",
            "r",
            &opts,
            None,
            None,
            &meta(),
            &MemStorage::default(),
        )
        .await
        .expect_err("the engine reads 410 as 'absent'");
        assert!(crate::backup::is_gone(&err));
    }

    /// One issue whose comments cannot be fetched used to abort every later
    /// issue of the repository.
    #[tokio::test]
    async fn a_failing_issue_is_recorded_and_the_rest_continue() {
        let client = MockBackupClient::new()
            .with_issues(vec![
                make_issue(1, false),
                make_issue(2, false),
                make_issue(3, false),
            ])
            .with_error("issue_comments:1", 500)
            .with_error("issue_events:2", 404);
        let storage = MemStorage::default();
        let outcome = backup_issues(
            &client,
            "o",
            "r",
            &all_threads(),
            None,
            None,
            &meta(),
            &storage,
        )
        .await
        .expect("not fatal");

        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(whats, ["issue #1 comments"], "404 means gone, not failed");
        for path in [
            "issue_events/1.json",
            "issue_comments/2.json",
            "issue_comments/3.json",
        ] {
            assert!(storage.get(&meta().join(path)).is_some(), "{path}");
        }
    }

    #[tokio::test]
    async fn a_fatal_error_still_stops_the_category() {
        let client = MockBackupClient::new()
            .with_issues(vec![make_issue(1, false)])
            .with_error("issue_comments:1", 401);
        let err = backup_issues(
            &client,
            "o",
            "r",
            &all_threads(),
            None,
            None,
            &meta(),
            &MemStorage::default(),
        )
        .await
        .expect_err("rejected credentials stop the run");
        assert!(err.is_fatal());
    }

    #[tokio::test]
    async fn pull_requests_are_not_counted_as_issues() {
        let client =
            MockBackupClient::new().with_issues(vec![make_issue(1, false), make_issue(2, true)]);
        let outcome = backup_issues(
            &client,
            "o",
            "r",
            &all_threads(),
            None,
            None,
            &meta(),
            &MemStorage::default(),
        )
        .await
        .expect("ok");
        assert_eq!(outcome.count, 1);
    }

    /// A comment deleted on GitHub stays in the per-issue file.
    #[tokio::test]
    async fn comment_files_accumulate() {
        let storage = MemStorage::default();
        storage
            .write_bytes(
                &meta().join("issue_comments/1.json"),
                br#"[{"id": 1, "body": "deleted upstream"}]"#,
            )
            .expect("seed");
        let client = MockBackupClient::new()
            .with_issues(vec![make_issue(1, false)])
            .with_raw(
                "issue_comments",
                vec![serde_json::json!({"id": 2, "body": "new"})],
            );
        backup_issues(
            &client,
            "o",
            "r",
            &all_threads(),
            None,
            None,
            &meta(),
            &storage,
        )
        .await
        .expect("ok");
        let on_disk: serde_json::Value =
            serde_json::from_slice(&storage.get(&meta().join("issue_comments/1.json")).unwrap())
                .unwrap();
        assert_eq!(
            on_disk,
            serde_json::json!([{"id": 1, "body": "deleted upstream"}, {"id": 2, "body": "new"}])
        );
    }

    /// Editing a comment does not change its issue's `updated_at`: the
    /// repository-wide list of comments edited since the watermark makes the
    /// issue's comments be fetched again.
    #[tokio::test]
    async fn an_edited_comment_refreshes_an_otherwise_unchanged_issue() {
        let storage = MemStorage::default();
        for n in [1, 2] {
            storage
                .write_bytes(&meta().join(format!("issue_comments/{n}.json")), b"[]")
                .expect("seed");
        }
        let client = MockBackupClient::new()
            .with_issues(vec![
                issue_updated(1, "2024-01-01T00:00:00Z"),
                issue_updated(2, "2024-01-01T00:00:00Z"),
            ])
            .with_raw(
                "repo_issue_comments",
                vec![serde_json::json!({
                    "id": 9, "issue_url": "https://api.github.com/repos/o/r/issues/2"
                })],
            );
        let opts = BackupOptions {
            issue_comments: true,
            ..Default::default()
        };
        backup_issues(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            None,
            &meta(),
            &storage,
        )
        .await
        .expect("ok");

        let calls = client.calls();
        assert!(calls.contains(&"repo_issue_comments:2024-03-01T00:00:00Z".to_string()));
        assert!(calls.contains(&"issue_comments:2".to_string()), "{calls:?}");
        assert!(
            !calls.contains(&"issue_comments:1".to_string()),
            "{calls:?}"
        );
    }

    #[tokio::test]
    async fn without_the_edited_comment_list_every_issue_is_fetched() {
        let storage = MemStorage::default();
        storage
            .write_bytes(&meta().join("issue_comments/1.json"), b"[]")
            .expect("seed");
        let client = MockBackupClient::new()
            .with_issues(vec![issue_updated(1, "2024-01-01T00:00:00Z")])
            .with_error("repo_issue_comments", 500);
        let opts = BackupOptions {
            issue_comments: true,
            ..Default::default()
        };
        let outcome = backup_issues(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            None,
            &meta(),
            &storage,
        )
        .await
        .expect("ok");
        assert_eq!(outcome.failed.len(), 1, "the failed listing is reported");
        assert!(client.calls().contains(&"issue_comments:1".to_string()));
    }
}
