// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Orchestration tests: failure isolation, fatal errors, cancellation, dry-run
//! purity, resume and incremental state.
//!
//! They drive the real [`BackupEngine`] with the in-memory client, storage and
//! git doubles.  The lock, state and checkpoint files live in a temporary
//! directory because those are always real files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use super::*;
use crate::backup::mock_client::MockBackupClient;
use crate::git::spy::SpyFailure;
use crate::git::test_support::SpyGitRunner;
use crate::storage::test_support::MemStorage;
use github_backup_types::backup_state::BackupState;
use github_backup_types::issue::Issue;
use github_backup_types::user::User;

const OWNER: &str = "octocat";

fn user() -> User {
    User {
        id: 1,
        login: OWNER.to_string(),
        user_type: "User".to_string(),
        avatar_url: String::new(),
        html_url: String::new(),
    }
}

fn repo_with(name: &str, private: bool, has_issues: bool) -> Repository {
    Repository {
        id: name.bytes().map(u64::from).sum(),
        full_name: format!("{OWNER}/{name}"),
        name: name.to_string(),
        owner: user(),
        private,
        fork: false,
        archived: false,
        disabled: false,
        description: None,
        clone_url: format!("https://github.com/{OWNER}/{name}.git"),
        ssh_url: format!("git@github.com:{OWNER}/{name}.git"),
        default_branch: Some("main".to_string()),
        size: 0,
        has_issues,
        has_wiki: false,
        created_at: Some("2024-01-01T00:00:00Z".to_string()),
        pushed_at: None,
        updated_at: Some("2024-01-01T00:00:00Z".to_string()),
        html_url: format!("https://github.com/{OWNER}/{name}"),
    }
}

fn repo(name: &str) -> Repository {
    repo_with(name, false, true)
}

fn issue(number: u64, updated_at: &str) -> Issue {
    Issue {
        id: number,
        number,
        title: format!("Issue #{number}"),
        body: None,
        state: "open".to_string(),
        user: Some(user()),
        labels: vec![],
        assignees: vec![],
        milestone: None,
        pull_request: None,
        comments: 0,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: updated_at.to_string(),
        closed_at: None,
        html_url: format!("https://github.com/{OWNER}/x/issues/{number}"),
    }
}

/// Sequential, repositories + issues + labels.
fn opts() -> BackupOptions {
    BackupOptions {
        repositories: true,
        issues: true,
        labels: true,
        concurrency: 1,
        ..Default::default()
    }
}

fn engine_with<S>(
    client: MockBackupClient,
    storage: S,
    git: SpyGitRunner,
    root: &Path,
    opts: BackupOptions,
) -> BackupEngine<MockBackupClient, S, SpyGitRunner>
where
    S: Storage + Clone + 'static,
{
    BackupEngine::new(client, storage, git, OutputConfig::new(root), opts)
}

fn state(root: &Path) -> Option<BackupState> {
    BackupState::load(&OutputConfig::new(root).backup_state_path(OWNER)).expect("readable state")
}

fn meta(root: &Path, name: &str, file: &str) -> PathBuf {
    OutputConfig::new(root)
        .repo_meta_dir(OWNER, name)
        .join(file)
}

/// A storage that refuses to write any path containing one of `deny`.
#[derive(Debug, Clone)]
struct DenyingStorage {
    inner: MemStorage,
    deny: Arc<Vec<&'static str>>,
}

impl DenyingStorage {
    fn new(deny: &[&'static str]) -> Self {
        Self {
            inner: MemStorage::default(),
            deny: Arc::new(deny.to_vec()),
        }
    }
    fn check(&self, path: &Path) -> Result<(), CoreError> {
        let text = path.to_string_lossy();
        if self.deny.iter().any(|d| text.contains(d)) {
            return Err(CoreError::io(
                path.display(),
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            ));
        }
        Ok(())
    }
}

impl Storage for DenyingStorage {
    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), CoreError> {
        self.check(path)?;
        self.inner.write_json(path, value)
    }
    fn write_bytes(&self, path: &Path, data: &[u8]) -> Result<(), CoreError> {
        self.check(path)?;
        self.inner.write_bytes(path, data)
    }
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, CoreError> {
        self.inner.read(path)
    }
    fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path)
    }
}

fn git_failure(code: i32, stderr: &str) -> SpyFailure {
    SpyFailure::Git {
        code,
        stderr: stderr.to_string(),
    }
}

