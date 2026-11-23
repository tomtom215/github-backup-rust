// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Async GitLab REST API v4 client for mirror repository management.
//!
//! Used to create projects on GitLab.com or a self-hosted GitLab CE/EE
//! instance before branches and tags are pushed to them.
//!
//! `POST /projects` places a project by **namespace id** (`namespace_id`);
//! without one GitLab creates it in the token user's personal namespace.  The
//! configured namespace path (a user, group or `group/subgroup`) is therefore
//! resolved to its id with `GET /namespaces/<path>` first.

use hyper::Method;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::config::GitLabConfig;
use crate::error::MirrorError;
use crate::http::{encode_segment, ApiHttp, Response};
use crate::push::verify_ours;
use crate::runner::MirrorDestination;

/// Async GitLab REST API v4 client.
///
/// Handles project existence checks and creation at a GitLab instance.
/// Compatible with GitLab.com and any self-hosted GitLab CE/EE deployment.
#[derive(Clone)]
pub struct GitLabClient {
    http: ApiHttp,
    config: GitLabConfig,
}

impl std::fmt::Debug for GitLabClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitLabClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Request body for `POST /api/v4/projects`.
#[derive(Debug, Serialize)]
struct CreateProjectRequest<'a> {
    name: &'a str,
    /// The repository path; the push URL is `<namespace>/<path>.git`.
    path: &'a str,
    /// The namespace (user or group) the project is created in.
    namespace_id: u64,
    description: &'a str,
    /// `"private"` or `"public"`.
    visibility: &'a str,
    /// `false`: the project stays empty until the push.
    initialize_with_readme: bool,
}

/// Subset of the GitLab project response used by this client.
#[derive(Debug, Deserialize)]
struct GitLabProject {
    /// Free-text description; carries the mirror marker.
    #[serde(default)]
    description: Option<String>,
    /// `true` while the project has no commits.
    #[serde(default)]
    empty_repo: bool,
}

/// Subset of the GitLab namespace response used by this client.
#[derive(Debug, Deserialize)]
struct Namespace {
    id: u64,
}

impl GitLabClient {
    /// Creates a new [`GitLabClient`] using the system CA bundle for TLS.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::MissingToken`] for an empty token,
    /// [`MirrorError::InsecureUrl`] unless the base URL is `https://`, and
    /// [`MirrorError::Tls`] if the native CA bundle cannot be loaded.
    pub fn new(config: GitLabConfig) -> Result<Self, MirrorError> {
        crate::client::check_destination(&config.base_url, &config.token)?;
        Ok(Self {
            http: ApiHttp::new()?,
            config,
        })
    }

    #[cfg(test)]
    fn for_tests(config: GitLabConfig) -> Self {
        Self {
            http: ApiHttp::for_tests(),
            config,
        }
    }

    /// Ensures the project `name` exists in the configured namespace.
    ///
    /// If it already exists it must carry `description` (the marker set when
    /// this tool created it) or still be empty; otherwise
    /// [`MirrorError::ForeignRepository`] is returned.  If not, it is created
    /// as an empty project, private or public, in that namespace.  A `400`
    /// saying the name or path "has already been taken" (the project appeared
    /// in between) is accepted only if the project can then be fetched and
    /// passes the same ownership check; every other error is returned.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::NamespaceNotFound`] if the namespace cannot be
    /// resolved and [`MirrorError::Api`] (with the response body) if the
    /// GitLab API responds with an error.
    pub async fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<(), MirrorError> {
        if let Some(existing) = self.fetch_project(name).await? {
            verify_ours(
                name,
                existing.description.as_deref(),
                existing.empty_repo,
                description,
            )?;
            info!(namespace = %self.config.namespace, project = %name, "GitLab mirror project already exists");
            return Ok(());
        }

        info!(namespace = %self.config.namespace, project = %name, "creating GitLab mirror project");
        let namespace_id = self.namespace_id().await?;
        let url = format!("{}/projects", self.config.api_base());
        let body = serde_json::to_vec(&CreateProjectRequest {
            name,
            path: name,
            namespace_id,
            description,
            visibility: if private { "private" } else { "public" },
            initialize_with_readme: false,
        })?;
        let response = self.send(Method::POST, &url, Some(body)).await?;
        match response.status {
            201 => Ok(()),
            400 if String::from_utf8_lossy(&response.body).contains("has already been taken") => {
                match self.fetch_project(name).await? {
                    Some(existing) => verify_ours(
                        name,
                        existing.description.as_deref(),
                        existing.empty_repo,
                        description,
                    ),
                    None => Err(response.into_error()),
                }
            }
            _ => Err(response.into_error()),
        }
    }

