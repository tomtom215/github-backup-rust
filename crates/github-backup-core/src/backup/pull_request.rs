// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request, review comment, commit, and review backup.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use tracing::{info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{
    backup::{
        all_json, describe_unparsed, json_str, json_u64,
        merge::{merge_and_write, reusable},
        number_after, Outcome,
    },
    error::CoreError,
    storage::Storage,
};

/// The most commits `GET /repos/{owner}/{repo}/pulls/{n}/commits` returns.
const PULL_COMMITS_CAP: usize = 250;

/// Backs up all pull requests (and optionally comments, commits, and reviews)
/// for a repository.
///
/// Writes:
/// - `meta_dir/pulls.json` – every PR, merged with what earlier runs stored
/// - `meta_dir/pull_comments/<number>.json` – review comments per PR
/// - `meta_dir/pull_commits/<number>.json` – commits per PR
/// - `meta_dir/pull_reviews/<number>.json` – reviews per PR
///
/// Like [`backup_issues`](super::issue::backup_issues), the PR list is
/// **always fetched in full** and merged into the existing `pulls.json`, so an
/// incremental run can never shrink the file; the per-PR files are merged
/// too (comments and reviews by `id`, commits by `sha`), so a review comment
/// deleted or a commit force-pushed away stays in the backup.  `since` is the
/// repository's incremental watermark; it only decides which PRs get their
/// comment, commit and review files re-fetched — those are skipped for PRs
/// not updated since the watermark and whose files are already on disk.  The
/// repository-wide list of review comments created or edited since the
/// watermark is asked for once, and every PR it names is re-fetched as well.
///
/// GitHub lists at most 250 commits of a pull request; when the list is that
/// long, the commits are taken from the comparison of the PR's base and head
/// commits instead, which is not capped.
///
/// A PR the typed model cannot read still gets its per-PR files by its
/// `number`; one without a `number` is recorded as a failure, as is any
/// per-PR list that cannot be fetched (the other PRs are still backed up).
///
/// Returns the number of pull requests fetched.
///
/// # Errors
///
/// Propagates [`CoreError`] from the PR listing or storage writes, and any
/// fatal error.
pub async fn backup_pull_requests(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    since: Option<&str>,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.pulls && !opts.pull_comments && !opts.pull_commits && !opts.pull_reviews {
        return Ok(outcome);
    }

    info!(owner, repo = repo_name, "fetching pull requests");
    // Never ask the API for a delta: the response is persisted wholesale.
    let pulls = client.list_pull_requests(owner, repo_name).await?;
    outcome.count = (pulls.len() + pulls.unparsed_count()) as u64;

    if opts.pulls {
        merge_and_write(storage, &meta_dir.join("pulls.json"), &pulls, "number")?;
    }

    if !opts.pull_comments && !opts.pull_commits && !opts.pull_reviews {
        return Ok(outcome);
    }

    let edited = edited_review_threads(client, owner, repo_name, opts, since, &mut outcome).await?;
    let since = if edited.is_some() { since } else { None };
    let edited = edited.unwrap_or_default();

    for (index, pr) in all_json(&pulls).enumerate() {
        let Some(number) = json_u64(pr, "number") else {
            outcome.fail(
                format!(
                    "pull request at position {index} ({})",
                    describe_unparsed(pr)
                ),
                "the API object has no `number`, so its comments, commits and reviews \
                 cannot be fetched",
            );
            continue;
        };
        let since = if edited.contains(&number) {
            None
        } else {
            since
        };
        backup_pull_files(
            client,
            owner,
            repo_name,
            opts,
            pr,
            number,
            since,
            meta_dir,
            storage,
            &mut outcome,
        )
        .await?;
    }

    Ok(outcome)
}

/// The numbers of the pull requests with a review comment created or edited
/// since the watermark; see `edited_comment_threads` in the issue backup.
async fn edited_review_threads(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    since: Option<&str>,
    outcome: &mut Outcome,
) -> Result<Option<BTreeSet<u64>>, CoreError> {
    let Some(since) = since.filter(|_| opts.pull_comments) else {
        return Ok(Some(BTreeSet::new()));
    };
    match client
        .list_repo_pull_comments_since(owner, repo_name, since)
        .await
    {
        Ok(comments) => Ok(Some(
            all_json(&comments)
                .filter_map(|c| json_str(c, "pull_request_url"))
                .filter_map(|url| number_after(url, "pulls"))
                .collect(),
        )),
        Err(e) => {
            warn!(owner, repo = repo_name, error = %e, "cannot list recently edited review comments; fetching every pull request's files");
            outcome.item_error("review comments edited since the last run", e)?;
            Ok(None)
        }
    }
}

