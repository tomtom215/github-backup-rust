// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Comment types that need nothing but their identity: commit comments and
//! gist comments.

use serde::{Deserialize, Serialize};

/// A comment on a commit (`GET /repos/{owner}/{repo}/comments`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitComment {
    /// Unique comment identifier.
    pub id: u64,
    /// SHA of the commit the comment is on (empty when omitted).
    #[serde(default)]
    pub commit_id: String,
}

/// A comment on a gist (`GET /gists/{gist_id}/comments`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistComment {
    /// Unique comment identifier.
    pub id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_parse_with_extra_properties() {
        let c: CommitComment = serde_json::from_value(serde_json::json!({
            "id": 1, "commit_id": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
            "body": "Great stuff", "path": "file1.txt", "position": 4, "line": 14
        }))
        .expect("commit comment");
        assert_eq!(c.id, 1);
        let g: GistComment =
            serde_json::from_value(serde_json::json!({"id": 1, "body": "Just commenting"}))
                .expect("gist comment");
        assert_eq!(g.id, 1);
    }
}
