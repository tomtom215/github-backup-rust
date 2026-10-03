// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Gist metadata type.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::user::User;

/// A GitHub gist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gist {
    /// Gist identifier (hex string).
    pub id: String,
    /// Short description, or `None` if empty.
    pub description: Option<String>,
    /// Whether the gist is public.
    pub public: bool,
    /// Owner of the gist, or `None` for anonymous gists.
    pub owner: Option<User>,
    /// Files included in the gist, keyed by filename.
    ///
    /// A sorted map, so that serialising the typed value is deterministic.
    pub files: BTreeMap<String, GistFile>,
    /// Git clone URL for the gist repository.
    pub git_pull_url: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// URL of the gist on GitHub.
    pub html_url: String,
}

/// Metadata for a single file within a [`Gist`].
///
/// The OpenAPI description requires none of the properties of a gist file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistFile {
    /// File name (empty when omitted; the key of [`Gist::files`] has it too).
    #[serde(default)]
    pub filename: String,
    /// MIME type (empty when omitted).
    #[serde(default, rename = "type")]
    pub mime_type: String,
    /// Language detected by GitHub's Linguist, or `None`.
    pub language: Option<String>,
    /// File size in bytes (`0` when omitted).
    #[serde(default)]
    pub size: u64,
    /// Whether the file content is truncated in the API response.
    pub truncated: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gist_deserialise_with_file_succeeds() {
        let json = serde_json::json!({
            "id": "2decf6c462d9b4418f2",
            "description": "Hello World",
            "public": true,
            "owner": null,
            "files": {
                "ring.erl": {
                    "filename": "ring.erl",
                    "type": "text/plain",
                    "language": "Erlang",
                    "size": 932,
                    "truncated": false
                }
            },
            "git_pull_url": "https://gist.github.com/2decf6c462d9b4418f2.git",
            "created_at": "2010-04-14T02:15:15Z",
            "updated_at": "2011-06-20T11:34:15Z",
            "html_url": "https://gist.github.com/2decf6c462d9b4418f2"
        });

        let gist: Gist = serde_json::from_value(json).expect("deserialise");
        assert_eq!(gist.id, "2decf6c462d9b4418f2");
        assert!(gist.public);
        assert!(gist.files.contains_key("ring.erl"));
    }

    #[test]
    fn gist_file_properties_are_all_optional() {
        let json = serde_json::json!({
            "id": "abc",
            "description": null,
            "public": false,
            "owner": null,
            "files": { "bare.txt": {} },
            "git_pull_url": "https://gist.github.com/abc.git",
            "created_at": "2010-04-14T02:15:15Z",
            "updated_at": "2011-06-20T11:34:15Z",
            "html_url": "https://gist.github.com/abc"
        });

        let gist: Gist = serde_json::from_value(json).expect("deserialise");

        let file = &gist.files["bare.txt"];
        assert!(file.filename.is_empty() && file.mime_type.is_empty());
        assert_eq!(file.size, 0);
    }

    #[test]
    fn serialising_a_gist_twice_yields_identical_bytes() {
        // A `HashMap` here made the key order differ between two maps built
        // from the same data (each map instance has its own hash seed).
        let make = || {
            let files = (0..12)
                .map(|i| {
                    let name = format!("file-{i}.txt");
                    let file = GistFile {
                        filename: name.clone(),
                        mime_type: "text/plain".to_string(),
                        language: None,
                        size: i,
                        truncated: None,
                    };
                    (name, file)
                })
                .collect();
            Gist {
                id: "abc".to_string(),
                description: None,
                public: true,
                owner: None,
                files,
                git_pull_url: "https://gist.github.com/abc.git".to_string(),
                created_at: "2020-01-01T00:00:00Z".to_string(),
                updated_at: "2020-01-01T00:00:00Z".to_string(),
                html_url: "https://gist.github.com/abc".to_string(),
            }
        };

        let first = serde_json::to_string(&make()).expect("first");
        let second = serde_json::to_string(&make()).expect("second");

        assert_eq!(first, second);
    }
}
