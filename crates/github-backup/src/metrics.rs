// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Prometheus textfile-collector metrics (`--prometheus-metrics FILE`).
//!
//! The file is rewritten after every run, so a metric that must survive a
//! failed run — "when did a backup last *succeed*" — is carried over from the
//! previous file instead of being overwritten with a lie.  Alert rules can
//! then use `time() - github_backup_last_success_timestamp_seconds` and
//! `absent(...)` to catch both a stale backup and one that never succeeded.

use std::path::Path;

use github_backup_core::BackupStats;

const LAST_SUCCESS: &str = "github_backup_last_success_timestamp_seconds";

/// Escapes a label value per the Prometheus text exposition format.
fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Extracts the previous run's last-success timestamp for `label` from an
/// existing metrics file, if it has one.
fn previous_last_success(existing: &str, label: &str) -> Option<u64> {
    let prefix = format!("{LAST_SUCCESS}{{{label}}} ");
    existing
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.trim().parse().ok())
}

/// Renders the metrics text for one run.
///
/// `carried_last_success` is the value to keep when this run did not succeed.
fn render(
    owner: &str,
    stats: &BackupStats,
    started_at_unix: u64,
    carried_last_success: Option<u64>,
) -> String {
    let label = format!("owner=\"{}\"", escape_label(owner));
    let success = !stats.has_failures();
    let mut out = String::new();
    let mut metric = |name: &str, kind: &str, help: &str, value: String| {
        out.push_str(&format!(
            "# HELP {name} {help}\n# TYPE {name} {kind}\n{name}{{{label}}} {value}\n"
        ));
    };

    metric(
        "github_backup_repos_backed_up",
        "gauge",
        "Repositories backed up completely in the last run",
        stats.repos_backed_up().to_string(),
    );
    metric(
        "github_backup_repos_discovered",
        "gauge",
        "Repositories discovered in the owner listing",
        stats.repos_discovered().to_string(),
    );
    metric(
        "github_backup_repos_errored",
        "gauge",
        "Repositories with at least one failed step in the last run",
        stats.repos_errored().to_string(),
    );
    metric(
        "github_backup_failures",
        "gauge",
        "Failures recorded in the last run (steps that could not be backed up)",
        stats.failure_count().to_string(),
    );
    metric(
        "github_backup_issues_fetched",
        "gauge",
        "Issues fetched in the last run",
        stats.issues_fetched().to_string(),
    );
    metric(
        "github_backup_prs_fetched",
        "gauge",
        "Pull requests fetched in the last run",
        stats.prs_fetched().to_string(),
    );
    metric(
        "github_backup_duration_seconds",
        "gauge",
        "Duration of the last backup run in seconds",
        format!("{:.3}", stats.elapsed_secs()),
    );
    metric(
        "github_backup_last_run_timestamp_seconds",
        "gauge",
        "Unix time at which the last backup run started",
        started_at_unix.to_string(),
    );
    metric(
        "github_backup_success",
        "gauge",
        "Whether the last run backed up everything it was asked to (1) or not (0)",
        if success { "1" } else { "0" }.to_string(),
    );
    let last_success = if success {
        Some(started_at_unix)
    } else {
        carried_last_success
    };
    if let Some(ts) = last_success {
        metric(
            LAST_SUCCESS,
            "gauge",
            "Unix time at which the last fully successful backup run started",
            ts.to_string(),
        );
    }
    out
}

/// Writes the metrics file for a finished run.
///
/// The write is atomic: node_exporter's textfile collector polls the path every
/// few seconds and must never see a half-written file.
///
/// # Errors
///
/// Returns a message if the directory cannot be created or the file written.
pub(crate) fn write_prometheus_metrics(
    path: &Path,
    owner: &str,
    stats: &BackupStats,
    started_at_unix: u64,
) -> Result<(), String> {
    let label = format!("owner=\"{}\"", escape_label(owner));
    let carried = std::fs::read_to_string(path)
        .ok()
        .and_then(|existing| previous_last_success(&existing, &label));
    let text = render(owner, stats, started_at_unix, carried);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create metrics dir: {e}"))?;
        }
    }
    let tmp = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => path.with_extension(format!("{ext}.tmp")),
        None => path.with_extension("tmp"),
    };
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("write metrics tmp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename metrics tmp: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str, name: &str) -> Option<String> {
        text.lines()
            .find(|l| l.starts_with(&format!("{name}{{")))
            .and_then(|l| l.rsplit(' ').next())
            .map(str::to_string)
    }

    fn failing_stats() -> BackupStats {
        let stats = BackupStats::new();
        stats.record_failure("o/r", "wiki", "boom");
        stats
    }

    #[test]
    fn a_clean_run_reports_success_and_stamps_last_success() {
        let text = render("octocat", &BackupStats::new(), 1_700_000_000, Some(5));
        assert_eq!(value(&text, "github_backup_success").as_deref(), Some("1"));
        assert_eq!(value(&text, "github_backup_failures").as_deref(), Some("0"));
        assert_eq!(
            value(&text, LAST_SUCCESS).as_deref(),
            Some("1700000000"),
            "a success replaces the carried value"
        );
    }

    /// The old exporter wrote the *current* start time as "last success" even
    /// after a failed run, so a stale-backup alert could never fire.
    #[test]
    fn a_failed_run_keeps_the_previous_last_success_time() {
        let text = render(
            "octocat",
            &failing_stats(),
            1_700_000_500,
            Some(1_700_000_000),
        );
        assert_eq!(value(&text, "github_backup_success").as_deref(), Some("0"));
        assert_eq!(value(&text, "github_backup_failures").as_deref(), Some("1"));
        assert_eq!(
            value(&text, "github_backup_last_run_timestamp_seconds").as_deref(),
            Some("1700000500")
        );
        assert_eq!(
            value(&text, LAST_SUCCESS).as_deref(),
            Some("1700000000"),
            "the failed run must not look like a success"
        );
    }

    #[test]
    fn a_failed_first_run_has_no_last_success_metric_at_all() {
        let text = render("octocat", &failing_stats(), 1_700_000_500, None);
        assert!(
            !text.contains(LAST_SUCCESS),
            "never succeeded: the series must be absent so absent() alerts fire"
        );
    }

    #[test]
    fn the_previous_value_is_read_back_from_the_file_for_the_same_owner_only() {
        let label = "owner=\"octocat\"";
        let existing = format!("{LAST_SUCCESS}{{{label}}} 1234\nother 1\n");
        assert_eq!(previous_last_success(&existing, label), Some(1234));
        assert_eq!(previous_last_success(&existing, "owner=\"someone\""), None);
        assert_eq!(previous_last_success("garbage", label), None);
    }

    #[test]
    fn label_values_are_escaped() {
        let text = render("a\"b\\c", &BackupStats::new(), 1, None);
        assert!(text.contains(r#"owner="a\"b\\c""#), "{text}");
    }

    #[test]
    fn writing_carries_the_value_across_runs_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("metrics").join("github_backup.prom");

        write_prometheus_metrics(&path, "octocat", &BackupStats::new(), 1_000).expect("run 1");
        write_prometheus_metrics(&path, "octocat", &failing_stats(), 2_000).expect("run 2");

        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(value(&text, LAST_SUCCESS).as_deref(), Some("1000"));
        assert_eq!(value(&text, "github_backup_success").as_deref(), Some("0"));
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("list")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["github_backup.prom".to_string()]);
    }
}
