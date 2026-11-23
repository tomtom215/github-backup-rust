// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Webhook notification support.
//!
//! Sends a fire-and-forget HTTP POST to a user-configured URL after the
//! primary backup completes (success or failure).  Notification failures
//! are logged as warnings and never cause the backup process to exit with
//! a non-zero code.
//!
//! # Formats ([`NotifyFormat`])
//!
//! - `json`: this tool's own object (see `WebhookPayload`).
//! - `slack`: `{"text": "..."}`, what a Slack incoming webhook requires.
//! - `discord`: `{"content": "..."}` (at most 2000 characters), what a Discord
//!   webhook requires.
//! - `healthchecks`: a POST to the URL on success and to `<URL>/fail`
//!   otherwise, with the same text as body (healthchecks.io stores it).
//!
//! # Security
//!
//! The webhook payload contains the backup owner name, counters, and any
//! error message.  Always use an `https://` URL so this data is not
//! transmitted in plaintext.  A warning is emitted when a plain `http://`
//! URL is supplied.

use bytes::Bytes;
use chrono::Utc;
use github_backup_client::ProxyClient;
use github_backup_core::Failure;
use http_body_util::Full;
use hyper::{Method, Request, StatusCode};
use tracing::{debug, warn};

const NOTIFY_TIMEOUT_SECS: u64 = 15;

/// How many failed items are listed in the payload; the rest are only counted,
/// so a run with thousands of failures cannot produce an unbounded request.
const MAX_LISTED_FAILURES: usize = 20;

/// Longest Discord message `content`.
const DISCORD_MAX_CHARS: usize = 2000;

/// The body format of the notification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum NotifyFormat {
    /// This tool's JSON object.
    #[default]
    Json,
    /// A Slack incoming-webhook message.
    Slack,
    /// A Discord webhook message.
    Discord,
    /// A healthchecks.io-style success / `/fail` ping.
    Healthchecks,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Everything that was asked for succeeded.
    Success,
    /// The run finished but some items could not be backed up.
    Partial,
    /// The run could not be completed.
    Failure,
    /// The run was stopped by SIGINT / SIGTERM.
    Interrupted,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failure => "failure",
            Self::Interrupted => "interrupted",
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

/// A short, human-readable account of the run for chat messages and pings.
/// Like the JSON payload it names failed items but not their messages.
fn summary_text(n: &Notification<'_>) -> String {
    use std::fmt::Write as _;
    let mut text = match n.status {
        Status::Success => format!(
            "github-backup: backup of {} succeeded ({} repositories).",
            n.owner, n.repos_backed_up
        ),
        Status::Partial => format!(
            "github-backup: backup of {} is INCOMPLETE: {} item(s) could not be backed up \
             ({} repositories complete, {} incomplete).",
            n.owner,
            n.failures.len(),
            n.repos_backed_up,
            n.repos_errored
        ),
        Status::Failure => format!(
            "github-backup: backup of {} FAILED: {}",
            n.owner,
            n.error.unwrap_or("the run could not be carried out")
        ),
        Status::Interrupted => format!(
            "github-backup: backup of {} was INTERRUPTED before it finished.",
            n.owner
        ),
    };
    for f in n.failures.iter().take(MAX_LISTED_FAILURES) {
        let _ = write!(text, "\n- {}: {}", f.scope, f.step);
    }
    if n.failures.len() > MAX_LISTED_FAILURES {
        let _ = write!(
            text,
            "\n- ... and {} more",
            n.failures.len() - MAX_LISTED_FAILURES
        );
    }
    text
}

