// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! End-to-end integration tests for the backup pipeline.
//!
//! These tests exercise complete backup scenarios using [`MockBackupClient`]
//! and [`MemStorage`], verifying that the full chain from API call → JSON
//! serialisation → storage write works correctly without touching the network
//! or the real filesystem.

use std::collections::HashMap;
use std::path::PathBuf;

use github_backup_types::{
    config::{BackupOptions, BackupTarget},
    discussion::DiscussionCategory,
    package::PackageRepository,
    project::ProjectCard,
    pull_request::PullRequestRef,
    ClassicProject, Collaborator, DeployKey, Discussion, DiscussionComment, Hook, Issue,
    IssueComment, Label, Milestone, Package, PackageVersion, ProjectColumn, PullRequest, Release,
    SecurityAdvisory, Team, User,
};

use crate::backup::mock_client::MockBackupClient;
use crate::backup::{
    collaborators::backup_collaborators, deploy_keys::backup_deploy_keys,
    discussion::backup_discussions, hooks::backup_hooks, issue::backup_issues,
    labels::backup_labels, milestones::backup_milestones, package::backup_packages,
    project::backup_projects, pull_request::backup_pull_requests, release::backup_releases,
    security_advisories::backup_security_advisories, topics::backup_topics,
    user_data::backup_user_data,
};
use crate::storage::test_support::MemStorage;

// ── Shared test fixtures ───────────────────────────────────────────────────

const OWNER: &str = "octocat";
const REPO: &str = "Hello-World";

fn meta_dir() -> PathBuf {
    PathBuf::from(format!("/backup/{OWNER}/json/repos/{REPO}"))
}

fn owner_json_dir() -> PathBuf {
    PathBuf::from(format!("/backup/{OWNER}/json"))
}

fn make_user() -> User {
    User {
        id: 1,
        login: OWNER.to_string(),
        user_type: "User".to_string(),
        avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
        html_url: format!("https://github.com/{OWNER}"),
    }
}

fn make_pr_ref(branch: &str) -> PullRequestRef {
    PullRequestRef {
        label: format!("{OWNER}:{branch}"),
        ref_name: branch.to_string(),
        sha: "abc123".to_string(),
        repo: None,
    }
}

// ── Issues ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_issues_writes_issues_json_and_comments() {
    let issue = Issue {
        id: 1,
        number: 1,
        title: "Bug in foo".to_string(),
        body: Some("Details here".to_string()),
        state: "open".to_string(),
        user: make_user(),
        labels: vec![],
        assignees: vec![],
        comments: 1,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        closed_at: None,
        html_url: format!("https://github.com/{OWNER}/{REPO}/issues/1"),
        pull_request: None,
        milestone: None,
    };
    let comment = IssueComment {
        id: 10,
        user: make_user(),
        body: Some("Fixed!".to_string()),
        created_at: "2024-01-02T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
        html_url: format!("https://github.com/{OWNER}/{REPO}/issues/1#issuecomment-10"),
    };

    let client = MockBackupClient::new()
        .with_issues(vec![issue])
        .with_issue_comments(vec![comment]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        issues: true,
        issue_comments: true,
        ..Default::default()
    };

    let count = backup_issues(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_issues");

    assert_eq!(count, 1, "one issue fetched");
    let issues_path = meta_dir().join("issues.json");
    assert!(storage.get(&issues_path).is_some(), "issues.json written");

    // Comment file per issue number.
    let comment_path = meta_dir().join("issue_comments").join("1.json");
    assert!(storage.get(&comment_path).is_some(), "comment file written");
}

