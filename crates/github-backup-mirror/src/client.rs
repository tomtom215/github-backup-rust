// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Async Gitea REST API v1 client.
//!
//! Used to create repositories at the mirror destination before pushing.
//! Compatible with Gitea, Codeberg, Forgejo, and any Gitea-API-compatible
//! service.

use hyper::Method;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::config::GiteaConfig;
use crate::error::MirrorError;
use crate::http::{encode_segment, ApiHttp, Response};
use crate::push::verify_ours;
use crate::runner::MirrorDestination;

/// Async Gitea REST API client for mirror management.
///
/// Handles repository existence checks and creation.  The HTTP client is
/// cheaply cloneable via the underlying `Arc`-wrapped connection pool.
#[derive(Clone)]
pub struct GiteaClient {
    http: ApiHttp,
    config: GiteaConfig,
}

impl std::fmt::Debug for GiteaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GiteaClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Request body for `POST /user/repos` and `POST /orgs/{org}/repos`.
#[derive(Debug, Serialize)]
struct CreateRepoRequest<'a> {
    name: &'a str,
    description: &'a str,
    private: bool,
    /// Do not auto-initialise — the repo will be populated by a push.
    auto_init: bool,
}

/// Subset of the Gitea repository response used by this client.
#[derive(Debug, Deserialize)]
struct GiteaRepo {
    /// Free-text description; carries the mirror marker.
    #[serde(default)]
    description: Option<String>,
    /// `true` while the repository has no commits.
    #[serde(default)]
    empty: bool,
}

impl GiteaClient {
    /// Creates a new [`GiteaClient`] using the system CA bundle for TLS.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::MissingToken`] for an empty token,
    /// [`MirrorError::InsecureUrl`] unless the base URL is `https://`, and
    /// [`MirrorError::Tls`] if the native CA bundle cannot be loaded.
    pub fn new(config: GiteaConfig) -> Result<Self, MirrorError> {
        check_destination(&config.base_url, &config.token)?;
        Ok(Self {
            http: ApiHttp::new()?,
            config,
        })
    }

    #[cfg(test)]
    fn for_tests(config: GiteaConfig) -> Self {
        Self {
            http: ApiHttp::for_tests(),
            config,
        }
    }

    /// Ensures the repository `name` exists at the mirror destination.
    ///
    /// If it already exists it must carry `description` (the marker set when
    /// this tool created it) or still be empty; otherwise it belongs to
    /// someone else and [`MirrorError::ForeignRepository`] is returned.  If it
    /// does not exist, it is created as an empty repository (`private` or
    /// public) under the configured owner (the token's account, or an
    /// organisation).  A `409 Conflict` from the create call (the repository
    /// appeared in between) is accepted only after the same ownership check.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::Api`] (with the response body) if the Gitea API
    /// responds with an error, for example `422` for a name it rejects.
    pub async fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<(), MirrorError> {
        if let Some(existing) = self.fetch_repo(name).await? {
            verify_ours(
                name,
                existing.description.as_deref(),
                existing.empty,
                description,
            )?;
            info!(owner = %self.config.owner, repo = %name, "mirror repository already exists");
            return Ok(());
        }

        info!(owner = %self.config.owner, repo = %name, "creating mirror repository");
        let response = self.create_repo(name, description, private).await?;
        match response.status {
            201 => Ok(()),
            // Created by someone (or an earlier attempt) since the lookup.
            409 => match self.fetch_repo(name).await? {
                Some(existing) => verify_ours(
                    name,
                    existing.description.as_deref(),
                    existing.empty,
                    description,
                ),
                None => Err(response.into_error()),
            },
            _ => Err(response.into_error()),
        }
    }

    fn auth(&self) -> (&'static str, String) {
        ("Authorization", format!("token {}", self.config.token))
    }

    async fn send(
        &self,
        method: Method,
        url: &str,
        json: Option<Vec<u8>>,
    ) -> Result<Response, MirrorError> {
        let (name, value) = self.auth();
        self.http.send(method, url, (name, &value), json).await
    }

