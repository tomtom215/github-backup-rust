// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`GitHubClient`] — the primary entry point for GitHub API interactions.

use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::{debug, warn};

use github_backup_types::config::Credential;
use github_backup_types::{
    Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone, PullRequest, PullRequestComment,
    PullRequestCommit, PullRequestReview, Release, Repository, SecurityAdvisory, User,
};

use crate::error::ClientError;
use crate::pagination::parse_next_link;
use crate::rate_limit::RateLimitInfo;

const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));
const PER_PAGE: u32 = 100;
/// Maximum number of times to retry a rate-limited request.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;

type HyperClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

/// Async GitHub REST API v3 client.
///
/// Construct via [`GitHubClient::new`]. The client is cheaply cloneable
/// (the underlying hyper connection pool is `Arc`-wrapped).
#[derive(Clone)]
pub struct GitHubClient {
    http: HyperClient,
    credential: Credential,
}

impl std::fmt::Debug for GitHubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubClient")
            .field("credential", &"[redacted]")
            .finish()
    }
}

impl GitHubClient {
    /// Creates a new [`GitHubClient`] using the system CA certificate bundle.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Tls`] if the native CA bundle cannot be loaded.
    pub fn new(credential: Credential) -> Result<Self, ClientError> {
        let tls_config = build_tls_config()?;
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_only()
            .enable_http1()
            .build();

        let http = Client::builder(TokioExecutor::new()).build(https);

        Ok(Self { http, credential })
    }

    // ── User & org endpoints ──────────────────────────────────────────────

    /// Lists repositories owned by a user.
    ///
    /// Includes all repository types the credential has access to. Private
    /// repositories are returned when the token has the `repo` scope.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_user_repos(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/users/{username}/repos?type=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists repositories belonging to an organisation.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_repos(&self, org: &str) -> Result<Vec<Repository>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/orgs/{org}/repos?type=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the followers of a user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_followers(&self, username: &str) -> Result<Vec<User>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/users/{username}/followers?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the users that `username` is following.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_following(&self, username: &str) -> Result<Vec<User>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/users/{username}/following?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories starred by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/users/{username}/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories watched by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_watched(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/users/{username}/subscriptions?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Gists ─────────────────────────────────────────────────────────────

    /// Returns gists owned by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_gists(&self, username: &str) -> Result<Vec<Gist>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/users/{username}/gists?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns gists starred by the authenticated user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred_gists(&self) -> Result<Vec<Gist>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/gists/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Issues ────────────────────────────────────────────────────────────

