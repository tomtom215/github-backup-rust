// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`BackupClient`] — abstract interface over the GitHub REST API.
//!
//! This trait covers every API method used by the backup engine.  The
//! production implementation is [`GitHubClient`], but test code can
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

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use github_backup_types::{
    Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone, PullRequest, PullRequestComment,
    PullRequestCommit, PullRequestReview, Release, Repository, SecurityAdvisory, User,
};

use crate::error::ClientError;
use crate::GitHubClient;

/// Boxed, pinned, send future returned by every [`BackupClient`] method.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// All GitHub API operations required by the backup engine.
///
/// The production implementation is [`GitHubClient`]. Tests substitute a
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
    fn list_issues<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
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
    fn list_pull_requests<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
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

    // ── Assets ────────────────────────────────────────────────────────────

    /// Downloads a release asset and returns the raw bytes.
    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ClientError>>;
}

// ── Blanket impl for the production client ────────────────────────────────

impl BackupClient for GitHubClient {
    fn list_user_repos<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_user_repos(self, username))
    }

    fn list_org_repos<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_org_repos(self, org))
    }

    fn list_followers<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        Box::pin(GitHubClient::list_followers(self, username))
    }

    fn list_following<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        Box::pin(GitHubClient::list_following(self, username))
    }

    fn list_starred<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_starred(self, username))
    }

    fn list_watched<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_watched(self, username))
    }

    fn list_gists<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Gist>, ClientError>> {
        Box::pin(GitHubClient::list_gists(self, username))
    }

    fn list_starred_gists<'a>(&'a self) -> BoxFuture<'a, Result<Vec<Gist>, ClientError>> {
        Box::pin(GitHubClient::list_starred_gists(self))
    }

    fn list_issues<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Issue>, ClientError>> {
        Box::pin(GitHubClient::list_issues(self, owner, repo))
    }

    fn list_issue_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Vec<IssueComment>, ClientError>> {
        Box::pin(GitHubClient::list_issue_comments(
            self,
            owner,
            repo,
            issue_number,
        ))
    }

    fn list_issue_events<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Vec<IssueEvent>, ClientError>> {
        Box::pin(GitHubClient::list_issue_events(
            self,
            owner,
            repo,
            issue_number,
        ))
    }

    fn list_pull_requests<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<PullRequest>, ClientError>> {
        Box::pin(GitHubClient::list_pull_requests(self, owner, repo))
    }

    fn list_pull_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestComment>, ClientError>> {
        Box::pin(GitHubClient::list_pull_comments(
            self, owner, repo, pr_number,
        ))
    }

    fn list_pull_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestCommit>, ClientError>> {
        Box::pin(GitHubClient::list_pull_commits(
            self, owner, repo, pr_number,
        ))
    }

    fn list_pull_reviews<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestReview>, ClientError>> {
        Box::pin(GitHubClient::list_pull_reviews(
            self, owner, repo, pr_number,
        ))
    }

    fn list_labels<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Label>, ClientError>> {
        Box::pin(GitHubClient::list_labels(self, owner, repo))
    }

    fn list_milestones<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Milestone>, ClientError>> {
        Box::pin(GitHubClient::list_milestones(self, owner, repo))
    }

    fn list_releases<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Release>, ClientError>> {
        Box::pin(GitHubClient::list_releases(self, owner, repo))
    }

    fn list_hooks<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Hook>, ClientError>> {
        Box::pin(GitHubClient::list_hooks(self, owner, repo))
    }

    fn list_security_advisories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SecurityAdvisory>, ClientError>> {
        Box::pin(GitHubClient::list_security_advisories(self, owner, repo))
    }

    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ClientError>> {
        Box::pin(GitHubClient::download_release_asset(self, asset_url))
    }
}
