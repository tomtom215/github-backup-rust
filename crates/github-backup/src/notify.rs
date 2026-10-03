// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Webhook notification support.
//!
//! Sends a fire-and-forget HTTP POST to a user-configured URL after the
//! primary backup completes (success or failure).  Notification failures
//! are logged as warnings and never cause the backup process to exit with
//! a non-zero code.
//!
//! # Security
//!
//! The webhook payload contains the backup owner name, counters, and any
//! error message.  Always use an `https://` URL so this data is not
//! transmitted in plaintext.  A warning is emitted when a plain `http://`
//! URL is supplied.

use bytes::Bytes;
use chrono::Utc;
use github_backup_core::Failure;
use http_body_util::Full;
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::{debug, warn};

const NOTIFY_TIMEOUT_SECS: u64 = 15;

/// How many failed items are listed in the payload; the rest are only counted,
/// so a run with thousands of failures cannot produce an unbounded request.
const MAX_LISTED_FAILURES: usize = 20;

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Everything that was asked for succeeded.
    Success,
    /// The run finished but some items could not be backed up.
    Partial,
    /// The run could not be completed.
    Failure,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failure => "failure",
        }
    }
}

/// What the webhook is told about a run.
pub struct Notification<'a> {
    /// How the run ended.
    pub status: Status,
    /// GitHub username or organisation that was backed up.
    pub owner: &'a str,
    /// Why the run failed, for [`Status::Failure`].
    pub error: Option<&'a str>,
    /// Repositories backed up completely.
    pub repos_backed_up: u64,
    /// Repositories with at least one failed step.
    pub repos_errored: u64,
    /// Every recorded failure.
    pub failures: &'a [Failure],
}

/// One failed item in the payload.  The message is left out on purpose: it can
/// be long and the webhook is an external service; the report file has it.
#[derive(serde::Serialize)]
struct FailedItem<'a> {
    scope: &'a str,
    step: &'a str,
}

/// JSON payload sent to the webhook URL.
#[derive(serde::Serialize)]
struct WebhookPayload<'a> {
    /// `"success"`, `"partial"` or `"failure"`.
    status: &'a str,
    /// GitHub username or organisation that was backed up.
    owner: &'a str,
    /// ISO 8601 UTC timestamp of the backup completion.
    timestamp: String,
    /// Human-readable error message, present only when `status == "failure"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    /// Number of repositories backed up completely.
    repos_backed_up: u64,
    /// Number of repositories with at least one failed step.
    repos_errored: u64,
    /// Total number of failures recorded during the run.
    failure_count: u64,
    /// The first failures (at most [`MAX_LISTED_FAILURES`]), without messages.
    failed: Vec<FailedItem<'a>>,
}

impl<'a> WebhookPayload<'a> {
    fn new(n: &Notification<'a>, timestamp: String) -> Self {
        Self {
            status: n.status.as_str(),
            owner: n.owner,
            timestamp,
            error: n.error,
            repos_backed_up: n.repos_backed_up,
            repos_errored: n.repos_errored,
            failure_count: n.failures.len() as u64,
            failed: n
                .failures
                .iter()
                .take(MAX_LISTED_FAILURES)
                .map(|f| FailedItem {
                    scope: &f.scope,
                    step: &f.step,
                })
                .collect(),
        }
    }
}

/// `scheme://host` of `url`: all that may appear in logs, because the path (and
/// any userinfo) of a Slack, Discord or Teams webhook is a bearer secret.
fn display_host(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let host = authority.rsplit('@').next().unwrap_or("");
            format!("{scheme}://{host}")
        }
        None => "<invalid webhook URL>".to_string(),
    }
}

/// Posts a JSON notification to `url`.
///
/// The function is "fire and forget": any error (network, TLS, non-2xx
/// response) is logged at `WARN` level and silently ignored.
///
/// A warning is emitted when `url` uses plain HTTP so operators are aware
/// that backup metadata will be transmitted unencrypted.
pub async fn send_webhook(url: &str, notification: &Notification<'_>) {
    // Warn when the URL is plain HTTP — the payload contains owner name and
    // error messages that should not travel over an unencrypted connection.
    let shown = display_host(url);
    if url.starts_with("http://") {
        warn!(
            url = %shown,
            "webhook URL uses plain HTTP; backup metadata (owner, error messages) \
             will be transmitted unencrypted. Use an https:// URL to protect this data."
        );
    }

    let payload = WebhookPayload::new(notification, utc_now_iso8601());

    let body_bytes = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "failed to serialise webhook payload");
            return;
        }
    };

    match send_post(url, body_bytes).await {
        Ok(status_code) if status_code.is_success() => {
            debug!(url = %shown, http_status = %status_code, "webhook notification sent");
        }
        Ok(status_code) => {
            warn!(url = %shown, http_status = %status_code, "webhook notification returned non-2xx status");
        }
        Err(e) => {
            warn!(url = %shown, error = %e, "webhook notification failed");
        }
    }
}

