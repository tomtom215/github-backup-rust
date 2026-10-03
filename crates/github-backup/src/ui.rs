// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Human-facing terminal output: colour detection, the pre-run plan banner,
//! the post-run summary, and the first-contact quickstart.

use github_backup_types::backup_state::BackupRunHistory;
use github_backup_types::config::OutputConfig;

use crate::cli::Args;

/// Returns `true` if the current stdout supports ANSI colour codes.
///
/// Honours the same conventions as [`init_tracing`]: `NO_COLOR` disables,
/// `CLICOLOR_FORCE=1` forces, otherwise we autodetect TTY.
pub(crate) fn use_ansi() -> bool {
    use std::io::IsTerminal as _;
    if no_color_env_set() {
        return false;
    }
    if std::env::var("CLICOLOR_FORCE").as_deref() == Ok("1") {
        return true;
    }
    std::io::stdout().is_terminal()
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

/// Prints a colour-coded summary banner at the end of a run.
///
/// Non-technical users tend to scroll past pages of structured log lines
/// without internalising the numbers; this single block gives them a
/// clear pass/fail signal and a one-line tally they can copy into a
/// ticket or status update.
pub(crate) fn print_summary_banner(stats: &github_backup_core::BackupStats, elapsed_secs: u64) {
    let ansi = use_ansi();
    let bold = if ansi { "\x1b[1m" } else { "" };
    let green = if ansi { "\x1b[32m" } else { "" };
    let yellow = if ansi { "\x1b[33m" } else { "" };
    let red = if ansi { "\x1b[31m" } else { "" };
    let reset = if ansi { "\x1b[0m" } else { "" };

    let errored = stats.repos_errored();
    let backed_up = stats.repos_backed_up();
    let skipped = stats.repos_skipped();
    let (glyph, colour, headline) = if errored > 0 {
        (
            if ansi { "✗" } else { "[fail]" },
            red,
            "backup completed with errors",
        )
    } else if backed_up == 0 && skipped == 0 {
        // Zero repos found often means a wrong target or insufficient
        // scope.  Surface it loudly with an actionable suggestion.
        (
            if ansi { "⚠" } else { "[warn]" },
            yellow,
            "backup completed but no repositories were processed",
        )
    } else {
        (
            if ansi { "✓" } else { "[ ok ]" },
            green,
            "backup completed successfully",
        )
    };

    eprintln!();
    eprintln!("{colour}{glyph}{reset}  {bold}{headline}{reset}");
    eprintln!("   {} repo(s) backed up", backed_up);
    if skipped > 0 {
        eprintln!("   {} repo(s) skipped (already in checkpoint)", skipped);
    }
    if errored > 0 {
        eprintln!("   {colour}{errored} repo(s) errored{reset}");
    }
    if stats.issues_fetched() > 0 {
        eprintln!("   {} issue(s) fetched", stats.issues_fetched());
    }
    if stats.prs_fetched() > 0 {
        eprintln!("   {} pull request(s) fetched", stats.prs_fetched());
    }
    if stats.gists_backed_up() > 0 {
        eprintln!("   {} gist(s) backed up", stats.gists_backed_up());
    }
    eprintln!("   elapsed: {}", format_duration(elapsed_secs));

    if backed_up == 0 && skipped == 0 && errored == 0 {
        eprintln!();
        eprintln!(
            "   {yellow}hint{reset}: zero repositories found.  \
             Check OWNER spelling, confirm the token has access, and \
             enable at least one of --repositories / --all / --gists / …"
        );
    }
    eprintln!();
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
}