// ── Clean runs ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_clean_run_reports_no_failures_and_records_incremental_state() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let git = SpyGitRunner::default();
    let client = MockBackupClient::new().with_user_repos(vec![repo("a"), repo("b")]);
    let engine = engine_with(client, storage.clone(), git.clone(), root.path(), opts());

    let stats = engine.run(OWNER).await.expect("run");

    assert!(!stats.has_failures(), "{:?}", stats.failures());
    assert_eq!(stats.repos_backed_up(), 2);
    assert_eq!(stats.repos_errored(), 0);
    assert_eq!(
        git.recorded_calls().len(),
        2,
        "one mirror clone per repository"
    );

    let saved = state(root.path()).expect("state written after a run");
    assert!(saved.last_successful_run.is_some());
    assert_eq!(saved.repos_backed_up, 2);
    assert!(saved.repos.contains_key("octocat/a") && saved.repos.contains_key("octocat/b"));

    let out = OutputConfig::new(root.path());
    assert!(
        !out.backup_checkpoint_path(OWNER).exists(),
        "a finished run leaves no checkpoint"
    );
    let repos_json = storage
        .get(&out.owner_json_dir(OWNER).join("repos.json"))
        .expect("repos.json written");
    let listed: serde_json::Value = serde_json::from_slice(&repos_json).expect("json");
    assert_eq!(listed.as_array().expect("array").len(), 2);
}

#[tokio::test]
async fn repos_json_leaves_out_what_the_options_exclude() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let client = MockBackupClient::new()
        .with_user_repos(vec![repo("public"), repo_with("secret", true, true)]);
    // `private` is false: the private repository must not be backed up, and its
    // name and description must not leak into the backup directory either.
    let engine = engine_with(
        client,
        storage.clone(),
        SpyGitRunner::default(),
        root.path(),
        opts(),
    );

    let stats = engine.run(OWNER).await.expect("run");

    assert_eq!(stats.repos_backed_up(), 1);
    let text = String::from_utf8(
        storage
            .get(
                &OutputConfig::new(root.path())
                    .owner_json_dir(OWNER)
                    .join("repos.json"),
            )
            .expect("repos.json"),
    )
    .expect("utf8");
    assert!(text.contains("public"), "{text}");
    assert!(
        !text.contains("secret"),
        "excluded repo leaked into repos.json: {text}"
    );
}

// ── Failure isolation ────────────────────────────────────────────────────────

/// The audit's headline defect: a failed clone used to be logged and the run
/// still ended "successful".  Now it is a recorded failure, the rest of that
/// repository and every other repository still run, and the failed repository
/// is not given a watermark.
#[tokio::test]
async fn a_failed_clone_is_recorded_and_everything_else_still_runs() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let git = SpyGitRunner::default().failing_when_url_contains(
        "/bad.git",
        git_failure(128, "fatal: unable to access: Connection reset"),
    );
    let client = MockBackupClient::new()
        .with_user_repos(vec![repo("good"), repo("bad"), repo("other")])
        .with_issues(vec![issue(1, "2024-01-01T00:00:00Z")]);
    let engine = engine_with(client, storage.clone(), git.clone(), root.path(), opts());

    let stats = engine.run(OWNER).await.expect("not fatal, so Ok");

    assert!(stats.has_failures());
    assert_eq!(stats.failure_count(), 1, "{:?}", stats.failures());
    let failure = &stats.failures()[0];
    assert_eq!(failure.scope, "octocat/bad");
    assert_eq!(failure.step, "repository");
    assert!(
        failure.message.contains("Connection reset"),
        "{}",
        failure.message
    );

    assert_eq!(stats.repos_backed_up(), 2);
    assert_eq!(stats.repos_errored(), 1);
    assert_eq!(
        git.recorded_calls().len(),
        3,
        "the repo after the bad one is still cloned"
    );
    assert!(
        storage
            .get(&meta(root.path(), "bad", "issues.json"))
            .is_some(),
        "the failed clone must not cost the same repository's issues"
    );

    let saved = state(root.path()).expect("state written");
    assert!(saved.repos.contains_key("octocat/good"));
    assert!(saved.repos.contains_key("octocat/other"));
    assert!(
        !saved.repos.contains_key("octocat/bad"),
        "a repository with a failure must not advance its watermark"
    );
    assert_eq!(
        saved.last_successful_run, None,
        "a run with failures is not a successful run"
    );
}

