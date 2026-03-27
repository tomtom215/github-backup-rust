// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository milestone type.

use serde::{Deserialize, Serialize};

use crate::user::User;

/// A repository milestone used to group issues and pull requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    /// Numeric milestone identifier.
    pub id: u64,
    /// Repository-scoped milestone number.
    pub number: u64,
    /// Milestone title.
    pub title: String,
    /// Optional long-form description.
    pub description: Option<String>,
    /// State: `"open"` or `"closed"`.
    pub state: String,
    /// User who created this milestone.
    pub creator: Option<User>,
    /// Count of open issues/PRs associated with this milestone.
    pub open_issues: u64,
    /// Count of closed issues/PRs associated with this milestone.
    pub closed_issues: u64,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// ISO 8601 due date, or `None` if not set.
    pub due_on: Option<String>,
    /// ISO 8601 closed timestamp, or `None` if still open.
    pub closed_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestone_deserialise_minimal_succeeds() {
        let json = r#"{
            "id": 1002604,
            "number": 1,
            "title": "v1.0",
            "description": "Tracking milestone for v1.0",
            "state": "open",
            "creator": null,
            "open_issues": 4,
            "closed_issues": 8,
            "created_at": "2011-04-10T20:09:31Z",
            "updated_at": "2014-03-03T18:58:10Z",
            "due_on": "2012-10-09T23:39:01Z",
            "closed_at": null
        }"#;

        let ms: Milestone = serde_json::from_str(json).expect("deserialise");
        assert_eq!(ms.title, "v1.0");
        assert_eq!(ms.state, "open");
        assert_eq!(ms.open_issues, 4);
    }
}
