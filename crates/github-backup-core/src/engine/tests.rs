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
    let git = SpyGitRunner::default();
    let client = MockBackupClient::new().with_user_repos(vec![repo("a"), repo("b")]);
    let engine = engine_with(
        client,
        MemStorage::default(),
        git.clone(),
        root.path(),
        opts(),
    );
    let mut cp = BackupCheckpoint {
        run_started_at: "2026-01-01T00:00:00Z".into(),
        last_updated_at: crate::time::now_utc(),
        fingerprint: engine.checkpoint_fingerprint(OWNER),
        ..Default::default()
    };
    cp.mark_complete_and_save("octocat/a", &out.backup_checkpoint_path(OWNER))
        .expect("seed checkpoint");

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
    storage
        .write_bytes(&comments, br#"["sentinel"]"#)
        .expect("mark");

    // Run 2: the issue is older than the watermark and its file exists → skipped.
    make(with_comments.clone()).run(OWNER).await.expect("run 2");
    assert_eq!(
        storage.get(&comments).as_deref(),
        Some(br#"["sentinel"]"#.as_slice()),
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
        Some(br#"["sentinel"]"#.as_slice()),
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

/// Regression (audit GS-13, e2e S9a): a checkpoint left by a crashed run made
/// the next run — hours later, on schedule — skip refreshing every repository
/// the crashed run had finished.
#[tokio::test]
async fn an_old_checkpoint_is_not_resumed_so_every_repository_is_refreshed() {
    let root = tempfile::tempdir().expect("tempdir");
    let out = OutputConfig::new(root.path());
    let day_ago = crate::time::format_utc(crate::time::minus(
        jiff::Timestamp::now(),
        jiff::SignedDuration::from_hours(24),
    ));
    let mut cp = BackupCheckpoint {
        run_started_at: day_ago.clone(),
        last_updated_at: day_ago,
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

    assert_eq!(git.recorded_calls().len(), 2, "a is refreshed too");
    assert_eq!(stats.repos_skipped(), 0);
}

/// A checkpoint from an older version has no activity time: never resumed.
#[tokio::test]
async fn a_checkpoint_without_an_activity_time_is_not_resumed() {
    let root = tempfile::tempdir().expect("tempdir");
    let out = OutputConfig::new(root.path());
    std::fs::create_dir_all(out.owner_json_dir(OWNER)).expect("dir");
    std::fs::write(
        out.backup_checkpoint_path(OWNER),
        r#"{"completed_repos":["octocat/a"],"run_started_at":""}"#,
    )
    .expect("legacy checkpoint");

    let git = SpyGitRunner::default();
    let client = MockBackupClient::new().with_user_repos(vec![repo("a")]);
    let engine = engine_with(
        client,
        MemStorage::default(),
        git.clone(),
        root.path(),
        opts(),
    );
    engine.run(OWNER).await.expect("run");
    assert_eq!(git.recorded_calls().len(), 1);
}

/// A repository name from the API becomes a directory name; one that could
/// leave the output tree must be refused, loudly, without any git or file work.
#[tokio::test]
async fn a_repository_named_like_a_path_is_refused_not_cloned() {
    let root = tempfile::tempdir().expect("tempdir");
    let git = SpyGitRunner::default();
    let client = MockBackupClient::new().with_user_repos(vec![repo("../../escape"), repo("fine")]);
    let engine = engine_with(
        client,
        MemStorage::default(),
        git.clone(),
        root.path(),
        opts(),
    );

    let stats = engine.run(OWNER).await.expect("run");

    assert_eq!(
        stats.repos_backed_up(),
        1,
        "the safe repository is still backed up"
    );
    assert_eq!(stats.failure_count(), 1, "{:?}", stats.failures());
    assert!(stats.failures()[0]
        .message
        .contains("not a safe directory name"));
    let urls: Vec<String> = git.recorded_calls().into_iter().map(|c| c.url).collect();
    assert_eq!(urls.len(), 1, "{urls:?}");
    assert!(urls[0].ends_with("/fine.git"));
}

/// Regression: cancelling while the repository listing is in flight (a server
/// that never answers) must end the run at once, not after the client timeout.
#[tokio::test]
async fn cancelling_interrupts_the_repository_listing_request() {
    let root = tempfile::tempdir().expect("tempdir");
    let client = MockBackupClient::new().with_repo_listing_that_never_answers();
    let engine = engine_with(
        client,
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        opts(),
    );
    let cancel = engine.cancel_handle();

    let run = tokio::spawn(async move { engine.run(OWNER).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    cancel.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), run)
        .await
        .expect("cancel must end the run promptly")
        .expect("no panic");
    assert!(matches!(result, Err(CoreError::Interrupted)), "{result:?}");
}

/// The git verification and stall limit chosen on the engine reach every git
/// command it runs (they used to be unreachable: always off / 600 s).
#[test]
fn git_settings_reach_the_clone_options() {
    let root = tempfile::tempdir().expect("tempdir");
    let engine = engine_with(
        MockBackupClient::new(),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        opts(),
    );
    let defaults = engine.make_clone_opts(CancelFlag::new());
    assert!(!defaults.run_fsck);
    assert_eq!(defaults.stall_timeout_secs, 600);

    let engine = engine.with_git_fsck(true).with_git_stall_timeout(30);
    let chosen = engine.make_clone_opts(CancelFlag::new());
    assert!(chosen.run_fsck);
    assert_eq!(chosen.stall_timeout_secs, 30);
    assert_eq!(
        engine
            .with_git_stall_timeout(0)
            .make_clone_opts(CancelFlag::new())
            .stall_timeout_secs,
        1,
        "a zero limit would stop every git at once"
    );
}

// ── Checkpoint fingerprint, dry run, failures around the checkpoint ──────────

fn seed_recent_checkpoint(root: &Path, fingerprint: String, done: &[&str]) -> PathBuf {
    let path = OutputConfig::new(root).backup_checkpoint_path(OWNER);
    let mut cp = BackupCheckpoint {
        run_started_at: crate::time::now_utc(),
        last_updated_at: crate::time::now_utc(),
        fingerprint,
        ..Default::default()
    };
    for name in done {
        cp.mark_complete_and_save(name, &path)
            .expect("seed checkpoint");
    }
    path
}

/// Regression: an interrupted narrow run (only `--repositories`) followed
/// within the resume window by a run with more categories skipped the
/// repositories the narrow run had finished, so their issues were never
/// backed up, and the run reported success.
#[tokio::test]
async fn a_checkpoint_of_a_run_with_other_options_is_not_resumed() {
    let root = tempfile::tempdir().expect("tempdir");
    let narrow = BackupOptions {
        issues: false,
        labels: false,
        ..opts()
    };
    let narrow_engine = engine_with(
        MockBackupClient::new(),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        narrow,
    );
    seed_recent_checkpoint(
        root.path(),
        narrow_engine.checkpoint_fingerprint(OWNER),
        &["octocat/a"],
    );

    let storage = MemStorage::default();
    let git = SpyGitRunner::default();
    let client = MockBackupClient::new()
        .with_user_repos(vec![repo("a"), repo("b")])
        .with_issues(vec![issue(1, "2024-01-01T00:00:00Z")]);
    let engine = engine_with(client, storage.clone(), git.clone(), root.path(), opts());
    assert_ne!(
        engine.checkpoint_fingerprint(OWNER),
        narrow_engine.checkpoint_fingerprint(OWNER)
    );

    let stats = engine.run(OWNER).await.expect("run");

    assert_eq!(stats.repos_skipped(), 0);
    assert_eq!(git.recorded_calls().len(), 2, "a is backed up again");
    assert!(storage
        .get(&meta(root.path(), "a", "issues.json"))
        .is_some());

    // Concurrency does not change the work, so it does not change the print.
    let wider = engine_with(
        MockBackupClient::new(),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        BackupOptions {
            concurrency: 8,
            ..opts()
        },
    );
    assert_eq!(
        wider.checkpoint_fingerprint(OWNER),
        engine.checkpoint_fingerprint(OWNER)
    );
}

/// Regression: `--dry-run` deleted the checkpoint of an interrupted run.
#[tokio::test]
async fn a_dry_run_keeps_the_checkpoint() {
    let root = tempfile::tempdir().expect("tempdir");
    let dry = BackupOptions {
        dry_run: true,
        ..opts()
    };
    let engine = engine_with(
        MockBackupClient::new().with_user_repos(vec![repo("a")]),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        dry,
    );
    let path = seed_recent_checkpoint(root.path(), "anything".into(), &["octocat/a"]);
    let before = std::fs::read(&path).expect("read");

    engine.run(OWNER).await.expect("run");

    assert_eq!(std::fs::read(&path).expect("still there"), before);
}

/// A checkpoint that cannot be deleted at the end would make the next run
/// skip repositories: that is a failure of the run, not a log line.
#[tokio::test]
async fn a_checkpoint_that_cannot_be_deleted_is_a_failure() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = OutputConfig::new(root.path()).backup_checkpoint_path(OWNER);
    // A directory in its place cannot be removed as a file.
    std::fs::create_dir_all(path.join("blocker")).expect("mkdir");
    let engine = engine_with(
        MockBackupClient::new().with_user_repos(vec![repo("a")]),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        opts(),
    );

    let stats = engine.run(OWNER).await.expect("run");

    let failures = stats.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].step, "checkpoint");
}

/// Regression: temporary files left by a killed run stayed for ever (and went
/// into the manifest and the S3 upload).  A run sweeps them once it holds the
/// lock; a dry run touches nothing.
#[tokio::test]
async fn a_run_sweeps_temporary_files_left_by_a_killed_run() {
    let root = tempfile::tempdir().expect("tempdir");
    let json_dir = OutputConfig::new(root.path()).owner_json_dir(OWNER);
    std::fs::create_dir_all(json_dir.join("repos/a")).expect("mkdir");
    let leftover = json_dir.join("repos/a/.issues.json.tmp-999-1");
    std::fs::write(&leftover, b"partial").expect("write");

    let dry = BackupOptions {
        dry_run: true,
        ..opts()
    };
    engine_with(
        MockBackupClient::new().with_user_repos(vec![repo("a")]),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        dry,
    )
    .run(OWNER)
    .await
    .expect("dry run");
    assert!(leftover.exists(), "a dry run writes and deletes nothing");

    engine_with(
        MockBackupClient::new().with_user_repos(vec![repo("a")]),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        opts(),
    )
    .run(OWNER)
    .await
    .expect("run");
    assert!(!leftover.exists());
}

// ── Events ───────────────────────────────────────────────────────────────────

fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<EngineEvent>) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

/// Every `RepoStarted` is followed by a `RepoCompleted` for the same name.
fn assert_every_start_completes(events: &[EngineEvent]) {
    for event in events {
        if let EngineEvent::RepoStarted { name } = event {
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    EngineEvent::RepoCompleted { name: done, .. } if done == name
                )),
                "{name} started but never completed: {events:?}"
            );
        }
    }
}

