// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Top-level [`Args`] struct parsed from the command line.

use std::path::PathBuf;

use clap::builder::{PossibleValuesParser, TypedValueParser as _};
use clap::{ArgGroup, Parser};

use github_backup_types::config::MirrorType;

use super::categories;
use super::clone_type::CliCloneType;
use crate::notify::NotifyFormat;

/// GitHub backup tool.
///
/// Backs up repositories, issues, pull requests, releases, gists, wikis, and
/// relationship data for a GitHub user or organisation.
///
/// # Authentication
///
/// The token is taken from `--token`, `GITHUB_TOKEN`, `GH_TOKEN`, the config
/// file, or the GitHub CLI (`gh auth token`), in that order; `--device-auth`
/// authenticates interactively via the GitHub OAuth device flow instead
/// (requires a registered OAuth App — see `--oauth-client-id`).
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
/// Use `--s3-bucket` (and related flags) to sync the JSON metadata (and, with
/// `--s3-include-assets`, release assets) to any S3-compatible object store
/// (AWS, Backblaze B2, MinIO, …).  Repository clones are not uploaded.
///
/// # Configuration File
///
/// Load defaults from a TOML configuration file with `--config <FILE>`
/// (default: `~/.config/github-backup/config.toml` when it exists).  A value
/// on the command line wins over the file; a switch set in either is on.
///
/// # Examples
///
/// Back up everything the token's account owns (token from `GITHUB_TOKEN`,
/// `GH_TOKEN` or `gh auth login`):
/// ```text
/// github-backup --output /backup
/// ```
///
/// Back up only repositories and issues for an org, with 8 parallel workers:
/// ```text
/// github-backup my-org --output /backup --repositories --issues --concurrency 8
/// ```
///
/// Use the OAuth device flow:
/// ```text
/// github-backup octocat --device-auth --oauth-client-id YOUR_APP_ID \
///   --output /backup
/// ```
///
/// Back up repositories and mirror them to Codeberg (token in MIRROR_TOKEN;
/// mirrors need full clones, so keep the default clone type):
/// ```text
/// github-backup octocat --output /backup --repositories \
///   --mirror-to https://codeberg.org --mirror-owner your_username
/// ```
///
/// Load settings from a config file:
/// ```text
/// github-backup --config /etc/github-backup/config.toml
/// ```
#[derive(Debug, Clone, Parser)]
#[command(
    name = "github-backup",
    version,
    about = "GitHub backup: repositories, issues, PRs, releases, gists, wikis, and metadata",
    long_about = None,
    after_help = "EXAMPLES:\n\
        \n  \
        Back up everything your token can reach (after `gh auth login`):\n    \
        github-backup --output ~/github-backups\n\
        \n  \
        Back up an organisation, leaving out webhooks and packages:\n    \
        github-backup my-org --output ~/github-backups --skip hooks,packages\n\
        \n  \
        Keep running and back up once a day (containers, services):\n    \
        github-backup --output /backup --every 24h\n\
        \n  \
        Launch the interactive TUI:\n    \
        github-backup octocat --tui\n\
        \n  \
        Diagnose problems before scheduling cron:\n    \
        github-backup octocat --doctor\n\
        \n  \
        Print the OAuth scopes needed for the current flag set:\n    \
        github-backup myorg --org --all --list-scopes\n\
        \n  \
        Validate config + connectivity without performing a backup:\n    \
        github-backup --config /etc/github-backup/config.toml --check\n\
        \n  \
        Mirror everything to Codeberg afterwards:\n    \
        github-backup octocat --output ~/gh \\\n      \
        --mirror-to https://codeberg.org \\\n      \
        --mirror-token $CODEBERG_TOKEN --mirror-owner alice\n\
        \n\
        EXIT STATUS:\n  \
        0  everything that was asked for succeeded\n  \
        1  the run could not be carried out (bad config, rejected token, ...)\n  \
        2  usage error\n  \
        3  the run finished but some items could not be backed up\n  \
        130/143  interrupted (Ctrl+C / SIGTERM)\n\
        With --every, a stop while waiting for the next run exits 0.\n\
        \n\
        MORE:  https://tomtom215.github.io/github-backup-rust/\n",
)]
#[command(group(
    ArgGroup::new("auth")
        .required(false)   // relaxed: config file may supply the token
        .args(["token", "device_auth"]),
))]
#[command(group(
    // Modes that replace the backup: at most one per invocation.
    ArgGroup::new("mode")
        .multiple(false)
        .args([
            "tui", "list_scopes", "doctor", "check", "verify", "decrypt",
            "restore", "every", "print_config_template", "completions",
        ]),
))]
pub struct Args {
    /// GitHub username or organisation name to back up.
    ///
    /// Defaults to `owner` from the config file, or else to the account the
    /// token belongs to.  An organisation is recognised as one even without
    /// `--org`.
    #[arg(value_name = "OWNER")]
    pub owner: Option<String>,

