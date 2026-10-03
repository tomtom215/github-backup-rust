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
//!   "failure_count": 0,
//!   "failures": [],
//!   "success": true
//! }
//! ```
//!
//! `success` is `true` only when **nothing** failed (`failure_count == 0`):
//! every failed step — a repository that could not be cloned, an issue list that
//! could not be fetched, an S3 upload that errored — is listed in `failures`
//! with its `scope`, `step` and `message`, and makes the run unsuccessful.

use chrono::{DateTime, TimeZone, Utc};
use github_backup_core::BackupStats;

/// Builds the JSON summary report.
///
/// The schema is **append-only stable**: existing keys keep the same name and
/// type across releases.  Monitoring jobs may pin to the set of keys
/// currently documented in the module-level doc-comment.
#[must_use]
pub fn build_report(owner: &str, stats: &BackupStats, started_at_unix: u64) -> serde_json::Value {
    let started_iso = unix_secs_to_iso8601(started_at_unix);
    let elapsed = stats.elapsed_secs();
    let finished_iso = unix_secs_to_iso8601(started_at_unix.saturating_add(elapsed as u64));

    serde_json::json!({
        "tool_version": env!("CARGO_PKG_VERSION"),
        "schema_version": 1,
        "owner": owner,
        "started_at": started_iso,
        "finished_at": finished_iso,
        "duration_secs": elapsed,
        "repos_discovered": stats.repos_discovered(),
        "repos_backed_up": stats.repos_backed_up(),
        "repos_skipped": stats.repos_skipped(),
        "repos_errored": stats.repos_errored(),
        "gists_backed_up": stats.gists_backed_up(),
        "issues_fetched": stats.issues_fetched(),
        "prs_fetched": stats.prs_fetched(),
        "workflows_fetched": stats.workflows_fetched(),
        "failure_count": stats.failure_count(),
        "failures": stats.failures(),
        "success": !stats.has_failures(),
    })
}

/// Writes a JSON summary report to `path`.
///
/// The report includes counters, elapsed time, tool version, the list of
/// failures and an ISO 8601 timestamp so monitoring systems can parse and alert
/// on backup health.  See [`build_report`] for the schema.
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
    let report = build_report(owner, stats, started_at_unix);
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create report directory: {e}"))?;
        }
    }
    write_atomic(path, json.as_bytes())
}

/// Writes `bytes` to `path` atomically.
///
/// Writes to a sibling `*.tmp` file first, then renames over `path`.  This
/// avoids leaving a half-written report visible to monitoring systems if the
/// process is interrupted mid-write — a common failure mode when the host
/// is shutting down (the same SIGTERM that kills `github-backup` also kills
/// the Prometheus node exporter that reads the file moments later).
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => path.with_extension(format!("{ext}.tmp")),
        None => path.with_extension("tmp"),
    };
    std::fs::write(&tmp, bytes).map_err(|e| format!("cannot write report tmp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        // Best-effort cleanup so we don't leave the .tmp on the filesystem.
        let _ = std::fs::remove_file(&tmp);
        format!("cannot rename report tmp into place: {e}")
    })
}

/// Current Unix time in seconds.
///
/// Falls back to `0` when the system clock is somehow before the epoch
/// (effectively impossible on any host we run on, but the saturating
/// fallback avoids a panic in pathological environments).
pub(crate) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