/// Fetches (or keeps) the review comment, commit and review files of one PR.
#[allow(clippy::too_many_arguments)]
async fn backup_pull_files(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    pr: &Value,
    number: u64,
    since: Option<&str>,
    meta_dir: &Path,
    storage: &impl Storage,
    outcome: &mut Outcome,
) -> Result<(), CoreError> {
    let file = |dir: &str| meta_dir.join(dir).join(format!("{number}.json"));
    let comments_path = file("pull_comments");
    let commits_path = file("pull_commits");
    let reviews_path = file("pull_reviews");

    let mut wanted: Vec<&Path> = Vec::new();
    if opts.pull_comments {
        wanted.push(&comments_path);
    }
    if opts.pull_commits {
        wanted.push(&commits_path);
    }
    if opts.pull_reviews {
        wanted.push(&reviews_path);
    }
    let updated_at = json_str(pr, "updated_at").unwrap_or_default();
    if reusable(storage, updated_at, since, &wanted) {
        return Ok(());
    }

    if opts.pull_comments {
        match client.list_pull_comments(owner, repo_name, number).await {
            Ok(comments) => merge_and_write(storage, &comments_path, &comments, "id")?,
            Err(e) => outcome.item_error(format!("pull request #{number} review comments"), e)?,
        }
    }

    if opts.pull_commits {
        match client.list_pull_commits(owner, repo_name, number).await {
            Ok(commits) if commits.len() + commits.unparsed_count() >= PULL_COMMITS_CAP => {
                backup_long_pull_commits(
                    client,
                    owner,
                    repo_name,
                    pr,
                    number,
                    &commits_path,
                    storage,
                    outcome,
                )
                .await?;
            }
            Ok(commits) => merge_and_write(storage, &commits_path, &commits, "sha")?,
            Err(e) => outcome.item_error(format!("pull request #{number} commits"), e)?,
        }
    }

    if opts.pull_reviews {
        match client.list_pull_reviews(owner, repo_name, number).await {
            Ok(reviews) => merge_and_write(storage, &reviews_path, &reviews, "id")?,
            Err(e) => outcome.item_error(format!("pull request #{number} reviews"), e)?,
        }
    }
    Ok(())
}

