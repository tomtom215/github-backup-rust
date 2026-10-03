// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue listing endpoints.
//!
//! Covers issue lists, per-issue comments, and per-issue timeline events.

use github_backup_types::{Issue, IssueComment, IssueEvent, Page};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── Issues ────────────────────────────────────────────────────────────

    /// Lists all issues for a repository, **including pull requests**.
    ///
    /// GitHub's issues endpoint returns every pull request as an issue too
    /// (with a `pull_request` stub; see [`Issue::is_pull_request`]).
    ///
    /// `since` — when `Some`, only returns issues updated at or after the
    /// given ISO 8601 timestamp (e.g. `"2024-01-01T00:00:00Z"`).
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
        let mut url = format!("{api}/repos/{owner}/{repo}/issues?state=all&per_page={PER_PAGE}");
        if let Some(s) = since {
            url.push_str("&since=");
            url.push_str(s);
        }
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
}
