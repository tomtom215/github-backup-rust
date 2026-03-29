// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`BackupClient`] — abstract interface over the GitHub REST API.
//!
//! This trait covers every API method used by the backup engine.  The
//! production implementation is [`crate::GitHubClient`], but test code can
//! substitute a lightweight mock that returns pre-configured fixtures without
//! making any network requests.
//!
//! # Why a separate trait?
//!
//! Decoupling backup logic from the concrete HTTP client enables:
//!
//! 1. **Unit tests** that run without network access or live credentials.
//! 2. **Clearer API surface**: the engine only depends on what it actually uses.
//! 3. **Alternative implementations** (e.g. a caching proxy).
//!
//! # Object safety
//!
//! The trait uses `Pin<Box<dyn Future>>` returns so it is **object-safe** and
//! can be used with `dyn BackupClient` where dynamic dispatch is desired.
//!
//! # Modules
//!
//! - [`mod@impl_github`] — blanket `impl BackupClient for GitHubClient`.

mod impl_github;

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use github_backup_types::{
    Branch, ClassicProject, Collaborator, DeployKey, Discussion, DiscussionComment, Environment,
    Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone, Package, PackageVersion,
    PullRequest, PullRequestComment, PullRequestCommit, PullRequestReview, ProjectColumn,
    Release, Repository, SecurityAdvisory, Team, User, Workflow, WorkflowRun,
};

use crate::error::ClientError;

/// Boxed, pinned, send future returned by every [`BackupClient`] method.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// All GitHub API operations required by the backup engine.
///
/// The production implementation is [`crate::GitHubClient`]. Tests substitute a
/// `MockClient` (available in the `test_support` module) that returns
/// pre-configured data.
pub trait BackupClient: Send + Sync {
    // ── Repositories ──────────────────────────────────────────────────────

    /// Lists repositories owned by a user.
    fn list_user_repos<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>>;

    /// Lists repositories belonging to an organisation.
    fn list_org_repos<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>>;

    // ── User social graph ─────────────────────────────────────────────────

    /// Returns the followers of a user.
    fn list_followers<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>>;

    /// Returns the users that `username` is following.
    fn list_following<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>>;

    /// Returns repositories starred by `username`.
    fn list_starred<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>>;

    /// Returns repositories watched by `username`.
    fn list_watched<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>>;

    // ── Gists ─────────────────────────────────────────────────────────────

    /// Returns gists owned by `username`.
    fn list_gists<'a>(&'a self, username: &'a str)
        -> BoxFuture<'a, Result<Vec<Gist>, ClientError>>;

