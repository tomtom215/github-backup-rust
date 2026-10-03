// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository metadata returned by the GitHub Repositories API.

use serde::{Deserialize, Serialize};

use crate::user::User;

/// The typed view of a repository object from the list endpoints
/// (`GET /users/{u}/repos`, `/user/repos`, `/orgs/{o}/repos`, starred,
/// subscriptions).
///
/// Only the fields the backup logic reads are modelled; GitHub returns about
/// 90 more (topics, license, counters, URL templates, ...).  They are not
/// lost: the backup writes the original JSON, see [`crate::Raw`].
///
/// Fields the OpenAPI description does not list as required default instead
/// of failing the parse, so one unusual repository never aborts a listing.
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
    #[serde(default)]
    pub archived: bool,
    /// Whether the repository is disabled.
    #[serde(default)]
    pub disabled: bool,
    /// Short description, or `None` if unset.
    pub description: Option<String>,
    /// HTTPS clone URL.
    pub clone_url: String,
    /// SSH clone URL.
    pub ssh_url: String,
    /// Default branch name (e.g. `"main"`), or `None` if GitHub omitted it.
    #[serde(default)]
    pub default_branch: Option<String>,
    /// Repository size in kilobytes as reported by GitHub (0 when omitted).
    #[serde(default)]
    pub size: u64,
    /// Whether this repository has issues enabled.
    #[serde(default)]
    pub has_issues: bool,
    /// Whether this repository has a wiki enabled.
    #[serde(default)]
    pub has_wiki: bool,
    /// ISO 8601 timestamp of repository creation (`null` on some objects).
    pub created_at: Option<String>,
    /// ISO 8601 timestamp of last push (`null` for an empty repository).
    pub pushed_at: Option<String>,
    /// ISO 8601 timestamp of last metadata update (`null` on some objects).
    pub updated_at: Option<String>,
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
        assert_eq!(repo.default_branch.as_deref(), Some("main"));
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

    #[test]
    fn repository_accepts_the_nullable_timestamps_of_the_spec() {
        // `created_at`, `updated_at` and `pushed_at` are nullable in the
        // `minimal-repository` schema (an empty repository has no push yet).
        let mut value: serde_json::Value = serde_json::from_str(sample_json()).expect("json");
        value["created_at"] = serde_json::Value::Null;
        value["updated_at"] = serde_json::Value::Null;
        value["pushed_at"] = serde_json::Value::Null;

        let repo: Repository = serde_json::from_value(value).expect("deserialise");

        assert!(repo.created_at.is_none());
        assert!(repo.updated_at.is_none());
        assert!(repo.pushed_at.is_none());
    }

    #[test]
    fn repository_defaults_the_fields_the_spec_does_not_require() {
        // `minimal-repository` does not require any of these.
        let mut value: serde_json::Value = serde_json::from_str(sample_json()).expect("json");
        let object = value.as_object_mut().expect("object");
        for key in [
            "archived",
            "disabled",
            "default_branch",
            "size",
            "has_issues",
            "has_wiki",
            "created_at",
            "updated_at",
            "pushed_at",
        ] {
            object.remove(key);
        }

        let repo: Repository = serde_json::from_value(value).expect("deserialise");

        assert!(!repo.archived && !repo.disabled && !repo.has_issues && !repo.has_wiki);
        assert_eq!(repo.size, 0);
        assert!(repo.default_branch.is_none());
        assert!(repo.created_at.is_none() && repo.updated_at.is_none());
    }
}