/// Normalises a `--since` value to `YYYY-MM-DDTHH:MM:SSZ` (UTC).
///
/// Accepts a bare date (`2024-01-01`, taken as midnight UTC) or a full RFC 3339
/// timestamp with any offset.
///
/// # Errors
///
/// Returns what is wrong with `value` when it is neither.
pub fn normalise_since(value: &str) -> Result<String, String> {
    let value = value.trim();
    if is_valid_iso8601(value) {
        let dt = DateTime::parse_from_rfc3339(value).map_err(|e| e.to_string())?;
        return Ok(dt
            .with_timezone(&Utc)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string());
    }
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .ok_or_else(|| {
            "expected a date like 2024-01-01 or a timestamp like 2024-01-01T00:00:00Z".to_string()
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalise_since_accepts_dates_and_timestamps() {
        use super::normalise_since;
        assert_eq!(
            normalise_since("2024-01-01").unwrap(),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            normalise_since("2024-01-01T12:00:00+02:00").unwrap(),
            "2024-01-01T10:00:00Z",
            "offsets are converted to UTC"
        );
        assert_eq!(
            normalise_since(" 2024-01-01T00:00:00Z ").unwrap(),
            "2024-01-01T00:00:00Z"
        );
        for bad in [
            "",
            "yesterday",
            "2024-13-01",
            "2024-02-30",
            "2024-01-01 00:00:00",
        ] {
            assert!(normalise_since(bad).is_err(), "{bad:?} must be rejected");
        }
    }

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

    // ── write_atomic ──────────────────────────────────────────────────────

    #[test]
    fn write_atomic_creates_target_and_removes_tmp() {
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("report.json");
        super::write_atomic(&target, b"{\"ok\": true}").expect("write");
        assert_eq!(
            std::fs::read(&target).expect("read"),
            b"{\"ok\": true}",
            "target should contain the bytes we wrote"
        );
        // Neither `report.tmp` nor `report.json.tmp` should remain.
        for ext in ["tmp", "json.tmp"] {
            let stray = target.with_extension(ext);
            assert!(
                !stray.exists(),
                "tmp file {} must not be left behind",
                stray.display()
            );
        }
    }

    #[test]
    fn write_atomic_overwrites_existing_file() {
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("r.json");
        std::fs::write(&target, b"old").expect("seed");
        super::write_atomic(&target, b"new").expect("overwrite");
        assert_eq!(std::fs::read(&target).expect("read"), b"new");
    }

    // ── build_report ──────────────────────────────────────────────────────

    #[test]
    fn report_of_a_clean_run_is_successful_with_an_empty_failure_list() {
        let stats = BackupStats::new();
        stats.add_discovered(3);
        stats.inc_backed_up();
        let r = build_report("octocat", &stats, 1_705_305_600);
        assert_eq!(r["owner"], "octocat");
        assert_eq!(r["started_at"], "2024-01-15T08:00:00Z");
        assert_eq!(r["success"], true);
        assert_eq!(r["failure_count"], 0);
        assert_eq!(r["failures"], serde_json::json!([]));
        assert_eq!(r["repos_discovered"], 3);
        assert_eq!(r["schema_version"], 1);
    }

    /// A run that lost data is not a success, even if no repository counter
    /// says so: here every repository counter is clean but a gist failed.
    #[test]
    fn report_is_unsuccessful_whenever_anything_failed() {
        let stats = BackupStats::new();
        stats.record_failure("octocat", "gist abc", "fatal: boom");
        assert_eq!(stats.repos_errored(), 0);
        let r = build_report("octocat", &stats, 0);
        assert_eq!(r["success"], false);
        assert_eq!(r["failure_count"], 1);
        assert_eq!(r["failures"][0]["scope"], "octocat");
        assert_eq!(r["failures"][0]["step"], "gist abc");
        assert_eq!(r["failures"][0]["message"], "fatal: boom");
    }

    #[test]
    fn every_documented_key_is_present() {
        let r = build_report("o", &BackupStats::new(), 0);
        for key in [
            "tool_version",
            "schema_version",
            "owner",
            "started_at",
            "finished_at",
            "duration_secs",
            "repos_discovered",
            "repos_backed_up",
            "repos_skipped",
            "repos_errored",
            "gists_backed_up",
            "issues_fetched",
            "prs_fetched",
            "workflows_fetched",
            "failure_count",
            "failures",
            "success",
        ] {
            assert!(r.get(key).is_some(), "report is missing `{key}`");
        }
    }

    #[test]
    fn write_report_creates_parent_directories_and_valid_json() {
        use tempfile::tempdir;
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("report.json");
        write_report(&path, "octocat", &BackupStats::new(), 0).expect("write");
        let parsed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("valid JSON");
        assert_eq!(parsed["owner"], "octocat");
    }
}