#[tokio::test]
async fn backup_issues_disabled_writes_nothing() {
    let client = MockBackupClient::new();
    let storage = MemStorage::default();
    let opts = BackupOptions {
        issues: false,
        ..Default::default()
    };

    backup_issues(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("noop");

    assert_eq!(storage.len(), 0, "nothing written when issues disabled");
}

// ── Pull Requests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_pull_requests_writes_prs_json() {
    let pr = PullRequest {
        id: 1,
        number: 42,
        title: "Add feature".to_string(),
        body: Some("Description".to_string()),
        state: "open".to_string(),
        merged: Some(false),
        user: make_user(),
        labels: vec![],
        assignees: vec![],
        commits: Some(1),
        additions: Some(10),
        deletions: Some(2),
        changed_files: Some(1),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        closed_at: None,
        merged_at: None,
        html_url: format!("https://github.com/{OWNER}/{REPO}/pull/42"),
        head: make_pr_ref("feature-branch"),
        base: make_pr_ref("main"),
        milestone: None,
    };

    let client = MockBackupClient::new().with_pull_requests(vec![pr]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        pulls: true,
        ..Default::default()
    };

    let count = backup_pull_requests(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_pull_requests");

    assert_eq!(count, 1);
    let prs_path = meta_dir().join("pulls.json");
    assert!(storage.get(&prs_path).is_some(), "pulls.json written");
}

// ── Labels ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_labels_writes_labels_json() {
    let label = Label {
        id: 1,
        name: "bug".to_string(),
        color: "ee0701".to_string(),
        description: Some("Something isn't working".to_string()),
        default: true,
    };

    let client = MockBackupClient::new().with_labels(vec![label]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        labels: true,
        ..Default::default()
    };

    backup_labels(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_labels");

    let labels_path = meta_dir().join("labels.json");
    assert!(storage.get(&labels_path).is_some(), "labels.json written");
}

// ── Milestones ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_milestones_writes_milestones_json() {
    let milestone = Milestone {
        id: 1,
        number: 1,
        title: "v1.0".to_string(),
        description: Some("First release".to_string()),
        state: "open".to_string(),
        creator: Some(make_user()),
        open_issues: 3,
        closed_issues: 10,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        due_on: None,
        closed_at: None,
    };

    let client = MockBackupClient::new().with_milestones(vec![milestone]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        milestones: true,
        ..Default::default()
    };

    backup_milestones(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_milestones");

    let path = meta_dir().join("milestones.json");
    assert!(storage.get(&path).is_some(), "milestones.json written");
}

// ── Releases ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_releases_writes_releases_json() {
    let release = Release {
        id: 1,
        tag_name: "v1.0.0".to_string(),
        name: Some("Version 1.0.0".to_string()),
        body: Some("Release notes".to_string()),
        draft: false,
        prerelease: false,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        published_at: Some("2024-01-02T00:00:00Z".to_string()),
        author: make_user(),
        html_url: format!("https://github.com/{OWNER}/{REPO}/releases/tag/v1.0.0"),
        assets: vec![],
        tarball_url: Some(format!(
            "https://github.com/{OWNER}/{REPO}/archive/v1.0.0.tar.gz"
        )),
        zipball_url: Some(format!(
            "https://github.com/{OWNER}/{REPO}/archive/v1.0.0.zip"
        )),
    };

    let client = MockBackupClient::new().with_releases(vec![release]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        releases: true,
        ..Default::default()
    };

    backup_releases(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_releases");

    let path = meta_dir().join("releases.json");
    assert!(storage.get(&path).is_some(), "releases.json written");
}

// ── Hooks ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_hooks_writes_hooks_json() {
    let mut config = HashMap::new();
    config.insert(
        "url".to_string(),
        serde_json::Value::String("https://example.com/webhook".to_string()),
    );

    let hook = Hook {
        id: 1,
        hook_type: "Repository".to_string(),
        name: "web".to_string(),
        active: true,
        events: vec!["push".to_string()],
        config,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let client = MockBackupClient::new().with_hooks(vec![hook]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        hooks: true,
        ..Default::default()
    };

    backup_hooks(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_hooks");

    let path = meta_dir().join("hooks.json");
    assert!(storage.get(&path).is_some(), "hooks.json written");
}

// ── Security advisories ────────────────────────────────────────────────────

#[tokio::test]
async fn backup_security_advisories_writes_json() {
    let advisory = SecurityAdvisory {
        ghsa_id: "GHSA-xxxx-yyyy-zzzz".to_string(),
        cve_id: None,
        summary: "Critical vulnerability".to_string(),
        description: Some("Details about the vulnerability".to_string()),
        severity: "critical".to_string(),
        state: "published".to_string(),
        vulnerabilities: vec![],
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        published_at: Some("2024-01-01T00:00:00Z".to_string()),
        html_url: format!(
            "https://github.com/{OWNER}/{REPO}/security/advisories/GHSA-xxxx-yyyy-zzzz"
        ),
    };

    let client = MockBackupClient::new().with_security_advisories(vec![advisory]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        security_advisories: true,
        ..Default::default()
    };

    backup_security_advisories(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_security_advisories");

    let path = meta_dir().join("security_advisories.json");
    assert!(
        storage.get(&path).is_some(),
        "security_advisories.json written"
    );
}

// ── Deploy keys ────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_deploy_keys_writes_json() {
    let key = DeployKey {
        id: 1,
        key: "ssh-rsa AAAAB3NzaC1yc2EAAAA...".to_string(),
        url: format!("https://api.github.com/repos/{OWNER}/{REPO}/keys/1"),
        title: "CI deploy key".to_string(),
        verified: true,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        read_only: true,
        added_by: Some(OWNER.to_string()),
        last_used: None,
    };

    let client = MockBackupClient::new().with_deploy_keys(vec![key]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        deploy_keys: true,
        ..Default::default()
    };

    backup_deploy_keys(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_deploy_keys");

    let path = meta_dir().join("deploy_keys.json");
    assert!(storage.get(&path).is_some(), "deploy_keys.json written");
}

// ── Collaborators ──────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_collaborators_writes_json() {
    use github_backup_types::collaborator::CollaboratorPermissions;

    let collab = Collaborator {
        id: 2,
        login: "collaborator-user".to_string(),
        user_type: "User".to_string(),
        avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
        html_url: "https://github.com/collaborator-user".to_string(),
        role_name: Some("write".to_string()),
        permissions: Some(CollaboratorPermissions {
            pull: true,
            triage: true,
            push: true,
            maintain: false,
            admin: false,
        }),
    };

    let client = MockBackupClient::new().with_collaborators(vec![collab]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        collaborators: true,
        ..Default::default()
    };

    backup_collaborators(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_collaborators");

    let path = meta_dir().join("collaborators.json");
    assert!(storage.get(&path).is_some(), "collaborators.json written");
}

// ── Topics ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_topics_writes_json() {
    let client =
        MockBackupClient::new().with_topics(vec!["rust".to_string(), "backup".to_string()]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        topics: true,
        ..Default::default()
    };

    backup_topics(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_topics");

    let path = meta_dir().join("topics.json");
    assert!(storage.get(&path).is_some(), "topics.json written");

    let bytes = storage.get(&path).unwrap();
    let content = String::from_utf8(bytes).unwrap();
    assert!(content.contains("rust"));
    assert!(content.contains("backup"));
}

// ── User data ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_user_data_writes_followers_and_following() {
    let follower = make_user();
    let followee = User {
        id: 99,
        login: "followee".to_string(),
        user_type: "User".to_string(),
        avatar_url: String::new(),
        html_url: "https://github.com/followee".to_string(),
    };

    let client = MockBackupClient::new()
        .with_followers(vec![follower])
        .with_following(vec![followee]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        followers: true,
        following: true,
        ..Default::default()
    };

    backup_user_data(&client, OWNER, &opts, &owner_json_dir(), &storage)
        .await
        .expect("backup_user_data");

    assert!(
        storage
            .get(&owner_json_dir().join("followers.json"))
            .is_some(),
        "followers.json written"
    );
    assert!(
        storage
            .get(&owner_json_dir().join("following.json"))
            .is_some(),
        "following.json written"
    );
}

#[tokio::test]
async fn backup_user_data_org_mode_writes_members_and_teams() {
    let member = make_user();
    let team = Team {
        id: 1,
        node_id: "T_1".to_string(),
        url: "https://api.github.com/teams/1".to_string(),
        html_url: "https://github.com/orgs/octocat/teams/dev-team".to_string(),
        name: "dev-team".to_string(),
        slug: "dev-team".to_string(),
        description: Some("Developers".to_string()),
        privacy: "closed".to_string(),
        notification_setting: None,
        permission: "push".to_string(),
        members_url: "https://api.github.com/teams/1/members{/member}".to_string(),
        repositories_url: "https://api.github.com/teams/1/repos".to_string(),
        parent: None,
    };

    let client = MockBackupClient::new()
        .with_org_members(vec![member])
        .with_org_teams(vec![team]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        org_members: true,
        org_teams: true,
        target: BackupTarget::Org,
        ..Default::default()
    };

    backup_user_data(&client, OWNER, &opts, &owner_json_dir(), &storage)
        .await
        .expect("backup_user_data org");

    assert!(
        storage
            .get(&owner_json_dir().join("org_members.json"))
            .is_some(),
        "org_members.json written"
    );
    assert!(
        storage
            .get(&owner_json_dir().join("org_teams.json"))
            .is_some(),
        "org_teams.json written"
    );
}

// ── Discussions ────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_discussions_writes_json_and_comments() {
    let discussion = Discussion {
        number: 1,
        title: "How to use?".to_string(),
        body: "Question body".to_string(),
        locked: false,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        html_url: format!("https://github.com/{OWNER}/{REPO}/discussions/1"),
        user: make_user(),
        comments: 1,
        category: Some(DiscussionCategory {
            id: 1,
            name: "Q&A".to_string(),
            description: String::new(),
            is_answerable: true,
        }),
        answered: false,
    };
    let comment = DiscussionComment {
        id: 100,
        body: "Here's how!".to_string(),
        created_at: "2024-01-02T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
        html_url: format!("https://github.com/{OWNER}/{REPO}/discussions/1#discussioncomment-100"),
        user: make_user(),
    };

    let client = MockBackupClient::new()
        .with_discussions(vec![discussion])
        .with_discussion_comments(vec![comment]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        discussions: true,
        ..Default::default()
    };

    let count = backup_discussions(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_discussions");

    assert_eq!(count, 1, "one discussion fetched");
    assert!(
        storage.get(&meta_dir().join("discussions.json")).is_some(),
        "discussions.json written"
    );
    let comment_path = meta_dir().join("discussion_comments_1.json");
    assert!(
        storage.get(&comment_path).is_some(),
        "discussion comment file written"
    );
}

// ── Projects ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_projects_writes_json_and_columns() {
    let project = ClassicProject {
        id: 1,
        number: 1,
        name: "Backlog".to_string(),
        body: Some("Work in progress".to_string()),
        state: "open".to_string(),
        creator: make_user(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        html_url: format!("https://github.com/{OWNER}/{REPO}/projects/1"),
        open_issues_count: Some(5),
    };
    let column = ProjectColumn {
        id: 10,
        name: "To Do".to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        cards: vec![ProjectCard {
            id: 100,
            note: Some("First card".to_string()),
            archived: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            content_url: None,
        }],
    };

    let client = MockBackupClient::new()
        .with_repo_projects(vec![project])
        .with_project_columns(vec![column]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        projects: true,
        ..Default::default()
    };

    backup_projects(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("backup_projects");

    assert!(
        storage.get(&meta_dir().join("projects.json")).is_some(),
        "projects.json written"
    );
    let col_path = meta_dir().join("project_columns_1.json");
    assert!(
        storage.get(&col_path).is_some(),
        "project columns file written"
    );
}

// ── Packages ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn backup_packages_writes_json_and_versions() {
    let package = Package {
        id: 1,
        name: "my-image".to_string(),
        package_type: "container".to_string(),
        visibility: "public".to_string(),
        version_count: 1,
        html_url: "https://github.com/users/octocat/packages/container/my-image".to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        owner: make_user(),
        repository: Some(PackageRepository {
            name: REPO.to_string(),
            full_name: format!("{OWNER}/{REPO}"),
            private: false,
        }),
    };
    let version = PackageVersion {
        id: 1,
        name: "sha256:abc123".to_string(),
        html_url: "https://github.com/users/octocat/packages/container/my-image/1".to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        metadata: None,
    };

    let client = MockBackupClient::new()
        .with_packages(vec![package])
        .with_package_versions(vec![version]);
    let storage = MemStorage::default();
    let opts = BackupOptions {
        packages: true,
        ..Default::default()
    };

    let count = backup_packages(&client, OWNER, &opts, &owner_json_dir(), &storage)
        .await
        .expect("backup_packages");

    assert!(count > 0, "at least one package file written");

    let pkg_path = owner_json_dir().join("packages_container.json");
    assert!(
        storage.get(&pkg_path).is_some(),
        "packages_container.json written"
    );

    let ver_path = owner_json_dir().join("package_versions_container_my-image.json");
    assert!(
        storage.get(&ver_path).is_some(),
        "package versions file written"
    );
}

// ── Multi-module pipeline scenario ─────────────────────────────────────────

/// Smoke-tests the complete per-repo metadata pipeline in one shot:
/// issues, PRs, labels, and discussions are all backed up together.
#[tokio::test]
async fn full_repo_metadata_pipeline_smoke_test() {
    let client = MockBackupClient::new()
        .with_issues(vec![Issue {
            id: 1,
            number: 1,
            title: "Issue 1".to_string(),
            body: None,
            state: "open".to_string(),
            user: make_user(),
            labels: vec![],
            assignees: vec![],
            comments: 0,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            closed_at: None,
            html_url: format!("https://github.com/{OWNER}/{REPO}/issues/1"),
            pull_request: None,
            milestone: None,
        }])
        .with_pull_requests(vec![PullRequest {
            id: 1,
            number: 1,
            title: "PR 1".to_string(),
            body: None,
            state: "open".to_string(),
            merged: Some(false),
            user: make_user(),
            labels: vec![],
            assignees: vec![],
            commits: Some(1),
            additions: Some(1),
            deletions: Some(0),
            changed_files: Some(1),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            closed_at: None,
            merged_at: None,
            html_url: format!("https://github.com/{OWNER}/{REPO}/pull/1"),
            head: make_pr_ref("feature"),
            base: make_pr_ref("main"),
            milestone: None,
        }])
        .with_labels(vec![Label {
            id: 1,
            name: "enhancement".to_string(),
            color: "84b6eb".to_string(),
            description: None,
            default: false,
        }])
        .with_discussions(vec![Discussion {
            number: 1,
            title: "Discussion".to_string(),
            body: String::new(),
            locked: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: String::new(),
            user: make_user(),
            comments: 0,
            category: None,
            answered: false,
        }]);

    let storage = MemStorage::default();
    let opts = BackupOptions {
        issues: true,
        pulls: true,
        labels: true,
        discussions: true,
        ..Default::default()
    };

    backup_issues(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("issues");
    backup_pull_requests(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("prs");
    backup_labels(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("labels");
    backup_discussions(&client, OWNER, REPO, &opts, &meta_dir(), &storage)
        .await
        .expect("discussions");

    for file in &[
        "issues.json",
        "pulls.json",
        "labels.json",
        "discussions.json",
    ] {
        let path = meta_dir().join(file);
        assert!(storage.get(&path).is_some(), "{file} should be written");
    }
}
