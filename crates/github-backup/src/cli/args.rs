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
    /// Enable every backup category in a single flag.
    ///
    /// Equivalent to combining **all** of the following flags:
    ///
    /// Repositories & git:
    ///   `--repositories` `--forks` `--private` `--wikis`
    ///
    /// Issues & pull requests:
    ///   `--issues` `--issue-comments` `--issue-events`
    ///   `--pulls` `--pull-comments` `--pull-commits` `--pull-reviews`
    ///
    /// Repository metadata:
    ///   `--labels` `--milestones` `--releases` `--release-assets`
    ///   `--hooks` `--security-advisories` `--topics` `--branches`
    ///   `--deploy-keys` `--collaborators`
    ///
    /// User / org data:
    ///   `--starred` `--watched` `--followers` `--following`
    ///   `--gists` `--starred-gists`
    ///   `--org-members` `--org-teams`
    ///
    /// GitHub Actions & environments:
    ///   `--actions` `--environments`
    ///
    /// **Not included** (opt-in only, can generate very large output):
    ///   `--action-runs`  — full workflow run history
    ///   `--clone-starred` — clone every starred repository
    ///
    /// **Not controlled by `--all`** (output/behaviour flags):
    ///   `--lfs` `--prefer-ssh` `--no-prune` `--clone-type` `--concurrency`
    #[arg(long, conflicts_with_all = [
        "repositories", "issues", "issue_comments", "issue_events",
        "pulls", "pull_comments", "pull_commits", "pull_reviews",
        "labels", "milestones", "releases", "release_assets",
        "hooks", "security_advisories", "wikis",
        "starred", "watched", "followers", "following",
        "gists", "starred_gists", "topics", "branches",
        "deploy_keys", "collaborators", "org_members", "org_teams",
        "actions", "environments", "discussions", "projects", "packages",
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
    /// Record the list of repositories starred by the owner as JSON.
    #[arg(long)]
    pub starred: bool,

    /// Clone every starred repository as a bare mirror.
    ///
    /// Uses a durable queue at
    /// `<output>/<owner>/json/starred_clone_queue.json` that persists across
    /// runs.  Re-run with this flag to resume an interrupted clone.
    ///
    /// Not included in `--all` because it can consume significant disk space
    /// and time for users with many starred repositories.
    #[arg(long)]
    pub clone_starred: bool,

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

    /// Back up deploy keys for each repository (requires admin access).
    ///
    /// Repositories where the token lacks admin access are skipped silently.
    #[arg(long)]
    pub deploy_keys: bool,

    /// Back up the list of collaborators for each repository (requires admin access).
    ///
    /// Repositories where the token lacks admin access are skipped silently.
    #[arg(long)]
    pub collaborators: bool,

    // ── Organisation data ──────────────────────────────────────────────────
    /// Back up the member list of the organisation (requires `--org`).
    ///
    /// Ignored when backing up a user account.
    #[arg(long)]
    pub org_members: bool,

    /// Back up the team list of the organisation (requires `--org`).
    ///
    /// Ignored when backing up a user account.
    #[arg(long)]
    pub org_teams: bool,

    // ── GitHub Actions ─────────────────────────────────────────────────────
    /// Back up GitHub Actions workflow metadata for each repository.
    ///
    /// Saves `workflows.json` to each repository's metadata directory.
    /// The actual workflow YAML files are already captured by the git clone.
    #[arg(long)]
    pub actions: bool,

    /// Back up GitHub Actions workflow run history.
    ///
    /// For each workflow, saves `workflow_runs_<id>.json`. Can generate very
    /// large files for active repositories; opt in deliberately.
    /// Requires `--actions`.
    #[arg(long, requires = "actions")]
    pub action_runs: bool,

    // ── Deployment environments ────────────────────────────────────────────
    /// Back up deployment environment configurations for each repository.
    ///
    /// Saves `environments.json` with protection rules, required reviewers,
    /// and branch policies.
    #[arg(long)]
    pub environments: bool,

    // ── GitHub Discussions ─────────────────────────────────────────────────
    /// Back up GitHub Discussions threads and their comments.
    ///
    /// Requires the Discussions feature to be enabled on the repository.
    /// Saves `discussions.json` and per-discussion comment files.
    #[arg(long)]
    pub discussions: bool,

    // ── Classic Projects ───────────────────────────────────────────────────
    /// Back up Classic Projects (v1) and their column structure.
    ///
    /// Requires Classic Projects to be enabled on the repository.
    /// Saves `projects.json` and per-project column files.
    #[arg(long)]
    pub projects: bool,

    // ── GitHub Packages ────────────────────────────────────────────────────
    /// Back up GitHub Packages metadata for the target user.
    ///
    /// Requires the `read:packages` OAuth scope.  Iterates over all supported
    /// package ecosystems (container, npm, maven, rubygems, nuget, docker) and
    /// saves package list and version metadata to the owner's JSON directory.
    #[arg(long)]
    pub packages: bool,

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

    /// Override the hostname used in git clone URLs.
    ///
    /// For GitHub Enterprise Server instances where the API host and the git
    /// clone host differ (e.g. behind separate load balancers).  The hostname
    /// in every `clone_url` / `ssh_url` returned by the API is replaced with
    /// this value before it is passed to git.
    ///
    /// Example: `--api-url https://github-api.example.com/api/v3
    ///            --clone-host github-git.example.com`
    ///
    /// Can also be set via the `GITHUB_CLONE_HOST` environment variable.
    #[arg(
        long,
        value_name = "HOST",
        env = "GITHUB_CLONE_HOST",
        hide_env_values = false
    )]
    pub clone_host: Option<String>,

    // ── Push-mirror options ────────────────────────────────────────────────
    /// Push repository mirrors to a remote Git hosting instance after backup.
    ///
    /// Supported destinations depend on `--mirror-type`:
    ///
    /// - `gitea` (default): Gitea, Codeberg (<https://codeberg.org>), Forgejo.
    /// - `gitlab`: GitLab.com or any self-hosted GitLab CE/EE instance.
    ///
    /// Provide the base URL, e.g. `https://codeberg.org` or
    /// `https://gitlab.com`.
    #[arg(long, value_name = "URL")]
    pub mirror_to: Option<String>,

    /// Mirror destination type.
    ///
    /// Accepted values:
    /// - `gitea` (default) — Gitea, Codeberg, Forgejo (Gitea REST API v1)
    /// - `gitlab`          — GitLab.com or self-hosted GitLab CE/EE (REST API v4)
    #[arg(
        long,
        value_name = "TYPE",
        default_value = "gitea",
        requires = "mirror_to"
    )]
    pub mirror_type: String,

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

    /// Owner name at the mirror destination (username or org/namespace).
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
    ///
    /// Defaults to `us-east-1` when not specified.
    #[arg(long, value_name = "REGION", requires = "s3_bucket")]
    pub s3_region: Option<String>,

    /// Key prefix for all S3 objects (e.g., `github-backup/`).
    #[arg(long, value_name = "PREFIX", requires = "s3_bucket")]
    pub s3_prefix: Option<String>,

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
    ///
    /// This explicit `Option` form lets the config file supply the value when
    /// the CLI flag is absent, while still allowing `--concurrency 4` to
    /// override the config file's value correctly.
    #[arg(long, value_name = "N")]
    pub concurrency: Option<usize>,

    /// Log what would be done without writing any files or running git.
    #[arg(long)]
    pub dry_run: bool,

    // ── Manifest & integrity ───────────────────────────────────────────────
    /// Write a SHA-256 hash manifest after the backup completes.
    ///
    /// Writes `<output>/<owner>/json/backup_manifest.json` containing the
    /// SHA-256 digest of every backed-up JSON file.  Use `--verify` on a
    /// subsequent run to confirm the backup has not been tampered with.
    #[arg(long)]
    pub manifest: bool,

    /// Verify the integrity of an existing backup instead of running a backup.
    ///
    /// Reads `<output>/<owner>/json/backup_manifest.json` and checks that
    /// every file's SHA-256 digest matches.  Exits with an error if any
    /// file is missing, changed, or unexpected.
    ///
    /// Requires `--output` and OWNER.  Does not contact the GitHub API.
    #[arg(long, conflicts_with = "all")]
    pub verify: bool,

    // ── Retention / pruning ────────────────────────────────────────────────
    /// Keep only the N most recent backup snapshot directories and delete
    /// older ones.
    ///
    /// Backup snapshots are detected as date-stamped subdirectories under
    /// `<output>` matching the pattern `YYYY-MM-DD*`.  Requires `--output`.
    #[arg(long, value_name = "N")]
    pub keep_last: Option<usize>,

    /// Delete backup snapshot directories older than N days.
    ///
    /// Combined with `--keep-last`, both constraints are applied and
    /// whichever removes more snapshots wins.
    #[arg(long, value_name = "DAYS")]
    pub max_age_days: Option<u64>,

    // ── Prometheus metrics ─────────────────────────────────────────────────
    /// Write Prometheus-compatible metrics to this file after the backup.
    ///
    /// Emits counters for repositories backed up, issues fetched, etc. in the
    /// Prometheus text exposition format.  Useful for push-gateway or node
    /// exporter textfile collector integration.
    #[arg(long, value_name = "FILE")]
    pub prometheus_metrics: Option<std::path::PathBuf>,

    // ── Diff ──────────────────────────────────────────────────────────────
    /// Compare the current backup with a previous backup directory and print
    /// a summary of what changed (repos added/removed, issue counts, etc.).
    ///
    /// Provide the path to the *previous* backup's owner JSON directory
    /// (e.g. `/var/backup/2025-12-01/octocat/json`).  Does not contact the
    /// GitHub API.
    #[arg(long, value_name = "PREV_JSON_DIR")]
    pub diff_with: Option<std::path::PathBuf>,

    // ── Restore ───────────────────────────────────────────────────────────
    /// Restore backed-up data to a GitHub organisation.
    ///
    /// Re-creates issues, labels, and milestones from the JSON backup in
    /// `<output>/<owner>/json` to the target organisation.  Requires
    /// `--restore-target-org` and a token with write access.
    ///
    /// **Warning:** This modifies GitHub data.  Use with care.
    #[arg(long)]
    pub restore: bool,

    /// Target organisation for `--restore`.
    #[arg(long, value_name = "ORG", requires = "restore")]
    pub restore_target_org: Option<String>,

    // ── Encryption ────────────────────────────────────────────────────────
    /// Encrypt backup data before writing to S3 using AES-256-GCM.
    ///
    /// Provide a 32-byte hex-encoded encryption key (64 hex characters).
    /// Can also be set via the `BACKUP_ENCRYPT_KEY` environment variable.
    ///
    /// The key is never written to disk or logged.
    #[arg(
        long,
        value_name = "HEX_KEY",
        env = "BACKUP_ENCRYPT_KEY",
        hide_env_values = true,
        requires = "s3_bucket"
    )]
    pub encrypt_key: Option<String>,

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
