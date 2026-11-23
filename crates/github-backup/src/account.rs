// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Who is being backed up: the account the token belongs to (the default
//! OWNER) and whether OWNER is a user or an organisation.
//!
//! Two small `GET` requests made before the run by the API client itself
//! ([`GitHubClient::account_login`], [`GitHubClient::account_kind`]), with
//! its proxy, TLS, retry and rate-limit handling.

use github_backup_client::{ClientError, GitHubClient};
use github_backup_types::config::Credential;

pub(crate) use github_backup_client::AccountKind;

/// The API base used when `--api-url` is not given.
pub(crate) const DEFAULT_API_URL: &str = "https://api.github.com";

/// The account look-ups, with errors worded for the log.
pub(crate) struct Api {
    client: GitHubClient,
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
        Ok(Self {
            client: GitHubClient::with_api_url(credential.clone(), base)
                .map_err(|e| e.to_string())?,
        })
    }

    /// A plain-HTTP client for tests against a local server.
    #[cfg(test)]
    pub(crate) fn for_tests(base: &str, credential: &Credential) -> Self {
        Self {
            client: GitHubClient::for_tests_at(credential.clone(), base),
        }
    }

    /// The login of the account the token belongs to (`GET /user`).
    ///
    /// # Errors
    ///
    /// When the token is refused there (a GitHub App installation token cannot
    /// use it) or the API cannot be reached.
    pub(crate) async fn authenticated_login(&self) -> Result<String, String> {
        self.client
            .account_login()
            .await
            .map_err(|e| describe("/user", &e))
    }

    /// Whether `owner` is a user or an organisation; `None` if it does not
    /// exist (`GET /users/{owner}` answers 404).
    ///
    /// # Errors
    ///
    /// Any other unexpected answer, or the API cannot be reached.
    pub(crate) async fn account_kind(&self, owner: &str) -> Result<Option<AccountKind>, String> {
        self.client
            .account_kind(owner)
            .await
            .map_err(|e| describe(&format!("/users/{owner}"), &e))
    }
}

/// `GET <path> answered HTTP <status>: <message>` for an error answer, the
/// client's own message otherwise.
fn describe(path: &str, e: &ClientError) -> String {
    match e {
        ClientError::ApiError { status, body } => {
            let message = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .map(|m| format!(": {}", m.chars().take(120).collect::<String>()))
                .unwrap_or_default();
            format!("GET {path} answered HTTP {status}{message}")
        }
        e => e.to_string(),
    }
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

    /// The look-ups share the API client's retry policy: a `502` is retried,
    /// not reported as the token's account being unknown.
    #[tokio::test]
    async fn a_transient_server_error_is_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            for (status, body) in [(502, "{}"), (200, r#"{"login":"alice"}"#)] {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 4096];
                let _ = s.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(reply.as_bytes()).await;
                let _ = s.shutdown().await;
            }
        });
        let api = Api::for_tests(&base, &Credential::Token("t".into()));
        assert_eq!(api.authenticated_login().await.unwrap(), "alice");
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
