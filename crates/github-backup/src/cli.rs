// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Command-line argument definitions using [`clap`].

use std::path::PathBuf;
use std::str::FromStr;

use clap::{ArgGroup, Parser};

/// Comprehensive GitHub backup tool.
///
/// Backs up repositories, issues, pull requests, releases, gists, wikis, and
/// relationship data for a GitHub user or organisation.
///
/// # Authentication
///
/// Provide a personal access token (classic or fine-grained) via `--token` or
/// the `GITHUB_TOKEN` environment variable, **or** use `--device-auth` to
/// authenticate interactively via the GitHub OAuth device flow (requires
/// a registered OAuth App — see `--oauth-client-id`).
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
        .args(["token", "device_auth"]),
))]
pub struct Args {
    /// GitHub username or organisation name to back up.
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
    pub token: Option<String>,

    /// Authenticate interactively using the GitHub OAuth device flow.
    ///
    /// Opens a browser code at `github.com/login/device`.  Requires
    /// `--oauth-client-id`.
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
    #[arg(short = 'o', long = "output", value_name = "DIR", default_value = ".")]
    pub output: PathBuf,

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

impl Args {
    /// Converts the parsed CLI arguments into a [`BackupOptions`] struct.
    #[must_use]
    pub fn into_backup_options(self) -> github_backup_types::config::BackupOptions {
        use github_backup_types::config::{BackupOptions, BackupTarget};

        let target = if self.org {
            BackupTarget::Org
        } else {
            BackupTarget::User
        };

        let clone_type = self.clone_type.into_clone_type();

        if self.all {
            return BackupOptions {
                target,
                prefer_ssh: self.prefer_ssh,
                clone_type,
                lfs: self.lfs,
                no_prune: self.no_prune,
                dry_run: self.dry_run,
                concurrency: self.concurrency,
                ..BackupOptions::all()
            };
        }

        BackupOptions {
            target,
            repositories: self.repositories,
            forks: self.forks,
            private: self.private,
            prefer_ssh: self.prefer_ssh,
            clone_type,
            lfs: self.lfs,
            no_prune: self.no_prune,
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
            dry_run: self.dry_run,
            concurrency: self.concurrency,
        }
    }
}

// ── Clone type argument ───────────────────────────────────────────────────────

/// CLI representation of `--clone-type`.
///
/// Parses the human-friendly strings accepted by the `--clone-type` flag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CliCloneType {
    /// `git clone --mirror`
    #[default]
    Mirror,
    /// `git clone --bare`
    Bare,
    /// `git clone` (full working tree)
    Full,
    /// `git clone --depth <n>`
    Shallow(u32),
}

impl CliCloneType {
    /// Converts to the corresponding [`github_backup_types::config::CloneType`].
    #[must_use]
    pub fn into_clone_type(self) -> github_backup_types::config::CloneType {
        use github_backup_types::config::CloneType;
        match self {
            Self::Mirror => CloneType::Mirror,
            Self::Bare => CloneType::Bare,
            Self::Full => CloneType::Full,
            Self::Shallow(d) => CloneType::Shallow(d),
        }
    }
}

