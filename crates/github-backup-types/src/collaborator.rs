// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository collaborator type.

use serde::{Deserialize, Serialize};

/// Fine-grained permission flags granted to a repository collaborator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollaboratorPermissions {
    /// Can read (pull) the repository.
    pub pull: bool,
    /// Can triage issues and pull requests.
    pub triage: bool,
    /// Can write (push) to the repository.
    pub push: bool,
    /// Can manage the repository (but not settings/access).
    pub maintain: bool,
    /// Full administrative access.
    pub admin: bool,
}

/// A collaborator on a repository together with their permission level.
///
/// The GitHub API returns user fields at the top level alongside `permissions`
/// and `role_name`.  Requires admin access to the repository.
///
/// Source: `GET /repos/{owner}/{repo}/collaborators`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collaborator {
    /// Numeric user identifier.
    pub id: u64,
    /// GitHub login handle.
    pub login: String,
    /// Account type: `"User"`, `"Organization"`, or `"Bot"`.
    #[serde(rename = "type")]
    pub user_type: String,
    /// URL of the user's avatar image.
    pub avatar_url: String,
    /// URL of the user's GitHub profile page.
    pub html_url: String,
    /// Role name assigned to this collaborator (e.g. `"write"`, `"admin"`).
    #[serde(default)]
    pub role_name: Option<String>,
    /// Fine-grained permission flags for this collaborator.
    #[serde(default)]
    pub permissions: Option<CollaboratorPermissions>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collaborator_roundtrip() {
        let c = Collaborator {
            id: 1,
            login: "octocat".to_string(),
            user_type: "User".to_string(),
            avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
            html_url: "https://github.com/octocat".to_string(),
            role_name: Some("write".to_string()),
            permissions: Some(CollaboratorPermissions {
                pull: true,
                triage: true,
                push: true,
                maintain: false,
                admin: false,
            }),
        };
        let json = serde_json::to_string(&c).expect("serialise");
        let decoded: Collaborator = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(c.id, decoded.id);
        assert_eq!(c.login, decoded.login);
    }

    #[test]
    fn collaborator_deserialise_without_permissions() {
        let json = r#"{
            "id": 5,
            "login": "contributor",
            "type": "User",
            "avatar_url": "https://example.com/av.png",
            "html_url": "https://github.com/contributor"
        }"#;
        let c: Collaborator = serde_json::from_str(json).expect("deserialise");
        assert_eq!(c.login, "contributor");
        assert!(c.permissions.is_none());
        assert!(c.role_name.is_none());
    }
}
