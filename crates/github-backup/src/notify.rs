// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Webhook notification support.
//!
//! Sends a fire-and-forget HTTP POST to a user-configured URL after the
//! primary backup completes (success or failure).  Notification failures
//! are logged as warnings and never cause the backup process to exit with
//! a non-zero code.

use bytes::Bytes;
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
pub async fn send_webhook(
    url: &str,
    owner: &str,
    status: &str,
    error: Option<&str>,
    repos_backed_up: u64,
    repos_errored: u64,
) {
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
        .header("User-Agent", concat!("github-backup-rust/", env!("CARGO_PKG_VERSION")))
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
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let (y, mo, d, h, mi, s) = unix_to_ymd_hms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Converts a Unix timestamp to (year, month, day, hour, minute, second).
fn unix_to_ymd_hms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86_400;

    // Days since 1970-01-01 → Gregorian date (simplified Euclidean)
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };

    (y, mo, d, h, m, s)
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
    fn unix_to_ymd_hms_epoch() {
        let (y, mo, d, h, m, s) = unix_to_ymd_hms(0);
        assert_eq!((y, mo, d, h, m, s), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn unix_to_ymd_hms_known_date() {
        // 2025-03-30T12:00:00Z = 1743336000
        let (y, mo, d, h, m, s) = unix_to_ymd_hms(1_743_336_000);
        assert_eq!((y, mo, d, h, m, s), (2025, 3, 30, 12, 0, 0));
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
