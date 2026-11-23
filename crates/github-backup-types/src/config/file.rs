// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! TOML configuration file schema and parser.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::CloneType;

/// TOML configuration file schema.
///
/// Load from disk with [`ConfigFile::from_toml_str`] or
/// [`ConfigFile::from_path`].  All fields are optional; missing fields fall
/// back to CLI defaults.  A typical minimal config looks like:
///
/// ```toml
/// owner = "octocat"
/// output = "/var/backup/github"
/// concurrency = 8
///
/// repositories = true
/// issues = true
/// pulls = true
/// releases = true
/// wikis = true
/// ```
///
/// # Token security
///
/// Storing tokens in config files is convenient but less secure than providing
/// them via the `GITHUB_TOKEN` environment variable.  Prefer environment
/// variables in automated environments; restrict config file permissions to
/// `0600` when a token must be stored.  The `Debug` output shows `token`,
/// `mirror_token` and `s3_secret_key` as `<redacted>`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// GitHub username or organisation name to back up.
    pub owner: Option<String>,

    /// GitHub personal access token.
    ///
    /// Prefer the `GITHUB_TOKEN` environment variable over storing tokens here.
    pub token: Option<String>,

    /// GitHub App to authenticate as, instead of a token: its client ID
    /// (recommended) or numeric App ID.  Needs `app_key_path` (or the
    /// `GITHUB_APP_PRIVATE_KEY` environment variable).
    pub app_id: Option<String>,

    /// Path of the GitHub App's private key (the `.pem` file GitHub issued).
    pub app_key_path: Option<PathBuf>,

    /// The GitHub App installation to act as.  Without it the installation is
    /// looked up from `owner` (organisation first, then user).
    pub app_installation_id: Option<u64>,

    /// Override the GitHub API base URL for GitHub Enterprise Server.
    ///
    /// Example: `"https://github.example.com/api/v3"`.
    /// Defaults to `https://api.github.com` when absent.
    pub api_url: Option<String>,

    /// Root directory where backup artefacts will be written.
    pub output: Option<PathBuf>,

    /// Maximum number of repositories to back up concurrently (default: 4).
    pub concurrency: Option<usize>,

    /// Treat `owner` as a GitHub organisation.
    pub org: Option<bool>,

    // ── Category flags ─────────────────────────────────────────────────────
    /// Enable all backup categories.
    pub all: Option<bool>,

    /// Clone/mirror repositories.
    pub repositories: Option<bool>,
    /// Include forked repositories.
    pub forks: Option<bool>,
    /// Include private repositories.
    pub private: Option<bool>,
    /// Back up issue metadata.
    pub issues: Option<bool>,
    /// Back up issue comment threads.
    pub issue_comments: Option<bool>,
    /// Back up issue timeline events.
    pub issue_events: Option<bool>,
    /// Back up pull request metadata.
    pub pulls: Option<bool>,
    /// Back up pull request review comments.
    pub pull_comments: Option<bool>,
    /// Back up pull request commit lists.
    pub pull_commits: Option<bool>,
    /// Back up pull request reviews.
    pub pull_reviews: Option<bool>,
    /// Back up repository labels.
    pub labels: Option<bool>,
    /// Back up repository milestones.
    pub milestones: Option<bool>,
    /// Back up release metadata.
    pub releases: Option<bool>,
    /// Download release binary assets.
    pub release_assets: Option<bool>,
    /// Back up webhook configurations.
    pub hooks: Option<bool>,
    /// Back up published security advisories.
    pub security_advisories: Option<bool>,
    /// Clone repository wikis.
    pub wikis: Option<bool>,
    /// Back up starred repositories (JSON list).
    pub starred: Option<bool>,
    /// Clone every starred repository as a git mirror.
    pub clone_starred: Option<bool>,
    /// Back up watched repositories.
    pub watched: Option<bool>,
    /// Back up follower list.
    pub followers: Option<bool>,
    /// Back up following list.
    pub following: Option<bool>,
    /// Back up owned gists.
    pub gists: Option<bool>,
    /// Back up starred gists.
    pub starred_gists: Option<bool>,

    /// Back up repository topics.
    pub topics: Option<bool>,
    /// Back up the list of repository branches.
    pub branches: Option<bool>,
    /// Back up deploy keys for each repository.
    pub deploy_keys: Option<bool>,
    /// Back up the list of repository collaborators.
    pub collaborators: Option<bool>,
    /// Back up the member list of the organisation.
    pub org_members: Option<bool>,
    /// Back up the team list of the organisation.
    pub org_teams: Option<bool>,

    /// Back up GitHub Actions workflow metadata.
    pub actions: Option<bool>,
    /// Back up GitHub Actions workflow run history.
    pub action_runs: Option<bool>,
    /// Back up repository deployment environment configurations.
    pub environments: Option<bool>,
    /// Back up GitHub Discussions (with comments, replies and categories).
    pub discussions: Option<bool>,
    /// Back up the owner's Projects (v2), their fields and items.
    pub projects: Option<bool>,
    /// Back up the comments on commits of each repository.
    pub commit_comments: Option<bool>,
    /// Back up GitHub Packages metadata for the target user or organisation.
    pub packages: Option<bool>,

    /// Categories to leave out of the backup (names as for `--skip`, e.g.
    /// `["hooks", "packages"]`).
    pub skip: Option<Vec<String>>,

    /// Only back up repositories whose name (not `owner/name`) matches one of
    /// these glob patterns.  A TOML array of strings, e.g.
    /// `include_repos = ["rust-*", "docs"]`.
    pub include_repos: Option<Vec<String>>,
    /// Exclude repositories whose name matches one of these glob patterns (a
    /// TOML array of strings).
    pub exclude_repos: Option<Vec<String>>,

    /// Expert override, like `--since`: treat items last updated before this
    /// date (`2026-01-01`) or ISO 8601 timestamp as already backed up, for
    /// every repository, instead of the per-repository watermarks.  Issue and
    /// pull request lists are asked only for items updated since then and
    /// merged into the stored lists (in full when a stored list is missing);
    /// the comments, events, commits and reviews of items not changed since
    /// then (whose files exist) are not fetched again.  Never stored.
    pub since: Option<String>,

    /// Override the hostname used in git clone URLs.
    ///
    /// Useful for GHES deployments where the API host and git clone host differ.
    pub clone_host: Option<String>,

    // ── Clone behaviour ────────────────────────────────────────────────────
    /// Clone using SSH URLs instead of HTTPS.
    pub prefer_ssh: Option<bool>,

    /// How to clone repositories (`mirror`, `bare`, `full`, `shallow:<depth>`).
    ///
    /// Defaults to `mirror` when absent.
    pub clone_type: Option<CloneType>,

    /// Enable Git LFS when cloning.
    pub lfs: Option<bool>,

    /// Delete branches and tags from the local clone when they were deleted
    /// on the remote (default: keep them).
    pub prune: Option<bool>,

    /// Deprecated and ignored: keeping deleted refs is now the default.  Still
    /// accepted so that existing configuration files keep loading.
    pub no_prune: Option<bool>,

    /// Check every repository with `git fsck` after each clone and update.
    pub verify_git: Option<bool>,

    /// Seconds a git command may go without output before it is stopped.
    pub git_timeout: Option<u64>,

    // ── Report ────────────────────────────────────────────────────────────
    /// Write a JSON summary report to this file after the backup completes.
    pub report: Option<PathBuf>,

    // ── Mirror destination ─────────────────────────────────────────────────
    /// Push repository mirrors to this Gitea-compatible base URL after backup.
    ///
    /// Supported hosts: Gitea, Codeberg, Forgejo.
    pub mirror_to: Option<String>,

    /// API token for the mirror destination.
    ///
    /// Prefer the `MIRROR_TOKEN` environment variable over storing tokens here.
    pub mirror_token: Option<String>,

    /// Owner name at the mirror destination (username or org).
    pub mirror_owner: Option<String>,

    /// Mirror destination type (default: `gitea`).
    pub mirror_type: Option<MirrorType>,

    /// Create every repository as private at the mirror destination (this is
    /// the default; the key exists to say so explicitly).
    pub mirror_private: Option<bool>,

    /// Create mirrors of public repositories as public.  Mirrors of private
    /// repositories stay private regardless.
    pub mirror_public: Option<bool>,

    // ── S3 storage ─────────────────────────────────────────────────────────
    /// S3 bucket to sync backup metadata to.
    ///
    /// Works with AWS S3, Backblaze B2, MinIO, Cloudflare R2, Spaces, Wasabi.
    pub s3_bucket: Option<String>,

    /// AWS region for the S3 bucket (default: `us-east-1`).
    pub s3_region: Option<String>,

    /// Key prefix for all S3 objects (default: empty).
    pub s3_prefix: Option<String>,

    /// Custom S3-compatible endpoint URL (for B2, MinIO, R2, etc.).
    pub s3_endpoint: Option<String>,

    /// AWS access key ID.
    ///
    /// Prefer the `AWS_ACCESS_KEY_ID` environment variable over storing here.
    pub s3_access_key: Option<String>,

    /// AWS secret access key.
    ///
    /// Prefer the `AWS_SECRET_ACCESS_KEY` environment variable over storing here.
    pub s3_secret_key: Option<String>,

    /// Also upload binary release assets to S3 (can be very large).
    pub s3_include_assets: Option<bool>,
}

