// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub OAuth 2.0 [Device Authorization Flow] (RFC 8628).
//!
//! Allows users to authenticate in the browser without a PAT.  The CLI
//! prints a short user code and a URL; the user visits the URL, enters the
//! code, and authorises the app.  The CLI polls until the token is ready or
//! the session expires.
//!
//! The requests go through the same proxy-aware HTTPS client as the API
//! (`HTTPS_PROXY`, `NO_PROXY`, ...), to the login host of the GitHub instance:
//! `https://github.com` for `https://api.github.com`, `https://HOST` for a
//! GitHub Enterprise Server API at `https://HOST/api/v3`.
//!
//! # Registering an OAuth App
//!
//! You need a GitHub OAuth App to use this flow.  Create one at
//! <https://github.com/settings/developers> (on GitHub Enterprise Server, in
//! that server's developer settings).  You do not need to set a callback URL —
//! the device flow does not use redirects.
//!
//! # Scopes
//!
//! For a full backup (including private repos and gists), request:
//! `repo gist read:org`.
//!
//! [Device Authorization Flow]: https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow

use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{header, Request, StatusCode};
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::time::Instant;
use tracing::{debug, info, warn};
use url::Url;

use crate::client::{collect_body_limited, validate_api_url, USER_AGENT};
use crate::error::ClientError;
use crate::proxy::ProxyClient;

const GITHUB_API_URL: &str = "https://api.github.com";
/// Limit on each OAuth request: on the response headers, and on the silence
/// between two chunks of the body.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// RFC 8628, section 3.5: without an `interval`, poll every 5 seconds; after
/// `slow_down`, 5 seconds more.
const DEFAULT_INTERVAL_SECS: u64 = 5;

// ── Response types ────────────────────────────────────────────────────────────

/// Response from the GitHub device code endpoint.
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

/// What one poll of the token endpoint means.
#[derive(Debug, PartialEq, Eq)]
enum Poll {
    /// The user authorised the app.
    Token(String),
    /// `authorization_pending`: not yet.
    Pending,
    /// `slow_down`, with the new interval if the server names one.
    SlowDown(Option<u64>),
    /// A failure worth polling again for (server error, no connection).
    Transient(String),
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Runs the complete GitHub OAuth Device Flow against github.com and returns
/// an access token.  For GitHub Enterprise Server use [`device_flow_for`].
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
/// See [`device_flow_for`].
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
    device_flow_for(None, client_id, scope, on_code).await
}

