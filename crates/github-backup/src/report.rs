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
    use std::time::{Duration, UNIX_EPOCH};

    let started_dt = UNIX_EPOCH + Duration::from_secs(started_at_unix);
    let started_iso = unix_to_iso8601(started_dt);

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

/// Formats a `SystemTime` as an RFC 3339 / ISO 8601 UTC string.
///
/// Output format: `"YYYY-MM-DDTHH:MM:SSZ"`.
///
/// Implemented without external date/time dependencies using the civil-date
/// algorithm from Howard Hinnant's date library.
pub fn unix_to_iso8601(t: std::time::SystemTime) -> String {
    use std::time::UNIX_EPOCH;

    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;

    // Days since epoch → year/month/day
    // Algorithm: https://howardhinnant.github.io/date_algorithms.html
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };

    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Checks whether a string looks like an ISO 8601 / RFC 3339 timestamp.
///
/// Accepts the most common subset: `YYYY-MM-DDTHH:MM:SSZ` or with a `±HH:MM`
/// offset.  This is a quick sanity check, not a full validator — the GitHub
/// API will return a clear error for out-of-range dates.
#[must_use]
pub fn is_valid_iso8601(s: &str) -> bool {
    // Minimum: "2024-01-01T00:00:00Z" = 20 chars
    if s.len() < 20 {
        return false;
    }
    let bytes = s.as_bytes();
    // YYYY-MM-DD
    bytes[4] == b'-'
        && bytes[7] == b'-'
        // T separator
        && (bytes[10] == b'T' || bytes[10] == b't')
        // HH:MM:SS
        && bytes[13] == b':'
        && bytes[16] == b':'
        // Timezone: Z or +/-HH:MM
        && (bytes[19] == b'Z'
            || bytes[19] == b'z'
            || bytes[19] == b'+'
            || bytes[19] == b'-')
        // All date/time digit fields
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[11..13].iter().all(u8::is_ascii_digit)
        && bytes[14..16].iter().all(u8::is_ascii_digit)
        && bytes[17..19].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    // ── is_valid_iso8601 ──────────────────────────────────────────────────

    #[test]
    fn valid_utc_z_suffix() {
        assert!(is_valid_iso8601("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn valid_lowercase_t_separator() {
        assert!(is_valid_iso8601("2024-06-15t12:30:45Z"));
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

    // ── unix_to_iso8601 ───────────────────────────────────────────────────

    #[test]
    fn epoch_formats_correctly() {
        let t = UNIX_EPOCH;
        assert_eq!(unix_to_iso8601(t), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_timestamp_formats_correctly() {
        // Unix timestamp 1_705_305_600 = 2024-01-15T08:00:00Z
        let t = UNIX_EPOCH + Duration::from_secs(1_705_305_600);
        assert_eq!(unix_to_iso8601(t), "2024-01-15T08:00:00Z");
    }

    #[test]
    fn new_years_2026() {
        // Unix timestamp 1_767_225_600 = 2026-01-01T00:00:00Z
        let t = UNIX_EPOCH + Duration::from_secs(1_767_225_600);
        assert_eq!(unix_to_iso8601(t), "2026-01-01T00:00:00Z");
    }

    #[test]
    fn output_matches_iso8601_pattern() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let s = unix_to_iso8601(t);
        // Must be 20 chars and pass our own validator
        assert_eq!(s.len(), 20);
        assert!(is_valid_iso8601(&s), "output must be valid ISO 8601");
    }
}
