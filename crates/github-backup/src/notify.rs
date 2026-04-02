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
use http_body_util::Full;
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::{debug, warn};

const NOTIFY_TIMEOUT_SECS: u64 = 15;

/// JSON payload sent to the webhook URL.
#[derive(serde::Serialize)]
struct WebhookPayload<'a> {
    /// `"success"` or `"failure"`.
    status: &'a str,
    /// GitHub username or organisation that was backed up.
    owner: &'a str,
    /// ISO 8601 UTC timestamp of the backup completion.
    timestamp: String,
    /// Human-readable error message, present only when `status == "failure"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    /// Number of repositories backed up (0 on failure).
    repos_backed_up: u64,
    /// Number of repositories that encountered errors (0 on success).
    repos_errored: u64,
}

/// Posts a JSON notification to `url`.
///
/// The function is "fire and forget": any error (network, TLS, non-2xx
/// response) is logged at `WARN` level and silently ignored.
///
/// A warning is emitted when `url` uses plain HTTP so operators are aware
/// that backup metadata will be transmitted unencrypted.
pub async fn send_webhook(
    url: &str,
    owner: &str,
    status: &str,
    error: Option<&str>,
    repos_backed_up: u64,
    repos_errored: u64,
) {
    // Warn when the URL is plain HTTP — the payload contains owner name and
    // error messages that should not travel over an unencrypted connection.
    if url.starts_with("http://") {
        warn!(
            url,
            "webhook URL uses plain HTTP; backup metadata (owner, error messages) \
             will be transmitted unencrypted. Use an https:// URL to protect this data."
        );
    }

    let payload = WebhookPayload {
        status,
        owner,
        timestamp: utc_now_iso8601(),
        error,
        repos_backed_up,
        repos_errored,
    };

    let body_bytes = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "failed to serialise webhook payload");
            return;
        }
    };

    match send_post(url, body_bytes).await {
        Ok(status_code) if status_code.is_success() => {
            debug!(url, http_status = %status_code, "webhook notification sent");
        }
        Ok(status_code) => {
            warn!(url, http_status = %status_code, "webhook notification returned non-2xx status");
        }
        Err(e) => {
            warn!(url, error = %e, "webhook notification failed");
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
    .map_err(|_| format!("webhook POST to {url} timed out after {NOTIFY_TIMEOUT_SECS}s"))?
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

    #[test]
    fn webhook_payload_serialises_failure() {
        let payload = WebhookPayload {
            status: "failure",
            owner: "octocat",
            timestamp: "2025-03-30T12:00:00Z".to_string(),
            error: Some("backup failed: rate limit"),
            repos_backed_up: 0,
            repos_errored: 3,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"status\":\"failure\""));
        assert!(json.contains("\"error\":\"backup failed: rate limit\""));
    }

    #[test]
    fn webhook_payload_omits_error_on_success() {
        let payload = WebhookPayload {
            status: "success",
            owner: "octocat",
            timestamp: "2025-03-30T12:00:00Z".to_string(),
            error: None,
            repos_backed_up: 42,
            repos_errored: 0,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(!json.contains("\"error\":"));
        assert!(json.contains("\"repos_backed_up\":42"));
    }
}
