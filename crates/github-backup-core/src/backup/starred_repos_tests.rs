// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Unit tests for [`super::backup_starred_repos`].

use super::*;
use crate::backup::mock_client::MockBackupClient;
use crate::git::test_support::SpyGitRunner;
use github_backup_types::config::{BackupOptions, CloneType};
use tempfile::TempDir;

fn clone_opts() -> CloneOptions {
    CloneOptions::unauthenticated()
}

fn starred_opts() -> BackupOptions {
    BackupOptions {
        clone_starred: true,
        clone_type: CloneType::Mirror,
        ..Default::default()
    }
}

#[tokio::test]
async fn disabled_flag_is_noop() {
    let client = MockBackupClient::new();
    let git = SpyGitRunner::default();
    let dir = TempDir::new().unwrap();
    let opts = BackupOptions::default(); // clone_starred = false

    backup_starred_repos(
        &client,
        &git,
        "octocat",
        &opts,
        dir.path(),
        &dir.path().join("queue.json"),
        &clone_opts(),
    )
    .await
    .expect("noop");

    assert_eq!(
        git.recorded_calls().len(),
        0,
        "no git calls for disabled flag"
    );
}

#[tokio::test]
async fn dry_run_is_noop() {
    let client = MockBackupClient::new();
    let git = SpyGitRunner::default();
    let dir = TempDir::new().unwrap();
    let opts = BackupOptions {
        clone_starred: true,
        dry_run: true,
        ..Default::default()
    };

    backup_starred_repos(
        &client,
        &git,
        "octocat",
        &opts,
        dir.path(),
        &dir.path().join("queue.json"),
        &clone_opts(),
    )
    .await
    .expect("dry_run noop");

    assert_eq!(git.recorded_calls().len(), 0, "dry-run must not clone");
}

#[tokio::test]
async fn empty_starred_list_writes_queue_with_no_items() {
    let client = MockBackupClient::new(); // default: empty starred list
    let git = SpyGitRunner::default();
    let dir = TempDir::new().unwrap();
    let queue_path = dir.path().join("queue.json");

    backup_starred_repos(
        &client,
        &git,
        "octocat",
        &starred_opts(),
        dir.path(),
        &queue_path,
        &clone_opts(),
    )
    .await
    .expect("empty list");

    assert_eq!(git.recorded_calls().len(), 0, "nothing to clone");
}

#[tokio::test]
async fn repos_are_cloned_into_subdirectory() {
    use github_backup_types::user::User;
    use github_backup_types::Repository;

    let repo = Repository {
        id: 1,
        full_name: "rust-lang/rust".to_string(),
        name: "rust".to_string(),
        owner: User {
            id: 42,
            login: "rust-lang".to_string(),
            user_type: "Organization".to_string(),
            avatar_url: String::new(),
            html_url: String::new(),
        },
        private: false,
        fork: false,
        archived: false,
        disabled: false,
        description: None,
        clone_url: "https://github.com/rust-lang/rust.git".to_string(),
        ssh_url: "git@github.com:rust-lang/rust.git".to_string(),
        default_branch: "master".to_string(),
        size: 500_000,
        has_issues: true,
        has_wiki: true,
        created_at: "2010-01-01T00:00:00Z".to_string(),
        pushed_at: None,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        html_url: "https://github.com/rust-lang/rust".to_string(),
    };

    let client = MockBackupClient::new().with_starred(vec![repo]);
    let git = SpyGitRunner::default();
    let dir = TempDir::new().unwrap();
    let queue_path = dir.path().join("queue.json");

    backup_starred_repos(
        &client,
        &git,
        "octocat",
        &starred_opts(),
        dir.path(),
        &queue_path,
        &clone_opts(),
    )
    .await
    .expect("clone starred");

    let calls = git.recorded_calls();
    assert_eq!(calls.len(), 1, "expected one git call");
    assert_eq!(calls[0].method, "mirror_clone");

    // Verify queue persisted with Done state.
    assert!(queue_path.exists(), "queue file should be written");
    let queue =
        starred_queue::load_or_create(&queue_path, "octocat", &[]).expect("load saved queue");
    assert_eq!(queue.items.len(), 1);
    assert_eq!(queue.items[0].state, CloneState::Done);
}

#[tokio::test]
async fn second_run_skips_done_items() {
    use github_backup_types::starred_queue::StarredQueueItem;
    use github_backup_types::user::User;
    use github_backup_types::Repository;

    let repo = Repository {
        id: 999,
        full_name: "octocat/hello".to_string(),
        name: "hello".to_string(),
        owner: User {
            id: 1,
            login: "octocat".to_string(),
            user_type: "User".to_string(),
            avatar_url: String::new(),
            html_url: String::new(),
        },
        private: false,
        fork: false,
        archived: false,
        disabled: false,
        description: None,
        clone_url: "https://github.com/octocat/hello.git".to_string(),
        ssh_url: "git@github.com:octocat/hello.git".to_string(),
        default_branch: "main".to_string(),
        size: 10,
        has_issues: true,
        has_wiki: false,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        pushed_at: None,
        updated_at: "2020-01-01T00:00:00Z".to_string(),
        html_url: "https://github.com/octocat/hello".to_string(),
    };

    let dir = TempDir::new().unwrap();
    let queue_path = dir.path().join("queue.json");

    // Pre-populate the queue with a Done entry for this repo.
    let mut pre_queue = github_backup_types::starred_queue::StarredCloneQueue {
        version: starred_queue::QUEUE_VERSION,
        owner: "octocat".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        items: vec![StarredQueueItem {
            id: 999,
            full_name: "octocat/hello".to_string(),
            clone_url: "https://github.com/octocat/hello.git".to_string(),
            ssh_url: "git@github.com:octocat/hello.git".to_string(),
            size_kb: 10,
            state: CloneState::Done,
            retries: 0,
            last_error: None,
            finished_at: Some("2026-01-01T00:00:00Z".to_string()),
        }],
    };
    starred_queue::save(&mut pre_queue, &queue_path).expect("pre-save");

    let client = MockBackupClient::new().with_starred(vec![repo]);
    let git = SpyGitRunner::default();

    backup_starred_repos(
        &client,
        &git,
        "octocat",
        &starred_opts(),
        dir.path(),
        &queue_path,
        &clone_opts(),
    )
    .await
    .expect("second run");

    assert_eq!(
        git.recorded_calls().len(),
        0,
        "Done item must not be re-cloned"
    );
}

#[test]
fn compute_rate_eta_zero_when_no_elapsed() {
    let (rate, eta) = compute_rate_eta(0, std::time::Duration::from_secs(0), 100);
    assert_eq!(rate, 0.0);
    assert!(eta.is_none());
}

#[test]
fn compute_rate_eta_reasonable_values() {
    // 10 repos in 60 seconds → 10 repo/min, 100 pending → ETA = 600s
    let (rate, eta) = compute_rate_eta(10, std::time::Duration::from_secs(60), 100);
    assert!((rate - 10.0).abs() < 0.01, "rate should be 10.0 repos/min");
    assert_eq!(eta, Some(600));
}
