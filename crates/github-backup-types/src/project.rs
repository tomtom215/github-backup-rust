// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Projects (v2) API types.
//!
//! The models name only what the backup needs to walk a project (its number,
//! its field ids, its items' ids); the files hold the objects exactly as the
//! API returned them.

use serde::{Deserialize, Serialize};

/// A Projects v2 project (`GET /users/{username}/projectsV2`,
/// `GET /orgs/{org}/projectsV2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectV2 {
    /// Unique project identifier.
    pub id: u64,
    /// Project number within its owner; the routes address a project by it.
    pub number: u64,
    /// Project title (empty when omitted).
    #[serde(default)]
    pub title: String,
}

/// A field (column) of a Projects v2 project, built-in or custom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectV2Field {
    /// Unique field identifier; item listings name the fields to return by it.
    pub id: u64,
    /// Field name (empty when omitted).
    #[serde(default)]
    pub name: String,
}

/// An item (issue, pull request or draft issue) of a Projects v2 project,
/// with the values of the fields it was requested with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectV2Item {
    /// Unique item identifier.
    pub id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_parses_the_documented_shape() {
        let json = serde_json::json!({
            "id": 2, "node_id": "MDc6UHJvamVjdDEwMDI2MDM=", "number": 2,
            "title": "My Projects", "description": null, "public": true,
            "created_at": "2022-04-28T12:00:00Z", "updated_at": "2022-04-28T12:00:00Z"
        });
        let project: ProjectV2 = serde_json::from_value(json).expect("project");
        assert_eq!((project.id, project.number), (2, 2));
        assert_eq!(project.title, "My Projects");
    }

    #[test]
    fn field_and_item_need_only_their_id() {
        let field: ProjectV2Field =
            serde_json::from_value(serde_json::json!({"id": 12345, "data_type": "text"}))
                .expect("field");
        assert_eq!(field.id, 12345);
        let item: ProjectV2Item =
            serde_json::from_value(serde_json::json!({"id": 13, "content_type": "Issue"}))
                .expect("item");
        assert_eq!(item.id, 13);
    }
}
