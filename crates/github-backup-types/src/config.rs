// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Backup configuration types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Authentication credential used to interact with the GitHub API.
#[derive(Debug, Clone)]
pub enum Credential {
    /// Classic or fine-grained personal access token.
    ///
    /// Used as `Authorization: Bearer <token>` on every API request.
    Token(String),
}

impl Credential {
    /// Returns the `Authorization` header value for this credential.
    #[must_use]
    pub fn authorization_header(&self) -> String {
        match self {
            Credential::Token(t) => format!("Bearer {t}"),
        }
    }
}

/// Selects how repositories are cloned during backup.
///
/// The default ([`CloneType::Mirror`]) produces a bare mirror suitable for
/// complete backups and restores.  Other modes trade completeness for clone
/// speed or working-tree access.
///
/// # Serialisation
///
/// Unit variants serialise as lowercase strings (`"mirror"`, `"bare"`,
/// `"full"`).  The shallow variant serialises as `{"shallow": <depth>}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloneType {
    /// `git clone --mirror` — fetches all refs (branches, tags, notes, …).
    ///
    /// The result is a bare repository that mirrors the remote exactly.
    /// This is the recommended choice for backup purposes because it captures
    /// the complete repository state and can be restored with `git clone`.
    #[default]
    Mirror,
    /// `git clone --bare` — bare clone without remote-tracking metadata.
    ///
    /// Similar to `Mirror` but does not set up remote-tracking refs.  Slightly
    /// smaller than a mirror.
    Bare,
    /// Standard `git clone` — creates a full working-tree clone.
    ///
    /// Use this if you need to browse or build the backed-up source code
    /// directly.  Requires more disk space than bare clones.
    Full,
    /// `git clone --depth <n>` — shallow clone with limited commit history.
    ///
    /// Significantly reduces disk usage at the cost of losing history beyond
    /// `depth` commits per branch.  Not suitable for archival backups.
    Shallow(u32),
}

/// Root output path and per-owner subdirectory layout.
#[derive(Debug, Clone)]
pub struct OutputConfig {
    /// Root backup directory supplied by the user.
    pub root: PathBuf,
}

impl OutputConfig {
    /// Creates an [`OutputConfig`] rooted at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { root: path.into() }
    }

    /// Returns the directory for bare git clones: `<root>/<owner>/git/repos/`.
    #[must_use]
    pub fn repos_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("git").join("repos")
    }

    /// Returns the directory for wiki git clones: `<root>/<owner>/git/wikis/`.
    #[must_use]
    pub fn wikis_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("git").join("wikis")
    }

    /// Returns the directory for gist git clones: `<root>/<owner>/git/gists/`.
    #[must_use]
    pub fn gists_git_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("git").join("gists")
    }

    /// Returns the JSON metadata directory for a repository:
    /// `<root>/<owner>/json/repos/<repo>/`.
    #[must_use]
    pub fn repo_meta_dir(&self, owner: &str, repo: &str) -> PathBuf {
        self.root.join(owner).join("json").join("repos").join(repo)
    }

    /// Returns the JSON metadata directory for gists:
    /// `<root>/<owner>/json/gists/`.
    #[must_use]
    pub fn gists_meta_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("json").join("gists")
    }

    /// Returns the top-level JSON directory for an owner:
    /// `<root>/<owner>/json/`.
    #[must_use]
    pub fn owner_json_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("json")
    }

    /// Returns the path for a top-level owner JSON file (followers, starred…):
    /// `<root>/<owner>/json/<filename>`.
    #[must_use]
    pub fn owner_json(&self, owner: &str, filename: &str) -> PathBuf {
        self.root.join(owner).join("json").join(filename)
    }
}

/// Whether the backup target is a user account or an organisation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackupTarget {
    /// GitHub user account (default).
    #[default]
    User,
    /// GitHub organisation.
    Org,
}

