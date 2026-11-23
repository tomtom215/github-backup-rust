// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request listing endpoints.
//!
//! Covers pull request lists, review comments, commit lists, submitted
//! reviews, the repository-wide review-comment listing and the commit
//! comparison used for pull requests with more than 250 commits.

use github_backup_types::{
    Page, PullRequest, PullRequestComment, PullRequestCommit, PullRequestReview,
};

use serde_json::Value;

use crate::decode::extend_page;
use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};
use super::{encode_path, encode_segment};

impl GitHubClient {
    // ── Pull Requests ─────────────────────────────────────────────────────

    /// Lists all pull requests for a repository, in every state.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_pull_requests(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<PullRequest>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/pulls?state=all&per_page={PER_PAGE}");
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
    ) -> Result<Page<PullRequestComment>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/pulls/{pr_number}/comments?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists commits included in a specific pull request.
    ///
    /// GitHub returns **at most 250** commits here; for longer pull requests
    /// use [`compare_commits`](Self::compare_commits).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_pull_commits(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Page<PullRequestCommit>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/pulls/{pr_number}/commits?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists every commit reachable from `head` but not from `base`
    /// (`GET /repos/{owner}/{repo}/compare/{base}...{head}`), so the list is
    /// not capped at 250 commits.
    ///
    /// Without `page`/`per_page` the comparison returns at most 250 commits;
    /// with them it is paginated.  The pages are requested one by one until
    /// the response's `total_commits` have arrived or a page comes back
    /// short, which does not depend on a `Link` header.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors, and fails
    /// when a page has no `commits` array.
    pub async fn compare_commits(
        &self,
        owner: &str,
        repo: &str,
        base: &str,
        head: &str,
    ) -> Result<Page<PullRequestCommit>, ClientError> {
        let api = self.api();
        let (base, head) = (encode_path(base), encode_path(head));
        let mut commits = Page::new();
        for page in 1.. {
            let url = format!(
                "{api}/repos/{owner}/{repo}/compare/{base}...{head}?per_page={PER_PAGE}&page={page}"
            );
            let (mut body, _) = self
                .get_json_with_link::<serde_json::Map<String, Value>>(&url)
                .await?;
            let total = body.get("total_commits").and_then(Value::as_u64);
            let Some(Value::Array(values)) = body.remove("commits") else {
                return Err(ClientError::Json(serde::de::Error::custom(format!(
                    "response from {url} has no `commits` array"
                ))));
            };
            let short = values.len() < PER_PAGE as usize;
            extend_page(&mut commits, &url, values);
            let have = (commits.len() + commits.unparsed_count()) as u64;
            if short || total.is_none_or(|total| have >= total) {
                break;
            }
        }
        Ok(commits)
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
    ) -> Result<Page<PullRequestReview>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/pulls/{pr_number}/reviews?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists the review comments of a whole repository that were created or
    /// edited at or after `since` (ISO 8601),
    /// `GET /repos/{owner}/{repo}/pulls/comments?since=`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_repo_pull_comments_since(
        &self,
        owner: &str,
        repo: &str,
        since: &str,
    ) -> Result<Page<PullRequestComment>, ClientError> {
        let api = self.api();
        let since = encode_segment(since);
        let url =
            format!("{api}/repos/{owner}/{repo}/pulls/comments?since={since}&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
