// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue listing endpoints.
//!
//! Covers issue lists, per-issue comments and events, and the
//! repository-wide comment listing used to find edited comments.

use github_backup_types::{Issue, IssueComment, IssueEvent, Page};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};
use super::encode_segment;

impl GitHubClient {
    // ── Issues ────────────────────────────────────────────────────────────

    /// Lists the issues of a repository, **including pull requests**, in
    /// every state: all of them, or with `Some(since)` (ISO 8601) only those
    /// updated at or after `since` (`GET .../issues?state=all&since=`).
    ///
    /// GitHub's issues endpoint returns every pull request as an issue too
    /// (with a `pull_request` stub; see [`Issue::is_pull_request`]).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        since: Option<&str>,
    ) -> Result<Page<Issue>, ClientError> {
        let api = self.api();
        let since = since
            .map(|since| format!("&since={}", encode_segment(since)))
            .unwrap_or_default();
        let url = format!("{api}/repos/{owner}/{repo}/issues?state=all{since}&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists comments on a specific issue or pull request.
    ///
    /// Issues and pull requests share one number space, so for a pull request
    /// this is its conversation thread (inline review comments are
    /// [`list_pull_comments`](GitHubClient::list_pull_comments)).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_issue_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Page<IssueComment>, ClientError> {
        let api = self.api();
        let url = format!(
            "{api}/repos/{owner}/{repo}/issues/{issue_number}/comments?per_page={PER_PAGE}"
        );
        self.get_all_pages(&url).await
    }

    /// Lists the events (`closed`, `labeled`, `assigned`, ...) of a specific
    /// issue or pull request (`GET .../issues/{n}/events`).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_issue_events(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Page<IssueEvent>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/issues/{issue_number}/events?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists the issue and pull-request conversation comments of a whole
    /// repository that were created or edited at or after `since` (ISO 8601),
    /// `GET /repos/{owner}/{repo}/issues/comments?since=`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_repo_issue_comments_since(
        &self,
        owner: &str,
        repo: &str,
        since: &str,
    ) -> Result<Page<IssueComment>, ClientError> {
        let api = self.api();
        let since = encode_segment(since);
        let url =
            format!("{api}/repos/{owner}/{repo}/issues/comments?since={since}&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
