// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! End-to-end tests that the backup files hold the API response, not a
//! re-serialised typed projection: [`MockBackupClient`] serves raw payloads
//! (taken from GitHub's own example responses), the backup modules write them
//! to [`MemStorage`], and the files are compared with what was served.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use github_backup_types::{
    config::BackupOptions, Branch, BranchCommit, BranchProtection, Hook, Raw, Repository,
};

use crate::backup::mock_client::MockBackupClient;
use crate::backup::{
    branches::backup_branches, hooks::backup_hooks, issue::backup_issues,
    pull_request::backup_pull_requests, release::backup_releases, repository::backup_repository,
};
use crate::git::{test_support::SpyGitRunner, CloneOptions};
use crate::storage::test_support::MemStorage;

const ISSUES: &str =
    include_str!("../../../github-backup-types/tests/fixtures/issues/example.json");
const ISSUES_MIXED: &str =
    include_str!("../../../github-backup-types/tests/fixtures/issues/mixed_with_garbage.json");
const ISSUE_COMMENTS: &str =
    include_str!("../../../github-backup-types/tests/fixtures/issue_comments/example.json");
const PULLS: &str = include_str!("../../../github-backup-types/tests/fixtures/pulls/example.json");
const RELEASES: &str =
    include_str!("../../../github-backup-types/tests/fixtures/releases/example.json");
const USER_REPOS: &str =
    include_str!("../../../github-backup-types/tests/fixtures/user_repos/example.json");

fn meta() -> PathBuf {
    PathBuf::from("/meta")
}

fn values(text: &str) -> Vec<Value> {
    match serde_json::from_str(text).expect("fixture json") {
        Value::Array(items) => items,
        other => panic!("fixture is not an array: {other}"),
    }
}

fn written(storage: &MemStorage, name: &str) -> Value {
    let bytes = storage
        .get(&meta().join(name))
        .unwrap_or_else(|| panic!("{name} was not written"));
    serde_json::from_slice(&bytes).expect("written file is JSON")
}

fn written_bytes(storage: &MemStorage, name: &str) -> Vec<u8> {
    storage
        .get(&meta().join(name))
        .unwrap_or_else(|| panic!("{name} was not written"))
}

#[tokio::test]
async fn issues_json_is_what_the_api_returned() {
    let client = MockBackupClient::new().with_raw("issues", values(ISSUES));
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
        &meta(),
        &storage,
    )
    .await
    .expect("backup_issues");

    let served: Value = serde_json::from_str(ISSUES).expect("fixture");
    assert_eq!(written(&storage, "issues.json"), served);
    // The typed `Issue` has no `reactions`, `node_id`, `author_association`...
    assert!(written(&storage, "issues.json")[0].get("node_id").is_some());
}

#[tokio::test]
async fn one_unparseable_issue_neither_aborts_nor_vanishes_and_the_rest_continue() {
    let client = MockBackupClient::new()
        .with_raw("issues", values(ISSUES_MIXED))
        .with_raw("issue_comments", values(ISSUE_COMMENTS));
    let storage = MemStorage::default();
    let opts = BackupOptions {
        issues: true,
        issue_comments: true,
        ..Default::default()
    };

    let count = backup_issues(
        &client,
        "octocat",
        "Hello-World",
        &opts,
        None,
        &meta(),
        &storage,
    )
    .await
    .expect("a bad element must not fail the category");

    assert_eq!(count, 2, "the two well-formed issues are counted");
    let issues = written(&storage, "issues.json");
    assert_eq!(
        issues.as_array().map(Vec::len),
        Some(5),
        "all five elements, including the three that do not parse, are on disk"
    );
    assert!(issues.as_array().expect("array").contains(&Value::Null));
    for number in [1347, 1348] {
        assert!(
            storage
                .get(&meta().join("issue_comments").join(format!("{number}.json")))
                .is_some(),
            "comments of the parseable issue #{number} are still fetched"
        );
    }
}

#[tokio::test]
async fn comments_are_stored_as_served() {
    let client = MockBackupClient::new()
        .with_raw("issues", values(ISSUES))
        .with_raw("issue_comments", values(ISSUE_COMMENTS));
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
        &meta(),
        &storage,
    )
    .await
    .expect("backup_issues");

    let served: Value = serde_json::from_str(ISSUE_COMMENTS).expect("fixture");
    assert_eq!(written(&storage, "issue_comments/1347.json"), served);
}

