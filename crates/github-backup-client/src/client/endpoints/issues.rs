// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue listing endpoints.
//!
//! Covers issue lists, per-issue comments, and per-issue timeline events.

use github_backup_types::{Issue, IssueComment, IssueEvent};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── Issues ────────────────────────────────────────────────────────────

    /// Lists all issues (excluding pull requests) for a repository.
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
    ) -> Result<Vec<Issue>, ClientError> {
        let api = self.api();
        let mut url = format!("{api}/repos/{owner}/{repo}/issues?state=all&per_page={PER_PAGE}");
        if let Some(s) = since {
            url.push_str("&since=");
            url.push_str(s);
        }
        self.get_all_pages(&url).await
    }

    /// Lists comments on a specific issue.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_issue_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<IssueComment>, ClientError> {
        let api = self.api();
        let url = format!(
            "{api}/repos/{owner}/{repo}/issues/{issue_number}/comments?per_page={PER_PAGE}"
        );
        self.get_all_pages(&url).await
    }

    /// Lists timeline events for a specific issue.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_issue_events(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Vec<IssueEvent>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/issues/{issue_number}/events?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
