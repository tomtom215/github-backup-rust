// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Who is being backed up: the account the token belongs to (the default
//! OWNER) and whether OWNER is a user or an organisation.
//!
//! Two small `GET` requests made before the run, through the same proxy and
//! TLS set-up as the API client.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use hyper::{Method, Request};
use serde_json::Value;

use github_backup_client::{GitHubClient, ProxyClient};
use github_backup_types::config::Credential;

/// The API base used when `--api-url` is not given.
pub(crate) const DEFAULT_API_URL: &str = "https://api.github.com";

/// Limit on each request (headers and body).
const TIMEOUT: Duration = Duration::from_secs(20);

/// Largest body read; profiles are a few kilobytes.
const MAX_BODY: usize = 1 << 20;

/// What kind of account OWNER is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountKind {
    User,
    Organization,
}

/// A minimal authenticated client for the profile endpoints.
pub(crate) struct Api {
    http: ProxyClient,
    base: String,
    authorization: Option<String>,
}

impl Api {
    /// A client for `api_url` (default: api.github.com).
    ///
    /// # Errors
    ///
    /// The URL is rejected unless it is `https://` (the API client's own rule,
    /// so the token never travels in clear text), or the CA bundle is missing.
    pub(crate) fn new(credential: &Credential, api_url: Option<&str>) -> Result<Self, String> {
        let base = api_url.unwrap_or(DEFAULT_API_URL);
        GitHubClient::with_api_url(Credential::Anonymous, base).map_err(|e| e.to_string())?;
        Ok(Self {
            http: ProxyClient::from_env(false).map_err(|e| e.to_string())?,
            base: base.trim_end_matches('/').to_string(),
            authorization: credential.authorization_header(),
        })
    }

    /// A plain-HTTP client for tests against a local server.
    #[cfg(test)]
    pub(crate) fn for_tests(base: &str, credential: &Credential) -> Self {
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        Self {
            http: ProxyClient::new(Default::default(), tls, true),
            base: base.to_string(),
            authorization: credential.authorization_header(),
        }
    }

    /// `GET {base}{path}`: the status and the JSON body (`Null` if none).
    async fn get(&self, path: &str) -> Result<(u16, Value), String> {
        let mut req = Request::builder()
            .method(Method::GET)
            .uri(format!("{}{path}", self.base))
            .header("Accept", "application/vnd.github+json")
            .header(
                "User-Agent",
                concat!("github-backup-rust/", env!("CARGO_PKG_VERSION")),
            );
        if let Some(auth) = &self.authorization {
            req = req.header("Authorization", auth);
        }
        let req = req
            .body(Full::new(Bytes::new()))
            .map_err(|e| e.to_string())?;
        let fetch = async {
            let response = self.http.request(req).await.map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let body = http_body_util::Limited::new(response.into_body(), MAX_BODY)
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            Ok::<_, String>((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
        };
        tokio::time::timeout(TIMEOUT, fetch).await.map_err(|_| {
            format!(
                "{}{path}: no answer within {}s",
                self.base,
                TIMEOUT.as_secs()
            )
        })?
    }

    /// The login of the account the token belongs to (`GET /user`).
    ///
    /// # Errors
    ///
    /// When the token is refused there (a GitHub App installation token cannot
    /// use it) or the API cannot be reached.
    pub(crate) async fn authenticated_login(&self) -> Result<String, String> {
        match self.get("/user").await? {
            (200, body) => body["login"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "GET /user returned no login".to_string()),
            (status, body) => Err(format!(
                "GET /user answered HTTP {status}{}",
                message_suffix(&body)
            )),
        }
    }

    /// Whether `owner` is a user or an organisation; `None` if it does not
    /// exist (`GET /users/{owner}` answers 404).
    ///
    /// # Errors
    ///
    /// Any other unexpected answer, or the API cannot be reached.
    pub(crate) async fn account_kind(&self, owner: &str) -> Result<Option<AccountKind>, String> {
        match self.get(&format!("/users/{owner}")).await? {
            (200, body) => Ok(Some(if body["type"] == "Organization" {
                AccountKind::Organization
            } else {
                AccountKind::User
            })),
            (404, _) => Ok(None),
            (status, body) => Err(format!(
                "GET /users/{owner} answered HTTP {status}{}",
                message_suffix(&body)
            )),
        }
    }
}

/// `: <message>` from a GitHub error body, or nothing.
fn message_suffix(body: &Value) -> String {
    body["message"]
        .as_str()
        .map(|m| format!(": {}", m.chars().take(120).collect::<String>()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves one canned answer per request path; records the request heads.
    async fn server(
        routes: &'static [(&'static str, u16, &'static str)],
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(head);
                let (status, body) = routes
                    .iter()
                    .find(|(p, _, _)| *p == path)
                    .map_or((404, "{\"message\":\"Not Found\"}"), |(_, s, b)| (*s, *b));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(reply.as_bytes()).await;
                let _ = s.shutdown().await;
            }
        });
        (base, seen)
    }

    #[tokio::test]
    async fn the_login_of_the_token_is_found_with_the_token_sent() {
        let (base, seen) = server(&[("/user", 200, r#"{"login":"alice","type":"User"}"#)]).await;
        let api = Api::for_tests(&base, &Credential::Token("ghp_secret".into()));
        assert_eq!(api.authenticated_login().await.unwrap(), "alice");
        let head = seen.lock().unwrap()[0].to_ascii_lowercase();
        assert!(head.contains("authorization: bearer ghp_secret"), "{head}");
    }

    #[tokio::test]
    async fn a_refused_token_is_an_error_with_githubs_message() {
        let (base, _) = server(&[("/user", 401, r#"{"message":"Bad credentials"}"#)]).await;
        let api = Api::for_tests(&base, &Credential::Token("x".into()));
        let e = api.authenticated_login().await.unwrap_err();
        assert!(e.contains("401") && e.contains("Bad credentials"), "{e}");
    }

    #[tokio::test]
    async fn users_organisations_and_missing_accounts_are_told_apart() {
        let (base, _) = server(&[
            ("/users/alice", 200, r#"{"login":"alice","type":"User"}"#),
            (
                "/users/acme",
                200,
                r#"{"login":"acme","type":"Organization"}"#,
            ),
        ])
        .await;
        let api = Api::for_tests(&base, &Credential::Anonymous);
        assert_eq!(
            api.account_kind("alice").await.unwrap(),
            Some(AccountKind::User)
        );
        assert_eq!(
            api.account_kind("acme").await.unwrap(),
            Some(AccountKind::Organization)
        );
        assert_eq!(api.account_kind("nobody").await.unwrap(), None);
    }

    #[test]
    fn plain_http_api_urls_are_refused_before_any_request() {
        let e = Api::new(
            &Credential::Token("t".into()),
            Some("http://ghe.example/api/v3"),
        );
        assert!(e.is_err());
    }
}