/// Runs the device flow on the GitHub instance whose REST API is at
/// `api_url` (`None` for github.com) and returns an access token for it.
///
/// The login host is derived from the API URL: `https://api.HOST` signs in at
/// `https://HOST`, and a GitHub Enterprise Server API at `https://HOST/api/v3`
/// signs in at `https://HOST`.  The proxy environment variables are honoured.
/// A connection failure, a timeout or a 5xx answer while polling is retried
/// until the code expires.
///
/// # Errors
///
/// - [`ClientError::InvalidApiUrl`] when `api_url` is not an `https://` URL
///   whose login host can be derived;
/// - [`ClientError::OAuthExpired`] / [`ClientError::OAuthDenied`] when the
///   code expires or the user refuses;
/// - [`ClientError::OAuth`] for any other OAuth error (with GitHub's
///   description), and [`ClientError`] for a device-code request that fails
///   or an answer the flow does not recognise.
pub async fn device_flow_for<F>(
    api_url: Option<&str>,
    client_id: &str,
    scope: &str,
    on_code: F,
) -> Result<String, ClientError>
where
    F: FnOnce(&str, &str),
{
    let login = login_base(api_url.unwrap_or(GITHUB_API_URL))?;
    let http = ProxyClient::from_env(false)?;
    run(
        &http,
        &login,
        client_id,
        scope,
        on_code,
        Duration::from_secs(1),
    )
    .await
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// The login origin of the GitHub instance whose REST API is at `api_url`.
fn login_base(api_url: &str) -> Result<String, ClientError> {
    validate_api_url(api_url)?;
    let url = Url::parse(api_url)?;
    let host = url.host_str().unwrap_or_default();
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    if let Some(login_host) = host.strip_prefix("api.") {
        return Ok(format!("https://{login_host}{port}"));
    }
    if url.path().trim_end_matches('/').ends_with("/api/v3") {
        return Ok(format!("https://{host}{port}"));
    }
    Err(ClientError::InvalidApiUrl(format!(
        "cannot tell the sign-in host of {api_url:?} for the device flow (expected \
         https://api.HOST or https://HOST/api/v3); create a token on that server and \
         pass it with --token instead"
    )))
}

/// The whole flow against the login origin `login`; `unit` is one second of
/// the intervals the server names (tests shrink it).
async fn run<F>(
    http: &ProxyClient,
    login: &str,
    client_id: &str,
    scope: &str,
    on_code: F,
    unit: Duration,
) -> Result<String, ClientError>
where
    F: FnOnce(&str, &str),
{
    let device = request_device_code(http, login, client_id, scope).await?;
    info!(
        verification_uri = %device.verification_uri,
        expires_in = device.expires_in,
        "device code obtained"
    );
    on_code(&device.user_code, &device.verification_uri);
    let token = poll_for_token(http, login, client_id, &device, unit).await?;
    info!("OAuth device flow complete, token obtained");
    Ok(token)
}

/// Posts to the device code endpoint to start the flow.
async fn request_device_code(
    http: &ProxyClient,
    login: &str,
    client_id: &str,
    scope: &str,
) -> Result<DeviceCodeResponse, ClientError> {
    let url = format!("{login}/login/device/code");
    let (status, body) =
        post_form(http, &url, &[("client_id", client_id), ("scope", scope)]).await?;
    if let Some(error) = oauth_error(&body) {
        return Err(error);
    }
    if !status.is_success() {
        return Err(ClientError::ApiError {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&body).into_owned(),
        });
    }
    Ok(serde_json::from_slice(&body)?)
}

/// Polls the token endpoint until the user authorises, the code expires, or
/// an unrecoverable error occurs.
///
/// Periodically logs a heart-beat with the time remaining so the operator
/// can tell whether the session is still alive (and how long they have left
/// to enter the code).  The sleep between polls is **deadline-aware**: if the
/// next poll would land after the deadline the flow waits until the deadline
/// and then returns [`ClientError::OAuthExpired`].
async fn poll_for_token(
    http: &ProxyClient,
    login: &str,
    client_id: &str,
    device: &DeviceCodeResponse,
    unit: Duration,
) -> Result<String, ClientError> {
    let secs = |n: u64| unit.saturating_mul(u32::try_from(n).unwrap_or(u32::MAX));
    let url = format!("{login}/login/oauth/access_token");
    let mut interval = device
        .interval
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(DEFAULT_INTERVAL_SECS);
    let started = Instant::now();
    let deadline = started + secs(device.expires_in);
    let mut last_heartbeat = started;
    // Log a heart-beat at most every 60 seconds.
    let heartbeat_period = secs(60);

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(ClientError::OAuthExpired);
        }
        // Sleep until the next poll, but never past the deadline, so that
        // the operator gets the explicit "expired" error.
        tokio::time::sleep(secs(interval).min(deadline - now)).await;

        if last_heartbeat.elapsed() >= heartbeat_period {
            info!(
                seconds_remaining = deadline.saturating_duration_since(Instant::now()).as_secs(),
                "still waiting for OAuth authorisation"
            );
            last_heartbeat = Instant::now();
        }

        debug!("polling GitHub for OAuth token");
        let answer = post_form(
            http,
            &url,
            &[
                ("client_id", client_id),
                ("device_code", &device.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
        )
        .await;
        let poll = match answer {
            Ok((status, body)) => interpret_poll(status, &body)?,
            Err(
                e
                @ (ClientError::Transport(_) | ClientError::Body(_) | ClientError::Timeout { .. }),
            ) => Poll::Transient(e.to_string()),
            Err(e) => return Err(e),
        };
        match poll {
            Poll::Token(token) => return Ok(token),
            Poll::Pending => debug!("authorization_pending, continuing to poll"),
            Poll::SlowDown(asked) => {
                interval = (interval + DEFAULT_INTERVAL_SECS).max(asked.unwrap_or(0));
                warn!(
                    new_interval_secs = interval,
                    "GitHub requested slower polling"
                );
            }
            Poll::Transient(why) => warn!(
                error = %why,
                "could not poll the OAuth token endpoint; trying again until the code expires"
            ),
        }
    }
}