/// The commits of a PR whose commit list hit GitHub's 250-commit cap: the
/// comparison of its base and head commits, which is not capped.  If that is
/// impossible the PR is recorded as a failure (and nothing is written, so a
/// later run tries again).
#[allow(clippy::too_many_arguments)]
async fn backup_long_pull_commits(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    pr: &Value,
    number: u64,
    commits_path: &Path,
    storage: &impl Storage,
    outcome: &mut Outcome,
) -> Result<(), CoreError> {
    let what = format!("pull request #{number} commits");
    let sha = |side: &str| pr.get(side).and_then(|s| json_str(s, "sha"));
    let (Some(base), Some(head)) = (sha("base"), sha("head")) else {
        outcome.fail(
            what,
            "GitHub lists at most 250 commits and the pull request has no base and head \
             commit to compare; the commit list would be incomplete",
        );
        return Ok(());
    };
    info!(
        owner,
        repo = repo_name,
        number,
        "pull request has 250 or more commits; listing them by comparing its base and head"
    );
    match client.compare_commits(owner, repo_name, base, head).await {
        Ok(commits) => merge_and_write(storage, commits_path, &commits, "sha"),
        Err(e) => {
            let e = CoreError::from(e);
            if e.is_fatal() {
                return Err(e);
            }
            outcome.fail(
                what,
                format!("GitHub lists at most 250 commits and comparing base and head failed: {e}"),
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::pull_request::{PullRequest, PullRequestRef};
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

    fn make_pr_ref() -> PullRequestRef {
        PullRequestRef {
            label: "octocat:main".to_string(),
            ref_name: "main".to_string(),
            sha: "abc123".to_string(),
            repo: None,
        }
    }

    fn make_pr(number: u64) -> PullRequest {
        PullRequest {
            id: number,
            number,
            title: format!("PR #{number}"),
            body: None,
            state: "open".to_string(),
            user: Some(make_user()),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            head: make_pr_ref(),
            base: make_pr_ref(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            merged_at: None,
            closed_at: None,
            html_url: format!("https://github.com/octocat/repo/pull/{number}"),
        }
    }

    #[tokio::test]
    async fn backup_pull_requests_all_flags_false_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_pull_requests_flag_true_writes_pulls_json() {
        let pr = make_pr(1);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pulls: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert!(storage.get(&PathBuf::from("/meta/pulls.json")).is_some());
    }

    #[tokio::test]
    async fn backup_pull_requests_comments_written_per_pr() {
        let pr = make_pr(5);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pull_comments: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert!(storage
            .get(&PathBuf::from("/meta/pull_comments/5.json"))
            .is_some());
    }

    #[tokio::test]
    async fn backup_pull_requests_commits_and_reviews_written_per_pr() {
        let pr = make_pr(3);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pull_commits: true,
            pull_reviews: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert!(storage
            .get(&PathBuf::from("/meta/pull_commits/3.json"))
            .is_some());
        assert!(storage
            .get(&PathBuf::from("/meta/pull_reviews/3.json"))
            .is_some());
    }

    #[tokio::test]
    async fn backup_pull_requests_only_pulls_no_per_pr_data() {
        let pr = make_pr(1);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        // Only pulls = true, all per-PR flags false
        let opts = BackupOptions {
            pulls: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        // Only pulls.json, no per-PR files
        assert_eq!(storage.len(), 1);
        assert!(storage.get(&PathBuf::from("/meta/pulls.json")).is_some());
    }

    fn pr_updated(number: u64, updated_at: &str) -> PullRequest {
        PullRequest {
            updated_at: updated_at.to_string(),
            ..make_pr(number)
        }
    }

    fn stored_numbers(storage: &MemStorage) -> Vec<u64> {
        let bytes = storage
            .get(&PathBuf::from("/meta/pulls.json"))
            .expect("pulls.json written");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
        value
            .as_array()
            .expect("array")
            .iter()
            .map(|p| p["number"].as_u64().expect("number"))
            .collect()
    }

    /// Same data-loss bug as issues: an automatic `since` made every later run
    /// overwrite `pulls.json` with only the changed pull requests.
    #[tokio::test]
    async fn incremental_run_never_shrinks_pulls_json() {
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pulls: true,
            ..Default::default()
        };
        let meta = PathBuf::from("/meta");

        let first = MockBackupClient::new().with_pull_requests(vec![
            pr_updated(1, "2024-01-01T00:00:00Z"),
            pr_updated(2, "2024-01-02T00:00:00Z"),
        ]);
        backup_pull_requests(&first, "o", "r", &opts, None, &meta, &storage)
            .await
            .expect("run 1");
        assert_eq!(stored_numbers(&storage), vec![1, 2]);

        let second =
            MockBackupClient::new().with_pull_requests(vec![pr_updated(2, "2024-06-01T00:00:00Z")]);
        backup_pull_requests(
            &second,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            &meta,
            &storage,
        )
        .await
        .expect("run 2");
        assert_eq!(stored_numbers(&storage), vec![1, 2]);

        let third = MockBackupClient::new();
        backup_pull_requests(
            &third,
            "o",
            "r",
            &opts,
            Some("2024-07-01T00:00:00Z"),
            &meta,
            &storage,
        )
        .await
        .expect("run 3");
        assert_eq!(
            stored_numbers(&storage),
            vec![1, 2],
            "an empty listing must not erase the backup"
        );
    }

    #[tokio::test]
    async fn watermark_skips_unchanged_prs_whose_files_exist() {
        let storage = MemStorage::default();
        for dir in ["pull_comments", "pull_commits", "pull_reviews"] {
            storage
                .write_bytes(
                    &PathBuf::from(format!("/meta/{dir}/1.json")),
                    br#"["kept"]"#,
                )
                .expect("seed");
        }
        let client = MockBackupClient::new().with_pull_requests(vec![
            pr_updated(1, "2024-01-01T00:00:00Z"),
            pr_updated(2, "2024-06-01T00:00:00Z"),
        ]);
        let opts = BackupOptions {
            pull_comments: true,
            pull_commits: true,
            pull_reviews: true,
            ..Default::default()
        };
        backup_pull_requests(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");

        for dir in ["pull_comments", "pull_commits", "pull_reviews"] {
            assert_eq!(
                storage
                    .get(&PathBuf::from(format!("/meta/{dir}/1.json")))
                    .as_deref(),
                Some(br#"["kept"]"#.as_slice()),
                "{dir}/1.json: unchanged PR must be left alone"
            );
            assert!(
                storage
                    .get(&PathBuf::from(format!("/meta/{dir}/2.json")))
                    .is_some_and(|b| b != br#"["kept"]"#),
                "{dir}/2.json: changed PR must be fetched"
            );
        }
    }

    #[tokio::test]
    async fn unchanged_pr_with_a_missing_file_is_still_fetched() {
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from("/meta/pull_commits/1.json"), br#"["kept"]"#)
            .expect("seed");
        let client =
            MockBackupClient::new().with_pull_requests(vec![pr_updated(1, "2024-01-01T00:00:00Z")]);
        let opts = BackupOptions {
            pull_commits: true,
            pull_reviews: true,
            ..Default::default()
        };
        backup_pull_requests(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");

        assert!(storage
            .get(&PathBuf::from("/meta/pull_reviews/1.json"))
            .is_some());
    }

    fn commits(n: usize) -> Vec<serde_json::Value> {
        (0..n)
            .map(|i| serde_json::json!({"sha": format!("{i:040x}")}))
            .collect()
    }

    fn commit_opts() -> BackupOptions {
        BackupOptions {
            pull_commits: true,
            ..Default::default()
        }
    }

    /// GitHub lists at most 250 commits of a pull request; a longer one used
    /// to be stored truncated without a word.
    #[tokio::test]
    async fn a_pull_request_with_250_commits_is_listed_by_comparison() {
        let client = MockBackupClient::new()
            .with_pull_requests(vec![make_pr(1)])
            .with_raw("pull_commits", commits(250))
            .with_raw("compare_commits", commits(300));
        let storage = MemStorage::default();
        let outcome = backup_pull_requests(
            &client,
            "o",
            "r",
            &commit_opts(),
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("ok");

        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(client
            .calls()
            .contains(&"compare_commits:abc123...abc123".to_string()));
        let stored: serde_json::Value = serde_json::from_slice(
            &storage
                .get(&PathBuf::from("/meta/pull_commits/1.json"))
                .expect("written"),
        )
        .expect("json");
        assert_eq!(stored.as_array().map(Vec::len), Some(300));
    }

    #[tokio::test]
    async fn a_truncated_commit_list_that_cannot_be_completed_is_a_failure() {
        let client = MockBackupClient::new()
            .with_pull_requests(vec![make_pr(1)])
            .with_raw("pull_commits", commits(250))
            .with_error("compare_commits", 500);
        let storage = MemStorage::default();
        let outcome = backup_pull_requests(
            &client,
            "o",
            "r",
            &commit_opts(),
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("not fatal");
        assert_eq!(outcome.failed.len(), 1);
        assert!(outcome.failed[0].1.contains("250"), "{:?}", outcome.failed);
        assert!(storage
            .get(&PathBuf::from("/meta/pull_commits/1.json"))
            .is_none());
    }

    #[tokio::test]
    async fn a_failing_pull_request_is_recorded_and_the_rest_continue() {
        let client = MockBackupClient::new()
            .with_pull_requests(vec![make_pr(1), make_pr(2)])
            .with_error("pull_reviews:1", 502);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pull_reviews: true,
            pull_comments: true,
            ..Default::default()
        };
        let outcome = backup_pull_requests(
            &client,
            "o",
            "r",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("not fatal");
        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(whats, ["pull request #1 reviews"]);
        assert!(storage
            .get(&PathBuf::from("/meta/pull_reviews/2.json"))
            .is_some());
    }

    #[tokio::test]
    async fn an_edited_review_comment_refreshes_an_otherwise_unchanged_pull_request() {
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from("/meta/pull_comments/1.json"), b"[]")
            .expect("seed");
        let client = MockBackupClient::new()
            .with_pull_requests(vec![make_pr(1)])
            .with_raw(
                "repo_pull_comments",
                vec![serde_json::json!({
                    "id": 5, "pull_request_url": "https://api.github.com/repos/o/r/pulls/1"
                })],
            );
        let opts = BackupOptions {
            pull_comments: true,
            ..Default::default()
        };
        backup_pull_requests(
            &client,
            "o",
            "r",
            &opts,
            Some("2030-01-01T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("ok");
        assert!(client.calls().contains(&"pull_comments:1".to_string()));
    }
}
