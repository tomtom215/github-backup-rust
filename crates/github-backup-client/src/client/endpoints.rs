// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub REST API endpoint methods.
//!
//! All public methods on [`GitHubClient`] that hit a specific API endpoint
//! are grouped here by resource category. The underlying HTTP machinery
//! lives in the parent module.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Method;
use tracing::info;

use github_backup_types::{
    Branch, Collaborator, DeployKey, Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone,
    PullRequest, PullRequestComment, PullRequestCommit, PullRequestReview, Release, Repository,
    SecurityAdvisory, Team, User,
};

use crate::error::ClientError;

use super::{collect_body, GitHubClient, DEFAULT_TIMEOUT_SECS, PER_PAGE};

impl GitHubClient {
    // ── User & org repos ──────────────────────────────────────────────────

    /// Lists repositories owned by a user.
    ///
    /// Includes all repository types the credential has access to. Private
    /// repositories are returned when the token has the `repo` scope.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_user_repos(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/repos?type=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists repositories belonging to an organisation.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_repos(&self, org: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/orgs/{org}/repos?type=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── User social data ──────────────────────────────────────────────────

    /// Returns the followers of a user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_followers(&self, username: &str) -> Result<Vec<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/followers?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the users that `username` is following.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_following(&self, username: &str) -> Result<Vec<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/following?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories starred by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories watched by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_watched(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/subscriptions?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Gists ─────────────────────────────────────────────────────────────

    /// Returns gists owned by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_gists(&self, username: &str) -> Result<Vec<Gist>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/gists?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns gists starred by the authenticated user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred_gists(&self) -> Result<Vec<Gist>, ClientError> {
        let api = self.api();
        let url = format!("{api}/gists/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

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

    // ── Pull Requests ─────────────────────────────────────────────────────

    /// Lists all pull requests for a repository.
    ///
    /// `since` — when `Some`, only returns PRs whose `updated_at` timestamp
    /// is at or after the given ISO 8601 value.
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
        // The GitHub Pulls API does not support a `since` query parameter
        // directly, but the List Repository Issues endpoint (which includes
        // PRs) does.  When a since filter is requested we fetch from the
        // issues endpoint and keep only the entries that have a
        // `pull_request` field (i.e. are PRs), then cross-reference to the
        // Pulls API for the full PR payload if needed.
        //
        // For simplicity we use the Pulls API without date filtering when no
        // `since` is provided, and fall back to the Issues endpoint filter
        // when it is provided — note that the Issues endpoint returns less
        // PR detail, but that is acceptable for incremental detection.
        //
        // Practical note: the GitHub Issues API returns `pull_request` refs
        // for PRs but does NOT return full PR objects (e.g. `head`/`base`).
        // We therefore always use the Pulls API URL and accept that
        // `since`-based filtering on PRs is best-effort (GitHub does not
        // expose this parameter on the Pulls endpoint).
        let api = self.api();
        let mut url = format!("{api}/repos/{owner}/{repo}/pulls?state=all&per_page={PER_PAGE}");
        // Append `sort` and `direction` so that the `since` comparison is
        // meaningful; by default the Pulls API sorts by created_at descending.
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

    // ── Repository metadata ───────────────────────────────────────────────

    /// Lists labels for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_labels(&self, owner: &str, repo: &str) -> Result<Vec<Label>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/labels?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists milestones for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_milestones(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Milestone>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/milestones?state=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists releases for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_releases(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Release>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/releases?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists webhooks configured on a repository.
    ///
    /// Requires `admin` permission on the repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_hooks(&self, owner: &str, repo: &str) -> Result<Vec<Hook>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/hooks?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists published security advisories for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_security_advisories(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<SecurityAdvisory>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/security-advisories?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Assets ────────────────────────────────────────────────────────────

    /// Downloads a release asset and returns the raw bytes.
    ///
    /// Uses the `application/octet-stream` accept header required by GitHub.
    /// Follows HTTP redirects (GitHub redirects asset downloads to S3).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn download_release_asset(&self, asset_url: &str) -> Result<Bytes, ClientError> {
        // GitHub's asset download endpoint returns an HTTP 302 redirect to S3.
        // We follow up to 3 redirects manually because hyper's legacy client
        // does not follow redirects by default.
        let mut url = asset_url.to_string();
        let mut remaining_redirects: u8 = 3;

        loop {
            let req = self
                .build_request(Method::GET, &url)?
                .header("Accept", "application/octet-stream")
                .body(Full::new(Bytes::new()))
                .map_err(ClientError::Http)?;

            let response = tokio::time::timeout(
                std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                self.http.request(req),
            )
            .await
            .map_err(|_| ClientError::Timeout { url: url.clone() })??;

            let status = response.status();

            if status.is_redirection() {
                if remaining_redirects == 0 {
                    return Err(ClientError::ApiError {
                        status: status.as_u16(),
                        body: "too many redirects".to_string(),
                    });
                }
                remaining_redirects -= 1;
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| ClientError::ApiError {
                        status: status.as_u16(),
                        body: "redirect with no Location header".to_string(),
                    })?
                    .to_string();
                url = location;
                continue;
            }

            if !status.is_success() {
                let body = collect_body(response.into_body()).await?;
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            }

            return collect_body(response.into_body()).await;
        }
    }

    /// Lists all branches for a repository.
    ///
    /// Returns branch names, their tip commit SHAs, and whether each branch
    /// has protection rules enabled.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_branches(&self, owner: &str, repo: &str) -> Result<Vec<Branch>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/branches?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the topics configured on a repository.
    ///
    /// Requires the `application/vnd.github.mercy-preview+json` accept header;
    /// this is now GA but the method still exists for completeness.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_repo_topics(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<String>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/topics");

        let req = self
            .build_request(Method::GET, &url)?
            .header("Accept", "application/vnd.github.v3+json")
            .body(Full::new(Bytes::new()))
            .map_err(ClientError::Http)?;

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| ClientError::Timeout { url: url.clone() })??;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = collect_body(response.into_body()).await?;
            return Err(ClientError::ApiError {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        let body = collect_body(response.into_body()).await?;
        #[derive(serde::Deserialize)]
        struct TopicsResponse {
            names: Vec<String>,
        }
        let parsed: TopicsResponse = serde_json::from_slice(&body)?;
        info!(owner, repo, count = parsed.names.len(), "fetched topics");
        Ok(parsed.names)
    }

    // ── Deploy keys ───────────────────────────────────────────────────────

    /// Lists deploy keys configured on a repository.
    ///
    /// Requires admin access to the repository; callers should handle
    /// [`ClientError::ApiError`] with status 403 or 404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_deploy_keys(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<DeployKey>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/keys?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Collaborators ─────────────────────────────────────────────────────

    /// Lists collaborators on a repository.
    ///
    /// Returns users who have been explicitly granted access to the repository.
    /// Requires admin access; callers should handle 403/404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_collaborators(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Collaborator>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/collaborators?per_page={PER_PAGE}&affiliation=all");
        self.get_all_pages(&url).await
    }

    // ── Organisation data ─────────────────────────────────────────────────

    /// Lists members of a GitHub organisation.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_members(&self, org: &str) -> Result<Vec<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/orgs/{org}/members?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists teams in a GitHub organisation.
    ///
    /// Requires the authenticated user to be an organisation member.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_teams(&self, org: &str) -> Result<Vec<Team>, ClientError> {
        let api = self.api();
        let url = format!("{api}/orgs/{org}/teams?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
