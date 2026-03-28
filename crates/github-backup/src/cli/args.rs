// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Top-level [`Args`] struct parsed from the command line.

use std::path::PathBuf;

use clap::{ArgGroup, Parser};

use super::clone_type::CliCloneType;

/// Comprehensive GitHub backup tool.
///
/// Backs up repositories, issues, pull requests, releases, gists, wikis, and
/// relationship data for a GitHub user or organisation.
///
/// # Authentication
///
/// Provide a personal access token (classic or fine-grained) via `--token` or
/// the `GITHUB_TOKEN` environment variable, **or** use `--device-auth` to
/// authenticate interactively via the GitHub OAuth device flow (requires a
/// registered OAuth App — see `--oauth-client-id`).
///
/// Fine-grained tokens are recommended for long-running or scheduled backups.
///
/// # Clone Types
///
/// By default repositories are cloned as bare mirrors (`--clone-type mirror`).
/// Choose `bare`, `full`, or `shallow:<depth>` to trade completeness for
/// speed or working-tree access.
///
/// # Mirror to Self-Hosted Git
///
/// After the primary backup, use `--mirror-to` to push every cloned
/// repository as a mirror to a Gitea-compatible instance (Codeberg, Forgejo,
/// self-hosted Gitea, …).
///
/// # S3 Storage
///
/// Use `--s3-bucket` (and related flags) to sync JSON metadata and release
/// assets to any S3-compatible object store (AWS, Backblaze B2, MinIO, …).
///
/// # Configuration File
///
/// Load defaults from a TOML configuration file with `--config <FILE>`.
/// Command-line flags override values from the config file.
///
/// # Examples
///
/// Back up everything for a user:
/// ```text
/// github-backup octocat --token ghp_xxx --output /backup --all
/// ```
///
/// Back up only repositories and issues for an org, with 8 parallel workers:
/// ```text
/// github-backup my-org --token ghp_xxx --output /backup --org \
///   --repositories --issues --concurrency 8
/// ```
///
/// Use the OAuth device flow:
/// ```text
/// github-backup octocat --device-auth --oauth-client-id YOUR_APP_ID \
///   --output /backup --all
/// ```
///
/// Shallow-clone repos and mirror to Codeberg:
/// ```text
/// github-backup octocat --token ghp_xxx --output /backup --repositories \
///   --clone-type shallow:5 \
///   --mirror-to https://codeberg.org \
///   --mirror-token CODEBERG_TOKEN --mirror-owner your_username
/// ```
///
/// Load settings from a config file:
/// ```text
/// github-backup --config /etc/github-backup/config.toml
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
        .required(false)   // relaxed: config file may supply the token
        .args(["token", "device_auth"]),
))]
pub struct Args {
    /// GitHub username or organisation name to back up.
    ///
    /// May be omitted when a `--config` file supplies `owner`.
    #[arg(value_name = "OWNER")]
    pub owner: Option<String>,

    // ── Configuration file ─────────────────────────────────────────────────
    /// Path to a TOML configuration file.
    ///
    /// Values in the file act as defaults; explicit CLI flags take precedence.
    /// See the documentation for the full schema.
    #[arg(long, short = 'c', value_name = "FILE")]
    pub config: Option<PathBuf>,

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
    pub token: Option<String>,

    /// Authenticate interactively using the GitHub OAuth device flow.
    ///
    /// Opens a browser code entry at `github.com/login/device`.
    /// Requires `--oauth-client-id`.
    #[arg(long)]
    pub device_auth: bool,