    /// Returns the repository `name` at the mirror destination, if it exists.
    async fn fetch_repo(&self, name: &str) -> Result<Option<GiteaRepo>, MirrorError> {
        let url = format!(
            "{}/repos/{}/{}",
            self.config.api_base(),
            encode_segment(&self.config.owner),
            encode_segment(name)
        );
        debug!(url = %url, "checking if mirror repo exists");
        let response = self.send(Method::GET, &url, None).await?;
        match response.status {
            200 => Ok(Some(serde_json::from_slice(&response.body)?)),
            404 => Ok(None),
            _ => Err(response.into_error()),
        }
    }

    /// Returns the login of the account the token belongs to.
    async fn authenticated_login(&self) -> Result<String, MirrorError> {
        #[derive(Deserialize)]
        struct Me {
            login: String,
        }
        let url = format!("{}/user", self.config.api_base());
        let response = self.send(Method::GET, &url, None).await?;
        if response.status != 200 {
            return Err(response.into_error());
        }
        Ok(serde_json::from_slice::<Me>(&response.body)?.login)
    }

    /// Sends the create request and returns the response, whatever its status.
    async fn create_repo(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<Response, MirrorError> {
        // `/user/repos` always creates under the *token's* account.  When the
        // configured owner is another account (an organisation), the repository
        // must be created through the organisation endpoint or the push below
        // would target a repository that does not exist.
        let login = self.authenticated_login().await?;
        let url = create_repo_url(&self.config.api_base(), &self.config.owner, &login);
        let body = serde_json::to_vec(&CreateRepoRequest {
            name,
            description,
            private,
            auto_init: false,
        })?;
        self.send(Method::POST, &url, Some(body)).await
    }
}

impl MirrorDestination for GiteaClient {
    async fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<(), MirrorError> {
        GiteaClient::ensure_repo_exists(self, name, description, private).await
    }
    fn repo_clone_url(&self, name: &str) -> String {
        self.config.repo_clone_url(name)
    }
    fn push_username(&self) -> &'static str {
        "x-access-token"
    }
    fn token(&self) -> &str {
        &self.config.token
    }
    fn force_private(&self) -> bool {
        self.config.private
    }
    fn prune(&self) -> bool {
        self.config.prune
    }
    fn base_url(&self) -> &str {
        &self.config.base_url
    }
}

/// Refuses a destination the token must not be sent to: no token, or not
/// `https://`.  Both clients run this when they are built; callers can run it
/// earlier to report the problem at start-up.
///
/// # Errors
///
/// Returns [`MirrorError::MissingToken`] or [`MirrorError::InsecureUrl`].
pub fn check_destination(base_url: &str, token: &str) -> Result<(), MirrorError> {
    if token.trim().is_empty() {
        return Err(MirrorError::MissingToken);
    }
    let is_https = base_url
        .split_once("://")
        .is_some_and(|(scheme, rest)| scheme.eq_ignore_ascii_case("https") && !rest.is_empty());
    if !is_https {
        return Err(MirrorError::InsecureUrl {
            url: base_url.to_owned(),
        });
    }
    Ok(())
}