/// Sends an HTTP POST request with a JSON body to `url`.
async fn send_post(url: &str, body: Vec<u8>) -> Result<StatusCode, String> {
    let http = build_client()?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(url)
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            concat!("github-backup-rust/", env!("CARGO_PKG_VERSION")),
        )
        .header("Content-Length", body.len().to_string())
        .body(Full::new(Bytes::from(body)))
        .map_err(|e| format!("build request: {e}"))?;

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(NOTIFY_TIMEOUT_SECS),
        http.request(req),
    )
    .await
    .map_err(|_| {
        format!(
            "webhook POST to {} timed out after {NOTIFY_TIMEOUT_SECS}s",
            display_host(url)
        )
    })?
    .map_err(|e: hyper_util::client::legacy::Error| format!("HTTP error: {e}"))?;

    Ok(response.status())
}

/// Builds a hyper HTTPS client using the system native CA bundle.
fn build_client() -> Result<
    Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        Full<Bytes>,
    >,
    String,
> {
    let mut root_store = rustls::RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    if cert_result.certs.is_empty() {
        return Err(format!(
            "no CA certificates found: {}",
            cert_result
                .errors
                .first()
                .map(|e| e.to_string())
                .unwrap_or_default()
        ));
    }
    root_store.add_parsable_certificates(cert_result.certs);
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .build();

    Ok(Client::builder(TokioExecutor::new()).build(https))
}

/// Returns the current UTC time as an ISO 8601 string (`YYYY-MM-DDTHH:MM:SSZ`).
fn utc_now_iso8601() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_now_iso8601_format() {
        let ts = utc_now_iso8601();
        // Should match YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(ts.len(), 20);
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[19..20], "Z");
    }

    fn failure(scope: &str, step: &str) -> Failure {
        Failure {
            scope: scope.to_string(),
            step: step.to_string(),
            message: "a long and possibly sensitive stderr tail".to_string(),
        }
    }

    fn payload_json(n: &Notification<'_>) -> serde_json::Value {
        serde_json::to_value(WebhookPayload::new(n, "2025-03-30T12:00:00Z".into())).expect("json")
    }

    #[test]
    fn payload_serialises_failure_with_the_error() {
        let n = Notification {
            status: Status::Failure,
            owner: "octocat",
            error: Some("backup failed: rate limit"),
            repos_backed_up: 0,
            repos_errored: 3,
            failures: &[],
        };
        let v = payload_json(&n);
        assert_eq!(v["status"], "failure");
        assert_eq!(v["error"], "backup failed: rate limit");
        assert_eq!(v["repos_errored"], 3);
    }

    #[test]
    fn payload_omits_error_on_success() {
        let n = Notification {
            status: Status::Success,
            owner: "octocat",
            error: None,
            repos_backed_up: 42,
            repos_errored: 0,
            failures: &[],
        };
        let v = payload_json(&n);
        assert_eq!(v["status"], "success");
        assert!(v.get("error").is_none());
        assert_eq!(v["repos_backed_up"], 42);
        assert_eq!(v["failure_count"], 0);
        assert_eq!(v["failed"], serde_json::json!([]));
    }

    /// A run that lost data must not be announced as a success.
    #[test]
    fn a_partial_run_is_reported_as_partial_with_what_failed() {
        let failures = vec![failure("octocat/a", "wiki"), failure("octocat/b", "issues")];
        let n = Notification {
            status: Status::Partial,
            owner: "octocat",
            error: None,
            repos_backed_up: 8,
            repos_errored: 2,
            failures: &failures,
        };
        let v = payload_json(&n);
        assert_eq!(v["status"], "partial");
        assert_eq!(v["failure_count"], 2);
        assert_eq!(v["failed"][0]["scope"], "octocat/a");
        assert_eq!(v["failed"][1]["step"], "issues");
        let text = v.to_string();
        assert!(
            !text.contains("sensitive stderr"),
            "failure messages must stay out of the webhook: {text}"
        );
    }

    #[test]
    fn the_failure_list_is_capped_but_the_count_is_exact() {
        let failures: Vec<Failure> = (0..MAX_LISTED_FAILURES + 30)
            .map(|i| failure(&format!("o/r{i}"), "clone"))
            .collect();
        let n = Notification {
            status: Status::Partial,
            owner: "o",
            error: None,
            repos_backed_up: 0,
            repos_errored: failures.len() as u64,
            failures: &failures,
        };
        let v = payload_json(&n);
        assert_eq!(v["failure_count"], (MAX_LISTED_FAILURES + 30) as u64);
        assert_eq!(
            v["failed"].as_array().expect("array").len(),
            MAX_LISTED_FAILURES
        );
    }

    #[test]
    fn only_scheme_and_host_of_a_webhook_url_are_ever_shown() {
        assert_eq!(
            display_host("https://hooks.slack.com/services/T000/B000/SECRETSECRET"),
            "https://hooks.slack.com"
        );
        assert_eq!(
            display_host("https://user:pw@host.example:8443/path?token=x#f"),
            "https://host.example:8443"
        );
        assert_eq!(display_host("not a url"), "<invalid webhook URL>");
    }
}
