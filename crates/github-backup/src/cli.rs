// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Command-line argument definitions using [`clap`].

use std::path::PathBuf;

use clap::{ArgGroup, Parser};

/// Comprehensive GitHub backup tool.
///
/// Backs up repositories, issues, pull requests, releases, gists, wikis, and
/// relationship data for a GitHub user or organisation.
///
/// # Authentication
///
/// Provide a personal access token (classic or fine-grained) via:
/// - The `--token` flag
/// - The `GITHUB_TOKEN` environment variable
///
/// Fine-grained tokens are recommended for long-running or scheduled backups
/// because their narrow permission scope limits the blast radius of accidental
/// exposure.
///
/// # Examples
///
/// Back up everything for a user:
/// ```text
/// github-backup octocat --token ghp_xxx --output /backup --all
/// ```
///
/// Back up only repositories and issues:
/// ```text
/// github-backup octocat --token ghp_xxx --output /backup --repositories --issues
/// ```
#[derive(Debug, Parser)]
#[command(
    name = "github-backup",
    version,
    about = "Comprehensive GitHub backup: repos, issues, PRs, releases, gists, and more",
    long_about = None,
)]
#[command(group(
    ArgGroup::new("auth")
        .required(true)
        .args(["token"]),
))]
pub struct Args {
    /// GitHub username or organisation to back up.
    #[arg(value_name = "OWNER")]
    pub owner: String,

