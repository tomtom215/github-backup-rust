// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository webhook (hook) type.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A repository webhook configuration.
///
/// # Security note
/// Hook configurations can contain sensitive data such as payload URLs that
/// embed tokens (GitHub masks a configured secret as `********`).  Only users
/// with `admin` permission on the repository can retrieve hooks.  Backup
/// artefacts containing hook data should be stored securely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    /// Numeric hook identifier.
    pub id: u64,
    /// Hook type (always `"Repository"` for repo hooks).
    #[serde(rename = "type")]
    pub hook_type: String,
    /// Delivery name (e.g. `"web"`).
    pub name: String,
    /// Whether the hook is active.
    pub active: bool,
    /// Events that trigger this hook (e.g. `["push", "pull_request"]`).
    pub events: Vec<String>,
    /// Hook configuration (URL, content type, ...), in the order GitHub sent it.
    ///
    /// A JSON object map rather than a `HashMap`, so that serialising the typed
    /// value is deterministic.
    pub config: Map<String, Value>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_deserialise_web_hook_succeeds() {
        let json = serde_json::json!({
            "id": 1,
            "type": "Repository",
            "name": "web",
            "active": true,
            "events": ["push", "pull_request"],
            "config": {
                "url": "https://example.com/webhook",
                "content_type": "json"
            },
            "created_at": "2011-09-06T17:26:27Z",
            "updated_at": "2011-09-06T20:39:23Z"
        });

        let hook: Hook = serde_json::from_value(json).expect("deserialise");
        assert_eq!(hook.name, "web");
        assert!(hook.active);
        assert_eq!(hook.events, vec!["push", "pull_request"]);
    }

    #[test]
    fn serialising_a_hook_twice_yields_identical_bytes() {
        // With a `HashMap` config each instance iterated in its own order, so
        // two hooks built from identical data serialised differently.
        let make = || {
            let mut config = Map::new();
            for i in 0..12 {
                config.insert(format!("key-{i}"), Value::from(i));
            }
            Hook {
                id: 1,
                hook_type: "Repository".to_string(),
                name: "web".to_string(),
                active: true,
                events: vec!["push".to_string()],
                config,
                created_at: "2020-01-01T00:00:00Z".to_string(),
                updated_at: "2020-01-01T00:00:00Z".to_string(),
            }
        };

        let first = serde_json::to_string(&make()).expect("first");
        let second = serde_json::to_string(&make()).expect("second");

        assert_eq!(first, second);
    }

    #[test]
    fn hook_config_keeps_the_key_order_github_sent() {
        let text = r#"{"id":1,"type":"Repository","name":"web","active":true,"events":[],
            "config":{"url":"https://e.x","insecure_ssl":"0","content_type":"json"},
            "created_at":"a","updated_at":"b"}"#;

        let hook: Hook = serde_json::from_str(text).expect("deserialise");

        let keys: Vec<&str> = hook.config.keys().map(String::as_str).collect();
        assert_eq!(keys, ["url", "insecure_ssl", "content_type"]);
    }
}
