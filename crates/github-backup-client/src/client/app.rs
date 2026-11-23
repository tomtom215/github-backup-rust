// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The GitHub App requests: finding the installation, minting installation
//! tokens, and listing the repositories an installation can reach.
//!
//! Endpoints (GitHub Docs, REST API "GitHub Apps" and "Installations"):
//! `GET /app`, `GET /app/installations/{id}`, `GET /orgs/{org}/installation`,
//! `GET /users/{username}/installation` and
//! `POST /app/installations/{id}/access_tokens` take the App's JWT;
//! `GET /installation/repositories` takes an installation token and lists
//! every repository the installation can access, private ones included.

use std::collections::BTreeMap;
use std::sync::Arc;

use hyper::Method;
use serde_json::Value;
use tokio::time::Instant;
use tracing::debug;

use github_backup_types::{Page, Repository};

use crate::api_client::BoxFuture;
use crate::app_auth::{AppAuth, Minted};
use crate::error::ClientError;
use crate::token::{GitToken, TokenSource};

use super::endpoints::without_clone_tokens;
use super::{unix_now, Call, GitHubClient, PER_PAGE};

/// A GitHub App installation, as GitHub describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    /// The installation ID.
    pub id: u64,
    /// The login of the account the App is installed on.
    pub account: String,
    /// Whether that account is an organisation.
    pub organization: bool,
    /// `all` or `selected`: whether the installation reaches every repository
    /// of the account or only those chosen when it was installed.
    pub repository_selection: String,
    /// The permissions granted, by name (`contents`, `issues`, ...), with
    /// their level (`read`, `write`, `admin`).
    pub permissions: BTreeMap<String, String>,
}