/// Regression: filtered-out, dry-run and panicked repositories were reported
/// as started but never as completed, so the TUI showed them running for ever.
#[tokio::test]
async fn every_started_repository_is_reported_as_completed() {
    let root = tempfile::tempdir().expect("tempdir");
    let client = MockBackupClient::new().with_user_repos(vec![
        repo("ok"),
        repo_with("secret", true, true),
        repo("boom"),
    ]);
    let git = SpyGitRunner::default().failing_when_url_contains("/boom.git", SpyFailure::Panic);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let no_private = BackupOptions {
        private: false,
        ..opts()
    };
    let engine = engine_with(client, MemStorage::default(), git, root.path(), no_private)
        .with_event_channel(tx);

    let stats = engine.run(OWNER).await.expect("run");
    let events = drain(&mut rx);

    assert_every_start_completes(&events);
    assert!(
        !events.iter().any(|e| matches!(
            e,
            EngineEvent::RepoStarted { name } if name == "octocat/secret"
        )),
        "a filtered-out repository is not started: {events:?}"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        EngineEvent::RepoCompleted { name, success: false, error: Some(msg) }
            if name == "octocat/boom" && msg.contains("panicked")
    )));
    assert_eq!(stats.repos_errored(), 1);

    // A dry run starts nothing.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    engine_with(
        MockBackupClient::new().with_user_repos(vec![repo("a")]),
        MemStorage::default(),
        SpyGitRunner::default(),
        root.path(),
        BackupOptions {
            dry_run: true,
            ..opts()
        },
    )
    .with_event_channel(tx)
    .run(OWNER)
    .await
    .expect("dry run");
    let events = drain(&mut rx);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, EngineEvent::RepoStarted { .. })),
        "{events:?}"
    );
}