/// Interprets one answer of the token endpoint.  GitHub answers `200` both
/// with a token and with an OAuth error; RFC 8628 servers answer an error
/// with `400`.  Both are accepted.
fn interpret_poll(status: StatusCode, body: &[u8]) -> Result<Poll, ClientError> {
    let json: Option<Map<String, Value>> = serde_json::from_slice(body).ok();
    let field = |name: &str| {
        json.as_ref()
            .and_then(|o| o.get(name))
            .and_then(Value::as_str)
    };
    if status.is_success() {
        if let Some(token) = field("access_token").filter(|t| !t.is_empty()) {
            return Ok(Poll::Token(token.to_string()));
        }
    }
    match field("error") {
        Some("authorization_pending") => return Ok(Poll::Pending),
        Some("slow_down") => {
            let interval = json
                .as_ref()
                .and_then(|o| o.get("interval"))
                .and_then(Value::as_u64);
            return Ok(Poll::SlowDown(interval));
        }
        _ => {}
    }
    if let Some(error) = oauth_error(body) {
        return Err(error);
    }
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Ok(Poll::Transient(format!("HTTP {status}")));
    }
    // The body is not quoted: an unexpected success answer could hold a
    // credential.
    Err(ClientError::ApiError {
        status: status.as_u16(),
        body: format!(
            "unexpected answer from the OAuth token endpoint ({} bytes, neither a token \
             nor an OAuth error)",
            body.len()
        ),
    })
}

