// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Shared [`MockBackupClient`] for unit tests across backup modules.
//!
//! Only compiled in test builds.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use github_backup_client::{BackupClient, BoxFuture, ClientError};
use github_backup_types::{
    Branch, Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone, PullRequest,
    PullRequestComment, PullRequestCommit, PullRequestReview, Release, Repository,
    SecurityAdvisory, User,
};

/// Configurable [`BackupClient`] for unit tests.
///
/// All methods return empty collections by default; use builder methods to
/// pre-load specific fixtures.
#[derive(Default, Clone)]
pub struct MockBackupClient {
    inner: Arc<Mutex<MockData>>,
}

#[derive(Default)]
struct MockData {
    user_repos: Vec<Repository>,
    gists: Vec<Gist>,
    starred_gists: Vec<Gist>,
    issues: Vec<Issue>,
    issue_comments: Vec<IssueComment>,
    issue_events: Vec<IssueEvent>,
    pull_requests: Vec<PullRequest>,
    pull_comments: Vec<PullRequestComment>,
    pull_commits: Vec<PullRequestCommit>,
    pull_reviews: Vec<PullRequestReview>,
    labels: Vec<Label>,
    milestones: Vec<Milestone>,
    releases: Vec<Release>,
    hooks: Vec<Hook>,
    security_advisories: Vec<SecurityAdvisory>,
    followers: Vec<User>,
    following: Vec<User>,
    starred: Vec<Repository>,
    watched: Vec<Repository>,
    asset_bytes: Vec<u8>,
    topics: Vec<String>,
    branches: Vec<Branch>,
}

#[allow(dead_code)]
impl MockBackupClient {
    /// Creates a new empty [`MockBackupClient`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-loads the issues list.
    pub fn with_issues(self, issues: Vec<Issue>) -> Self {
        self.inner.lock().unwrap().issues = issues;
        self
    }

    /// Pre-loads the pull requests list.
    pub fn with_pull_requests(self, prs: Vec<PullRequest>) -> Self {
        self.inner.lock().unwrap().pull_requests = prs;
        self
    }

    /// Pre-loads the releases list.
    pub fn with_releases(self, releases: Vec<Release>) -> Self {
        self.inner.lock().unwrap().releases = releases;
        self
    }

    /// Pre-loads the gists list.
    pub fn with_gists(self, gists: Vec<Gist>) -> Self {
        self.inner.lock().unwrap().gists = gists;
        self
    }

    /// Pre-loads the starred gists list.
    pub fn with_starred_gists(self, gists: Vec<Gist>) -> Self {
        self.inner.lock().unwrap().starred_gists = gists;
        self
    }

    /// Pre-loads followers.
    pub fn with_followers(self, followers: Vec<User>) -> Self {
        self.inner.lock().unwrap().followers = followers;
        self
    }

    /// Pre-loads following.
    pub fn with_following(self, following: Vec<User>) -> Self {
        self.inner.lock().unwrap().following = following;
        self
    }

    /// Pre-loads starred repositories.
    pub fn with_starred(self, repos: Vec<Repository>) -> Self {
        self.inner.lock().unwrap().starred = repos;
        self
    }

    /// Pre-loads watched repositories.
    pub fn with_watched(self, repos: Vec<Repository>) -> Self {
        self.inner.lock().unwrap().watched = repos;
        self
    }

    /// Pre-loads asset bytes.
    pub fn with_asset_bytes(self, bytes: Vec<u8>) -> Self {
        self.inner.lock().unwrap().asset_bytes = bytes;
        self
    }

    /// Pre-loads labels.
    pub fn with_labels(self, labels: Vec<Label>) -> Self {
        self.inner.lock().unwrap().labels = labels;
        self
    }

    /// Pre-loads topics.
    pub fn with_topics(self, topics: Vec<String>) -> Self {
        self.inner.lock().unwrap().topics = topics;
        self
    }

    /// Pre-loads branches.
    pub fn with_branches(self, branches: Vec<Branch>) -> Self {
        self.inner.lock().unwrap().branches = branches;
        self
    }
}

// Helper macro to cut down on boilerplate.
macro_rules! boxed_empty {
    ($t:ty) => {
        Box::pin(async { Ok(Vec::<$t>::new()) })
    };
}

impl BackupClient for MockBackupClient {
    fn list_user_repos<'a>(
        &'a self,
        _username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        let d = self.inner.lock().unwrap().user_repos.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_org_repos<'a>(
        &'a self,
        _org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        boxed_empty!(Repository)
    }

    fn list_followers<'a>(
        &'a self,
        _username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        let d = self.inner.lock().unwrap().followers.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_following<'a>(
        &'a self,
        _username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        let d = self.inner.lock().unwrap().following.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_starred<'a>(
        &'a self,
        _username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        let d = self.inner.lock().unwrap().starred.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_watched<'a>(
        &'a self,
        _username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        let d = self.inner.lock().unwrap().watched.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_gists<'a>(
        &'a self,
        _username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Gist>, ClientError>> {
        let d = self.inner.lock().unwrap().gists.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_starred_gists<'a>(&'a self) -> BoxFuture<'a, Result<Vec<Gist>, ClientError>> {
        let d = self.inner.lock().unwrap().starred_gists.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_issues<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<Issue>, ClientError>> {
        let d = self.inner.lock().unwrap().issues.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_issue_comments<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _issue_number: u64,
    ) -> BoxFuture<'a, Result<Vec<IssueComment>, ClientError>> {
        let d = self.inner.lock().unwrap().issue_comments.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_issue_events<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _issue_number: u64,
    ) -> BoxFuture<'a, Result<Vec<IssueEvent>, ClientError>> {
        let d = self.inner.lock().unwrap().issue_events.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_pull_requests<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<PullRequest>, ClientError>> {
        let d = self.inner.lock().unwrap().pull_requests.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_pull_comments<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestComment>, ClientError>> {
        let d = self.inner.lock().unwrap().pull_comments.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_pull_commits<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestCommit>, ClientError>> {
        let d = self.inner.lock().unwrap().pull_commits.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_pull_reviews<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestReview>, ClientError>> {
        let d = self.inner.lock().unwrap().pull_reviews.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_labels<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Label>, ClientError>> {
        let d = self.inner.lock().unwrap().labels.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_milestones<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Milestone>, ClientError>> {
        let d = self.inner.lock().unwrap().milestones.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_releases<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Release>, ClientError>> {
        let d = self.inner.lock().unwrap().releases.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_hooks<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Hook>, ClientError>> {
        let d = self.inner.lock().unwrap().hooks.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_security_advisories<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SecurityAdvisory>, ClientError>> {
        let d = self.inner.lock().unwrap().security_advisories.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_repo_topics<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, ClientError>> {
        let d = self.inner.lock().unwrap().topics.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_branches<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Branch>, ClientError>> {
        let d = self.inner.lock().unwrap().branches.clone();
        Box::pin(async move { Ok(d) })
    }

    fn download_release_asset<'a>(
        &'a self,
        _asset_url: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ClientError>> {
        let d = self.inner.lock().unwrap().asset_bytes.clone();
        Box::pin(async move { Ok(Bytes::from(d)) })
    }
}
