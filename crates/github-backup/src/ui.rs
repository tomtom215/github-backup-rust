// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Human-facing terminal output: colour detection, the pre-run plan banner,
//! the post-run summary, and the first-contact quickstart.

use github_backup_core::{BackupStats, Failure};
use github_backup_types::backup_state::BackupRunHistory;
use github_backup_types::config::OutputConfig;

use crate::cli::Args;

/// Returns `true` if the human-facing output may use ANSI colour codes.
///
/// Everything in this module is written to **stderr** (stdout stays free for
/// machine-readable output), so it is stderr that must be a terminal: with
/// `2>run.log` the codes would otherwise end up in the log file.  Honours the
/// same conventions as `setup::init_tracing`: `NO_COLOR` disables,
/// `CLICOLOR_FORCE=1` forces, otherwise autodetect.
pub(crate) fn use_ansi() -> bool {
    use std::io::IsTerminal as _;
    if no_color_env_set() {
        return false;
    }
    if std::env::var("CLICOLOR_FORCE").as_deref() == Ok("1") {
        return true;
    }
    std::io::stderr().is_terminal()
}

/// Returns `true` iff `NO_COLOR` is set to a non-empty value, per the
/// official <https://no-color.org> spec.
///
/// We use `var()` (not `var_os()`) so a Dockerfile or systemd unit that
/// declares `NO_COLOR=` (empty value, common pattern when a tool sets
/// the variable for downstream processes) does not accidentally
/// suppress colour.
pub(crate) fn no_color_env_set() -> bool {
    matches!(std::env::var("NO_COLOR"), Ok(ref v) if !v.is_empty())
}

/// Prints a one-shot summary of what the run is about to do.
///
/// Aimed at non-technical operators who want to see, in one glance,
/// which owner is being backed up, where the files will land, and
/// roughly how long it will take if there's a previous run to compare
/// against.  Skipped under `--quiet` to keep cron / journald output
/// machine-friendly.
pub(crate) fn print_plan(
    owner: &str,
    output_path: &std::path::Path,
    opts: &github_backup_types::config::BackupOptions,
    dry_run: bool,
    output: &OutputConfig,
) {
    let bold = if use_ansi() { "\x1b[1m" } else { "" };
    let reset = if use_ansi() { "\x1b[0m" } else { "" };
    let dim = if use_ansi() { "\x1b[2m" } else { "" };

    let mode = if dry_run {
        format!("{bold}dry run{reset} (no files will be written)")
    } else {
        format!("{bold}backup{reset}")
    };
    let categories = enabled_category_list(opts);

    eprintln!();
    eprintln!("{bold}━━ github-backup ━━{reset}");
    eprintln!("  Owner       {owner}");
    eprintln!("  Mode        {mode}");
    eprintln!("  Output      {}", output_path.display());
    eprintln!("  Concurrency {}", opts.concurrency);
    eprintln!("  Categories  {categories}");
    if let Some(eta) = estimated_duration_from_history(output, owner) {
        eprintln!(
            "  Last run    {dim}~{}s elapsed → expect a similar duration{reset}",
            eta.as_secs()
        );
    }
    eprintln!();
}

/// The facts the end-of-run summary reports.
#[derive(Debug, Clone)]
pub(crate) struct SummaryView {
    pub backed_up: u64,
    pub skipped: u64,
    pub errored: u64,
    pub issues: u64,
    pub prs: u64,
    pub gists: u64,
    pub failures: Vec<Failure>,
    pub elapsed_secs: u64,
    pub dry_run: bool,
}

impl SummaryView {
    pub(crate) fn from_stats(stats: &BackupStats, elapsed_secs: u64, dry_run: bool) -> Self {
        Self {
            backed_up: stats.repos_backed_up(),
            skipped: stats.repos_skipped(),
            errored: stats.repos_errored(),
            issues: stats.issues_fetched(),
            prs: stats.prs_fetched(),
            gists: stats.gists_backed_up(),
            failures: stats.failures(),
            elapsed_secs,
            dry_run,
        }
    }
}