#[tokio::test]
async fn pulls_json_is_what_the_api_returned_and_invents_nothing() {
    let client = MockBackupClient::new().with_raw("pull_requests", values(PULLS));
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
        &meta(),
        &storage,
    )
    .await
    .expect("backup_pull_requests");

    let served: Value = serde_json::from_str(PULLS).expect("fixture");
    let on_disk = written(&storage, "pulls.json");
    assert_eq!(on_disk, served);
    for invented in [
        "merged",
        "commits",
        "changed_files",
        "additions",
        "deletions",
    ] {
        assert!(
            on_disk[0].get(invented).is_none(),
            "{invented} must not be invented"
        );
    }
    assert!(
        on_disk[0].get("_links").is_some(),
        "unmodelled `_links` is kept"
    );
}

#[tokio::test]
async fn releases_json_is_what_the_api_returned() {
    let client = MockBackupClient::new().with_raw("releases", values(RELEASES));
    let storage = MemStorage::default();
    let opts = BackupOptions {
        releases: true,
        ..Default::default()
    };

    backup_releases(&client, "octocat", "Hello-World", &opts, &meta(), &storage)
        .await
        .expect("backup_releases");

    let served: Value = serde_json::from_str(RELEASES).expect("fixture");
    assert_eq!(written(&storage, "releases.json"), served);
    assert!(written(&storage, "releases.json")[0]
        .get("target_commitish")
        .is_some());
}

#[tokio::test]
async fn info_json_is_the_full_repository_object() {
    let served = values(USER_REPOS).remove(0);
    let repo: Raw<Repository> = Raw::from_value(served.clone()).expect("repository parses");
    let storage = MemStorage::default();
    let opts = BackupOptions::default();

    backup_repository(
        &repo,
        &opts,
        &PathBuf::from("/git/repos"),
        &meta(),
        &storage,
        &SpyGitRunner::default(),
        &CloneOptions::unauthenticated(),
    )
    .await
    .expect("backup_repository");

    let on_disk = written(&storage, "info.json");
    assert_eq!(on_disk, served);
    for key in [
        "topics",
        "license",
        "visibility",
        "stargazers_count",
        "node_id",
    ] {
        assert!(
            on_disk.get(key).is_some() || served.get(key).is_none(),
            "{key} served by the API must be on disk"
        );
    }
    assert!(
        on_disk.as_object().expect("object").len() > 60,
        "all ~90 properties, not 19"
    );
}

fn branch(name: &str) -> Branch {
    Branch {
        name: name.to_string(),
        protected: true,
        commit: BranchCommit {
            sha: "abc".to_string(),
            url: "https://api.github.com/c/abc".to_string(),
        },
    }
}

fn protection(name: &str) -> BranchProtection {
    serde_json::from_value(serde_json::json!({
        "url": format!("https://api.github.com/repos/o/r/branches/{name}/protection"),
        "required_status_checks": {"contexts": ["ci"], "checks": []}
    }))
    .expect("protection without `strict` parses")
}

#[tokio::test]
async fn branch_protections_are_sorted_by_branch_and_identical_run_to_run() {
    let names = ["zeta", "alpha", "main", "release", "beta", "dev"];
    let mut protections = HashMap::new();
    for name in names {
        protections.insert(name.to_string(), protection(name));
    }
    let make = || {
        MockBackupClient::new()
            .with_branches(names.iter().map(|n| branch(n)).collect())
            .with_branch_protections(protections.clone())
    };
    let opts = BackupOptions {
        branches: true,
        ..Default::default()
    };

    let mut files = Vec::new();
    for _ in 0..2 {
        let storage = MemStorage::default();
        backup_branches(&make(), "o", "r", &opts, &meta(), &storage)
            .await
            .expect("backup_branches");
        files.push(written_bytes(&storage, "branch_protections.json"));
    }

    assert_eq!(files[0], files[1], "byte-identical across runs");
    let map: Value = serde_json::from_slice(&files[0]).expect("json");
    let keys: Vec<&str> = map
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["alpha", "beta", "dev", "main", "release", "zeta"]);
}

#[tokio::test]
async fn hooks_json_is_byte_identical_across_runs() {
    let make_hook = || {
        let mut config = serde_json::Map::new();
        for i in 0..10 {
            config.insert(format!("key-{i}"), Value::from(i));
        }
        Hook {
            id: 1,
            hook_type: "Repository".to_string(),
            name: "web".to_string(),
            active: true,
            events: vec!["push".to_string()],
            config,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    };
    let opts = BackupOptions {
        hooks: true,
        ..Default::default()
    };

    let mut files = Vec::new();
    for _ in 0..4 {
        let client = MockBackupClient::new().with_hooks(vec![make_hook()]);
        let storage = MemStorage::default();
        backup_hooks(&client, "o", "r", &opts, &meta(), &storage)
            .await
            .expect("backup_hooks");
        files.push(written_bytes(&storage, "hooks.json"));
    }

    assert!(
        files.windows(2).all(|w| w[0] == w[1]),
        "hooks.json must not vary between runs"
    );
}
