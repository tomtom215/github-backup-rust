// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! JSON summary report generation.
//!
//! After a backup run completes, [`write_report`] serialises key statistics to
//! a JSON file that monitoring systems (Prometheus push-gateway, Datadog,
//! custom alerts…) can parse to verify backup health.
//!
//! # Example report
//!
//! ```json
//! {
//!   "tool_version": "0.2.0",
//!   "owner": "octocat",
//!   "started_at": "2026-01-15T12:34:56Z",
//!   "duration_secs": 42,
//!   "repos_discovered": 10,
//!   "repos_backed_up": 9,
//!   "repos_skipped": 1,
//!   "repos_errored": 0,
//!   "gists_backed_up": 3,
//!   "issues_fetched": 150,
//!   "prs_fetched": 42,
//!   "workflows_fetched": 5,
//!   "success": true
//! }
//! ```

use chrono::{DateTime, TimeZone, Utc};
use github_backup_core::BackupStats;

/// Writes a JSON summary report to `path`.
///
/// The report includes counters, elapsed time, tool version, and an ISO 8601
/// timestamp so monitoring systems can parse and alert on backup health.
///
/// # Errors
///
/// Returns an error string if the file cannot be created or written.
pub fn write_report(
    path: &std::path::Path,
    owner: &str,
    stats: &BackupStats,
    started_at_unix: u64,
) -> Result<(), String> {
    let started_iso = unix_secs_to_iso8601(started_at_unix);

    let report = serde_json::json!({
        "tool_version": env!("CARGO_PKG_VERSION"),
        "owner": owner,
        "started_at": started_iso,
        "duration_secs": stats.elapsed_secs(),
        "repos_discovered": stats.repos_discovered(),
        "repos_backed_up": stats.repos_backed_up(),
        "repos_skipped": stats.repos_skipped(),
        "repos_errored": stats.repos_errored(),
        "gists_backed_up": stats.gists_backed_up(),
        "issues_fetched": stats.issues_fetched(),
        "prs_fetched": stats.prs_fetched(),
        "workflows_fetched": stats.workflows_fetched(),
        "success": stats.repos_errored() == 0,
    });
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create report directory: {e}"))?;
    }
    std::fs::write(path, json).map_err(|e| format!("cannot write report: {e}"))
}

/// Formats a Unix timestamp (seconds since epoch) as an RFC 3339 / ISO 8601
/// UTC string in the form `"YYYY-MM-DDTHH:MM:SSZ"`.
#[must_use]
pub fn unix_secs_to_iso8601(secs: u64) -> String {
    let dt: DateTime<Utc> = Utc
        .timestamp_opt(secs as i64, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap());
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Checks whether a string is a valid ISO 8601 / RFC 3339 timestamp.
///
/// Accepts the common forms used with the GitHub API:
/// - `YYYY-MM-DDTHH:MM:SSZ`
/// - `YYYY-MM-DDTHH:MM:SS+HH:MM` / `...−HH:MM`
///
/// Uses chrono for full validation including calendar correctness (month
/// range, day-in-month range, hour/minute/second range).  An explicit check
/// for the `T` separator at position 10 is applied first because some chrono
/// versions accept a space in that position (which RFC 3339 forbids).
#[must_use]
pub fn is_valid_iso8601(s: &str) -> bool {
    let bytes = s.as_bytes();
    // A bare date ("2024-01-01") or anything under 20 chars is not a full
    // RFC 3339 datetime.
    if bytes.len() < 20 {
        return false;
    }
    // RFC 3339 §5.6 requires 'T' (case-insensitive) as the separator.
    if bytes[10] != b'T' && bytes[10] != b't' {
        return false;
    }
    DateTime::parse_from_rfc3339(s).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_valid_iso8601 ──────────────────────────────────────────────────

    #[test]
    fn valid_utc_z_suffix() {
        assert!(is_valid_iso8601("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn valid_positive_offset() {
        assert!(is_valid_iso8601("2024-01-01T12:00:00+05:30"));
    }

    #[test]
    fn valid_negative_offset() {
        assert!(is_valid_iso8601("2024-01-01T12:00:00-08:00"));
    }

    #[test]
    fn invalid_too_short() {
        assert!(!is_valid_iso8601("2024-01-01"));
    }

    #[test]
    fn invalid_missing_t_separator() {
        assert!(!is_valid_iso8601("2024-01-01 00:00:00Z"));
    }

    #[test]
    fn invalid_non_digit_year() {
        assert!(!is_valid_iso8601("XXXX-01-01T00:00:00Z"));
    }

    #[test]
    fn invalid_missing_dashes() {
        assert!(!is_valid_iso8601("20240101T000000Z"));
    }

    #[test]
    fn invalid_empty_string() {
        assert!(!is_valid_iso8601(""));
    }

    /// Previously the hand-rolled validator accepted out-of-range values.
    #[test]
    fn invalid_out_of_range_values() {
        assert!(!is_valid_iso8601("2024-99-99T99:99:99Z"));
        assert!(!is_valid_iso8601("2024-13-01T00:00:00Z"));
        assert!(!is_valid_iso8601("2024-01-32T00:00:00Z"));
        assert!(!is_valid_iso8601("2024-01-01T25:00:00Z"));
        assert!(!is_valid_iso8601("2024-02-30T00:00:00Z"));
    }

    // ── unix_secs_to_iso8601 ──────────────────────────────────────────────

    #[test]
    fn epoch_formats_correctly() {
        assert_eq!(unix_secs_to_iso8601(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_timestamp_formats_correctly() {
        // Unix timestamp 1_705_305_600 = 2024-01-15T08:00:00Z
        assert_eq!(unix_secs_to_iso8601(1_705_305_600), "2024-01-15T08:00:00Z");
    }

    #[test]
    fn new_years_2026() {
        // Unix timestamp 1_767_225_600 = 2026-01-01T00:00:00Z
        assert_eq!(unix_secs_to_iso8601(1_767_225_600), "2026-01-01T00:00:00Z");
    }

    #[test]
    fn output_matches_is_valid_iso8601() {
        let s = unix_secs_to_iso8601(1_700_000_000);
        assert_eq!(s.len(), 20);
        assert!(is_valid_iso8601(&s), "output must be valid ISO 8601: {s}");
    }
}