#[tokio::test]
async fn a_failing_category_does_not_stop_the_other_categories() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = DenyingStorage::new(&["labels.json"]);
    let git = SpyGitRunner::default();
    let client = MockBackupClient::new()
        .with_user_repos(vec![repo("a")])
        .with_issues(vec![issue(1, "2024-01-01T00:00:00Z")]);
    let engine = engine_with(client, storage.clone(), git.clone(), root.path(), opts());

    let stats = engine.run(OWNER).await.expect("run");

    assert_eq!(stats.failure_count(), 1, "{:?}", stats.failures());
    assert_eq!(stats.failures()[0].step, "labels");
    assert_eq!(git.recorded_calls().len(), 1, "the clone still happened");
    assert!(
        storage
            .inner
            .get(&meta(root.path(), "a", "issues.json"))
            .is_some(),
        "issues were still backed up"
    );
    assert_eq!(stats.repos_errored(), 1);
    assert_eq!(
        stats.repos_backed_up(),
        0,
        "an incomplete repository is not 'backed up'"
    );
}

#[tokio::test]
async fn owner_level_failures_are_recorded_and_repositories_still_run() {
    let root = tempfile::tempdir().expect("tempdir");
    let git = SpyGitRunner::default()
        .failing_when_url_contains("gist.github.com/bad.git", git_failure(128, "fatal: boom"));
    let gist = |id: &str| github_backup_types::Gist {
        id: id.to_string(),
        description: None,
        public: true,
        owner: None,
        files: Default::default(),
        git_pull_url: format!("https://gist.github.com/{id}.git"),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        html_url: format!("https://gist.github.com/{id}"),
    };
    let client = MockBackupClient::new()
        .with_user_repos(vec![repo("a")])
        .with_gists(vec![gist("bad"), gist("fine")]);
    let opts = BackupOptions {
        gists: true,
        ..opts()
    };
    let engine = engine_with(client, MemStorage::default(), git, root.path(), opts);

    let stats = engine.run(OWNER).await.expect("run");

    assert_eq!(stats.gists_backed_up(), 1);
    assert_eq!(stats.failure_count(), 1, "{:?}", stats.failures());
    assert_eq!(stats.failures()[0].scope, OWNER);
    assert_eq!(stats.failures()[0].step, "gist bad");
    assert_eq!(
        stats.repos_backed_up(),
        1,
        "repositories were still processed"
    );
}

// ── Fatal errors, cancellation, resume ───────────────────────────────────────

#[tokio::test]
async fn a_fatal_error_stops_scheduling_and_is_returned() {
    let root = tempfile::tempdir().expect("tempdir");
    let git = SpyGitRunner::default().failing_when_url_contains("/b.git", SpyFailure::Interrupted);
    let client = MockBackupClient::new().with_user_repos(vec![repo("a"), repo("b"), repo("c")]);
    let engine = engine_with(
        client,
        MemStorage::default(),
        git.clone(),
        root.path(),
        opts(),
    );

    let err = engine.run(OWNER).await.expect_err("fatal must surface");

    assert!(matches!(err, CoreError::Interrupted), "{err:?}");
    let urls: Vec<String> = git.recorded_calls().into_iter().map(|c| c.url).collect();
    assert!(
        !urls.iter().any(|u| u.ends_with("/c.git")),
        "no repository may start after the fatal error: {urls:?}"
    );

    // The interrupted run keeps its checkpoint, with only the repository that
    // finished cleanly in it.
    let cp = BackupCheckpoint::load(&OutputConfig::new(root.path()).backup_checkpoint_path(OWNER))
        .expect("checkpoint");
    assert!(cp.is_complete("octocat/a"));
    assert!(
        !cp.is_complete("octocat/b"),
        "the interrupted repository is retried on resume"
    );
    assert!(
        state(root.path()).is_none(),
        "an aborted run writes no state"
    );
}

#[tokio::test]
async fn cancelling_before_the_run_does_no_work() {
    let root = tempfile::tempdir().expect("tempdir");
    let git = SpyGitRunner::default();
    let client = MockBackupClient::new().with_user_repos(vec![repo("a")]);
    let engine = engine_with(
        client,
        MemStorage::default(),
        git.clone(),
        root.path(),
        opts(),
    );
    engine.cancel_handle().cancel();

    let err = engine.run(OWNER).await.expect_err("cancelled");

    assert!(matches!(err, CoreError::Interrupted), "{err:?}");
    assert_eq!(git.recorded_calls().len(), 0);
}

#[tokio::test]
async fn resume_skips_repositories_the_interrupted_run_finished() {
    let root = tempfile::tempdir().expect("tempdir");
    let out = OutputConfig::new(root.path());
    let mut cp = BackupCheckpoint {
        run_started_at: "2026-01-01T00:00:00Z".into(),
        ..Default::default()
    };
    cp.mark_complete_and_save("octocat/a", &out.backup_checkpoint_path(OWNER))
        .expect("seed checkpoint");

    let git = SpyGitRunner::default();
    let client = MockBackupClient::new().with_user_repos(vec![repo("a"), repo("b")]);
    let engine = engine_with(
        client,
        MemStorage::default(),
        git.clone(),
        root.path(),
        opts(),
    );

    let stats = engine.run(OWNER).await.expect("run");

    let urls: Vec<String> = git.recorded_calls().into_iter().map(|c| c.url).collect();
    assert_eq!(urls.len(), 1, "{urls:?}");
    assert!(urls[0].ends_with("/b.git"));
    assert_eq!(stats.repos_skipped(), 1);
    assert_eq!(stats.repos_backed_up(), 1);
}

