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
//! # Lossless results
//!
//! Every list method returns a [`Page`]: the elements that fit the typed model
//! (as [`Raw`], which derefs to the model and serialises as the original JSON)
//! plus, verbatim, the elements that did not.  One unexpected object therefore
//! never fails a list, and writing a page to disk writes everything the API
//! returned.
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

use github_backup_types::{
    Branch, BranchProtection, ClassicProject, Collaborator, DeployKey, Discussion,
    DiscussionComment, Environment, Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone,
    Package, PackageVersion, Page, ProjectColumn, PullRequest, PullRequestComment,
    PullRequestCommit, PullRequestReview, Raw, Release, Repository, SecurityAdvisory, Team, User,
    Workflow, WorkflowRun,
};

use crate::error::ClientError;

/// Boxed, pinned, send future returned by every [`BackupClient`] method.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Destination of a streamed release-asset download.
///
/// [`BackupClient::download_release_asset`] hands the body to the sink chunk by
/// chunk, so an asset of any size is never held in memory.
pub trait AssetSink: Send {
    /// Appends `chunk` to the destination.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the chunk cannot be stored (disk full, ...);
    /// the download is then abandoned with [`ClientError::Io`].
    fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()>;
}

/// All GitHub API operations required by the backup engine.
///
/// The production implementation is [`crate::GitHubClient`]. Tests substitute a
/// `MockClient` (available in the `test_support` module) that returns
/// pre-configured data.
pub trait BackupClient: Send + Sync {
    // ── Repositories ──────────────────────────────────────────────────────

    /// Lists repositories owned by a user.
    ///
    /// When the credential belongs to `username` the listing includes the
    /// account's private repositories (`GET /user/repos`); for anyone else,
    /// and without a credential, only public repositories are visible
    /// (`GET /users/{username}/repos`).
    fn list_user_repos<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    /// Lists repositories belonging to an organisation.
    fn list_org_repos<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    // ── User social graph ─────────────────────────────────────────────────

    /// Returns the followers of a user.
    fn list_followers<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<User>, ClientError>>;

    /// Returns the users that `username` is following.
    fn list_following<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<User>, ClientError>>;

    /// Returns repositories starred by `username`.
    fn list_starred<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    /// Returns repositories watched by `username`.
    fn list_watched<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    // ── Gists ─────────────────────────────────────────────────────────────

    /// Returns gists owned by `username`.
    ///
    /// When the credential belongs to `username` the listing includes the
    /// account's secret gists (`GET /gists`); otherwise only public gists are
    /// visible (`GET /users/{username}/gists`).
    fn list_gists<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Gist>, ClientError>>;