    async fn send(
        &self,
        method: Method,
        url: &str,
        json: Option<Vec<u8>>,
    ) -> Result<Response, MirrorError> {
        // GitLab accepts personal access tokens in the `PRIVATE-TOKEN` header.
        self.http
            .send(method, url, ("PRIVATE-TOKEN", &self.config.token), json)
            .await
    }

    /// Resolves the configured namespace path to its id.
    async fn namespace_id(&self) -> Result<u64, MirrorError> {
        let url = format!(
            "{}/namespaces/{}",
            self.config.api_base(),
            encode_segment(&self.config.namespace)
        );
        let response = self.send(Method::GET, &url, None).await?;
        match response.status {
            200 => Ok(serde_json::from_slice::<Namespace>(&response.body)?.id),
            404 => Err(MirrorError::NamespaceNotFound {
                namespace: self.config.namespace.clone(),
            }),
            _ => Err(response.into_error()),
        }
    }

    /// Returns project `name` under the configured namespace, if it exists.
    async fn fetch_project(&self, name: &str) -> Result<Option<GitLabProject>, MirrorError> {
        // GitLab identifies projects by `namespace/name` (URL-encoded).
        let path = format!("{}/{}", self.config.namespace, name);
        let url = format!(
            "{}/projects/{}",
            self.config.api_base(),
            encode_segment(&path)
        );
        debug!(url = %url, "checking if GitLab project exists");
        let response = self.send(Method::GET, &url, None).await?;
        match response.status {
            200 => Ok(Some(serde_json::from_slice(&response.body)?)),
            404 => Ok(None),
            _ => Err(response.into_error()),
        }
    }
}

impl MirrorDestination for GitLabClient {
    async fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<(), MirrorError> {
        GitLabClient::ensure_repo_exists(self, name, description, private).await
    }
    fn repo_clone_url(&self, name: &str) -> String {
        self.config.repo_clone_url(name)
    }
    fn push_username(&self) -> &'static str {
        "oauth2"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeApi;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    const MARKER: &str = "GitHub mirror of octo/r";

    fn config(base_url: &str) -> GitLabConfig {
        GitLabConfig {
            base_url: base_url.to_owned(),
            token: "glpat-secret".to_owned(),
            namespace: "my-group/sub".to_owned(),
            private: true,
            prune: false,
        }
    }

    #[test]
    fn create_project_request_names_the_namespace_by_id_and_the_path() {
        let req = CreateProjectRequest {
            name: "my-repo",
            path: "my-repo",
            namespace_id: 42,
            description: "Mirror of my-repo",
            visibility: "private",
            initialize_with_readme: false,
        };
        let json: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(json["namespace_id"], 42);
        assert_eq!(json["path"], "my-repo");
        assert_eq!(json["visibility"], "private");
        assert_eq!(json["initialize_with_readme"], false);
        assert!(json.get("namespace_path").is_none());
    }

    #[test]
    fn a_client_is_never_built_without_a_token() {
        let mut cfg = config("https://gitlab.example");
        cfg.token = String::new();
        assert!(matches!(
            GitLabClient::new(cfg).unwrap_err(),
            MirrorError::MissingToken
        ));
    }