    /// GitHub OAuth App client ID (required when using `--device-auth`).
    ///
    /// Create an OAuth App at <https://github.com/settings/developers>.
    /// Can also be set via the `GITHUB_OAUTH_CLIENT_ID` environment variable.
    #[arg(
        long,
        value_name = "CLIENT_ID",
        env = "GITHUB_OAUTH_CLIENT_ID",
        requires = "device_auth"
    )]
    pub oauth_client_id: Option<String>,

    /// OAuth scopes to request (space-separated).
    ///
    /// Default: `"repo gist read:org"` — sufficient for a complete backup.
    #[arg(
        long,
        value_name = "SCOPES",
        default_value = "repo gist read:org",
        requires = "device_auth"
    )]
    pub oauth_scopes: String,

    // ── Output ─────────────────────────────────────────────────────────────
    /// Root directory where backup artefacts will be written.
    #[arg(short = 'o', long = "output", value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Write a JSON summary report to this file after the backup completes.
    ///
    /// The report contains counters for every backed-up category.
    /// Useful for monitoring and auditing.
    #[arg(long, value_name = "FILE")]
    pub report: Option<PathBuf>,

    // ── Target type ────────────────────────────────────────────────────────
    /// Treat OWNER as a GitHub organisation (uses the org repos API).
    ///
    /// Without this flag, OWNER is treated as a user account.
    #[arg(long)]
    pub org: bool,

    // ── Broad selectors ────────────────────────────────────────────────────
    /// Enable all backup categories (equivalent to every individual flag,
    /// except `--lfs`, `--prefer-ssh`, `--no-prune`, and `--concurrency`).
    #[arg(long, conflicts_with_all = [
        "repositories", "issues", "issue_comments", "issue_events",
        "pulls", "pull_comments", "pull_commits", "pull_reviews",
        "labels", "milestones", "releases", "release_assets",
        "hooks", "security_advisories", "wikis",
        "starred", "watched", "followers", "following",
        "gists", "starred_gists", "topics", "branches",
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

    /// How to clone repositories.
    ///
    /// Accepted values:
    /// - `mirror` (default) — `git clone --mirror`; complete backup
    /// - `bare`             — `git clone --bare`; no remote-tracking refs
    /// - `full`             — `git clone`; working-tree clone
    /// - `shallow:<depth>`  — `git clone --depth <n>`; limited history
    ///
    /// Example: `--clone-type shallow:10`
    #[arg(long, value_name = "TYPE", default_value = "mirror")]
    pub clone_type: CliCloneType,

    /// Clone with Git LFS support.
    #[arg(long)]
    pub lfs: bool,

    /// Do not prune deleted remote refs during git remote updates.
    #[arg(long)]
    pub no_prune: bool,

    // ── Issue options ──────────────────────────────────────────────────────
    /// Back up issue metadata.
    #[arg(long)]
    pub issues: bool,

    /// Back up issue comment threads.
    #[arg(long)]
    pub issue_comments: bool,

    /// Back up issue timeline events.
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

    /// Download release binary assets.
    ///
    /// Requires `--releases`.
    #[arg(long, requires = "releases")]
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

    // ── Additional repository metadata ─────────────────────────────────────
    /// Back up repository topics (tags).
    #[arg(long)]
    pub topics: bool,

    /// Back up the list of repository branches and their protection status.
    #[arg(long)]
    pub branches: bool,

    // ── Repository name filters ────────────────────────────────────────────
    /// Only back up repositories whose names match this glob pattern.
    ///
    /// Repeat the flag or separate patterns with commas:
    /// `--include-repos "rust-*"` or `--include-repos "foo,bar-*"`.
    ///
    /// Pattern syntax: `*` matches any sequence, `?` matches one character.
    /// Matching is case-insensitive.
    #[arg(long, value_name = "PATTERN", value_delimiter = ',')]
    pub include_repos: Vec<String>,

    /// Exclude repositories whose names match this glob pattern.
    ///
    /// Repeat the flag or separate patterns with commas.
    /// Takes precedence over `--include-repos`.
    #[arg(long, value_name = "PATTERN", value_delimiter = ',')]
    pub exclude_repos: Vec<String>,

    // ── Incremental filter ─────────────────────────────────────────────────
    /// Only fetch issues and pull requests updated at or after this timestamp.
    ///
    /// Accepts ISO 8601 format: `"2024-01-01T00:00:00Z"`.
    /// Useful for incremental backups.
    #[arg(long, value_name = "DATETIME")]
    pub since: Option<String>,

    // ── GitHub Enterprise ──────────────────────────────────────────────────
    /// Override the GitHub API base URL for GitHub Enterprise Server.
    ///
    /// Example: `https://github.example.com/api/v3`
    ///
    /// Defaults to `https://api.github.com`.
    /// Can also be set via the `GITHUB_API_URL` environment variable.
    #[arg(
        long,
        value_name = "URL",
        env = "GITHUB_API_URL",
        hide_env_values = false
    )]
    pub api_url: Option<String>,

    // ── Push-mirror options ────────────────────────────────────────────────
    /// Push repository mirrors to a Gitea-compatible instance after backup.
    ///
    /// Supported hosts: Gitea, Codeberg (<https://codeberg.org>), Forgejo.
    /// Provide the base URL, e.g. `https://codeberg.org`.
    #[arg(long, value_name = "URL")]
    pub mirror_to: Option<String>,

    /// API token for the mirror destination.
    ///
    /// Can also be set via the `MIRROR_TOKEN` environment variable.
    #[arg(
        long,
        value_name = "TOKEN",
        env = "MIRROR_TOKEN",
        hide_env_values = true,
        requires = "mirror_to"
    )]
    pub mirror_token: Option<String>,

    /// Owner name at the mirror destination (username or org).
    #[arg(long, value_name = "OWNER", requires = "mirror_to")]
    pub mirror_owner: Option<String>,

    /// Create repositories as private at the mirror destination.
    #[arg(long, requires = "mirror_to")]
    pub mirror_private: bool,

    // ── S3 storage options ─────────────────────────────────────────────────
    /// S3 bucket to sync backup metadata to.
    ///
    /// Works with AWS S3, Backblaze B2 (S3-compatible), MinIO, Cloudflare R2,
    /// DigitalOcean Spaces, and Wasabi.
    #[arg(long, value_name = "BUCKET")]
    pub s3_bucket: Option<String>,

    /// AWS region for the S3 bucket (e.g., `us-east-1`).
    #[arg(
        long,
        value_name = "REGION",
        default_value = "us-east-1",
        requires = "s3_bucket"
    )]
    pub s3_region: String,

    /// Key prefix for all S3 objects (e.g., `github-backup/`).
    #[arg(
        long,
        value_name = "PREFIX",
        default_value = "",
        requires = "s3_bucket"
    )]
    pub s3_prefix: String,

    /// Custom S3-compatible endpoint (for B2, MinIO, R2, etc.).
    ///
    /// Example for B2: `https://s3.us-west-004.backblazeb2.com`
    #[arg(long, value_name = "URL", requires = "s3_bucket")]
    pub s3_endpoint: Option<String>,

    /// AWS access key ID.
    ///
    /// Can also be set via the `AWS_ACCESS_KEY_ID` environment variable.
    #[arg(
        long,
        value_name = "KEY",
        env = "AWS_ACCESS_KEY_ID",
        hide_env_values = true,
        requires = "s3_bucket"
    )]
    pub s3_access_key: Option<String>,

    /// AWS secret access key.
    ///
    /// Can also be set via the `AWS_SECRET_ACCESS_KEY` environment variable.
    #[arg(
        long,
        value_name = "SECRET",
        env = "AWS_SECRET_ACCESS_KEY",
        hide_env_values = true,
        requires = "s3_bucket"
    )]
    pub s3_secret_key: Option<String>,

    /// Also upload binary release assets to S3 (can be very large).
    ///
    /// By default, only JSON metadata is uploaded; binary release assets
    /// are kept local only.
    #[arg(long, requires = "s3_bucket")]
    pub s3_include_assets: bool,

    // ── Execution ─────────────────────────────────────────────────────────
    /// Maximum number of repositories to back up in parallel.
    ///
    /// Defaults to 4. Set to 1 for sequential operation.
    #[arg(long, value_name = "N", default_value = "4")]
    pub concurrency: usize,

    /// Log what would be done without writing any files or running git.
    #[arg(long)]
    pub dry_run: bool,

    // ── Logging ────────────────────────────────────────────────────────────
    /// Suppress all non-error output.
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Increase log verbosity (`-v` = debug, `-vv` = trace).
    #[arg(long, short = 'v', action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
