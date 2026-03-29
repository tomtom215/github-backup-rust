// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Shared [`MockBackupClient`] for unit tests across backup modules.
//!
//! Only compiled in test builds.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use github_backup_client::{BackupClient, BoxFuture, ClientError};
use github_backup_types::{
    Branch, ClassicProject, Collaborator, DeployKey, Discussion, DiscussionComment, Environment,
    Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone, Package, PackageVersion,
    ProjectColumn, PullRequest, PullRequestComment, PullRequestCommit, PullRequestReview, Release,
    Repository, SecurityAdvisory, Team, User, Workflow, WorkflowRun,
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
    deploy_keys: Vec<DeployKey>,
    collaborators: Vec<Collaborator>,
    org_members: Vec<User>,
    org_teams: Vec<Team>,
    workflows: Vec<Workflow>,
    workflow_runs: Vec<WorkflowRun>,
    environments: Vec<Environment>,
    discussions: Vec<Discussion>,
    discussion_comments: Vec<DiscussionComment>,
    repo_projects: Vec<ClassicProject>,
    project_columns: Vec<ProjectColumn>,
    packages: Vec<Package>,
    package_versions: Vec<PackageVersion>,
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

    /// Pre-loads the issue comments list.
    pub fn with_issue_comments(self, comments: Vec<IssueComment>) -> Self {
        self.inner.lock().unwrap().issue_comments = comments;
        self
    }

    /// Pre-loads the milestones list.
    pub fn with_milestones(self, milestones: Vec<Milestone>) -> Self {
        self.inner.lock().unwrap().milestones = milestones;
        self
    }

    /// Pre-loads the hooks list.
    pub fn with_hooks(self, hooks: Vec<Hook>) -> Self {
        self.inner.lock().unwrap().hooks = hooks;
        self
    }

    /// Pre-loads the security advisories list.
    pub fn with_security_advisories(self, advisories: Vec<SecurityAdvisory>) -> Self {
        self.inner.lock().unwrap().security_advisories = advisories;
        self
    }

    /// Pre-loads the user repositories list.
    pub fn with_user_repos(self, repos: Vec<Repository>) -> Self {
        self.inner.lock().unwrap().user_repos = repos;
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

    /// Pre-loads deploy keys.
    pub fn with_deploy_keys(self, keys: Vec<DeployKey>) -> Self {
        self.inner.lock().unwrap().deploy_keys = keys;
        self
    }

    /// Pre-loads collaborators.
    pub fn with_collaborators(self, collaborators: Vec<Collaborator>) -> Self {
        self.inner.lock().unwrap().collaborators = collaborators;
        self
    }

    /// Pre-loads org members.
    pub fn with_org_members(self, members: Vec<User>) -> Self {
        self.inner.lock().unwrap().org_members = members;
        self
    }

    /// Pre-loads org teams.
    pub fn with_org_teams(self, teams: Vec<Team>) -> Self {
        self.inner.lock().unwrap().org_teams = teams;
        self
    }

    /// Pre-loads workflows.
    pub fn with_workflows(self, workflows: Vec<Workflow>) -> Self {
        self.inner.lock().unwrap().workflows = workflows;
        self
    }

    /// Pre-loads workflow runs.
    pub fn with_workflow_runs(self, runs: Vec<WorkflowRun>) -> Self {
        self.inner.lock().unwrap().workflow_runs = runs;
        self
    }

    /// Pre-loads deployment environments.
    pub fn with_environments(self, envs: Vec<Environment>) -> Self {
        self.inner.lock().unwrap().environments = envs;
        self
    }

    /// Pre-loads discussions.
    pub fn with_discussions(self, discussions: Vec<Discussion>) -> Self {
        self.inner.lock().unwrap().discussions = discussions;
        self
    }

    /// Pre-loads discussion comments.
    pub fn with_discussion_comments(self, comments: Vec<DiscussionComment>) -> Self {
        self.inner.lock().unwrap().discussion_comments = comments;
        self
    }

    /// Pre-loads classic projects.
    pub fn with_repo_projects(self, projects: Vec<ClassicProject>) -> Self {
        self.inner.lock().unwrap().repo_projects = projects;
        self
    }

    /// Pre-loads project columns.
    pub fn with_project_columns(self, columns: Vec<ProjectColumn>) -> Self {
        self.inner.lock().unwrap().project_columns = columns;
        self
    }

    /// Pre-loads packages.
    pub fn with_packages(self, packages: Vec<Package>) -> Self {
        self.inner.lock().unwrap().packages = packages;
        self
    }

    /// Pre-loads package versions.
    pub fn with_package_versions(self, versions: Vec<PackageVersion>) -> Self {
        self.inner.lock().unwrap().package_versions = versions;
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

    fn list_deploy_keys<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<DeployKey>, ClientError>> {
        let d = self.inner.lock().unwrap().deploy_keys.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_collaborators<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Collaborator>, ClientError>> {
        let d = self.inner.lock().unwrap().collaborators.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_org_members<'a>(
        &'a self,
        _org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        let d = self.inner.lock().unwrap().org_members.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_org_teams<'a>(
        &'a self,
        _org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Team>, ClientError>> {
        let d = self.inner.lock().unwrap().org_teams.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_workflows<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Workflow>, ClientError>> {
        let d = self.inner.lock().unwrap().workflows.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_workflow_runs<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _workflow_id: u64,
    ) -> BoxFuture<'a, Result<Vec<WorkflowRun>, ClientError>> {
        let d = self.inner.lock().unwrap().workflow_runs.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_environments<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Environment>, ClientError>> {
        let d = self.inner.lock().unwrap().environments.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_discussions<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Discussion>, ClientError>> {
        let d = self.inner.lock().unwrap().discussions.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_discussion_comments<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        _discussion_number: u64,
    ) -> BoxFuture<'a, Result<Vec<DiscussionComment>, ClientError>> {
        let d = self.inner.lock().unwrap().discussion_comments.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_repo_projects<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ClassicProject>, ClientError>> {
        let d = self.inner.lock().unwrap().repo_projects.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_project_columns<'a>(
        &'a self,
        _project_id: u64,
    ) -> BoxFuture<'a, Result<Vec<ProjectColumn>, ClientError>> {
        let d = self.inner.lock().unwrap().project_columns.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_user_packages<'a>(
        &'a self,
        _username: &'a str,
        _package_type: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Package>, ClientError>> {
        let d = self.inner.lock().unwrap().packages.clone();
        Box::pin(async move { Ok(d) })
    }

    fn list_package_versions<'a>(
        &'a self,
        _username: &'a str,
        _package_type: &'a str,
        _package_name: &'a str,
    ) -> BoxFuture<'a, Result<Vec<PackageVersion>, ClientError>> {
        let d = self.inner.lock().unwrap().package_versions.clone();
        Box::pin(async move { Ok(d) })
    }
}
