// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository metadata returned by the GitHub Repositories API.

use serde::{Deserialize, Serialize};

use crate::user::User;

/// Full repository metadata as returned by
/// `GET /repos/{owner}/{repo}` and the list variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    /// Numeric repository identifier (stable across renames and transfers).
    pub id: u64,
    /// `owner/repo` slug.
    pub full_name: String,
    /// Short repository name (without owner prefix).
    pub name: String,
    /// Repository owner.
    pub owner: User,
    /// Whether the repository is private.
    pub private: bool,
    /// Whether the repository is a fork of another repository.
    pub fork: bool,
    /// Whether the repository is archived (read-only).
    pub archived: bool,
    /// Whether the repository is disabled.
    pub disabled: bool,
    /// Short description, or `None` if unset.
    pub description: Option<String>,
    /// HTTPS clone URL.
    pub clone_url: String,
    /// SSH clone URL.
    pub ssh_url: String,
    /// Default branch name (e.g. `"main"`).
    pub default_branch: String,
    /// Repository size in kilobytes as reported by GitHub.
    pub size: u64,
    /// Whether this repository has issues enabled.
    pub has_issues: bool,
    /// Whether this repository has a wiki enabled.
    pub has_wiki: bool,
    /// ISO 8601 timestamp of repository creation.
    pub created_at: String,
    /// ISO 8601 timestamp of last push.
    pub pushed_at: Option<String>,
    /// ISO 8601 timestamp of last metadata update.
    pub updated_at: String,
    /// HTTPS URL of the repository's GitHub page.
    pub html_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> &'static str {
        r#"{
            "id": 1296269,
            "full_name": "octocat/Hello-World",
            "name": "Hello-World",
            "owner": {
                "id": 1,
                "login": "octocat",
                "type": "User",
                "avatar_url": "https://github.com/images/error/octocat_happy.gif",
                "html_url": "https://github.com/octocat"
            },
            "private": false,
            "fork": false,
            "archived": false,
            "disabled": false,
            "description": "This your first repo!",
            "clone_url": "https://github.com/octocat/Hello-World.git",
            "ssh_url": "git@github.com:octocat/Hello-World.git",
            "default_branch": "main",
            "size": 108,
            "has_issues": true,
            "has_wiki": true,
            "created_at": "2011-01-26T19:01:12Z",
            "pushed_at": "2011-01-26T19:06:43Z",
            "updated_at": "2011-01-26T19:14:43Z",
            "html_url": "https://github.com/octocat/Hello-World"
        }"#
    }

    #[test]
    fn repository_deserialise_returns_correct_fields() {
        let repo: Repository = serde_json::from_str(sample_json()).expect("deserialise");
        assert_eq!(repo.id, 1_296_269);
        assert_eq!(repo.full_name, "octocat/Hello-World");
        assert!(!repo.private);
        assert!(!repo.fork);
        assert_eq!(repo.default_branch, "main");
    }

    #[test]
    fn repository_roundtrip_preserves_all_fields() {
        let repo: Repository = serde_json::from_str(sample_json()).expect("deserialise");
        let json = serde_json::to_string(&repo).expect("serialise");
        let decoded: Repository = serde_json::from_str(&json).expect("re-deserialise");
        assert_eq!(repo, decoded);
    }

    #[test]
    fn repository_description_none_when_null() {
        let mut json = sample_json().to_string();
        json = json.replace(
            r#""description": "This your first repo!""#,
            r#""description": null"#,
        );
        let repo: Repository = serde_json::from_str(&json).expect("deserialise");
        assert!(repo.description.is_none());
    }
}
