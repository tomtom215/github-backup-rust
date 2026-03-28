// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub OAuth 2.0 [Device Authorization Flow].
//!
//! Allows users to authenticate in the browser without a PAT.  The CLI
//! prints a short user code and a URL; the user visits the URL, enters the
//! code, and authorises the app.  The CLI polls until the token is ready or
//! the session expires.
//!
//! # Registering an OAuth App
//!
//! You need a GitHub OAuth App to use this flow.  Create one at
//! <https://github.com/settings/developers>.  You do not need to set a
//! callback URL — the device flow does not use redirects.
//!
//! # Scopes
//!
//! For a full backup (including private repos and gists), request:
//! `repo gist read:org`.
//!
//! [Device Authorization Flow]: https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::error::ClientError;

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));

type HyperClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

// ── Response types ────────────────────────────────────────────────────────────

/// Response from the GitHub device code endpoint.
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// Successful access token response.
#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    #[allow(dead_code)]
    scope: String,
}

/// Error response when polling for an access token.
#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
    #[allow(dead_code)]
    error_description: Option<String>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Runs the complete GitHub OAuth Device Flow and returns an access token.
///
/// # Arguments
///
/// - `client_id`: Your GitHub OAuth App's client ID.
/// - `scope`: Space-separated OAuth scopes (e.g., `"repo gist read:org"`).
/// - `on_code`: A callback invoked with the user code and verification URL.
///   Use it to display instructions to the user.
///
/// # Errors
///
/// Returns [`ClientError`] if the device code cannot be obtained, if the
/// token expires, or if the user denies the request.
///
/// # Example
///
/// ```no_run
/// use github_backup_client::oauth::device_flow;
///
/// # async fn example() -> Result<(), github_backup_client::ClientError> {
/// let token = device_flow(
///     "your_client_id",
///     "repo gist",
///     |code, url| {
///         eprintln!("Open {url} and enter code: {code}");
///     },
/// ).await?;
/// println!("Got token: {token}");
/// # Ok(())
/// # }
/// ```
pub async fn device_flow<F>(client_id: &str, scope: &str, on_code: F) -> Result<String, ClientError>
where
    F: FnOnce(&str, &str),
{
    let http = build_oauth_client()?;

    // Step 1: Request device code.
    let device_resp = request_device_code(&http, client_id, scope).await?;

    info!(
        user_code = %device_resp.user_code,
        verification_uri = %device_resp.verification_uri,
        expires_in = device_resp.expires_in,
        "device code obtained"
    );

    // Notify the caller so they can display the code to the user.
    on_code(&device_resp.user_code, &device_resp.verification_uri);

    // Step 2: Poll for access token.
    let token = poll_for_token(
        &http,
        client_id,
        &device_resp.device_code,
        device_resp.interval,
        device_resp.expires_in,
    )
    .await?;

    info!("OAuth device flow complete, token obtained");
    Ok(token)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Posts to the GitHub device code endpoint to initiate the flow.
async fn request_device_code(
    http: &HyperClient,
    client_id: &str,
    scope: &str,
) -> Result<DeviceCodeResponse, ClientError> {
    let body = form_encode(&[("client_id", client_id), ("scope", scope)]);

    let req = Request::builder()
        .method(Method::POST)
        .uri(DEVICE_CODE_URL)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(Full::new(Bytes::from(body)))
        .map_err(ClientError::Http)?;

    let response = tokio::time::timeout(Duration::from_secs(30), http.request(req))
        .await
        .map_err(|_| ClientError::Timeout {
            url: DEVICE_CODE_URL.to_string(),
        })??;

    let status = response.status();
    let body_bytes = collect_body(response.into_body()).await?;

    if !status.is_success() {
        return Err(ClientError::ApiError {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&body_bytes).into_owned(),
        });
    }

    let resp: DeviceCodeResponse = serde_json::from_slice(&body_bytes)?;
    Ok(resp)
}

