// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub organisation team type.

use serde::{Deserialize, Serialize};

/// A team within a GitHub organisation.
///
/// Teams organise members into groups with shared repository access.  Backing
/// up team metadata documents the access structure at the time of backup.
///
/// Source: `GET /orgs/{org}/teams`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    /// Numeric team identifier.
    pub id: u64,
    /// GraphQL node identifier.
    pub node_id: String,
    /// API URL for this team resource.
    pub url: String,
    /// GitHub web page for this team.
    pub html_url: String,
    /// Human-readable team name.
    pub name: String,
    /// URL-safe slug used in API paths.
    pub slug: String,
    /// Optional description of the team's purpose.
    #[serde(default)]
    pub description: Option<String>,
    /// Privacy level: `"secret"` (visible only to members) or `"closed"` (visible to org members).
    pub privacy: String,
    /// Notification setting for team activity.
    #[serde(default)]
    pub notification_setting: Option<String>,
    /// Base permission level on team repositories: `"pull"`, `"triage"`, `"push"`, `"maintain"`, `"admin"`.
    pub permission: String,
    /// URL template for team member resources.
    pub members_url: String,
    /// URL for repositories accessible to this team.
    pub repositories_url: String,
    /// Parent team, if this is a nested (child) team.
    #[serde(default)]
    pub parent: Option<TeamParent>,
}

/// Summary of a parent team (to avoid deep nesting of the full `Team` type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamParent {
    /// Numeric identifier of the parent team.
    pub id: u64,
    /// Name of the parent team.
    pub name: String,
    /// Slug of the parent team.
    pub slug: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_roundtrip() {
        let team = Team {
            id: 1,
            node_id: "MDQ6VGVhbTE=".to_string(),
            url: "https://api.github.com/teams/1".to_string(),
            html_url: "https://github.com/orgs/my-org/teams/justice-league".to_string(),
            name: "Justice League".to_string(),
            slug: "justice-league".to_string(),
            description: Some("A great team.".to_string()),
            privacy: "closed".to_string(),
            notification_setting: Some("notifications_enabled".to_string()),
            permission: "push".to_string(),
            members_url: "https://api.github.com/teams/1/members{/member}".to_string(),
            repositories_url: "https://api.github.com/teams/1/repos".to_string(),
            parent: None,
        };
        let json = serde_json::to_string(&team).expect("serialise");
        let decoded: Team = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(team.id, decoded.id);
        assert_eq!(team.name, decoded.name);
    }

    #[test]
    fn team_deserialise_with_parent() {
        let json = r#"{
            "id": 2,
            "node_id": "MDQ6VGVhbTI=",
            "url": "https://api.github.com/teams/2",
            "html_url": "https://github.com/orgs/my-org/teams/sub-team",
            "name": "Sub Team",
            "slug": "sub-team",
            "privacy": "closed",
            "permission": "pull",
            "members_url": "https://api.github.com/teams/2/members{/member}",
            "repositories_url": "https://api.github.com/teams/2/repos",
            "parent": {
                "id": 1,
                "name": "Justice League",
                "slug": "justice-league"
            }
        }"#;
        let team: Team = serde_json::from_str(json).expect("deserialise");
        assert_eq!(team.id, 2);
        assert!(team.parent.is_some());
        assert_eq!(team.parent.unwrap().id, 1);
    }

    #[test]
    fn team_deserialise_without_optional_fields() {
        let json = r#"{
            "id": 3,
            "node_id": "MDQ6VGVhbTM=",
            "url": "https://api.github.com/teams/3",
            "html_url": "https://github.com/orgs/my-org/teams/alpha",
            "name": "Alpha",
            "slug": "alpha",
            "privacy": "secret",
            "permission": "admin",
            "members_url": "https://api.github.com/teams/3/members{/member}",
            "repositories_url": "https://api.github.com/teams/3/repos"
        }"#;
        let team: Team = serde_json::from_str(json).expect("deserialise");
        assert!(team.description.is_none());
        assert!(team.parent.is_none());
    }
}