    /// Lists all issues (excluding pull requests) for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_issues(&self, owner: &str, repo: &str) -> Result<Vec<Issue>, ClientError> {
        let url =
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues?state=all&per_page={PER_PAGE}");
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
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/comments?per_page={PER_PAGE}"
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
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/events?per_page={PER_PAGE}"
        );
        self.get_all_pages(&url).await
    }

    // ── Pull Requests ─────────────────────────────────────────────────────

    /// Lists all pull requests for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_pull_requests(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<PullRequest>, ClientError> {
        let url =
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls?state=all&per_page={PER_PAGE}");
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
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pr_number}/comments?per_page={PER_PAGE}"
        );
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
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pr_number}/commits?per_page={PER_PAGE}"
        );
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
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pr_number}/reviews?per_page={PER_PAGE}"
        );
        self.get_all_pages(&url).await
    }

    // ── Repository metadata ───────────────────────────────────────────────

    /// Lists labels for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_labels(&self, owner: &str, repo: &str) -> Result<Vec<Label>, ClientError> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/labels?per_page={PER_PAGE}");
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
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/milestones?state=all&per_page={PER_PAGE}"
        );
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
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/releases?per_page={PER_PAGE}");
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
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/hooks?per_page={PER_PAGE}");
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
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/security-advisories?per_page={PER_PAGE}"
        );
        self.get_all_pages(&url).await
    }

    /// Downloads a release asset and returns the raw bytes.
    ///
    /// Uses the `application/octet-stream` accept header required by GitHub.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn download_release_asset(&self, asset_url: &str) -> Result<Bytes, ClientError> {
        let req = self
            .build_request(Method::GET, asset_url)?
            .header("Accept", "application/octet-stream")
            .body(Full::new(Bytes::new()))
            .map_err(ClientError::Http)?;

        let response = self.http.request(req).await?;
        let status = response.status();

        if !status.is_success() {
            let body = collect_body(response.into_body()).await?;
            return Err(ClientError::ApiError {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        collect_body(response.into_body()).await
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Fetches all pages of a paginated endpoint, collecting results into
    /// a single `Vec<T>`.
    async fn get_all_pages<T>(&self, initial_url: &str) -> Result<Vec<T>, ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut results = Vec::new();
        let mut next_url: Option<String> = Some(initial_url.to_string());

        while let Some(url) = next_url.take() {
            debug!(url = %url, "GET");
            let (page, link_header) = self.get_json_with_link::<Vec<T>>(&url).await?;
            results.extend(page);
            next_url = link_header.as_deref().and_then(parse_next_link);
        }

        Ok(results)
    }

    /// Performs a single GET request and returns the deserialised body along
    /// with the raw `Link` header value (if present).
    async fn get_json_with_link<T>(&self, url: &str) -> Result<(T, Option<String>), ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut retries = 0u32;

        loop {
            let req = self
                .build_request(Method::GET, url)?
                .header("Accept", "application/vnd.github.v3+json")
                .body(Full::new(Bytes::new()))
                .map_err(ClientError::Http)?;

            let response = self.http.request(req).await?;
            let status = response.status();
            let headers = response.headers().clone();

            let rate_info = RateLimitInfo::from_headers(&headers);

            // Handle rate limiting (403 or 429 with Retry-After / RateLimit headers)
            if (status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS)
                && rate_info.map(|r| r.is_exhausted()).unwrap_or(false)
            {
                if retries >= MAX_RATE_LIMIT_RETRIES {
                    let wait = rate_info
                        .map(|r| r.seconds_until_reset(unix_now()))
                        .unwrap_or(60);
                    return Err(ClientError::RateLimitExceeded {
                        retry_after_secs: wait,
                    });
                }

                let wait = rate_info
                    .map(|r| r.seconds_until_reset(unix_now()).max(1))
                    .unwrap_or(60);

                warn!(wait_secs = wait, "rate limit hit, sleeping");
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                retries += 1;
                continue;
            }

            if !status.is_success() {
                let body = collect_body(response.into_body()).await?;
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            }

            let link_header = headers
                .get("link")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);

            let body = collect_body(response.into_body()).await?;
            let parsed: T = serde_json::from_slice(&body)?;

            return Ok((parsed, link_header));
        }
    }

    /// Builds a [`hyper::http::request::Builder`] pre-populated with auth
    /// and user-agent headers.
    fn build_request(
        &self,
        method: Method,
        url: &str,
    ) -> Result<hyper::http::request::Builder, ClientError> {
        Ok(Request::builder()
            .method(method)
            .uri(url)
            .header("Authorization", self.credential.authorization_header())
            .header("User-Agent", USER_AGENT))
    }
}

/// Returns the current time as a Unix timestamp in seconds.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Collects a hyper body into a [`Bytes`] buffer.
async fn collect_body(
    body: impl hyper::body::Body<Data = Bytes, Error = hyper::Error>,
) -> Result<Bytes, ClientError> {
    Ok(body.collect().await?.to_bytes())
}

/// Builds a [`rustls::ClientConfig`] using the system native CA bundle.
fn build_tls_config() -> Result<rustls::ClientConfig, ClientError> {
    let mut root_store = rustls::RootCertStore::empty();
    // rustls-native-certs 0.8 returns CertificateResult (not a Result<>).
    // Errors loading individual certs are non-fatal; we surface them only if
    // the store ends up empty.
    let cert_result = rustls_native_certs::load_native_certs();
    if cert_result.certs.is_empty() {
        let msg = cert_result
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no CA certificates found".to_string());
        return Err(ClientError::Tls(msg));
    }
    root_store.add_parsable_certificates(cert_result.certs);
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::config::Credential;

    #[test]
    fn github_client_new_succeeds_with_token() {
        let cred = Credential::Token("ghp_test".to_string());
        let result = GitHubClient::new(cred);
        assert!(result.is_ok(), "client construction should succeed");
    }

    #[test]
    fn github_client_debug_redacts_credential() {
        let cred = Credential::Token("secret_token".to_string());
        let client = GitHubClient::new(cred).expect("construct client");
        let debug_str = format!("{client:?}");
        assert!(
            !debug_str.contains("secret_token"),
            "credential must be redacted in Debug output"
        );
        assert!(debug_str.contains("[redacted]"));
    }
}