// ── Unparsed repository objects ──────────────────────────────────────────────

/// Regression: repository objects the typed model could not read were dropped
/// from repos.json and never cloned, silently, with exit status 0.  They are
/// now written verbatim and the run records a failure naming how many.
#[tokio::test]
async fn unreadable_repository_objects_are_kept_and_reported() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let good = serde_json::to_value(repo("a")).expect("json");
    let odd = serde_json::json!({"id": 99, "full_name": "octocat/odd", "name": null});
    let client = MockBackupClient::new().with_raw("user_repos", vec![good, odd.clone()]);
    let git = SpyGitRunner::default();
    let engine = engine_with(client, storage.clone(), git.clone(), root.path(), opts());

    let stats = engine.run(OWNER).await.expect("run");

    let failures = stats.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].step, "repository list");
    assert!(
        failures[0].message.starts_with("1 repository object"),
        "{}",
        failures[0].message
    );
    let path = OutputConfig::new(root.path())
        .owner_json_dir(OWNER)
        .join("repos.json");
    let written: Vec<serde_json::Value> =
        serde_json::from_slice(&storage.get(&path).expect("repos.json")).expect("parse");
    assert_eq!(written.len(), 2, "{written:?}");
    assert!(written.contains(&odd), "kept verbatim: {written:?}");
    assert_eq!(
        git.recorded_calls().len(),
        1,
        "only the readable one is cloned"
    );
}