    // ── Configuration file ─────────────────────────────────────────────────
    /// Path to a TOML configuration file.
    ///
    /// Without this flag, `$XDG_CONFIG_HOME/github-backup/config.toml`
    /// (`~/.config/github-backup/config.toml`; `%APPDATA%\github-backup\config.toml`
    /// on Windows) is loaded when it exists.  A value given on the command line
    /// wins over the file; a switch (`--issues`, `--org`, ...) is on when
    /// either turns it on, and `--skip` removes categories.
    #[arg(help_heading = "Configuration", long, short = 'c', value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Print an annotated TOML configuration template to stdout and exit.
    ///
    /// Useful for bootstrapping a new deployment:
    ///
    /// ```text
    /// github-backup --print-config-template > /etc/github-backup/config.toml
    /// chmod 600 /etc/github-backup/config.toml
    /// $EDITOR /etc/github-backup/config.toml
    /// github-backup --config /etc/github-backup/config.toml
    /// ```
    ///
    /// All values are commented out; uncomment and edit the ones you need.
    #[arg(help_heading = "Configuration", verbatim_doc_comment, long)]
    pub print_config_template: bool,

    /// Print a shell completion script to stdout and exit.
    ///
    /// Example: `github-backup --completions bash > /etc/bash_completion.d/github-backup`
    #[arg(help_heading = "Configuration", long, value_name = "SHELL")]
    pub completions: Option<clap_complete::Shell>,

    /// Run a self-diagnostic and exit.
    ///
    /// Checks the prerequisites for a working backup, in order:
    ///
    /// 1. `git` is installed (its version is reported; old versions warn).
    /// 2. The output directory is writable.
    /// 3. A GitHub credential is configured (or anonymous mode is OK).
    /// 4. Mirror and S3 settings are complete, when they are used.
    /// 5. The API (`api.github.com` or `--api-url`, through any
    ///    `HTTPS_PROXY`) is reachable and the token is accepted.
    /// 6. OWNER exists (without OWNER: which account the token belongs
    ///    to), and whether it is a user or an organisation.
    ///
    /// The configuration is validated first, exactly as for a backup.  It
    /// does not check OAuth scopes: use `--list-scopes` for the scopes the
    /// enabled categories need.
    ///
    /// Prints a coloured pass/fail summary and exits 0 if every check
    /// passed, 1 otherwise.  This is the fastest way to confirm a fresh
    /// install will succeed before scheduling a real run in cron / CI.
    #[arg(help_heading = "Configuration", verbatim_doc_comment, long)]
    pub doctor: bool,

    /// Validate the configuration and authentication, then exit without
    /// performing a backup.
    ///
    /// Runs the same checks as `--doctor`, then prints the resolved owner,
    /// output directory, API URL, concurrency and recommended token scopes.
    /// Use it to confirm what a configuration resolves to.
    #[arg(help_heading = "Configuration", long)]
    pub check: bool,

    /// Print the OAuth scopes recommended for the currently-enabled
    /// categories and exit.
    ///
    /// Useful when creating a personal access token: run with the same
    /// flags you intend to use, then paste the printed scope list into
    /// the token creation page.
    #[arg(help_heading = "Configuration", long)]
    pub list_scopes: bool,

    // ── Authentication ─────────────────────────────────────────────────────
    /// Personal access token (classic or fine-grained).
    ///
    /// Can also be set via the `GITHUB_TOKEN` or `GH_TOKEN` environment
    /// variable (preferred: a value on the command line is visible to other
    /// users in the process list).  Without any of these, the config file's
    /// `token` and then the GitHub CLI's login (`gh auth token`) are used.
    #[arg(
        help_heading = "Authentication",
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
    #[arg(help_heading = "Authentication", long)]
    pub device_auth: bool,

