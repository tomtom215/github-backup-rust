// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Minimal user / actor type returned in many GitHub API responses.

use serde::{Deserialize, Serialize};

/// A GitHub user or bot account as returned embedded in other API objects.
///
/// This is the *partial* user representation that appears inside issues, pull
/// requests, commits, etc. It is **not** the full user profile endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Numeric user identifier (stable across renames).
    pub id: u64,
    /// Login handle (may change).
    pub login: String,
    /// Account type: `"User"`, `"Organization"`, or `"Bot"`.
    #[serde(rename = "type")]
    pub user_type: String,
    /// URL of the user's GitHub profile avatar image.
    pub avatar_url: String,
    /// URL of the user's GitHub profile page.
    pub html_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_deserialise_minimal_returns_correct_fields() {
        let json = r#"{
            "id": 1,
            "login": "octocat",
            "type": "User",
            "avatar_url": "https://github.com/images/error/octocat_happy.gif",
            "html_url": "https://github.com/octocat"
        }"#;

        let user: User = serde_json::from_str(json).expect("deserialise user");
        assert_eq!(user.id, 1);
        assert_eq!(user.login, "octocat");
        assert_eq!(user.user_type, "User");
    }

    #[test]
    fn user_roundtrip_preserves_all_fields() {
        let user = User {
            id: 42,
            login: "testuser".to_string(),
            user_type: "User".to_string(),
            avatar_url: "https://example.com/avatar.png".to_string(),
            html_url: "https://github.com/testuser".to_string(),
        };
        let json = serde_json::to_string(&user).expect("serialise");
        let decoded: User = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(user, decoded);
    }
}