// ── Repeat runs: list deltas and skipped fetches ─────────────────────────────

/// A nightly run: the clone, issues with comments, pull requests.
fn nightly() -> BackupOptions {
    BackupOptions {
        repositories: true,
        issues: true,
        issue_comments: true,
        pulls: true,
        concurrency: 1,
        ..Default::default()
    }
}

const LONG_AGO: &str = "2020-06-01T00:00:00Z";

fn pushed(at: &str) -> Repository {
    Repository {
        pushed_at: Some(at.to_string()),
        ..repo("a")
    }
}

/// What one run of repository `a` asked for: API calls (repository listing
/// left out) and git operations.
#[derive(Debug)]
struct Run {
    calls: Vec<String>,
    git: Vec<String>,
    stats: BackupStats,
}

impl Run {
    /// The API calls with the `since` instants left out.
    fn shape(&self) -> Vec<String> {
        self.calls
            .iter()
            .map(|c| match c.split_once(" since ") {
                Some((call, _)) => format!("{call} since"),
                None => c.clone(),
            })
            .map(|c| match c.split_once(':') {
                Some((endpoint, arg)) if arg.starts_with("20") => format!("{endpoint}:<since>"),
                _ => c,
            })
            .collect()
    }

    fn fetched(&self) -> bool {
        !self.git.is_empty()
    }
}

async fn nightly_run(
    root: &Path,
    storage: &MemStorage,
    opts: BackupOptions,
    client: MockBackupClient,
    git: SpyGitRunner,
) -> Run {
    let stats = engine_with(client.clone(), storage.clone(), git.clone(), root, opts)
        .run(OWNER)
        .await
        .expect("run");
    Run {
        calls: client
            .calls()
            .into_iter()
            .filter(|c| !c.starts_with("user_repos:"))
            .collect(),
        git: git.recorded_calls().into_iter().map(|c| c.method).collect(),
        stats,
    }
}

fn client_for(
    repo: Repository,
    issues: Vec<Issue>,
    pulls: Vec<serde_json::Value>,
) -> MockBackupClient {
    MockBackupClient::new()
        .with_user_repos(vec![repo])
        .with_issues(issues)
        .with_raw("pull_requests", pulls)
}