/// How many failures the summary lists before pointing at the report.
const SUMMARY_FAILURES_SHOWN: usize = 15;

/// Longest failure message shown on one summary line.
const SUMMARY_MESSAGE_WIDTH: usize = 90;

/// Shortens `message` to its first line, at most `max` characters.
fn one_line(message: &str, max: usize) -> String {
    let first = message.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max {
        return first.to_string();
    }
    let cut: String = first.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Renders the end-of-run summary.
///
/// The headline is decided by what actually happened: a run with any failure is
/// reported as **incomplete** and lists what failed; a dry run says nothing was
/// written; only a run with no failures that processed something is a success.
pub(crate) fn render_summary(v: &SummaryView, ansi: bool) -> String {
    use std::fmt::Write as _;

    let (bold, green, yellow, red, reset) = if ansi {
        ("\x1b[1m", "\x1b[32m", "\x1b[33m", "\x1b[31m", "\x1b[0m")
    } else {
        ("", "", "", "", "")
    };

    let failures = v.failures.len();
    let processed = v.backed_up + v.errored;
    let (glyph, colour, headline) = if failures > 0 {
        (
            if ansi { "✗" } else { "[fail]" },
            red,
            format!(
                "backup finished with {failures} failure{} — it is incomplete",
                if failures == 1 { "" } else { "s" }
            ),
        )
    } else if v.dry_run {
        (
            if ansi { "○" } else { "[dry ]" },
            yellow,
            "dry run complete — nothing was written".to_string(),
        )
    } else if processed == 0 && v.skipped == 0 {
        // Zero repos found often means a wrong target or insufficient scope.
        (
            if ansi { "⚠" } else { "[warn]" },
            yellow,
            "backup finished but no repositories were processed".to_string(),
        )
    } else {
        (
            if ansi { "✓" } else { "[ ok ]" },
            green,
            "backup completed successfully".to_string(),
        )
    };

    let mut out = String::new();
    let _ = writeln!(out);
    let _ = writeln!(out, "{colour}{glyph}{reset}  {bold}{headline}{reset}");
    if v.dry_run {
        let _ = writeln!(
            out,
            "   {} repositor{} would be backed up",
            v.skipped,
            if v.skipped == 1 { "y" } else { "ies" }
        );
    } else {
        let _ = writeln!(out, "   {} repo(s) backed up completely", v.backed_up);
        if v.errored > 0 {
            let _ = writeln!(out, "   {colour}{} repo(s) incomplete{reset}", v.errored);
        }
        if v.skipped > 0 {
            let _ = writeln!(
                out,
                "   {} repo(s) skipped (filtered out, or already done in an interrupted run)",
                v.skipped
            );
        }
        if v.issues > 0 {
            let _ = writeln!(out, "   {} issue(s) fetched", v.issues);
        }
        if v.prs > 0 {
            let _ = writeln!(out, "   {} pull request(s) fetched", v.prs);
        }
        if v.gists > 0 {
            let _ = writeln!(out, "   {} gist(s) backed up", v.gists);
        }
    }
    let _ = writeln!(out, "   elapsed: {}", format_duration(v.elapsed_secs));

    if failures > 0 {
        let _ = writeln!(out);
        let _ = writeln!(out, "   {bold}what failed{reset}");
        let width = v
            .failures
            .iter()
            .take(SUMMARY_FAILURES_SHOWN)
            .map(|f| f.scope.chars().count())
            .max()
            .unwrap_or(0);
        for f in v.failures.iter().take(SUMMARY_FAILURES_SHOWN) {
            let _ = writeln!(
                out,
                "   {colour}•{reset} {:<width$}  {}: {}",
                f.scope,
                f.step,
                one_line(&f.message, SUMMARY_MESSAGE_WIDTH),
            );
        }
        if failures > SUMMARY_FAILURES_SHOWN {
            let _ = writeln!(
                out,
                "   … and {} more (all of them are in the --report file and the log)",
                failures - SUMMARY_FAILURES_SHOWN
            );
        }
        let _ = writeln!(
            out,
            "   Re-running retries them; progress on everything that succeeded is kept."
        );
    }

    if !v.dry_run && processed == 0 && v.skipped == 0 && failures == 0 {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "   {yellow}hint{reset}: zero repositories found.  \
             Check OWNER spelling, confirm the token has access, and \
             enable at least one of --repositories / --all / --gists / …"
        );
    }
    let _ = writeln!(out);
    out
}

