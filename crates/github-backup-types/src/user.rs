// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Minimal user / actor type returned in many GitHub API responses.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

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

/// Deserialises an optional user for the places where GitHub sends either
/// `null` **or** an empty object (`{}`) to say "no GitHub account", such as
/// the `author` / `committer` of a commit whose e-mail address is not linked to
/// an account.  Both become `None`.
pub(crate) fn deserialize_user_or_empty<'de, D>(deserializer: D) -> Result<Option<User>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(fields)) if fields.is_empty() => Ok(None),
        Some(other) => User::deserialize(other)
            .map(Some)
            .map_err(D::Error::custom),
    }
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

    #[derive(Debug, Deserialize)]
    struct Holder {
        #[serde(default, deserialize_with = "deserialize_user_or_empty")]
        who: Option<User>,
    }

    #[test]
    fn user_or_empty_maps_null_absent_and_empty_object_to_none() {
        for text in [r#"{"who":null}"#, r#"{}"#, r#"{"who":{}}"#] {
            let holder: Holder = serde_json::from_str(text).expect(text);
            assert!(holder.who.is_none(), "{text}");
        }
    }

    #[test]
    fn user_or_empty_parses_a_real_user_and_rejects_a_malformed_one() {
        let ok: Holder = serde_json::from_str(
            r#"{"who":{"id":1,"login":"a","type":"User","avatar_url":"","html_url":""}}"#,
        )
        .expect("parses");
        assert_eq!(ok.who.map(|u| u.login), Some("a".to_string()));

        let err = serde_json::from_str::<Holder>(r#"{"who":{"login":"a"}}"#)
            .expect_err("a half-filled user is an error, not None");
        assert!(err.to_string().contains("missing field"), "{err}");
    }
}