    // ── Authentication ─────────────────────────────────────────────────────
    /// Personal access token (classic or fine-grained).
    ///
    /// Can also be set via the `GITHUB_TOKEN` environment variable.
    #[arg(
        short = 't',
        long = "token",
        env = "GITHUB_TOKEN",
        value_name = "TOKEN",
        hide_env_values = true
    )]
    pub token: String,

    // ── Output ─────────────────────────────────────────────────────────────
    /// Root directory where backup artefacts will be written.
    #[arg(short = 'o', long = "output", value_name = "DIR", default_value = ".")]
    pub output: PathBuf,

    // ── Broad selectors ────────────────────────────────────────────────────
    /// Enable all backup categories (equivalent to supplying every individual
    /// flag, except `--lfs` and `--prefer-ssh`).
    #[arg(long, conflicts_with_all = [
        "repositories", "issues", "issue_comments", "issue_events",
        "pulls", "pull_comments", "pull_commits", "pull_reviews",
        "labels", "milestones", "releases", "release_assets",
        "hooks", "security_advisories", "wikis",
        "starred", "watched", "followers", "following",
        "gists", "starred_gists",
    ])]
    pub all: bool,

    // ── Repository options ─────────────────────────────────────────────────
    /// Clone/mirror repositories.
    #[arg(long)]
    pub repositories: bool,

    /// Include forked repositories.
    #[arg(long, short = 'F')]
    pub forks: bool,

    /// Include private repositories (requires appropriate token scope).
    #[arg(long, short = 'P')]
    pub private: bool,

    /// Clone using SSH URLs instead of HTTPS.
    #[arg(long)]
    pub prefer_ssh: bool,

    /// Clone with Git LFS support.
    #[arg(long)]
    pub lfs: bool,

    // ── Issue options ──────────────────────────────────────────────────────
    /// Back up issue metadata.
    #[arg(long)]
    pub issues: bool,

    /// Back up issue comment threads (requires `--issues`).
    #[arg(long)]
    pub issue_comments: bool,

    /// Back up issue timeline events (requires `--issues`).
    #[arg(long)]
    pub issue_events: bool,

    // ── Pull request options ───────────────────────────────────────────────
    /// Back up pull request metadata.
    #[arg(long)]
    pub pulls: bool,

    /// Back up pull request review comments.
    #[arg(long)]
    pub pull_comments: bool,

    /// Back up pull request commit lists.
    #[arg(long)]
    pub pull_commits: bool,

    /// Back up pull request reviews.
    #[arg(long)]
    pub pull_reviews: bool,

    // ── Repository metadata ────────────────────────────────────────────────
    /// Back up repository labels.
    #[arg(long)]
    pub labels: bool,

    /// Back up repository milestones.
    #[arg(long)]
    pub milestones: bool,

    /// Back up release metadata.
    #[arg(long)]
    pub releases: bool,

    /// Download release binary assets (requires `--releases`).
    #[arg(long)]
    pub release_assets: bool,

    /// Back up webhook configurations (requires admin token scope).
    #[arg(long)]
    pub hooks: bool,

    /// Back up published security advisories.
    #[arg(long)]
    pub security_advisories: bool,

    /// Clone repository wikis.
    #[arg(long)]
    pub wikis: bool,

    // ── User / org data ────────────────────────────────────────────────────
    /// Back up repositories starred by the owner.
    #[arg(long)]
    pub starred: bool,

    /// Back up repositories watched by the owner.
    #[arg(long)]
    pub watched: bool,

    /// Back up the owner's follower list.
    #[arg(long)]
    pub followers: bool,

    /// Back up the list of accounts the owner follows.
    #[arg(long)]
    pub following: bool,

    /// Back up gists owned by the owner.
    #[arg(long)]
    pub gists: bool,

    /// Back up gists starred by the authenticated user.
    #[arg(long)]
    pub starred_gists: bool,

    // ── Logging ────────────────────────────────────────────────────────────
    /// Suppress all non-error output.
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Increase log verbosity (`-v` = debug, `-vv` = trace).
    #[arg(long, short = 'v', action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl Args {
    /// Converts the parsed CLI arguments into a [`BackupOptions`] struct.
    #[must_use]
    pub fn into_backup_options(self) -> github_backup_types::config::BackupOptions {
        use github_backup_types::config::BackupOptions;

        if self.all {
            return BackupOptions {
                prefer_ssh: self.prefer_ssh,
                lfs: self.lfs,
                ..BackupOptions::all()
            };
        }

        BackupOptions {
            repositories: self.repositories,
            forks: self.forks,
            private: self.private,
            prefer_ssh: self.prefer_ssh,
            bare: true, // always use bare mirrors for git clones
            lfs: self.lfs,
            issues: self.issues,
            issue_comments: self.issue_comments,
            issue_events: self.issue_events,
            pulls: self.pulls,
            pull_comments: self.pull_comments,
            pull_commits: self.pull_commits,
            pull_reviews: self.pull_reviews,
            labels: self.labels,
            milestones: self.milestones,
            releases: self.releases,
            release_assets: self.release_assets,
            hooks: self.hooks,
            security_advisories: self.security_advisories,
            wikis: self.wikis,
            starred: self.starred,
            watched: self.watched,
            followers: self.followers,
            following: self.following,
            gists: self.gists,
            starred_gists: self.starred_gists,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn args_parse_minimal_required_fields() {
        let args = Args::parse_from(["github-backup", "octocat", "--token", "ghp_test"]);
        assert_eq!(args.owner, "octocat");
        assert_eq!(args.token, "ghp_test");
        assert!(!args.all);
    }

    #[test]
    fn args_parse_all_flag() {
        let args = Args::parse_from(["github-backup", "octocat", "--token", "t", "--all"]);
        assert!(args.all);
    }

    #[test]
    fn args_into_backup_options_all_enables_repositories() {
        let args = Args::parse_from(["github-backup", "octocat", "--token", "t", "--all"]);
        let opts = args.into_backup_options();
        assert!(opts.repositories);
        assert!(opts.issues);
        assert!(opts.pulls);
    }

    #[test]
    fn args_into_backup_options_individual_flags() {
        let args = Args::parse_from([
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--repositories",
            "--issues",
        ]);
        let opts = args.into_backup_options();
        assert!(opts.repositories);
        assert!(opts.issues);
        assert!(!opts.pulls);
    }

    #[test]
    fn args_parse_quiet_and_verbose() {
        let args = Args::parse_from(["github-backup", "octocat", "--token", "t", "-q"]);
        assert!(args.quiet);

        let args = Args::parse_from(["github-backup", "octocat", "--token", "t", "-vv"]);
        assert_eq!(args.verbose, 2);
    }
}