// ── Dry run ──────────────────────────────────────────────────────────────────

/// `--dry-run` must be inert: no lock directory, no state, no listing, no git.
#[tokio::test]
async fn a_dry_run_touches_nothing() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let git = SpyGitRunner::default();
    let client = MockBackupClient::new().with_user_repos(vec![repo("a"), repo("b")]);
    let dry = BackupOptions {
        dry_run: true,
        gists: true,
        starred: true,
        ..opts()
    };
    let engine = engine_with(client, storage.clone(), git.clone(), root.path(), dry);

    let stats = engine.run(OWNER).await.expect("run");

    assert_eq!(stats.repos_backed_up(), 0);
    assert_eq!(stats.repos_skipped(), 2);
    assert!(!stats.has_failures());
    assert_eq!(git.recorded_calls().len(), 0, "no git in a dry run");
    assert_eq!(storage.len(), 0, "no storage writes in a dry run");
    let on_disk: Vec<_> = std::fs::read_dir(root.path()).expect("read_dir").collect();
    assert!(
        on_disk.is_empty(),
        "a dry run must not create the lock, state or any directory: {on_disk:?}"
    );
}

// ── Issues disabled ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_repository_with_issues_switched_off_is_not_a_failure() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let client = MockBackupClient::new()
        .with_user_repos(vec![repo_with("noissues", false, false)])
        .with_issues(vec![issue(1, "2024-01-01T00:00:00Z")]);
    let engine = engine_with(
        client,
        storage.clone(),
        SpyGitRunner::default(),
        root.path(),
        opts(),
    );

    let stats = engine.run(OWNER).await.expect("run");

    assert!(!stats.has_failures());
    assert_eq!(stats.repos_backed_up(), 1);
    assert!(
        storage
            .get(&meta(root.path(), "noissues", "issues.json"))
            .is_none(),
        "the issues API answers 410 for such a repository; it must not be called"
    );
}

// ── Incremental, end to end ──────────────────────────────────────────────────

#[tokio::test]
async fn the_second_run_skips_unchanged_items_and_full_fetches_everything_again() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let comments = meta(root.path(), "a", "issue_comments/1.json");
    let make = |opts: BackupOptions| {
        let client = MockBackupClient::new()
            .with_user_repos(vec![repo("a")])
            .with_issues(vec![issue(1, "2020-06-01T00:00:00Z")]);
        engine_with(
            client,
            storage.clone(),
            SpyGitRunner::default(),
            root.path(),
            opts,
        )
    };
    let with_comments = BackupOptions {
        issue_comments: true,
        ..opts()
    };

    // Run 1: no watermark yet, so the comments file is fetched.
    make(with_comments.clone()).run(OWNER).await.expect("run 1");
    assert!(storage.get(&comments).is_some(), "run 1 fetches everything");

    // Mark the stored file so a refetch is visible.
    storage.write_bytes(&comments, b"sentinel").expect("mark");

    // Run 2: the issue is older than the watermark and its file exists → skipped.
    make(with_comments.clone()).run(OWNER).await.expect("run 2");
    assert_eq!(
        storage.get(&comments).as_deref(),
        Some(b"sentinel".as_slice()),
        "an unchanged issue must not be fetched again"
    );

    // Run 3 with --full: fetched again regardless of the watermark.
    let full = BackupOptions {
        full: true,
        ..with_comments
    };
    make(full).run(OWNER).await.expect("run 3");
    assert_ne!(
        storage.get(&comments).as_deref(),
        Some(b"sentinel".as_slice()),
        "--full must refetch"
    );
}

#[tokio::test]
async fn an_explicit_since_never_becomes_a_stored_watermark() {
    let root = tempfile::tempdir().expect("tempdir");
    let client = MockBackupClient::new().with_user_repos(vec![repo("a")]);
    let opts = BackupOptions {
        since: Some("2030-01-01T00:00:00Z".into()),
        ..opts()
    };
    let engine = engine_with(
        client,
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        opts,
    );

    engine.run(OWNER).await.expect("run");

    let saved = state(root.path()).expect("state");
    assert!(
        saved.repos.is_empty(),
        "an asserted --since must not poison later automatic runs: {:?}",
        saved.repos
    );
}