/// Polls the token endpoint until the user authorises, the token expires, or
/// an unrecoverable error occurs.
async fn poll_for_token(
    http: &HyperClient,
    client_id: &str,
    device_code: &str,
    interval_secs: u64,
    expires_in_secs: u64,
) -> Result<String, ClientError> {
    let mut poll_interval = Duration::from_secs(interval_secs.max(5));
    let deadline = std::time::Instant::now() + Duration::from_secs(expires_in_secs);

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(ClientError::OAuthExpired);
        }

        tokio::time::sleep(poll_interval).await;

        debug!("polling GitHub for OAuth token");

        let body = form_encode(&[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ]);

        let req = Request::builder()
            .method(Method::POST)
            .uri(ACCESS_TOKEN_URL)
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(ClientError::Http)?;

        let response = tokio::time::timeout(Duration::from_secs(30), http.request(req))
            .await
            .map_err(|_| ClientError::Timeout {
                url: ACCESS_TOKEN_URL.to_string(),
            })??;

        let status = response.status();
        let body_bytes = collect_body(response.into_body()).await?;

        if status != StatusCode::OK {
            return Err(ClientError::ApiError {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body_bytes).into_owned(),
            });
        }

        // The body may contain either a success or a known error.
        // Try access token first, then error response.
        if let Ok(token_resp) = serde_json::from_slice::<AccessTokenResponse>(&body_bytes) {
            if !token_resp.access_token.is_empty() {
                return Ok(token_resp.access_token);
            }
        }

        if let Ok(err_resp) = serde_json::from_slice::<TokenErrorResponse>(&body_bytes) {
            match err_resp.error.as_str() {
                "authorization_pending" => {
                    // Normal — user has not yet authorised.
                    debug!("authorization_pending, continuing to poll");
                }
                "slow_down" => {
                    // GitHub requests we increase the poll interval.
                    poll_interval += Duration::from_secs(5);
                    warn!(
                        new_interval_secs = poll_interval.as_secs(),
                        "GitHub requested slower polling"
                    );
                }
                "expired_token" => return Err(ClientError::OAuthExpired),
                "access_denied" => return Err(ClientError::OAuthDenied),
                other => {
                    return Err(ClientError::ApiError {
                        status: 200,
                        body: format!("OAuth error: {other}"),
                    });
                }
            }
        }
    }
}

/// URL-encodes a list of key-value pairs as an `application/x-www-form-urlencoded`
/// body.
fn form_encode(params: &[(&str, &str)]) -> Vec<u8> {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

/// Percent-encodes a string (RFC 3986 unreserved characters are not encoded).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Collects a hyper response body into bytes.
async fn collect_body(
    body: impl hyper::body::Body<Data = Bytes, Error = hyper::Error>,
) -> Result<Bytes, ClientError> {
    Ok(body.collect().await?.to_bytes())
}

/// Builds a minimal HTTPS client for the OAuth endpoints.
fn build_oauth_client() -> Result<HyperClient, ClientError> {
    let mut root_store = rustls::RootCertStore::empty();
    let certs = rustls_native_certs::load_native_certs();
    if certs.certs.is_empty() {
        let msg = certs
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no CA certificates found".to_string());
        return Err(ClientError::Tls(msg));
    }
    root_store.add_parsable_certificates(certs.certs);
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_only()
        .enable_http1()
        .build();

    Ok(Client::builder(TokioExecutor::new()).build(https))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encode_encodes_basic_params() {
        let body = form_encode(&[("client_id", "abc123"), ("scope", "repo gist")]);
        let body_str = String::from_utf8(body).unwrap();
        assert!(body_str.contains("client_id=abc123"));
        assert!(body_str.contains("scope=repo%20gist"));
    }

    #[test]
    fn percent_encode_leaves_unreserved_chars_unchanged() {
        assert_eq!(
            percent_encode("hello-world_foo.bar~baz"),
            "hello-world_foo.bar~baz"
        );
    }

    #[test]
    fn percent_encode_encodes_spaces_and_colons() {
        let encoded = percent_encode("repo gist:read");
        assert!(encoded.contains("%20"), "space should be %20");
        assert!(encoded.contains("%3A"), "colon should be %3A");
    }

    #[test]
    fn form_encode_multiple_params() {
        let body = form_encode(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", "abc"),
        ]);
        let body_str = String::from_utf8(body).unwrap();
        assert!(body_str.contains('&'), "params should be separated by &");
    }
}
