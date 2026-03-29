// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Mirror destination configuration types.
//!
//! Supports any Gitea-compatible instance (Gitea, Codeberg, Forgejo, etc.)
//! as well as GitLab and generic HTTPS-accessible Git servers.

use serde::{Deserialize, Serialize};

/// Configuration for mirroring repositories to a Gitea-compatible instance.
///
/// This covers Gitea, Codeberg (<https://codeberg.org>), Forgejo, and any
/// other service that implements the Gitea REST API v1.
///
/// # Example
///
/// Mirror to Codeberg:
/// ```no_run
/// use github_backup_mirror::config::GiteaConfig;
///
/// let cfg = GiteaConfig {
///     base_url: "https://codeberg.org".to_string(),
///     token: "your_codeberg_token".to_string(),
///     owner: "your_username".to_string(),
///     private: true,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GiteaConfig {
    /// Base URL of the Gitea instance (e.g., `https://codeberg.org`).
    ///
    /// Must not have a trailing slash.
    pub base_url: String,

    /// API token for the Gitea instance.
    ///
    /// Can also be set via the `MIRROR_TOKEN` environment variable.
    pub token: String,

    /// Username or organisation name at the mirror destination.
    ///
    /// Repositories will be created/updated under this owner.
    pub owner: String,

    /// Whether to create mirrored repositories as private.
    ///
    /// When `false`, repositories are created as public (matching the
    /// visibility of the source repository may require additional logic).
    pub private: bool,
}

impl GiteaConfig {
    /// Returns the Gitea API base URL (e.g., `https://codeberg.org/api/v1`).
    #[must_use]
    pub fn api_base(&self) -> String {
        format!("{}/api/v1", self.base_url.trim_end_matches('/'))
    }

    /// Returns the HTTPS clone URL for a repository at this Gitea instance.
    #[must_use]
    pub fn repo_clone_url(&self, repo_name: &str) -> String {
        format!(
            "{}/{}/{}.git",
            self.base_url.trim_end_matches('/'),
            self.owner,
            repo_name
        )
    }
}

/// Configuration for mirroring repositories to a GitLab instance.
///
/// Works with GitLab.com (`https://gitlab.com`) and self-hosted GitLab CE/EE
/// instances.  Uses the GitLab REST API v4 to create repositories and
/// `git push --mirror` to push the bare clone contents.
///
/// # Example
///
/// Mirror to GitLab.com:
/// ```no_run
/// use github_backup_mirror::config::GitLabConfig;
///
/// let cfg = GitLabConfig {
///     base_url: "https://gitlab.com".to_string(),
///     token: "glpat-xxxxxxxxxxxxxxxxxxxx".to_string(),
///     namespace: "alice".to_string(),
///     private: true,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabConfig {
    /// Base URL of the GitLab instance (e.g., `https://gitlab.com`).
    ///
    /// Must not have a trailing slash.
    pub base_url: String,

    /// GitLab personal access token (requires `api` scope).
    ///
    /// Can also be set via the `MIRROR_TOKEN` environment variable.
    pub token: String,

    /// GitLab namespace (username or group path) under which repositories are
    /// created.
    pub namespace: String,

    /// Whether to create mirrored repositories as private.
    pub private: bool,
}

impl GitLabConfig {
    /// Returns the GitLab API v4 base URL.
    #[must_use]
    pub fn api_base(&self) -> String {
        format!("{}/api/v4", self.base_url.trim_end_matches('/'))
    }

    /// Returns the HTTPS clone URL for a repository at this GitLab instance.
    #[must_use]
    pub fn repo_clone_url(&self, repo_name: &str) -> String {
        format!(
            "{}/{}/{}.git",
            self.base_url.trim_end_matches('/'),
            self.namespace,
            repo_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> GiteaConfig {
        GiteaConfig {
            base_url: "https://codeberg.org".to_string(),
            token: "secret".to_string(),
            owner: "alice".to_string(),
            private: true,
        }
    }

    #[test]
    fn api_base_appends_api_v1() {
        let cfg = sample_config();
        assert_eq!(cfg.api_base(), "https://codeberg.org/api/v1");
    }

    #[test]
    fn api_base_strips_trailing_slash() {
        let mut cfg = sample_config();
        cfg.base_url = "https://codeberg.org/".to_string();
        assert_eq!(cfg.api_base(), "https://codeberg.org/api/v1");
    }

    #[test]
    fn repo_clone_url_formats_correctly() {
        let cfg = sample_config();
        assert_eq!(
            cfg.repo_clone_url("my-repo"),
            "https://codeberg.org/alice/my-repo.git"
        );
    }
}