/// The endpoint that creates a repository under `owner` for a token that
/// belongs to `login`.
fn create_repo_url(api_base: &str, owner: &str, login: &str) -> String {
    if owner.eq_ignore_ascii_case(login) {
        format!("{api_base}/user/repos")
    } else {
        format!("{api_base}/orgs/{}/repos", encode_segment(owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeApi;

    const MARKER: &str = "GitHub mirror of octo/r";

    fn config(base_url: &str, owner: &str) -> GiteaConfig {
        GiteaConfig {
            base_url: base_url.to_owned(),
            token: "secret_token".to_owned(),
            owner: owner.to_owned(),
            private: true,
            prune: false,
        }
    }

    #[test]
    fn gitea_client_debug_redacts_token() {
        let client = GiteaClient::for_tests(config("https://codeberg.org", "alice"));
        let text = format!("{client:?}");
        assert!(!text.contains("secret_token"), "{text}");
        assert!(
            text.contains("[redacted]") && text.contains("alice"),
            "{text}"
        );
    }

    #[test]
    fn a_client_is_never_built_without_a_token_or_over_plain_http() {
        let mut cfg = config("https://codeberg.org", "alice");
        cfg.token = "  ".to_owned();
        assert!(matches!(
            GiteaClient::new(cfg).unwrap_err(),
            MirrorError::MissingToken
        ));
        let err = GiteaClient::new(config("http://gitea.lan", "alice")).unwrap_err();
        assert!(matches!(err, MirrorError::InsecureUrl { .. }), "{err:?}");
    }

    #[test]
    fn repositories_are_created_under_the_configured_owner() {
        let api = "https://git.example/api/v1";
        assert_eq!(
            create_repo_url(api, "Alice", "alice"),
            "https://git.example/api/v1/user/repos"
        );
        assert_eq!(
            create_repo_url(api, "my-org", "alice"),
            "https://git.example/api/v1/orgs/my-org/repos"
        );
    }

    #[test]
    fn create_repo_request_serialises_correctly() {
        let req = CreateRepoRequest {
            name: "my-repo",
            description: "A test repo",
            private: true,
            auto_init: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"name\":\"my-repo\""));
        assert!(json.contains("\"private\":true"));
        assert!(json.contains("\"auto_init\":false"));
    }

    /// Forgejo answers `422` for a validation error: that is a failure, and
    /// the message carries the server's explanation.
    #[tokio::test]
    async fn a_422_is_a_validation_error_with_its_body() {
        let api = FakeApi::start(|seen| match (seen.method.as_str(), seen.target.as_str()) {
            ("GET", "/api/v1/repos/alice/r") => (404, "{}".into()),
            ("GET", "/api/v1/user") => (200, "{\"login\":\"alice\"}".into()),
            ("POST", "/api/v1/user/repos") => (422, "{\"message\":\"name is reserved\"}".into()),
            _ => (500, "{}".into()),
        })
        .await;
        let client = GiteaClient::for_tests(config(&api.base_url, "alice"));
        let err = client
            .ensure_repo_exists("r", MARKER, true)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, MirrorError::Api { status: 422, .. }),
            "{err:?}"
        );
        assert!(err.to_string().contains("name is reserved"), "{err}");
    }

    /// `409` means it exists: accepted only when it turns out to be ours.
    #[tokio::test]
    async fn a_409_is_accepted_only_for_our_own_repository() {
        for (description, ok) in [(MARKER, true), ("someone else's", false)] {
            let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            let gets = std::sync::Arc::clone(&calls);
            let body = format!("{{\"description\":\"{description}\",\"empty\":false}}");
            let api = FakeApi::start(move |seen| {
                match (seen.method.as_str(), seen.target.as_str()) {
                    ("GET", "/api/v1/repos/my-org/r") => {
                        // Missing at first, present after the conflict.
                        if gets.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                            (404, "{}".into())
                        } else {
                            (200, body.clone())
                        }
                    }
                    ("GET", "/api/v1/user") => (200, "{\"login\":\"alice\"}".into()),
                    ("POST", "/api/v1/orgs/my-org/repos") => (409, String::new()),
                    _ => (500, "{}".into()),
                }
            })
            .await;
            let client = GiteaClient::for_tests(config(&api.base_url, "my-org"));
            let result = client.ensure_repo_exists("r", MARKER, true).await;
            assert_eq!(result.is_ok(), ok, "{description}: {result:?}");
            let auth = api.seen()[0]
                .headers
                .iter()
                .find(|(k, _)| k == "authorization")
                .map(|(_, v)| v.clone());
            assert_eq!(auth.as_deref(), Some("token secret_token"));
        }
    }
}