    /// A project for a group is created in that group (by namespace id), not
    /// in the token user's own namespace.
    #[tokio::test]
    async fn projects_are_created_in_the_configured_group() {
        let api = FakeApi::start(|seen| match (seen.method.as_str(), seen.target.as_str()) {
            ("GET", "/api/v4/projects/my-group%2Fsub%2Fr") => (404, "{}".into()),
            ("GET", "/api/v4/namespaces/my-group%2Fsub") => (
                200,
                "{\"id\":77,\"kind\":\"group\",\"full_path\":\"my-group/sub\"}".into(),
            ),
            ("POST", "/api/v4/projects") => (201, "{\"id\":1}".into()),
            _ => (500, "{}".into()),
        })
        .await;
        let client = GitLabClient::for_tests(config(&api.base_url));
        client.ensure_repo_exists("r", MARKER, true).await.unwrap();
        let post = api
            .seen()
            .into_iter()
            .find(|s| s.method == "POST")
            .expect("a create request");
        let body: serde_json::Value = serde_json::from_str(&post.body).unwrap();
        assert_eq!(body["namespace_id"], 77, "{body}");
        assert_eq!(body["path"], "r", "{body}");
        assert!(post
            .headers
            .iter()
            .any(|(k, v)| k == "private-token" && v == "glpat-secret"));
    }

    #[tokio::test]
    async fn an_unknown_namespace_is_an_error_and_nothing_is_created() {
        let api = FakeApi::start(|seen| match seen.method.as_str() {
            "GET" => (404, "{\"message\":\"404 Not found\"}".into()),
            _ => (201, "{}".into()),
        })
        .await;
        let client = GitLabClient::for_tests(config(&api.base_url));
        let err = client
            .ensure_repo_exists("r", MARKER, true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, MirrorError::NamespaceNotFound { .. }),
            "{err:?}"
        );
        assert!(api.seen().iter().all(|s| s.method != "POST"));
    }

    /// A `400` is a failure, with GitLab's explanation; only "has already
    /// been taken" followed by finding our own project is a success.
    #[tokio::test]
    async fn a_400_is_an_error_unless_the_project_turns_out_to_be_ours() {
        let cases = [
            ("{\"message\":{\"path\":[\"is invalid\"]}}", None, false),
            (
                "{\"message\":{\"name\":[\"has already been taken\"]}}",
                Some(MARKER),
                true,
            ),
            (
                "{\"message\":{\"name\":[\"has already been taken\"]}}",
                Some("not ours"),
                false,
            ),
            (
                "{\"message\":{\"name\":[\"has already been taken\"]}}",
                None,
                false,
            ),
        ];
        for (reply, after, ok) in cases {
            let lookups = Arc::new(AtomicU32::new(0));
            let counter = Arc::clone(&lookups);
            let api =
                FakeApi::start(
                    move |seen| match (seen.method.as_str(), seen.target.as_str()) {
                        ("GET", "/api/v4/projects/my-group%2Fsub%2Fr") => {
                            match (counter.fetch_add(1, Ordering::SeqCst), after) {
                                (1.., Some(description)) => (
                                    200,
                                    format!(
                                    "{{\"description\":\"{description}\",\"empty_repo\":false}}"
                                ),
                                ),
                                _ => (404, "{}".into()),
                            }
                        }
                        ("GET", "/api/v4/namespaces/my-group%2Fsub") => (200, "{\"id\":7}".into()),
                        ("POST", "/api/v4/projects") => (400, reply.to_owned()),
                        _ => (500, "{}".into()),
                    },
                )
                .await;
            let client = GitLabClient::for_tests(config(&api.base_url));
            let result = client.ensure_repo_exists("r", MARKER, true).await;
            assert_eq!(result.is_ok(), ok, "{reply} / {after:?}: {result:?}");
            if let Err(MirrorError::Api { status, body }) = &result {
                assert_eq!(*status, 400);
                assert_eq!(body, reply);
            }
        }
    }

    /// A transient `5xx` on create is retried.
    #[tokio::test]
    async fn a_server_error_on_create_is_retried() {
        let posts = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&posts);
        let api = FakeApi::start(move |seen| match seen.method.as_str() {
            "GET" if seen.target.contains("/namespaces/") => (200, "{\"id\":7}".into()),
            "GET" => (404, "{}".into()),
            _ => match counter.fetch_add(1, Ordering::SeqCst) {
                0 => (503, "{}".into()),
                _ => (201, "{}".into()),
            },
        })
        .await;
        let client = GitLabClient::for_tests(config(&api.base_url));
        client.ensure_repo_exists("r", MARKER, true).await.unwrap();
        assert_eq!(posts.load(Ordering::SeqCst), 2);
    }
}
