// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository deploy key type.

use serde::{Deserialize, Serialize};

/// A deploy key configured on a repository.
///
/// Deploy keys are SSH public keys that grant read (or optionally write)
/// access to a single repository.  Backing them up is useful for auditing
/// which machines have access to each repository and for disaster recovery.
///
/// Requires admin access to the repository to retrieve.
///
/// Source: `GET /repos/{owner}/{repo}/keys`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployKey {
    /// Numeric deploy key identifier.
    pub id: u64,
    /// Public SSH key value (e.g. `ssh-rsa AAAA...`).
    pub key: String,
    /// API URL for this deploy key resource.
    pub url: String,
    /// Human-readable label for the key.
    pub title: String,
    /// Whether GitHub has verified this key's fingerprint.
    pub verified: bool,
    /// ISO 8601 timestamp when the key was added.
    pub created_at: String,
    /// When `true`, the key grants read-only access; write access otherwise.
    pub read_only: bool,
    /// Login of the user who added the key (may be absent in older records).
    #[serde(default)]
    pub added_by: Option<String>,
    /// ISO 8601 timestamp of the last time this key was used.
    #[serde(default)]
    pub last_used: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploy_key_roundtrip() {
        let key = DeployKey {
            id: 1,
            key: "ssh-rsa AAAA...".to_string(),
            url: "https://api.github.com/repos/octocat/Hello-World/keys/1".to_string(),
            title: "CI key".to_string(),
            verified: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            read_only: true,
            added_by: Some("octocat".to_string()),
            last_used: None,
        };
        let json = serde_json::to_string(&key).expect("serialise");
        let decoded: DeployKey = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(key, decoded);
    }

    #[test]
    fn deploy_key_deserialise_without_optional_fields() {
        let json = r#"{
            "id": 2,
            "key": "ssh-ed25519 AAAA...",
            "url": "https://api.github.com/repos/octocat/repo/keys/2",
            "title": "deploy key",
            "verified": false,
            "created_at": "2025-06-01T12:00:00Z",
            "read_only": true
        }"#;
        let key: DeployKey = serde_json::from_str(json).expect("deserialise");
        assert_eq!(key.id, 2);
        assert!(key.added_by.is_none());
        assert!(key.last_used.is_none());
    }
}
