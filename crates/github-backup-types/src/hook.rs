// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository webhook (hook) type.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A repository webhook configuration.
///
/// # Security note
/// Hook configurations can contain sensitive data such as secrets and
/// payload URLs. Only users with `admin` permission on the repository can
/// retrieve hooks. Backup artefacts containing hook data should be stored
/// securely.
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
    /// Hook configuration (URL, content type, secret hash, etc.).
    pub config: HashMap<String, serde_json::Value>,
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
}