/// The OAuth error an answer carries, if any (`expired_token` and
/// `access_denied` as their own variants).
fn oauth_error(body: &[u8]) -> Option<ClientError> {
    let json: Map<String, Value> = serde_json::from_slice(body).ok()?;
    let error = json.get("error")?.as_str()?;
    Some(match error {
        "expired_token" => ClientError::OAuthExpired,
        "access_denied" => ClientError::OAuthDenied,
        _ => ClientError::OAuth {
            error: error.to_string(),
            description: json
                .get("error_description")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
    })
}

/// Posts a form to `url` and returns the status and the (bounded) body.
async fn post_form(
    http: &ProxyClient,
    url: &str,
    params: &[(&str, &str)],
) -> Result<(StatusCode, Bytes), ClientError> {
    let req = Request::post(url)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ACCEPT, "application/json")
        .body(Full::new(Bytes::from(form_encode(params))))?;
    let response = tokio::time::timeout(REQUEST_TIMEOUT, http.request(req))
        .await
        .map_err(|_| ClientError::Timeout {
            url: url.to_string(),
        })??;
    let status = response.status();
    let body = collect_body_limited(response.into_body(), REQUEST_TIMEOUT).await?;
    Ok((status, body))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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

    #[test]
    fn the_login_host_follows_the_api_url() {
        assert_eq!(
            login_base("https://api.github.com").unwrap(),
            "https://github.com"
        );
        assert_eq!(
            login_base("https://ghe.example.com/api/v3/").unwrap(),
            "https://ghe.example.com"
        );
        assert_eq!(
            login_base("https://ghe.example.com:8443/api/v3").unwrap(),
            "https://ghe.example.com:8443"
        );
        assert_eq!(
            login_base("https://api.acme.ghe.com").unwrap(),
            "https://acme.ghe.com"
        );
        for bad in ["https://proxy.example.com/github", "http://api.github.com"] {
            assert!(
                matches!(login_base(bad), Err(ClientError::InvalidApiUrl(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn poll_answers_are_told_apart() {
        let ok = StatusCode::OK;
        assert_eq!(
            interpret_poll(
                ok,
                br#"{"access_token":"gho_x","token_type":"bearer","scope":"repo"}"#
            )
            .unwrap(),
            Poll::Token("gho_x".into())
        );
        assert_eq!(
            interpret_poll(ok, br#"{"error":"authorization_pending"}"#).unwrap(),
            Poll::Pending
        );
        // RFC 8628 servers answer errors with 400.
        assert_eq!(
            interpret_poll(
                StatusCode::BAD_REQUEST,
                br#"{"error":"authorization_pending"}"#
            )
            .unwrap(),
            Poll::Pending
        );
        assert_eq!(
            interpret_poll(ok, br#"{"error":"slow_down","interval":10}"#).unwrap(),
            Poll::SlowDown(Some(10))
        );
        assert!(matches!(
            interpret_poll(StatusCode::BAD_GATEWAY, b"<html>").unwrap(),
            Poll::Transient(_)
        ));
        assert!(matches!(
            interpret_poll(ok, br#"{"error":"expired_token"}"#),
            Err(ClientError::OAuthExpired)
        ));
        assert!(matches!(
            interpret_poll(ok, br#"{"error":"access_denied"}"#),
            Err(ClientError::OAuthDenied)
        ));
    }

    #[test]
    fn an_unknown_oauth_error_keeps_its_description() {
        let err = interpret_poll(
            StatusCode::OK,
            br#"{"error":"device_flow_disabled","error_description":"Device Flow must be explicitly enabled for this App"}"#,
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("device_flow_disabled") && text.contains("must be explicitly enabled"),
            "{text}"
        );
    }

    #[test]
    fn an_unrecognised_success_answer_is_an_error_not_another_poll() {
        let err = interpret_poll(StatusCode::OK, br#"{"something":"else"}"#).unwrap_err();
        assert!(
            matches!(err, ClientError::ApiError { status: 200, .. }),
            "{err}"
        );
        assert!(
            !err.to_string().contains("something"),
            "body not quoted: {err}"
        );
    }

    /// What the fake login server does with the n-th token request.
    #[derive(Clone, Copy)]
    enum TokenAct {
        Drop,
        Reply(&'static str),
    }

    /// A fake login server: answers the device code request, then plays
    /// `token_acts` (the last one repeats); records every request target.
    async fn login_server(token_acts: Vec<TokenAct>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            let mut polls = 0usize;
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().expect("log").push(target.clone());
                let act = if target == "/login/device/code" {
                    TokenAct::Reply(
                        r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://ghe.test/login/device","expires_in":900,"interval":5}"#,
                    )
                } else {
                    let act = token_acts[polls.min(token_acts.len() - 1)];
                    polls += 1;
                    act
                };
                match act {
                    TokenAct::Drop => drop(stream),
                    TokenAct::Reply(body) => {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    }
                }
            }
        });
        (base, seen)
    }

    #[tokio::test]
    async fn the_flow_survives_a_lost_connection_and_slow_down_while_polling() {
        let (base, seen) = login_server(vec![
            TokenAct::Drop,
            TokenAct::Reply(r#"{"error":"authorization_pending"}"#),
            TokenAct::Reply(r#"{"error":"slow_down","interval":10}"#),
            TokenAct::Reply(r#"{"access_token":"gho_TOKEN","token_type":"bearer","scope":"repo"}"#),
        ])
        .await;
        let mut shown = None;

        let token = run(
            &ProxyClient::for_tests(),
            &base,
            "client",
            "repo",
            |code, url| shown = Some((code.to_string(), url.to_string())),
            Duration::from_millis(1),
        )
        .await
        .expect("token");

        assert_eq!(token, "gho_TOKEN");
        assert_eq!(
            shown,
            Some(("ABCD-1234".into(), "https://ghe.test/login/device".into()))
        );
        let seen = seen.lock().expect("seen");
        assert_eq!(seen[0], "/login/device/code");
        assert_eq!(seen.len(), 5, "{seen:?}");
        assert!(seen[1..].iter().all(|t| t == "/login/oauth/access_token"));
    }

    #[tokio::test]
    async fn a_device_flow_refused_by_the_server_reports_why() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 8192];
            let _ = stream.read(&mut buf).await;
            let body = r#"{"error":"incorrect_client_credentials","error_description":"The client_id is not valid."}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });

        let err = run(
            &ProxyClient::for_tests(),
            &base,
            "bad",
            "repo",
            |_, _| panic!("no code to show"),
            Duration::from_millis(1),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, ClientError::OAuth { error, description: Some(d) }
                if error == "incorrect_client_credentials" && d.contains("not valid")),
            "{err:?}"
        );
    }
}
