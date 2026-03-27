// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Gist metadata type.

use std::collections::HashMap;

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
    pub files: HashMap<String, GistFile>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistFile {
    /// File name.
    pub filename: String,
    /// MIME type.
    #[serde(rename = "type")]
    pub mime_type: String,
    /// Language detected by GitHub's Linguist, or `None`.
    pub language: Option<String>,
    /// File size in bytes.
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
}