/// Prints the end-of-run summary to stderr.
///
/// Non-technical users tend to scroll past pages of structured log lines
/// without internalising the numbers; this block gives them a clear pass/fail
/// signal, the tally, and — when something failed — exactly what.
pub(crate) fn print_summary_banner(stats: &BackupStats, elapsed_secs: u64, dry_run: bool) {
    let view = SummaryView::from_stats(stats, elapsed_secs, dry_run);
    eprint!("{}", render_summary(&view, use_ansi()));
}

/// Formats a duration in seconds as `Hh Mm Ss`, dropping leading zero
/// units so a 90-second run reads as `1m 30s` rather than `0h 1m 30s`.
pub(crate) fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Returns the elapsed duration of the most recent successful run from
/// the backup history, or `None` when no history file exists yet.
pub(crate) fn estimated_duration_from_history(
    output: &OutputConfig,
    owner: &str,
) -> Option<std::time::Duration> {
    let path = output.backup_history_path(owner);
    let history = BackupRunHistory::load(&path).ok()?;
    let last = history.entries.iter().rev().find(|e| e.success)?;
    Some(std::time::Duration::from_secs_f64(
        last.elapsed_secs.max(0.0),
    ))
}

/// Returns a short human-readable list of which categories are enabled.
///
/// Used in the pre-run plan banner so users can confirm at a glance that
/// the right set was selected.  Truncates after the first eight to keep
/// the line readable.
pub(crate) fn enabled_category_list(opts: &github_backup_types::config::BackupOptions) -> String {
    let mut cats: Vec<&'static str> = Vec::new();
    if opts.repositories {
        cats.push("repos");
    }
    if opts.issues {
        cats.push("issues");
    }
    if opts.pulls {
        cats.push("pulls");
    }
    if opts.releases {
        cats.push("releases");
    }
    if opts.wikis {
        cats.push("wikis");
    }
    if opts.gists {
        cats.push("gists");
    }
    if opts.starred {
        cats.push("starred");
    }
    if opts.clone_starred {
        cats.push("clone-starred");
    }
    if opts.actions {
        cats.push("actions");
    }
    if opts.environments {
        cats.push("environments");
    }
    if opts.discussions {
        cats.push("discussions");
    }
    if opts.projects {
        cats.push("projects");
    }
    if opts.packages {
        cats.push("packages");
    }
    if cats.is_empty() {
        return "(none — nothing to do!)".to_string();
    }
    if cats.len() > 8 {
        let head = cats[..8].join(", ");
        return format!("{head} (+{} more)", cats.len() - 8);
    }
    cats.join(", ")
}

/// Heuristic: was `github-backup` invoked with no meaningful arguments?
///
/// Used to swap the bare "no owner specified" error for a friendlier
/// quickstart message.  We treat any auth, config-file, output, category,
/// or special-mode flag as a real invocation; everything else is treated
/// as a first-contact run.
pub(crate) fn invoked_without_arguments(args: &Args) -> bool {
    args.owner.is_none()
        && args.config.is_none()
        && args.output.is_none()
        && args.token.is_none()
        && !args.device_auth
        && !args.tui
        && !args.doctor
        && !args.check
        && !args.list_scopes
        && !args.print_config_template
        && !args.verify
        && !args.decrypt
        && !args.restore
}

