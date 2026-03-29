// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request listing endpoints.
//!
//! Covers pull request lists, review comments, commit lists, and submitted
//! reviews for a given repository.

use github_backup_types::{PullRequest, PullRequestComment, PullRequestCommit, PullRequestReview};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── Pull Requests ─────────────────────────────────────────────────────

    /// Lists all pull requests for a repository.
    ///
    /// `since` — when `Some`, only returns PRs whose `updated_at` timestamp
    /// is at or after the given ISO 8601 value.
    ///
    /// # Note
    ///
    /// The GitHub Pulls API does not support a native `since` filter. When
    /// `since` is provided this method sorts by `updated` ascending to make
    /// incremental detection practical, but callers must still filter the
    /// results by `updated_at` themselves if a strict cutoff is required.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        since: Option<&str>,
    ) -> Result<Vec<PullRequest>, ClientError> {
        let api = self.api();
        let mut url = format!("{api}/repos/{owner}/{repo}/pulls?state=all&per_page={PER_PAGE}");
        if since.is_some() {
            url.push_str("&sort=updated&direction=asc");
        }
        self.get_all_pages(&url).await
    }

    /// Lists review comments on a specific pull request.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_pull_comments(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<PullRequestComment>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/pulls/{pr_number}/comments?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists commits included in a specific pull request.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_pull_commits(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<PullRequestCommit>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/pulls/{pr_number}/commits?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists reviews submitted on a specific pull request.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_pull_reviews(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<PullRequestReview>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/pulls/{pr_number}/reviews?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