/// Kind of server a push mirror goes to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorType {
    /// Gitea, Forgejo or Codeberg (Gitea REST API v1).
    #[default]
    Gitea,
    /// GitLab.com or self-hosted GitLab (REST API v4).
    Gitlab,
}

impl std::str::FromStr for MirrorType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "gitea" => Ok(Self::Gitea),
            "gitlab" => Ok(Self::Gitlab),
            other => Err(format!(
                "unknown mirror type '{other}' (expected 'gitea' or 'gitlab')"
            )),
        }
    }
}

impl std::fmt::Debug for ConfigFile {
    /// Every key, with the values of the three secret keys replaced by
    /// `<redacted>`: a configuration logged while debugging must not leak them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut shown = self.clone();
        for secret in [
            &mut shown.token,
            &mut shown.mirror_token,
            &mut shown.s3_secret_key,
        ] {
            if secret.is_some() {
                *secret = Some("<redacted>".to_string());
            }
        }
        match serde_json::to_value(&shown) {
            Ok(value) => write!(f, "ConfigFile {value}"),
            Err(_) => f.write_str("ConfigFile { .. }"),
        }
    }
}

impl ConfigFile {
    /// Parses a [`ConfigFile`] from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns an error string if the TOML is malformed or contains
    /// type-incompatible values.
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    /// Reads and parses a [`ConfigFile`] from a file on disk.
    ///
    /// # Errors
    ///
    /// Returns an error string if the file cannot be read or contains invalid
    /// TOML.
    pub fn from_path(path: &std::path::Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read config file '{}': {e}", path.display()))?;
        Self::from_toml_str(&content)
            .map_err(|e| format!("invalid config file '{}': {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_app_keys_parse() {
        let cfg = ConfigFile::from_toml_str(
            "app_id = \"Iv23liExample\"\napp_key_path = \"/run/secrets/app.pem\"\napp_installation_id = 42\n",
        )
        .expect("parse");
        assert_eq!(cfg.app_id.as_deref(), Some("Iv23liExample"));
        assert_eq!(
            cfg.app_key_path.as_deref(),
            Some(std::path::Path::new("/run/secrets/app.pem"))
        );
        assert_eq!(cfg.app_installation_id, Some(42));
    }
}