/// The URL to send to, the body and its content type, for `format`.
fn request_for(
    format: NotifyFormat,
    url: &str,
    n: &Notification<'_>,
    timestamp: String,
) -> Result<(String, Vec<u8>, &'static str), serde_json::Error> {
    const JSON: &str = "application/json";
    Ok(match format {
        NotifyFormat::Json => (
            url.to_string(),
            serde_json::to_vec(&WebhookPayload::new(n, timestamp))?,
            JSON,
        ),
        NotifyFormat::Slack => (
            url.to_string(),
            serde_json::to_vec(&serde_json::json!({ "text": summary_text(n) }))?,
            JSON,
        ),
        NotifyFormat::Discord => {
            let text = summary_text(n);
            let content = if text.chars().count() > DISCORD_MAX_CHARS {
                let cut: String = text.chars().take(DISCORD_MAX_CHARS - 1).collect();
                format!("{cut}…")
            } else {
                text
            };
            (
                url.to_string(),
                serde_json::to_vec(&serde_json::json!({ "content": content }))?,
                JSON,
            )
        }
        NotifyFormat::Healthchecks => {
            let base = url.trim_end_matches('/');
            let target = if n.status == Status::Success {
                base.to_string()
            } else {
                format!("{base}/fail")
            };
            (
                target,
                summary_text(n).into_bytes(),
                "text/plain; charset=utf-8",
            )
        }
    })
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
pub async fn send_webhook(url: &str, format: NotifyFormat, notification: &Notification<'_>) {
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

    let (target, body_bytes, content_type) =
        match request_for(format, url, notification, utc_now_iso8601()) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "failed to serialise webhook payload");
                return;
            }
        };

    match send_post(&target, body_bytes, content_type).await {
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
async fn send_post(url: &str, body: Vec<u8>, content_type: &str) -> Result<StatusCode, String> {
    // Same proxy handling (HTTP(S)_PROXY, ALL_PROXY, NO_PROXY) as the API client.
    let http = ProxyClient::from_env(true).map_err(|e| e.to_string())?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(url)
        .header("Content-Type", content_type)
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

    fn notification(status: Status, failures: &[Failure]) -> Notification<'_> {
        Notification {
            status,
            owner: "octocat",
            error: None,
            repos_backed_up: 8,
            repos_errored: 2,
            failures,
        }
    }

    fn body_json(format: NotifyFormat, n: &Notification<'_>) -> (String, serde_json::Value) {
        let (url, body, ctype) =
            request_for(format, "https://hooks.example/x", n, "t".into()).expect("request");
        assert_eq!(ctype, "application/json");
        (url, serde_json::from_slice(&body).expect("json body"))
    }

    /// Slack rejects a payload without `text` (`no_text`); the plain JSON
    /// payload has none, which is why a Slack URL never worked.
    #[test]
    fn slack_gets_a_text_field_naming_the_outcome_and_failures() {
        let failures = vec![failure("octocat/a", "wiki"), failure("octocat/b", "issues")];
        let n = notification(Status::Partial, &failures);
        let (url, v) = body_json(NotifyFormat::Slack, &n);
        assert_eq!(url, "https://hooks.example/x");
        let text = v["text"].as_str().expect("text");
        assert!(
            text.contains("octocat") && text.contains("INCOMPLETE"),
            "{text}"
        );
        assert!(text.contains("octocat/a: wiki"), "{text}");
        assert!(!text.contains("sensitive stderr"), "{text}");
        assert!(body_json(NotifyFormat::Json, &n).1["text"].is_null());
    }

    /// Discord needs `content`, at most 2000 characters.
    #[test]
    fn discord_gets_content_capped_at_2000_characters() {
        let failures: Vec<Failure> = (0..MAX_LISTED_FAILURES)
            .map(|i| failure(&format!("octocat/{}", "x".repeat(200 + i)), "clone"))
            .collect();
        let n = notification(Status::Partial, &failures);
        let (_, v) = body_json(NotifyFormat::Discord, &n);
        let content = v["content"].as_str().expect("content");
        assert_eq!(content.chars().count(), DISCORD_MAX_CHARS);
        assert!(content.ends_with('…'));
    }

    #[test]
    fn healthchecks_pings_the_url_on_success_and_url_fail_otherwise() {
        let ok = notification(Status::Success, &[]);
        let (url, body, ctype) = request_for(
            NotifyFormat::Healthchecks,
            "https://hc-ping.com/uuid/",
            &ok,
            "t".into(),
        )
        .expect("request");
        assert_eq!(url, "https://hc-ping.com/uuid");
        assert!(ctype.starts_with("text/plain"));
        assert!(String::from_utf8(body).expect("utf8").contains("succeeded"));
        for status in [Status::Partial, Status::Failure, Status::Interrupted] {
            let (url, _, _) = request_for(
                NotifyFormat::Healthchecks,
                "https://hc-ping.com/uuid",
                &notification(status, &[]),
                "t".into(),
            )
            .expect("request");
            assert_eq!(url, "https://hc-ping.com/uuid/fail", "{status:?}");
        }
    }

    #[test]
    fn an_interrupted_run_is_reported_as_interrupted() {
        let (_, v) = body_json(NotifyFormat::Json, &notification(Status::Interrupted, &[]));
        assert_eq!(v["status"], "interrupted");
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