impl Installation {
    fn from_json(value: &Value) -> Result<Self, ClientError> {
        let missing = |what: &str| ClientError::App(format!("the installation has no {what}"));
        Ok(Self {
            id: value["id"].as_u64().ok_or_else(|| missing("id"))?,
            account: value["account"]["login"]
                .as_str()
                .ok_or_else(|| missing("account"))?
                .to_string(),
            organization: value["target_type"] == "Organization"
                || value["account"]["type"] == "Organization",
            repository_selection: value["repository_selection"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            permissions: value["permissions"]
                .as_object()
                .map(|perms| {
                    perms
                        .iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// The installation tokens of a client, as a [`TokenSource`] for git.
struct InstallationTokens(GitHubClient);

impl TokenSource for InstallationTokens {
    fn token(&self) -> BoxFuture<'_, Result<String, ClientError>> {
        self.0.installation_token()
    }
}

impl GitHubClient {
    fn app_auth(&self) -> Result<&AppAuth, ClientError> {
        self.app
            .as_deref()
            .ok_or_else(|| ClientError::App("the credential is not a GitHub App".into()))
    }

    /// `GET {api}{path}` authenticated as the App itself (with its JWT).
    async fn get_as_app(&self, path: &str) -> Result<Value, ClientError> {
        let jwt = self.app_auth()?.jwt()?;
        let url = format!("{}{path}", self.api_base);
        let call = Call {
            bearer: Some(&jwt),
            ..Call::get(&url, &[])
        };
        let (body, _) = self.execute(&call).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// The name of the App the credential belongs to (`GET /app`).  Succeeds
    /// only when GitHub accepts the App's JWT.
    ///
    /// # Errors
    ///
    /// [`ClientError::App`] for a credential that is not a GitHub App;
    /// [`ClientError::ApiError`] with status 401 when the JWT is refused (a
    /// wrong App ID or key, or a local clock that is off by minutes).
    pub async fn app_name(&self) -> Result<String, ClientError> {
        let app = self.get_as_app("/app").await?;
        Ok(app["name"]
            .as_str()
            .or_else(|| app["slug"].as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// The installation the credential acts as: the configured installation
    /// ID, or else the App's installation on `owner`, looked up as an
    /// organisation and then as a user.
    ///
    /// # Errors
    ///
    /// [`ClientError::App`] when the credential is not a GitHub App, when
    /// neither an installation ID nor `owner` is given, or when the App is not
    /// installed on `owner`; API errors as for any request.
    pub async fn app_installation(&self, owner: Option<&str>) -> Result<Installation, ClientError> {
        if let Some(id) = self.app_auth()?.installation_id {
            let value = self.get_as_app(&format!("/app/installations/{id}")).await?;
            return Installation::from_json(&value);
        }
        let owner = owner.ok_or_else(|| {
            ClientError::App(
                "neither an installation ID nor OWNER is given, so the installation is unknown"
                    .into(),
            )
        })?;
        for path in [
            format!("/orgs/{owner}/installation"),
            format!("/users/{owner}/installation"),
        ] {
            match self.get_as_app(&path).await {
                Ok(value) => return Installation::from_json(&value),
                Err(ClientError::ApiError { status: 404, .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Err(ClientError::App(format!(
            "the App is not installed on the account {owner}; install it there \
             (the App's settings page, Install App)"
        )))
    }

    /// The current installation token, minted on first use and renewed when
    /// less than [`REFRESH_MARGIN`](crate::app_auth::REFRESH_MARGIN) is left
    /// of it.
    ///
    /// Single-flight: callers that arrive while a token is being minted wait
    /// for it and then share it, so concurrent workers never mint one each.
    pub(crate) fn installation_token(&self) -> BoxFuture<'_, Result<String, ClientError>> {
        // Boxed: minting sends a request, whose sending asks for the
        // credential again (answered by the JWT, so no real recursion).
        Box::pin(async move {
            let app = self.app_auth()?;
            let mut current = app.current.lock().await;
            if let Some(minted) = current.as_ref().filter(|m| m.is_fresh()) {
                return Ok(minted.token.to_string());
            }
            let id = app.installation_id.ok_or_else(|| {
                ClientError::App("no installation chosen to mint a token for".into())
            })?;
            let jwt = app.jwt()?;
            let url = format!("{}/app/installations/{id}/access_tokens", self.api_base);
            let call = Call {
                method: Method::POST,
                bearer: Some(&jwt),
                // Minting twice only yields two tokens: safe to repeat after a
                // lost connection or a 5xx.
                idempotent: true,
                ..Call::get(&url, &[])
            };
            let requested_at = Instant::now();
            let (body, _) = self.execute(&call).await?;
            let minted = Minted::from_response(&body, requested_at, unix_now())?;
            debug!(installation = id, "minted a GitHub App installation token");
            let token = minted.token.to_string();
            *current = Some(minted);
            Ok(token)
        })
    }

    /// The credential for `git`: the fixed token, a source of fresh
    /// installation tokens for a GitHub App, `None` without a credential.
    #[must_use]
    pub fn git_token(&self) -> Option<GitToken> {
        if self.app.is_some() {
            Some(GitToken::Refreshed(Arc::new(InstallationTokens(
                self.clone(),
            ))))
        } else {
            self.token().map(GitToken::Fixed)
        }
    }

    /// Whether the credential is a GitHub App.
    pub(crate) fn is_app(&self) -> bool {
        self.app.is_some()
    }

    /// The repositories of `owner` that the App installation can access
    /// (`GET /installation/repositories`), private ones included.  An
    /// installation limited to selected repositories lists only those.
    pub(crate) async fn list_installation_repos(
        &self,
        owner: &str,
    ) -> Result<Page<Repository>, ClientError> {
        let url = format!(
            "{}/installation/repositories?per_page={PER_PAGE}",
            self.api_base
        );
        let all: Page<Repository> = self.get_all_wrapped_pages(&url, "repositories").await?;
        let unparsed = all.unparsed().to_vec();
        let owned = all
            .into_items()
            .into_iter()
            .map(github_backup_types::Raw::into_json)
            .chain(unparsed)
            .filter(|repo| {
                repo["owner"]["login"]
                    .as_str()
                    .is_some_and(|login| login.eq_ignore_ascii_case(owner))
            });
        Ok(without_clone_tokens(Page::from_values(owned)))
    }
}