fn pull(number: u64, updated_at: &str) -> serde_json::Value {
    serde_json::json!({"number": number, "updated_at": updated_at})
}

/// The clone the spy git runner never makes.
fn make_clone(root: &Path) -> PathBuf {
    let clone = OutputConfig::new(root).repos_dir(OWNER).join("a.git");
    std::fs::create_dir_all(&clone).expect("clone dir");
    clone
}

/// The first nightly run of an unchanged repository, then the second.
async fn two_runs(root: &Path, storage: &MemStorage) -> Run {
    let client = || {
        client_for(
            pushed(LONG_AGO),
            vec![issue(1, LONG_AGO), issue(2, LONG_AGO)],
            vec![pull(3, LONG_AGO)],
        )
    };
    let first = nightly_run(root, storage, nightly(), client(), SpyGitRunner::default()).await;
    assert!(!first.stats.has_failures(), "{:?}", first.stats.failures());
    make_clone(root);
    nightly_run(root, storage, nightly(), client(), SpyGitRunner::default()).await
}

/// The request count of a repeat run (measured before and after this change):
/// the lists are asked only for what changed, and the clone is not fetched.
#[tokio::test]
async fn a_repeat_run_of_an_unchanged_repository_asks_only_for_changes() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let second = two_runs(root.path(), &storage).await;
    assert!(
        !second.stats.has_failures(),
        "{:?}",
        second.stats.failures()
    );
    assert_eq!(
        second.shape(),
        [
            "repository:a",
            // Did a pull request change (its refs are in the mirror)?  No.
            "pull_requests:a since",
            "issues:a since",
            "repo_issue_comments:<since>",
            "pull_requests:a since",
        ]
    );
    assert!(!second.fetched(), "nothing was pushed: {:?}", second.git);
    // The delta (empty) did not shrink the stored lists.
    let issues: serde_json::Value = serde_json::from_slice(
        &storage
            .get(&meta(root.path(), "a", "issues.json"))
            .expect("issues"),
    )
    .expect("json");
    assert_eq!(issues.as_array().map(Vec::len), Some(2));
    assert_eq!(second.stats.issues_fetched(), 2);
}

/// (a) Something changed: the changed issue arrives through the delta and is
/// merged; a push, or a pull request updated since, means a fetch.
#[tokio::test]
async fn a_changed_repository_is_fetched_and_its_changes_merged() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    two_runs(root.path(), &storage).await;
    let now = crate::time::now_utc();

    // A new issue and a push.
    let third = nightly_run(
        root.path(),
        &storage,
        nightly(),
        client_for(
            pushed(&now),
            vec![issue(1, LONG_AGO), issue(2, LONG_AGO), issue(9, &now)],
            vec![pull(3, LONG_AGO)],
        ),
        SpyGitRunner::default(),
    )
    .await;
    assert!(third.fetched(), "pushed since the last run");
    assert!(third.calls.contains(&"issue_comments:9".to_string()));
    assert!(!third.calls.contains(&"issue_comments:1".to_string()));
    let issues: serde_json::Value = serde_json::from_slice(
        &storage
            .get(&meta(root.path(), "a", "issues.json"))
            .expect("issues"),
    )
    .expect("json");
    let numbers: Vec<u64> = issues
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|i| i["number"].as_u64())
        .collect();
    assert_eq!(numbers, [1, 2, 9], "merged, never replaced by the delta");

    // No push, but a pull request was updated: its refs are in the mirror.
    let fourth = nightly_run(
        root.path(),
        &storage,
        nightly(),
        client_for(
            pushed(LONG_AGO),
            vec![],
            vec![pull(3, &crate::time::now_utc())],
        ),
        SpyGitRunner::default(),
    )
    .await;
    assert!(fourth.fetched(), "a pull request changed since");
}