    /// Returns gists starred by the authenticated user.
    fn list_starred_gists<'a>(&'a self) -> BoxFuture<'a, Result<Vec<Gist>, ClientError>>;

    // ── Issues ────────────────────────────────────────────────────────────

    /// Lists all issues for a repository.
    ///
    /// `since` — if `Some`, only returns issues updated at or after the given
    /// ISO 8601 timestamp (e.g. `"2024-01-01T00:00:00Z"`).
    fn list_issues<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<Issue>, ClientError>>;

    /// Lists comments on a specific issue.
    fn list_issue_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Vec<IssueComment>, ClientError>>;

    /// Lists timeline events for a specific issue.
    fn list_issue_events<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Vec<IssueEvent>, ClientError>>;

    // ── Pull Requests ─────────────────────────────────────────────────────

    /// Lists all pull requests for a repository.
    ///
    /// `since` — if `Some`, only returns PRs updated at or after the given
    /// ISO 8601 timestamp.
    fn list_pull_requests<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<PullRequest>, ClientError>>;

    /// Lists review comments on a specific pull request.
    fn list_pull_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestComment>, ClientError>>;

    /// Lists commits included in a specific pull request.
    fn list_pull_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestCommit>, ClientError>>;

    /// Lists reviews submitted on a specific pull request.
    fn list_pull_reviews<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestReview>, ClientError>>;

    // ── Repository metadata ───────────────────────────────────────────────

    /// Lists labels for a repository.
    fn list_labels<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Label>, ClientError>>;

    /// Lists milestones for a repository.
    fn list_milestones<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Milestone>, ClientError>>;

    /// Lists releases for a repository.
    fn list_releases<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Release>, ClientError>>;

    /// Lists webhooks configured on a repository.
    fn list_hooks<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Hook>, ClientError>>;

    /// Lists published security advisories for a repository.
    fn list_security_advisories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SecurityAdvisory>, ClientError>>;

    /// Returns the topics (tags) configured on a repository.
    fn list_repo_topics<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, ClientError>>;

    /// Lists all branches for a repository.
    fn list_branches<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Branch>, ClientError>>;

    // ── Assets ────────────────────────────────────────────────────────────

    /// Downloads a release asset and returns the raw bytes.
    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ClientError>>;

    // ── Deploy keys ───────────────────────────────────────────────────────

    /// Lists deploy keys configured on a repository.
    ///
    /// Callers should handle [`ClientError::ApiError`] with status 403/404
    /// gracefully (insufficient permissions).
    fn list_deploy_keys<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<DeployKey>, ClientError>>;

    // ── Collaborators ─────────────────────────────────────────────────────

    /// Lists collaborators on a repository.
    ///
    /// Callers should handle [`ClientError::ApiError`] with status 403/404
    /// gracefully (insufficient permissions).
    fn list_collaborators<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Collaborator>, ClientError>>;

    // ── Organisation data ─────────────────────────────────────────────────

    /// Lists members of a GitHub organisation.
    fn list_org_members<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>>;

    /// Lists teams in a GitHub organisation.
    fn list_org_teams<'a>(&'a self, org: &'a str) -> BoxFuture<'a, Result<Vec<Team>, ClientError>>;

    // ── GitHub Actions ────────────────────────────────────────────────────

    /// Lists GitHub Actions workflows defined in a repository.
    ///
    /// Callers should handle [`ClientError::ApiError`] with status 403/404
    /// gracefully (Actions may be disabled or the token lacks the `actions`
    /// scope).
    fn list_workflows<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Workflow>, ClientError>>;

    /// Lists workflow runs for a specific workflow.
    ///
    /// Returns up to the most recent runs (paginated).  Callers should handle
    /// 403/404 gracefully.
    fn list_workflow_runs<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        workflow_id: u64,
    ) -> BoxFuture<'a, Result<Vec<WorkflowRun>, ClientError>>;

    // ── Deployment environments ───────────────────────────────────────────

    /// Lists deployment environments configured on a repository.
    ///
    /// Callers should handle 403/404 gracefully.
    fn list_environments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Environment>, ClientError>>;

    // ── Discussions ───────────────────────────────────────────────────────

    /// Lists discussions for a repository.
    ///
    /// GitHub Discussions are only available for repositories that have the
    /// feature enabled.  Callers should handle 404 gracefully.
    fn list_discussions<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Discussion>, ClientError>>;

    /// Lists comments on a specific discussion.
    fn list_discussion_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        discussion_number: u64,
    ) -> BoxFuture<'a, Result<Vec<DiscussionComment>, ClientError>>;

    // ── Classic Projects ──────────────────────────────────────────────────

    /// Lists classic (v1) projects for a repository.
    ///
    /// Callers should handle 404 gracefully (project feature may be disabled).
    fn list_repo_projects<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ClassicProject>, ClientError>>;

    /// Lists columns in a classic project.
    fn list_project_columns<'a>(
        &'a self,
        project_id: u64,
    ) -> BoxFuture<'a, Result<Vec<ProjectColumn>, ClientError>>;

    // ── GitHub Packages ───────────────────────────────────────────────────

    /// Lists packages published by a user.
    ///
    /// Requires the `read:packages` scope.  Callers should handle 403/404
    /// gracefully.
    fn list_user_packages<'a>(
        &'a self,
        username: &'a str,
        package_type: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Package>, ClientError>>;

    /// Lists versions of a specific package.
    fn list_package_versions<'a>(
        &'a self,
        username: &'a str,
        package_type: &'a str,
        package_name: &'a str,
    ) -> BoxFuture<'a, Result<Vec<PackageVersion>, ClientError>>;
}