    /// Returns gists starred by the authenticated user.
    fn list_starred_gists<'a>(&'a self) -> BoxFuture<'a, Result<Page<Gist>, ClientError>>;

    // ── Issues ────────────────────────────────────────────────────────────

    /// Lists all issues for a repository, **including pull requests**.
    ///
    /// GitHub's issues API returns every pull request as an issue too (with a
    /// `pull_request` stub); `Issue::is_pull_request` tells them apart.
    ///
    /// `since` — if `Some`, only returns issues updated at or after the given
    /// ISO 8601 timestamp (e.g. `"2024-01-01T00:00:00Z"`).
    fn list_issues<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Page<Issue>, ClientError>>;

    /// Lists comments on a specific issue.
    ///
    /// Issues and pull requests share one number space, so for a pull request
    /// this returns its conversation comments (the discussion thread); inline
    /// review comments are [`list_pull_comments`](Self::list_pull_comments).
    fn list_issue_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Page<IssueComment>, ClientError>>;

    /// Lists the events (`closed`, `labeled`, `assigned`, ...) of a specific
    /// issue or pull request (`GET .../issues/{n}/events`; the richer
    /// `/timeline` is not used).
    fn list_issue_events<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Page<IssueEvent>, ClientError>>;

    // ── Pull Requests ─────────────────────────────────────────────────────

    /// Lists all pull requests for a repository.
    ///
    /// `since` — when `Some`, the list is requested sorted by `updated`
    /// ascending.  The GitHub pulls API has no `since` filter, so the list is
    /// always complete and callers that need a cutoff compare `updated_at`
    /// themselves.
    fn list_pull_requests<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Page<PullRequest>, ClientError>>;

    /// Lists review comments on a specific pull request.
    fn list_pull_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Page<PullRequestComment>, ClientError>>;

    /// Lists commits included in a specific pull request.
    fn list_pull_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Page<PullRequestCommit>, ClientError>>;

    /// Lists reviews submitted on a specific pull request.
    fn list_pull_reviews<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Page<PullRequestReview>, ClientError>>;

    // ── Repository metadata ───────────────────────────────────────────────

    /// Lists labels for a repository.
    fn list_labels<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Label>, ClientError>>;

    /// Lists milestones for a repository.
    fn list_milestones<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Milestone>, ClientError>>;

    /// Lists releases for a repository.
    fn list_releases<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Release>, ClientError>>;

    /// Lists webhooks configured on a repository.
    fn list_hooks<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Hook>, ClientError>>;

    /// Lists published security advisories for a repository.
    fn list_security_advisories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<SecurityAdvisory>, ClientError>>;

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
    ) -> BoxFuture<'a, Result<Page<Branch>, ClientError>>;

    /// Returns the detailed branch-protection rules for a single branch.
    ///
    /// Callers should handle [`ClientError::ApiError`] with status 403 (no
    /// admin access) or 404 (branch has no protection rules) gracefully.
    fn get_branch_protection<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        branch: &'a str,
    ) -> BoxFuture<'a, Result<Raw<BranchProtection>, ClientError>>;

    // ── Assets ────────────────────────────────────────────────────────────

    /// Streams a release asset into `sink` and returns the number of bytes
    /// delivered.
    ///
    /// The `Authorization` header is sent to `asset_url` only; a redirect to
    /// another host, port or scheme (GitHub redirects to its storage) is
    /// followed *without* it, and a redirect from HTTPS to HTTP is refused.
    /// Nothing is buffered: the body goes to the sink chunk by chunk.
    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
        sink: &'a mut dyn AssetSink,
    ) -> BoxFuture<'a, Result<u64, ClientError>>;

    // ── Deploy keys ───────────────────────────────────────────────────────

    /// Lists deploy keys configured on a repository.
    ///
    /// Callers should handle [`ClientError::ApiError`] with status 403/404
    /// gracefully (insufficient permissions).
    fn list_deploy_keys<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<DeployKey>, ClientError>>;

    // ── Collaborators ─────────────────────────────────────────────────────

    /// Lists collaborators on a repository.
    ///
    /// Callers should handle [`ClientError::ApiError`] with status 403/404
    /// gracefully (insufficient permissions).
    fn list_collaborators<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Collaborator>, ClientError>>;

    // ── Organisation data ─────────────────────────────────────────────────

    /// Lists members of a GitHub organisation.
    fn list_org_members<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Page<User>, ClientError>>;

    /// Lists teams in a GitHub organisation.
    fn list_org_teams<'a>(&'a self, org: &'a str)
        -> BoxFuture<'a, Result<Page<Team>, ClientError>>;

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
    ) -> BoxFuture<'a, Result<Page<Workflow>, ClientError>>;

    /// Lists the runs of a specific workflow, following every page.
    ///
    /// Callers should handle 403/404 gracefully.
    fn list_workflow_runs<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        workflow_id: u64,
    ) -> BoxFuture<'a, Result<Page<WorkflowRun>, ClientError>>;

    // ── Deployment environments ───────────────────────────────────────────

    /// Lists deployment environments configured on a repository.
    ///
    /// Callers should handle 403/404 gracefully.
    fn list_environments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Environment>, ClientError>>;

    // ── Discussions ───────────────────────────────────────────────────────

    /// Lists discussions for a repository.
    ///
    /// GitHub Discussions are only available for repositories that have the
    /// feature enabled.  Callers should handle 404 gracefully.
    fn list_discussions<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Discussion>, ClientError>>;

    /// Lists comments on a specific discussion.
    fn list_discussion_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        discussion_number: u64,
    ) -> BoxFuture<'a, Result<Page<DiscussionComment>, ClientError>>;

    // ── Classic Projects ──────────────────────────────────────────────────

    /// Lists classic (v1) projects for a repository.
    ///
    /// Callers should handle 404 gracefully (project feature may be disabled).
    fn list_repo_projects<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<ClassicProject>, ClientError>>;

    /// Lists columns in a classic project.
    fn list_project_columns<'a>(
        &'a self,
        project_id: u64,
    ) -> BoxFuture<'a, Result<Page<ProjectColumn>, ClientError>>;

    // ── GitHub Packages ───────────────────────────────────────────────────

    /// Lists packages published by a user.
    ///
    /// Requires the `read:packages` scope.  Callers should handle 403/404
    /// gracefully.
    fn list_user_packages<'a>(
        &'a self,
        username: &'a str,
        package_type: &'a str,
    ) -> BoxFuture<'a, Result<Page<Package>, ClientError>>;

    /// Lists versions of a specific package.
    fn list_package_versions<'a>(
        &'a self,
        username: &'a str,
        package_type: &'a str,
        package_name: &'a str,
    ) -> BoxFuture<'a, Result<Page<PackageVersion>, ClientError>>;
}