    /// GitHub OAuth App client ID (required when using `--device-auth`).
    ///
    /// Create an OAuth App at <https://github.com/settings/developers>.
    /// Can also be set via the `GITHUB_OAUTH_CLIENT_ID` environment variable,
    /// which is ignored unless `--device-auth` is given.
    //
    // No clap `requires = "device_auth"` here: clap would also apply it to the
    // environment variable, so merely having `GITHUB_OAUTH_CLIENT_ID` set (or
    // forwarded as an empty string by Compose/Unraid) would reject every run.
    // `Args::check_dependencies` enforces it for the command-line flag only.
    #[arg(
        help_heading = "Authentication",
        long,
        value_name = "CLIENT_ID",
        env = "GITHUB_OAUTH_CLIENT_ID"
    )]
    pub oauth_client_id: Option<String>,

    /// OAuth scopes to request (space-separated).
    ///
    /// Default: `"repo gist read:org read:packages read:project"` — every
    /// category of the default backup except webhooks, which need the
    /// administrative `admin:repo_hook` (without it, webhooks are skipped
    /// with a warning).  `--list-scopes` shows what the categories you enabled
    /// need.
    #[arg(
        help_heading = "Authentication",
        long,
        value_name = "SCOPES",
        default_value = "repo gist read:org read:packages read:project",
        requires = "device_auth"
    )]
    pub oauth_scopes: String,

    // ── Output ─────────────────────────────────────────────────────────────
    /// Root directory where backup artefacts will be written.
    #[arg(
        help_heading = "Output",
        short = 'o',
        long = "output",
        value_name = "DIR"
    )]
    pub output: Option<PathBuf>,

    /// Write a JSON summary report to this file after the backup completes.
    ///
    /// The report contains counters for every backed-up category.
    /// Useful for monitoring and auditing.
    #[arg(help_heading = "Output", long, value_name = "FILE")]
    pub report: Option<PathBuf>,

    // ── Target type ────────────────────────────────────────────────────────
    /// Treat OWNER as a GitHub organisation (uses the org repos API).
    ///
    /// Without this flag, OWNER is treated as a user account.
    #[arg(help_heading = "Target", long)]
    pub org: bool,

    // ── Broad selectors ────────────────────────────────────────────────────
    /// Enable every backup category in a single flag.
    ///
    /// This is also what a run does when no category is selected at all (on
    /// the command line or in the config file).  Equivalent to combining
    /// **all** of the following flags:
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
    /// Projects, packages, commit comments and attachments:
    ///   `--projects` `--packages` `--commit-comments` `--attachments`
    ///
    /// **Not included**:
    ///   `--action-runs`  — full workflow run history (can be very large)
    ///   `--clone-starred` — clone every starred repository (can be very large)
    ///   `--discussions`  — not backed up yet (GitHub serves them through
    ///   GraphQL only)
    ///
    /// **Not controlled by `--all`** (output/behaviour flags):
    ///   `--lfs` `--prefer-ssh` `--prune` `--clone-type` `--concurrency`
    #[arg(help_heading = "What to back up", long, conflicts_with_all = [
        "repositories", "issues", "issue_comments", "issue_events",
        "pulls", "pull_comments", "pull_commits", "pull_reviews",
        "labels", "milestones", "releases", "release_assets",
        "hooks", "security_advisories", "wikis",
        "starred", "watched", "followers", "following",
        "gists", "starred_gists", "topics", "branches",
        "deploy_keys", "collaborators", "org_members", "org_teams",
        "actions", "environments", "discussions", "projects", "packages",
        "commit_comments", "attachments",
    ])]
    pub all: bool,

    /// Leave these categories out (comma-separated or repeated).
    ///
    /// Works on the default (everything), `--all` and categories from the
    /// config file alike, e.g. `--skip hooks,packages,release-assets`.  The
    /// names are those of the category flags without `--`; `forks` and
    /// `private` leave out forked and private repositories.
    #[arg(
        help_heading = "What to back up",
        long,
        value_name = "CATEGORY",
        value_delimiter = ',',
        value_parser = PossibleValuesParser::new(categories::names())
    )]
    pub skip: Vec<String>,

    // ── Repository options ─────────────────────────────────────────────────
    /// Clone/mirror repositories.
    #[arg(help_heading = "Repositories and git", long)]
    pub repositories: bool,

    /// Include forked repositories.
    #[arg(help_heading = "Repositories and git", long, short = 'F')]
    pub forks: bool,

    /// Include private repositories (requires appropriate token scope).
    #[arg(help_heading = "Repositories and git", long, short = 'P')]
    pub private: bool,

    /// Clone using SSH URLs instead of HTTPS.
    #[arg(help_heading = "Repositories and git", long)]
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
    #[arg(
        help_heading = "Repositories and git",
        verbatim_doc_comment,
        long,
        value_name = "TYPE",
        default_value = "mirror"
    )]
    pub clone_type: CliCloneType,

    /// Clone with Git LFS support.
    #[arg(help_heading = "Repositories and git", long)]
    pub lfs: bool,

    /// Delete branches and tags from the local clone, and from the
    /// `--mirror-to` destination, when they were deleted on GitHub.
    ///
    /// Off by default: a backup keeps what GitHub no longer has, so a deleted
    /// branch or tag stays recoverable.  Branches that were force-pushed are
    /// overwritten either way (their old commits are not kept).  Turn this on
    /// to make each clone and each mirror an exact copy of GitHub's current
    /// refs.
    #[arg(
        help_heading = "Repositories and git",
        long,
        conflicts_with = "no_prune"
    )]
    pub prune: bool,

    /// Deprecated and ignored: not pruning is now the default.
    #[arg(long, hide = true)]
    pub no_prune: bool,

    /// Check every repository with `git fsck` after each clone and update.
    ///
    /// A repository that fails the check is recorded as a failure (exit
    /// status 3); a fresh clone that fails is discarded rather than kept.
    /// Off by default because it reads every object of every repository, which
    /// takes a while on large ones; a weekly run with it on is a good habit.
    #[arg(help_heading = "Repositories and git", long)]
    pub verify_git: bool,

    /// Seconds a git command may go without any output before it is stopped
    /// as stalled (default 600).
    ///
    /// Raise it for very slow links or huge repositories.  It is a limit on
    /// silence, not on total time.
    #[arg(
        help_heading = "Repositories and git",
        long,
        value_name = "SECS",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub git_timeout: Option<u64>,

    // ── Issue options ──────────────────────────────────────────────────────
    /// Back up issue metadata.
    #[arg(help_heading = "What to back up", long)]
    pub issues: bool,

    /// Back up issue comment threads.
    #[arg(help_heading = "What to back up", long)]
    pub issue_comments: bool,

    /// Back up issue timeline events.
    #[arg(help_heading = "What to back up", long)]
    pub issue_events: bool,

    // ── Pull request options ───────────────────────────────────────────────
    /// Back up pull request metadata.
    #[arg(help_heading = "What to back up", long)]
    pub pulls: bool,

    /// Back up pull request review comments.
    #[arg(help_heading = "What to back up", long)]
    pub pull_comments: bool,

    /// Back up pull request commit lists.
    #[arg(help_heading = "What to back up", long)]
    pub pull_commits: bool,

    /// Back up pull request reviews.
    #[arg(help_heading = "What to back up", long)]
    pub pull_reviews: bool,

    // ── Repository metadata ────────────────────────────────────────────────
    /// Back up repository labels.
    #[arg(help_heading = "What to back up", long)]
    pub labels: bool,

    /// Back up repository milestones.
    #[arg(help_heading = "What to back up", long)]
    pub milestones: bool,

    /// Back up release metadata.
    #[arg(help_heading = "What to back up", long)]
    pub releases: bool,

    /// Download release binary assets.
    ///
    /// Requires `--releases`.
    #[arg(help_heading = "What to back up", long, requires = "releases")]
    pub release_assets: bool,

    /// Back up webhook configurations (requires admin token scope).
    #[arg(help_heading = "What to back up", long)]
    pub hooks: bool,

    /// Back up published security advisories.
    #[arg(help_heading = "What to back up", long)]
    pub security_advisories: bool,

    /// Clone repository wikis.
    #[arg(help_heading = "What to back up", long)]
    pub wikis: bool,

    // ── User / org data ────────────────────────────────────────────────────
    /// Record the list of repositories starred by the owner as JSON.
    #[arg(help_heading = "What to back up", long)]
    pub starred: bool,

    /// Clone every starred repository as a bare mirror.
    ///
    /// Uses a durable queue at
    /// `<output>/<owner>/json/starred_clone_queue.json` that persists across
    /// runs.  Re-run with this flag to resume an interrupted clone.
    ///
    /// Not included in `--all` because it can consume significant disk space
    /// and time for users with many starred repositories.
    #[arg(help_heading = "What to back up", long)]
    pub clone_starred: bool,

    /// Back up repositories watched by the owner.
    #[arg(help_heading = "What to back up", long)]
    pub watched: bool,

    /// Back up the owner's follower list.
    #[arg(help_heading = "What to back up", long)]
    pub followers: bool,

    /// Back up the list of accounts the owner follows.
    #[arg(help_heading = "What to back up", long)]
    pub following: bool,

    /// Back up gists owned by the owner.
    #[arg(help_heading = "What to back up", long)]
    pub gists: bool,

    /// Back up gists starred by the authenticated user.
    #[arg(help_heading = "What to back up", long)]
    pub starred_gists: bool,

    // ── Additional repository metadata ─────────────────────────────────────
    /// Back up repository topics (tags).
    #[arg(help_heading = "What to back up", long)]
    pub topics: bool,

    /// Back up the list of repository branches and their protection status.
    #[arg(help_heading = "What to back up", long)]
    pub branches: bool,

    /// Back up deploy keys for each repository (requires admin access).
    ///
    /// Repositories where the token lacks admin access are skipped silently.
    #[arg(help_heading = "What to back up", long)]
    pub deploy_keys: bool,

    /// Back up the list of collaborators for each repository (requires admin access).
    ///
    /// Repositories where the token lacks admin access are skipped silently.
    #[arg(help_heading = "What to back up", long)]
    pub collaborators: bool,

    // ── Organisation data ──────────────────────────────────────────────────
    /// Back up the member list of the organisation (requires `--org`).
    ///
    /// Ignored when backing up a user account.
    #[arg(help_heading = "What to back up", long)]
    pub org_members: bool,

    /// Back up the team list of the organisation (requires `--org`).
    ///
    /// Ignored when backing up a user account.
    #[arg(help_heading = "What to back up", long)]
    pub org_teams: bool,

    // ── GitHub Actions ─────────────────────────────────────────────────────
    /// Back up GitHub Actions workflow metadata for each repository.
    ///
    /// Saves `workflows.json` to each repository's metadata directory.
    /// The actual workflow YAML files are already captured by the git clone.
    #[arg(help_heading = "What to back up", long)]
    pub actions: bool,

    /// Back up GitHub Actions workflow run history.
    ///
    /// For each workflow, saves `workflow_runs_<id>.json`. Can generate very
    /// large files for active repositories; opt in deliberately.  Only has
    /// an effect while `--actions` is on (as it is by default).
    #[arg(help_heading = "What to back up", long)]
    pub action_runs: bool,

    // ── Deployment environments ────────────────────────────────────────────
    /// Back up deployment environment configurations for each repository.
    ///
    /// Saves `environments.json` with protection rules, required reviewers,
    /// and branch policies.
    #[arg(help_heading = "What to back up", long)]
    pub environments: bool,

    // ── GitHub Discussions ─────────────────────────────────────────────────
    /// Request GitHub Discussions.  Not backed up yet: GitHub serves them
    /// through GraphQL only, and the run logs a warning saying so.
    #[arg(help_heading = "What to back up", long)]
    pub discussions: bool,

    // ── Projects ───────────────────────────────────────────────────────────
    /// Back up the owner's Projects: each project, its fields, and its items
    /// with every field value, under `json/projects/`.
    ///
    /// Needs the `read:project` scope (classic token) or Projects read access
    /// (fine-grained token).
    #[arg(help_heading = "What to back up", long)]
    pub projects: bool,

    // ── Commit comments ────────────────────────────────────────────────────
    /// Back up the comments made on commits of each repository
    /// (`commit_comments.json`).
    #[arg(help_heading = "What to back up", long)]
    pub commit_comments: bool,

    // ── Attachments ────────────────────────────────────────────────────────
    /// Download the files and images attached to issues, pull requests,
    /// comments, reviews and release notes, under `attachments/` in each
    /// repository's metadata directory.
    ///
    /// Private repositories' attachments need a classic token: GitHub is
    /// reported to refuse fine-grained and GitHub App tokens.
    #[arg(help_heading = "What to back up", long)]
    pub attachments: bool,

    // ── GitHub Packages ────────────────────────────────────────────────────
    /// Back up GitHub Packages metadata for the target user.
    ///
    /// Requires the `read:packages` OAuth scope.  Iterates over all supported
    /// package ecosystems (container, npm, maven, rubygems, nuget, docker) and
    /// saves package list and version metadata to the owner's JSON directory.
    #[arg(help_heading = "What to back up", long)]
    pub packages: bool,

    // ── Repository name filters ────────────────────────────────────────────
    /// Only back up repositories whose names match this glob pattern.
    ///
    /// Repeat the flag or separate patterns with commas:
    /// `--include-repos "rust-*"` or `--include-repos "foo,bar-*"`.
    ///
    /// Pattern syntax: `*` matches any sequence, `?` matches one character.
    /// Matching is case-insensitive.
    #[arg(
        help_heading = "Filters",
        long,
        value_name = "PATTERN",
        value_delimiter = ','
    )]
    pub include_repos: Vec<String>,

    /// Exclude repositories whose names match this glob pattern.
    ///
    /// Repeat the flag or separate patterns with commas.
    /// Takes precedence over `--include-repos`.
    #[arg(
        help_heading = "Filters",
        long,
        value_name = "PATTERN",
        value_delimiter = ','
    )]
    pub exclude_repos: Vec<String>,

    // ── Incremental behaviour ──────────────────────────────────────────────
    /// Treat everything updated before DATE as already backed up.
    ///
    /// Issue and pull request lists are always fetched in full and merged into
    /// the existing backup, so this never loses data.  It only skips
    /// re-fetching the comments, events, commits and reviews of items that have
    /// not changed since DATE and whose files already exist.
    ///
    /// Accepts `2024-01-01` or `2024-01-01T00:00:00Z`.  Without this flag each
    /// repository's own watermark from the previous run is used.
    #[arg(
        help_heading = "Incremental",
        long,
        value_name = "DATE",
        conflicts_with = "full"
    )]
    pub since: Option<String>,

    /// Ignore incremental state and fetch everything again.
    ///
    /// Re-fetches the comments, events, commits and reviews of every issue and
    /// pull request even if nothing changed since the previous run.
    #[arg(help_heading = "Incremental", long)]
    pub full: bool,

    // ── GitHub Enterprise ──────────────────────────────────────────────────
    /// Override the GitHub API base URL for GitHub Enterprise Server.
    ///
    /// Example: `https://github.example.com/api/v3`
    ///
    /// Defaults to `https://api.github.com`.
    /// Can also be set via the `GITHUB_API_URL` environment variable.
    #[arg(
        help_heading = "GitHub Enterprise",
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
        help_heading = "GitHub Enterprise",
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
    #[arg(
        help_heading = "Mirroring",
        verbatim_doc_comment,
        long,
        value_name = "URL"
    )]
    pub mirror_to: Option<String>,

    /// Mirror destination type.
    ///
    /// Accepted values:
    /// - `gitea` (default) — Gitea, Codeberg, Forgejo (Gitea REST API v1)
    /// - `gitlab`          — GitLab.com or self-hosted GitLab CE/EE (REST API v4)
    //
    // This and the other mirror and S3 settings need `--mirror-to` /
    // `--s3-bucket`, from the command line or the config file, so the check
    // is made after the merge (`Args::check_dependencies`), not by clap.
    #[arg(
        help_heading = "Mirroring",
        verbatim_doc_comment,
        long,
        value_name = "TYPE",
        value_parser = PossibleValuesParser::new(["gitea", "gitlab"])
            .map(|s| s.parse::<MirrorType>().expect("one of the possible values"))
    )]
    pub mirror_type: Option<MirrorType>,

    /// API token for the mirror destination.
    ///
    /// Can also be set via the `MIRROR_TOKEN` environment variable, which is
    /// ignored unless `--mirror-to` is given.
    //
    // No clap `requires = "mirror_to"`: see `oauth_client_id`.
    #[arg(
        help_heading = "Mirroring",
        long,
        value_name = "TOKEN",
        env = "MIRROR_TOKEN",
        hide_env_values = true
    )]
    pub mirror_token: Option<String>,

    /// Owner name at the mirror destination (username or org/namespace).
    #[arg(help_heading = "Mirroring", long, value_name = "OWNER")]
    pub mirror_owner: Option<String>,

    /// Create every mirror repository as private.  This is the default.
    #[arg(help_heading = "Mirroring", long, conflicts_with = "mirror_public")]
    pub mirror_private: bool,

    /// Create mirrors of public repositories as public.
    ///
    /// Without this flag every mirror is created private.  A repository counts
    /// as public only if the backup's `repos.json` lists it as public; a
    /// private source (or one whose visibility is unknown) is mirrored
    /// privately whatever this flag says.  It affects repositories as they are
    /// created: an existing mirror keeps its visibility.
    #[arg(help_heading = "Mirroring", long, conflicts_with = "mirror_private")]
    pub mirror_public: bool,

    // ── S3 storage options ─────────────────────────────────────────────────
    /// S3 bucket to sync backup metadata to (the bucket must already exist).
    ///
    /// Uploads the JSON metadata under `<prefix>/<owner>/json/`; repository
    /// clones are NOT uploaded.  Works with AWS S3, Backblaze B2, MinIO,
    /// Cloudflare R2, DigitalOcean Spaces, and Wasabi.  Failed uploads make
    /// the run fail.
    #[arg(help_heading = "S3 storage", long, value_name = "BUCKET")]
    pub s3_bucket: Option<String>,

    /// AWS region for the S3 bucket (e.g., `us-east-1`).
    ///
    /// Defaults to `us-east-1` when not specified.
    #[arg(help_heading = "S3 storage", long, value_name = "REGION")]
    pub s3_region: Option<String>,

    /// Key prefix for all S3 objects (e.g., `github-backup/`).
    ///
    /// Objects are stored as `<prefix>/<owner>/json/<path>`.  A trailing
    /// slash is optional.
    #[arg(help_heading = "S3 storage", long, value_name = "PREFIX")]
    pub s3_prefix: Option<String>,

    /// Custom S3-compatible endpoint, with scheme (for B2, MinIO, R2, etc.).
    ///
    /// Example for B2: `https://s3.us-west-004.backblazeb2.com`.  Private CAs
    /// are trusted through `SSL_CERT_FILE`.  `HTTPS_PROXY` is not used.
    #[arg(help_heading = "S3 storage", long, value_name = "URL")]
    pub s3_endpoint: Option<String>,

    /// AWS access key ID.
    ///
    /// Can also be set via the `AWS_ACCESS_KEY_ID` environment variable, which
    /// is ignored unless `--s3-bucket` is given — so having AWS credentials
    /// exported for other tools never affects a backup that does not use S3.
    //
    // No clap `requires = "s3_bucket"`: see `oauth_client_id`.
    #[arg(
        help_heading = "S3 storage",
        long,
        value_name = "KEY",
        env = "AWS_ACCESS_KEY_ID",
        hide_env_values = true
    )]
    pub s3_access_key: Option<String>,

    /// AWS secret access key.
    ///
    /// Can also be set via the `AWS_SECRET_ACCESS_KEY` environment variable,
    /// which is ignored unless `--s3-bucket` is given.
    //
    // No clap `requires = "s3_bucket"`: see `oauth_client_id`.
    #[arg(
        help_heading = "S3 storage",
        long,
        value_name = "SECRET",
        env = "AWS_SECRET_ACCESS_KEY",
        hide_env_values = true
    )]
    pub s3_secret_key: Option<String>,

    /// Session token for temporary AWS credentials.
    ///
    /// Can also be set via the `AWS_SESSION_TOKEN` environment variable, which
    /// is ignored unless `--s3-bucket` is given.
    //
    // No clap `requires = "s3_bucket"`: see `oauth_client_id`.
    #[arg(
        help_heading = "S3 storage",
        long,
        value_name = "TOKEN",
        env = "AWS_SESSION_TOKEN",
        hide_env_values = true
    )]
    pub s3_session_token: Option<String>,

    /// Also upload binary release assets to S3 (can be very large).
    ///
    /// By default, only JSON metadata is uploaded; binary release assets
    /// are kept local only.  Dropping this flag later never deletes assets
    /// that are already in the bucket.
    #[arg(help_heading = "S3 storage", long)]
    pub s3_include_assets: bool,

    /// Delete S3 objects that no longer exist in the local backup.
    ///
    /// After the upload phase, lists the objects under
    /// `<prefix>/<owner>/json/` and deletes those whose local file is gone.
    /// Nothing is deleted when the backup run had failures, when the local
    /// tree could not be fully read or holds no files, or with `--dry-run`
    /// (which only lists what would go).  Release assets that were merely not
    /// uploaded this time are kept.
    ///
    /// **Use with caution** — deletion is permanent unless the bucket has
    /// versioning enabled.
    #[arg(help_heading = "S3 storage", long)]
    pub s3_delete_stale: bool,

    // ── Execution ─────────────────────────────────────────────────────────
    /// Keep running: back up now, then again every INTERVAL.
    ///
    /// For containers and services that should not depend on cron.  INTERVAL
    /// is a whole number with a unit, `s`, `m`, `h` or `d` (`30m`, `6h`,
    /// `24h`, `7d`; at least one minute).  Each run starts INTERVAL after the
    /// previous one started, or at once if that one took longer.  A run that
    /// fails is logged and the next one still happens.  SIGINT / SIGTERM
    /// while waiting exits 0; during a run they interrupt it as usual.  The
    /// output lock is only held while a run is in progress.
    #[arg(
        help_heading = "Execution",
        long,
        value_name = "INTERVAL",
        value_parser = crate::schedule::parse_interval
    )]
    pub every: Option<std::time::Duration>,

    /// Maximum number of repositories to back up in parallel.
    ///
    /// Defaults to 4. Set to 1 for sequential operation.  Each repository is
    /// backed up by one worker (clone, wiki and metadata), so a higher value
    /// finishes sooner but makes more simultaneous requests to GitHub.
    #[arg(help_heading = "Execution", long, value_name = "N")]
    pub concurrency: Option<usize>,

    /// Log what would be done without writing any files or running git.
    #[arg(help_heading = "Execution", long)]
    pub dry_run: bool,

    // ── Manifest & integrity ───────────────────────────────────────────────
    /// Write a SHA-256 hash manifest after the backup completes.
    ///
    /// Writes `<output>/<owner>/json/backup_manifest.json` containing the
    /// SHA-256 digest of every backed-up JSON file.  Use `--verify` on a
    /// subsequent run to confirm the backup has not been tampered with.
    #[arg(help_heading = "Integrity", long)]
    pub manifest: bool,

    /// Verify the integrity of an existing backup instead of running a backup.
    ///
    /// Reads `<output>/<owner>/json/backup_manifest.json` and checks that
    /// every file's SHA-256 digest matches.  Exits with an error if any
    /// file is missing, changed, or unexpected.
    ///
    /// Requires OWNER (`--output` defaults to the current directory).  Does
    /// not contact the GitHub API.
    #[arg(help_heading = "Integrity", long)]
    pub verify: bool,

    // ── Retention / pruning ────────────────────────────────────────────────
    /// Deprecated and ignored (a warning is logged): nothing is deleted.
    /// Still accepted so that existing scheduled jobs keep running.
    #[arg(long, value_name = "N", hide = true)]
    pub keep_last: Option<usize>,

    /// Deprecated and ignored, like `--keep-last`.
    #[arg(long, value_name = "DAYS", hide = true)]
    pub max_age_days: Option<u64>,

    // ── Prometheus metrics ─────────────────────────────────────────────────
    /// Write Prometheus-compatible metrics to this file after the backup.
    ///
    /// Emits counters for repositories backed up, issues fetched, etc. in the
    /// Prometheus text exposition format.  Useful for push-gateway or node
    /// exporter textfile collector integration.
    #[arg(help_heading = "Monitoring and logging", long, value_name = "FILE")]
    pub prometheus_metrics: Option<std::path::PathBuf>,

    // ── Diff ──────────────────────────────────────────────────────────────
    /// Compare the current backup with a previous backup directory and print
    /// a summary of which repositories were added or removed.
    ///
    /// Provide the path to the *previous* backup's owner JSON directory
    /// (e.g. `/var/backup/2025-12-01/octocat/json`).  Does not contact the
    /// GitHub API.
    #[arg(help_heading = "Integrity", long, value_name = "PREV_JSON_DIR")]
    pub diff_with: Option<std::path::PathBuf>,

    // ── Restore ───────────────────────────────────────────────────────────
    /// Restore backed-up data to a GitHub organisation.
    ///
    /// Re-creates issues, labels, and milestones from the local JSON backup in
    /// `<output>/<owner>/json` in the matching repositories of the target
    /// (they must already exist).  This is a mode of its own: no backup is
    /// made and the source is not read, so it works after the source is gone.
    /// Safe to repeat: issues restored earlier are recognised and skipped.
    /// The target is `--restore-target-org`; restoring into OWNER itself
    /// (where the original issues may still be) needs `--restore-into-source`.
    /// A token with write access is required.
    ///
    /// **Warning:** This modifies GitHub data.  Use with care.
    #[arg(help_heading = "Restore", long)]
    pub restore: bool,

    /// Target organisation (or user) for `--restore`.
    #[arg(
        help_heading = "Restore",
        long,
        value_name = "ORG",
        requires = "restore"
    )]
    pub restore_target_org: Option<String>,

    /// Allow `--restore` to write into OWNER, the account the backup is of.
    ///
    /// Without it the target must be another account: restoring into a source
    /// whose issues still exist creates a second, open copy of every issue.
    #[arg(help_heading = "Restore", long, requires = "restore")]
    pub restore_into_source: bool,

    /// Skip the interactive confirmation prompt for `--restore`.
    ///
    /// By default `--restore` prints a warning banner and requires either
    /// interactive confirmation (TTY) or this flag (non-interactive / CI).
    /// Pass `--restore-yes` to acknowledge the warning and proceed without
    /// user input.
    #[arg(help_heading = "Restore", long, requires = "restore")]
    pub restore_yes: bool,

    // ── Encryption ────────────────────────────────────────────────────────
    /// Encrypt backup data before writing to S3 using AES-256-GCM.
    ///
    /// Provide a 32-byte hex-encoded encryption key (64 hex characters).
    /// Objects get a `.enc` suffix; object names and sizes stay visible.
    /// Losing the key makes the encrypted objects unrecoverable.
    /// **Prefer** supplying the key via the `BACKUP_ENCRYPT_KEY` environment
    /// variable rather than on the command line — a CLI flag is visible to
    /// any user running `ps aux` on the same host.
    ///
    /// Can also be set via the `BACKUP_ENCRYPT_KEY` environment variable.
    ///
    /// The key is never written to disk, and error messages do not echo any
    /// of its characters.
    #[arg(
        help_heading = "Encryption",
        long,
        value_name = "HEX_KEY",
        env = "BACKUP_ENCRYPT_KEY",
        hide_env_values = true
    )]
    pub encrypt_key: Option<String>,

    // ── Decrypt ───────────────────────────────────────────────────────────
    /// Decrypt a file previously encrypted by `--encrypt-key`.
    ///
    /// Reads the AES-256-GCM–encrypted blob from `--decrypt-input` and writes
    /// the recovered plaintext to `--decrypt-output`.  The same key used for
    /// encryption must be supplied via `--encrypt-key` or
    /// `BACKUP_ENCRYPT_KEY`.  Does not contact the GitHub API or perform a
    /// backup.
    ///
    /// Example:
    /// ```text
    /// github-backup --decrypt \
    ///   --encrypt-key "$BACKUP_ENCRYPT_KEY" \
    ///   --decrypt-input issues.json.enc \
    ///   --decrypt-output issues.json
    /// ```
    #[arg(
        help_heading = "Encryption",
        verbatim_doc_comment,
        long,
        requires = "encrypt_key"
    )]
    pub decrypt: bool,

    /// Path to the AES-256-GCM encrypted file to decrypt.
    ///
    /// Required when `--decrypt` is set.
    #[arg(
        help_heading = "Encryption",
        long,
        value_name = "FILE",
        requires = "decrypt"
    )]
    pub decrypt_input: Option<PathBuf>,

    /// Path where the decrypted plaintext will be written.
    ///
    /// Required when `--decrypt` is set.
    #[arg(
        help_heading = "Encryption",
        long,
        value_name = "FILE",
        requires = "decrypt"
    )]
    pub decrypt_output: Option<PathBuf>,

    // ── Webhook notification ───────────────────────────────────────────────
    /// Send a notification to this URL after each run.
    ///
    /// The outcome (success; partial when some items could not be backed up;
    /// failure; interrupted) is sent in the `--notify-format` format.  A
    /// notification that cannot be delivered is logged as a warning and never
    /// changes the exit status.
    ///
    /// Can also be set via the `BACKUP_NOTIFY_WEBHOOK` environment variable
    /// (preferred: the URL of a Slack or Discord webhook is a secret).
    #[arg(
        help_heading = "Monitoring and logging",
        long,
        value_name = "URL",
        env = "BACKUP_NOTIFY_WEBHOOK",
        hide_env_values = true
    )]
    pub notify_webhook: Option<String>,

    /// Payload format of `--notify-webhook`.
    ///
    /// - `json` (default): a JSON object with the status, owner, counters and
    ///   the names of the failed items
    /// - `slack`: a Slack incoming-webhook message (`{"text": ...}`)
    /// - `discord`: a Discord webhook message (`{"content": ...}`)
    /// - `healthchecks`: a ping to the URL on success and to `<URL>/fail`
    ///   otherwise (healthchecks.io and compatible services)
    ///
    /// Can also be set via the `BACKUP_NOTIFY_FORMAT` environment variable.
    #[arg(
        help_heading = "Monitoring and logging",
        verbatim_doc_comment,
        long,
        value_name = "FORMAT",
        value_enum,
        env = "BACKUP_NOTIFY_FORMAT",
        default_value = "json"
    )]
    pub notify_format: NotifyFormat,

    // ── Logging ────────────────────────────────────────────────────────────
    /// Suppress all non-error output.
    #[arg(help_heading = "Monitoring and logging", long, short = 'q')]
    pub quiet: bool,

    /// Increase log verbosity (`-v` = debug, `-vv` = trace).
    #[arg(help_heading = "Monitoring and logging", long, short = 'v', action = clap::ArgAction::Count)]
    pub verbose: u8,

    // ── Run history ───────────────────────────────────────────────────────
    /// Maximum number of backup run entries to retain in `backup_history.json`.
    ///
    /// Each successful run appends an entry to
    /// `<output>/<owner>/json/backup_history.json`.  When the file grows beyond
    /// this limit, the oldest entries are dropped.  Defaults to 20.
    #[arg(
        help_heading = "Monitoring and logging",
        long,
        value_name = "N",
        default_value = "20"
    )]
    pub history_size: usize,

    // ── TUI ────────────────────────────────────────────────────────────────
    /// Launch the interactive terminal user interface (TUI).
    ///
    /// Opens a full-screen interactive interface for configuring and running
    /// backups.  The command line and the config file pre-fill the form.  The
    /// TUI covers the repository and metadata categories only: mirroring, S3,
    /// reports, metrics, the webhook, `--every` and device-flow login are
    /// command-line only, and giving them together with `--tui` is an error.
    ///
    /// When invoked with only `--tui` (no other flags), the TUI starts with
    /// a blank configuration form ready for interactive input.
    #[arg(help_heading = "Interface", long)]
    pub tui: bool,
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