/// Selects which data categories to include in the backup.
///
/// All fields default to `false`; enable only what you need.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupOptions {
    // ── Target ────────────────────────────────────────────────────────────
    /// Whether the target is a user account or an organisation.
    pub target: BackupTarget,

    // ── Repository git clones ──────────────────────────────────────────────
    /// Clone/mirror all repositories (default: true when not overridden).
    pub repositories: bool,
    /// Include forked repositories.
    pub forks: bool,
    /// Include private repositories (requires appropriate token scope).
    pub private: bool,
    /// Clone using SSH URLs instead of HTTPS.
    pub prefer_ssh: bool,
    /// How to clone repositories.
    ///
    /// Defaults to [`CloneType::Mirror`] for complete, restorable backups.
    #[serde(default)]
    pub clone_type: CloneType,
    /// Enable Git LFS when cloning.
    pub lfs: bool,
    /// Do not prune deleted remote refs during updates.
    pub no_prune: bool,

    // ── Issues ────────────────────────────────────────────────────────────
    /// Backup issue metadata (title, body, state, labels, etc.).
    pub issues: bool,
    /// Backup issue comment threads.
    pub issue_comments: bool,
    /// Backup issue timeline events.
    pub issue_events: bool,

    // ── Pull Requests ─────────────────────────────────────────────────────
    /// Backup pull request metadata.
    pub pulls: bool,
    /// Backup pull request review comments.
    pub pull_comments: bool,
    /// Backup pull request commit lists.
    pub pull_commits: bool,
    /// Backup pull request reviews (requires an extra API call per PR).
    pub pull_reviews: bool,

    // ── Repository metadata ───────────────────────────────────────────────
    /// Backup repository labels.
    pub labels: bool,
    /// Backup repository milestones.
    pub milestones: bool,
    /// Backup repository releases (metadata + asset download).
    pub releases: bool,
    /// Backup release binary assets.
    pub release_assets: bool,
    /// Backup repository webhook configurations.
    pub hooks: bool,
    /// Backup security advisories (public repositories only).
    pub security_advisories: bool,
    /// Clone repository wikis.
    pub wikis: bool,

    // ── User / organisation data ──────────────────────────────────────────
    /// Backup the list of repositories starred by the target user.
    pub starred: bool,
    /// Backup the list of repositories watched by the target user.
    pub watched: bool,
    /// Backup the target user's follower list.
    pub followers: bool,
    /// Backup the target user's following list.
    pub following: bool,
    /// Backup gists owned by the target user.
    pub gists: bool,
    /// Backup gists starred by the target user.
    pub starred_gists: bool,

    // ── Execution options ─────────────────────────────────────────────────
    /// When `true`, log what would be done without writing any files or
    /// running any git commands.
    pub dry_run: bool,
    /// Maximum number of repositories to back up concurrently.
    /// `1` means fully sequential (safe default).
    pub concurrency: usize,
}

impl BackupOptions {
    /// Returns a configuration that enables every available backup category.
    ///
    /// Equivalent to the `--all` flag in the Python reference implementation,
    /// but also enables `starred_gists` and `pull_reviews`.
    #[must_use]
    pub fn all() -> Self {
        Self {
            target: BackupTarget::User,
            repositories: true,
            forks: true,
            private: true,
            prefer_ssh: false,
            clone_type: CloneType::Mirror,
            lfs: false,
            no_prune: false,
            issues: true,
            issue_comments: true,
            issue_events: true,
            pulls: true,
            pull_comments: true,
            pull_commits: true,
            pull_reviews: true,
            labels: true,
            milestones: true,
            releases: true,
            release_assets: true,
            hooks: true,
            security_advisories: true,
            wikis: true,
            starred: true,
            watched: true,
            followers: true,
            following: true,
            gists: true,
            starred_gists: true,
            dry_run: false,
            concurrency: 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_token_authorization_header_has_bearer_prefix() {
        let cred = Credential::Token("ghp_test123".to_string());
        assert_eq!(cred.authorization_header(), "Bearer ghp_test123");
    }

    #[test]
    fn output_config_repos_dir_ends_with_expected_suffix() {
        let cfg = OutputConfig::new("/backup");
        let path = cfg.repos_dir("octocat");
        assert_eq!(path, PathBuf::from("/backup/octocat/git/repos"));
    }

    #[test]
    fn output_config_repo_meta_dir_ends_with_expected_suffix() {
        let cfg = OutputConfig::new("/backup");
        let path = cfg.repo_meta_dir("octocat", "Hello-World");
        assert_eq!(
            path,
            PathBuf::from("/backup/octocat/json/repos/Hello-World")
        );
    }

    #[test]
    fn backup_options_all_enables_repositories() {
        let opts = BackupOptions::all();
        assert!(opts.repositories);
        assert!(opts.issues);
        assert!(opts.pulls);
        assert!(opts.releases);
    }

    #[test]
    fn backup_options_default_disables_all_categories() {
        let opts = BackupOptions::default();
        assert!(!opts.repositories);
        assert!(!opts.issues);
        assert!(!opts.pulls);
    }

    #[test]
    fn backup_options_roundtrip_json() {
        let opts = BackupOptions::all();
        let json = serde_json::to_string(&opts).expect("serialise");
        let decoded: BackupOptions = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(decoded.repositories, opts.repositories);
        assert_eq!(decoded.issues, opts.issues);
    }

    #[test]
    fn clone_type_default_is_mirror() {
        assert_eq!(CloneType::default(), CloneType::Mirror);
    }

    #[test]
    fn clone_type_serialises_as_lowercase_string() {
        let json = serde_json::to_string(&CloneType::Mirror).unwrap();
        assert_eq!(json, r#""mirror""#);
        let json = serde_json::to_string(&CloneType::Bare).unwrap();
        assert_eq!(json, r#""bare""#);
        let json = serde_json::to_string(&CloneType::Full).unwrap();
        assert_eq!(json, r#""full""#);
    }

    #[test]
    fn clone_type_shallow_serialises_with_depth() {
        let json = serde_json::to_string(&CloneType::Shallow(10)).unwrap();
        assert_eq!(json, r#"{"shallow":10}"#);
    }

    #[test]
    fn clone_type_roundtrips_json() {
        for ct in [
            CloneType::Mirror,
            CloneType::Bare,
            CloneType::Full,
            CloneType::Shallow(5),
        ] {
            let json = serde_json::to_string(&ct).unwrap();
            let decoded: CloneType = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, ct);
        }
    }

    #[test]
    fn backup_options_clone_type_defaults_to_mirror() {
        let opts = BackupOptions::default();
        assert_eq!(opts.clone_type, CloneType::Mirror);
    }
}