/// Prints a friendly quickstart for users who run `github-backup` with no
/// arguments.  Aimed at non-technical first-time operators who would
/// otherwise see an unfamiliar error and abandon the tool.
pub(crate) fn print_quickstart() {
    let bold = if use_ansi() { "\x1b[1m" } else { "" };
    let dim = if use_ansi() { "\x1b[2m" } else { "" };
    let reset = if use_ansi() { "\x1b[0m" } else { "" };

    let q = format!(
        r#"{bold}github-backup{reset} v{ver} — back up everything GitHub knows about an account.

{bold}Quickstart{reset}
  1.  Create a personal access token:
        https://github.com/settings/tokens/new
      For a full backup, tick the {bold}repo{reset} and {bold}read:org{reset} scopes.

  2.  Export the token so it does not appear in your shell history:
        {dim}$ export GITHUB_TOKEN=ghp_yourtokenhere{reset}

  3.  Run a real backup:
        {dim}$ github-backup octocat --output ~/github-backups --all{reset}

  4.  Or launch the interactive TUI for a guided run:
        {dim}$ github-backup octocat --tui{reset}

{bold}Helpful flags{reset}
  --doctor                    run pre-flight checks (git, network, token)
  --check                     validate config without performing a backup
  --list-scopes               print OAuth scopes for current flag set
  --print-config-template     write a fresh annotated TOML config to stdout
  --help                      full reference

Documentation:  https://tomtom215.github.io/github-backup-rust/
"#,
        ver = env!("CARGO_PKG_VERSION"),
    );
    eprintln!("{q}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoked_without_arguments_true_for_bare_run() {
        let a = crate::cli::test_support::parse(&["github-backup"]);
        assert!(invoked_without_arguments(&a));
    }

    #[test]
    fn invoked_without_arguments_false_for_owner() {
        let a = crate::cli::test_support::parse(&["github-backup", "octocat"]);
        assert!(!invoked_without_arguments(&a));
    }

    #[test]
    fn format_duration_seconds_only() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn format_duration_minutes_and_seconds() {
        assert_eq!(format_duration(60), "1m 0s");
        assert_eq!(format_duration(125), "2m 5s");
    }

    #[test]
    fn format_duration_includes_hours_when_long() {
        assert_eq!(format_duration(3661), "1h 1m 1s");
        assert_eq!(format_duration(7200), "2h 0m 0s");
    }

    #[test]
    fn enabled_category_list_handles_empty_set() {
        use github_backup_types::config::BackupOptions;
        let opts = BackupOptions::default();
        let out = enabled_category_list(&opts);
        assert!(out.contains("none"), "got {out:?}");
    }

    #[test]
    fn enabled_category_list_lists_enabled_categories() {
        use github_backup_types::config::BackupOptions;
        let opts = BackupOptions {
            repositories: true,
            issues: true,
            ..Default::default()
        };
        let out = enabled_category_list(&opts);
        assert!(out.contains("repos"));
        assert!(out.contains("issues"));
    }

    #[test]
    fn enabled_category_list_truncates_long_sets() {
        use github_backup_types::config::BackupOptions;
        let opts = BackupOptions {
            repositories: true,
            issues: true,
            pulls: true,
            releases: true,
            wikis: true,
            gists: true,
            starred: true,
            clone_starred: true,
            actions: true,
            environments: true,
            discussions: true,
            ..Default::default()
        };
        let out = enabled_category_list(&opts);
        assert!(out.contains("+"), "expected truncation marker in: {out}");
    }

    // ── render_summary ────────────────────────────────────────────────────

    fn view() -> SummaryView {
        SummaryView {
            backed_up: 10,
            skipped: 0,
            errored: 0,
            issues: 0,
            prs: 0,
            gists: 0,
            failures: vec![],
            elapsed_secs: 42,
            dry_run: false,
        }
    }

    fn failure(scope: &str, step: &str, message: &str) -> Failure {
        Failure {
            scope: scope.into(),
            step: step.into(),
            message: message.into(),
        }
    }

    #[test]
    fn a_clean_run_is_reported_as_success() {
        let out = render_summary(&view(), false);
        assert!(
            out.contains("[ ok ]  backup completed successfully"),
            "{out}"
        );
        assert!(out.contains("10 repo(s) backed up completely"), "{out}");
        assert!(!out.contains("what failed"), "{out}");
        assert!(out.contains("elapsed: 42s"), "{out}");
    }

    /// The audit's headline defect: a run that lost data used to print the
    /// same green "successfully" banner.
    #[test]
    fn any_failure_makes_the_headline_incomplete_and_lists_it() {
        let mut v = view();
        v.backed_up = 8;
        v.errored = 2;
        v.failures = vec![
            failure("octocat/a", "clone", "fatal: unable to access\nsecond line"),
            failure("octocat/long-name", "issues", "HTTP 500"),
        ];
        let out = render_summary(&v, false);
        assert!(
            out.contains("[fail]  backup finished with 2 failures — it is incomplete"),
            "{out}"
        );
        assert!(!out.contains("completed successfully"), "{out}");
        assert!(out.contains("2 repo(s) incomplete"), "{out}");
        assert!(out.contains("octocat/a"), "{out}");
        assert!(out.contains("clone: fatal: unable to access"), "{out}");
        assert!(
            !out.contains("second line"),
            "only the first line is shown: {out}"
        );
        assert!(out.contains("issues: HTTP 500"), "{out}");
        assert!(out.contains("Re-running retries them"), "{out}");
    }

    #[test]
    fn a_single_failure_uses_the_singular() {
        let mut v = view();
        v.failures = vec![failure("o/r", "wiki", "x")];
        assert!(render_summary(&v, false).contains("with 1 failure — it is incomplete"));
    }

    #[test]
    fn the_failure_list_is_capped_with_a_pointer_to_the_report() {
        let mut v = view();
        v.failures = (0..SUMMARY_FAILURES_SHOWN + 7)
            .map(|i| failure(&format!("o/r{i}"), "clone", "x"))
            .collect();
        let out = render_summary(&v, false);
        assert!(out.contains("and 7 more"), "{out}");
        assert!(out.contains("--report"), "{out}");
        assert!(
            !out.contains(&format!("o/r{}", SUMMARY_FAILURES_SHOWN)),
            "{out}"
        );
    }

    #[test]
    fn a_dry_run_says_nothing_was_written_and_never_claims_success() {
        let mut v = view();
        v.dry_run = true;
        v.backed_up = 0;
        v.skipped = 7;
        let out = render_summary(&v, false);
        assert!(
            out.contains("dry run complete — nothing was written"),
            "{out}"
        );
        assert!(out.contains("7 repositories would be backed up"), "{out}");
        assert!(!out.contains("backed up completely"), "{out}");
        assert!(!out.contains("completed successfully"), "{out}");
    }

    #[test]
    fn zero_repositories_is_a_warning_with_a_hint() {
        let mut v = view();
        v.backed_up = 0;
        let out = render_summary(&v, false);
        assert!(out.contains("[warn]"), "{out}");
        assert!(out.contains("zero repositories found"), "{out}");
    }

    #[test]
    fn colour_is_only_emitted_when_asked_for() {
        let mut v = view();
        v.failures = vec![failure("o/r", "clone", "x")];
        assert!(!render_summary(&v, false).contains('\x1b'));
        assert!(render_summary(&v, true).contains("\x1b[31m"));
    }

    #[test]
    fn one_line_keeps_the_first_line_and_truncates_with_an_ellipsis() {
        assert_eq!(one_line("a\nb", 10), "a");
        assert_eq!(one_line("abcdefghij", 5), "abcd…");
        assert_eq!(one_line("abc", 5), "abc");
        assert_eq!(one_line("", 5), "");
    }
}