impl FromStr for CliCloneType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mirror" => Ok(Self::Mirror),
            "bare" => Ok(Self::Bare),
            "full" => Ok(Self::Full),
            s if s.starts_with("shallow:") => {
                let depth_str = &s["shallow:".len()..];
                let depth: u32 = depth_str.parse().map_err(|_| {
                    format!("invalid depth '{depth_str}' in '{s}'; expected e.g. 'shallow:10'")
                })?;
                if depth == 0 {
                    return Err("shallow depth must be at least 1".to_string());
                }
                Ok(Self::Shallow(depth))
            }
            _ => Err(format!(
                "unknown clone type '{s}'; valid values: mirror, bare, full, shallow:<depth>"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use github_backup_types::config::CloneType;

    #[test]
    fn args_parse_minimal_required_fields() {
        let args = Args::parse_from(["github-backup", "octocat", "--token", "ghp_test"]);
        assert_eq!(args.owner, "octocat");
        assert_eq!(args.token.as_deref(), Some("ghp_test"));
        assert!(!args.all);
        assert!(!args.org);
    }

    #[test]
    fn args_parse_all_flag() {
        let args = Args::parse_from(["github-backup", "octocat", "--token", "t", "--all"]);
        assert!(args.all);
    }

    #[test]
    fn args_parse_org_flag() {
        let args = Args::parse_from(["github-backup", "myorg", "--token", "t", "--org", "--all"]);
        assert!(args.org);
        let opts = args.into_backup_options();
        assert_eq!(opts.target, github_backup_types::config::BackupTarget::Org);
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
    fn args_release_assets_requires_releases() {
        let result = Args::try_parse_from([
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--release-assets",
        ]);
        assert!(
            result.is_err(),
            "--release-assets without --releases should fail"
        );
    }

    #[test]
    fn args_parse_quiet_and_verbose() {
        let args = Args::parse_from(["github-backup", "octocat", "--token", "t", "-q"]);
        assert!(args.quiet);

        let args = Args::parse_from(["github-backup", "octocat", "--token", "t", "-vv"]);
        assert_eq!(args.verbose, 2);
    }

    #[test]
    fn args_parse_concurrency_and_dry_run() {
        let args = Args::parse_from([
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--concurrency",
            "8",
            "--dry-run",
        ]);
        assert_eq!(args.concurrency, 8);
        assert!(args.dry_run);
        let opts = args.into_backup_options();
        assert_eq!(opts.concurrency, 8);
        assert!(opts.dry_run);
    }

    #[test]
    fn args_parse_no_prune() {
        let args = Args::parse_from([
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--repositories",
            "--no-prune",
        ]);
        assert!(args.no_prune);
        let opts = args.into_backup_options();
        assert!(opts.no_prune);
    }

    #[test]
    fn cli_clone_type_parse_mirror() {
        assert_eq!(
            "mirror".parse::<CliCloneType>().unwrap(),
            CliCloneType::Mirror
        );
    }

    #[test]
    fn cli_clone_type_parse_bare() {
        assert_eq!("bare".parse::<CliCloneType>().unwrap(), CliCloneType::Bare);
    }

    #[test]
    fn cli_clone_type_parse_full() {
        assert_eq!("full".parse::<CliCloneType>().unwrap(), CliCloneType::Full);
    }

    #[test]
    fn cli_clone_type_parse_shallow() {
        assert_eq!(
            "shallow:10".parse::<CliCloneType>().unwrap(),
            CliCloneType::Shallow(10)
        );
    }

    #[test]
    fn cli_clone_type_parse_shallow_zero_is_error() {
        assert!("shallow:0".parse::<CliCloneType>().is_err());
    }

    #[test]
    fn cli_clone_type_parse_invalid_is_error() {
        assert!("invalid".parse::<CliCloneType>().is_err());
        assert!("shallow:abc".parse::<CliCloneType>().is_err());
    }

    #[test]
    fn cli_clone_type_into_clone_type_mirror() {
        assert_eq!(CliCloneType::Mirror.into_clone_type(), CloneType::Mirror);
    }

    #[test]
    fn cli_clone_type_into_clone_type_shallow() {
        assert_eq!(
            CliCloneType::Shallow(5).into_clone_type(),
            CloneType::Shallow(5)
        );
    }

    #[test]
    fn args_parse_clone_type_full() {
        let args = Args::parse_from([
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--repositories",
            "--clone-type",
            "full",
        ]);
        assert_eq!(args.clone_type, CliCloneType::Full);
        let opts = args.into_backup_options();
        assert_eq!(opts.clone_type, CloneType::Full);
    }

    #[test]
    fn args_parse_s3_flags() {
        let args = Args::parse_from([
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--repositories",
            "--s3-bucket",
            "my-bucket",
            "--s3-region",
            "eu-west-1",
            "--s3-access-key",
            "AKID",
            "--s3-secret-key",
            "SECRET",
        ]);
        assert_eq!(args.s3_bucket.as_deref(), Some("my-bucket"));
        assert_eq!(args.s3_region, "eu-west-1");
    }

    #[test]
    fn args_parse_mirror_flags() {
        let args = Args::parse_from([
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--repositories",
            "--mirror-to",
            "https://codeberg.org",
            "--mirror-token",
            "cb_token",
            "--mirror-owner",
            "alice",
        ]);
        assert_eq!(args.mirror_to.as_deref(), Some("https://codeberg.org"));
        assert_eq!(args.mirror_token.as_deref(), Some("cb_token"));
        assert_eq!(args.mirror_owner.as_deref(), Some("alice"));
    }
}