/// A bare clone holds no `refs/pull/*`, so no pull request check is made.
#[tokio::test]
async fn a_bare_clone_needs_no_pull_request_check() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    let bare = BackupOptions {
        clone_type: github_backup_types::config::CloneType::Bare,
        pulls: false,
        ..nightly()
    };
    let client = || {
        client_for(
            pushed(LONG_AGO),
            vec![],
            vec![pull(3, &crate::time::now_utc())],
        )
    };
    nightly_run(
        root.path(),
        &storage,
        bare.clone(),
        client(),
        SpyGitRunner::default(),
    )
    .await;
    make_clone(root.path());
    let second = nightly_run(
        root.path(),
        &storage,
        bare,
        client(),
        SpyGitRunner::default(),
    )
    .await;
    assert!(!second.fetched());
    assert!(
        !second.calls.iter().any(|c| c.starts_with("pull_requests:")),
        "{:?}",
        second.calls
    );
}

/// (b) What the shortcut relies on is gone: a missing list is fetched in
/// full, a missing clone is cloned again.
#[tokio::test]
async fn a_missing_list_or_clone_is_fetched_in_full() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    two_runs(root.path(), &storage).await;
    storage
        .remove_file(&meta(root.path(), "a", "issues.json"))
        .expect("remove issues.json");
    storage
        .write_bytes(&meta(root.path(), "a", "pulls.json"), b"{ damaged")
        .expect("damage pulls.json");
    std::fs::remove_dir(make_clone(root.path())).expect("remove clone");

    let third = nightly_run(
        root.path(),
        &storage,
        nightly(),
        client_for(
            pushed(LONG_AGO),
            vec![issue(1, LONG_AGO)],
            vec![pull(3, LONG_AGO)],
        ),
        SpyGitRunner::default(),
    )
    .await;
    assert!(
        third.calls.contains(&"issues:a".to_string()),
        "{:?}",
        third.calls
    );
    assert!(
        third.calls.contains(&"pull_requests:a".to_string()),
        "{:?}",
        third.calls
    );
    assert!(third.fetched(), "the clone is gone");
}

/// (c) `--full` takes no shortcut.
#[tokio::test]
async fn full_takes_no_shortcut() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    two_runs(root.path(), &storage).await;
    let full = BackupOptions {
        full: true,
        ..nightly()
    };
    let third = nightly_run(
        root.path(),
        &storage,
        full,
        client_for(
            pushed(LONG_AGO),
            vec![issue(1, LONG_AGO)],
            vec![pull(3, LONG_AGO)],
        ),
        SpyGitRunner::default(),
    )
    .await;
    assert_eq!(
        third.shape(),
        [
            "repository:a",
            "issues:a",
            "issue_comments:1",
            "pull_requests:a",
        ]
    );
    assert!(third.fetched());
}

/// (d) A run that failed for the repository is not built on: a fetch that
/// failed is repeated although nothing was pushed, and what changed while the
/// issue listing failed is asked for again (since the last clean run).
#[tokio::test]
async fn a_failed_run_for_the_repository_is_not_built_on() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage = MemStorage::default();
    two_runs(root.path(), &storage).await;
    let now = crate::time::now_utc();

    // A `--full` run (so it fetches) whose fetch and issue listing fail.
    let failing = nightly_run(
        root.path(),
        &storage,
        BackupOptions {
            full: true,
            ..nightly()
        },
        client_for(pushed(LONG_AGO), vec![], vec![]).with_error("issues", 502),
        SpyGitRunner::default().failing_when_url_contains("a.git", git_failure(128, "boom")),
    )
    .await;
    assert!(failing.fetched() && failing.stats.has_failures());

    let retry = nightly_run(
        root.path(),
        &storage,
        nightly(),
        client_for(
            pushed(LONG_AGO),
            vec![issue(1, LONG_AGO), issue(7, &now)],
            vec![],
        ),
        SpyGitRunner::default(),
    )
    .await;
    assert!(!retry.stats.has_failures(), "{:?}", retry.stats.failures());
    assert!(retry.fetched(), "the failed fetch is repeated");
    assert!(retry.calls.contains(&"issue_comments:7".to_string()));
    let issues: serde_json::Value = serde_json::from_slice(
        &storage
            .get(&meta(root.path(), "a", "issues.json"))
            .expect("issues"),
    )
    .expect("json");
    assert!(issues
        .as_array()
        .expect("array")
        .iter()
        .any(|i| i["number"] == 7));
}
