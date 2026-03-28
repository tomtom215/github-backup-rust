// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Branch metadata returned by the GitHub Branches API.
//!
//! See `GET /repos/{owner}/{repo}/branches`.

use serde::{Deserialize, Serialize};

/// Summary of a repository branch.
///
/// Returned by `GET /repos/{owner}/{repo}/branches` (all branches) and
/// `GET /repos/{owner}/{repo}/branches/{branch}` (single branch).
///
/// Stored as `branches.json` in the per-repository metadata directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    /// Branch name (e.g. `"main"`, `"develop"`).
    pub name: String,
    /// Whether the branch has branch-protection rules enabled.
    ///
    /// Detailed protection rules require an admin token and are not fetched
    /// here; this flag indicates that *some* protection is in place.
    pub protected: bool,
    /// The commit that the branch tip points to.
    pub commit: BranchCommit,
}

/// The commit referenced by a [`Branch`] tip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCommit {
    /// Full commit SHA-1 hash.
    pub sha: String,
    /// GitHub API URL for the commit object.
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_branch(name: &str, protected: bool) -> Branch {
        Branch {
            name: name.to_string(),
            protected,
            commit: BranchCommit {
                sha: "abc1234".to_string(),
                url: "https://api.github.com/repos/owner/repo/commits/abc1234".to_string(),
            },
        }
    }

    #[test]
    fn branch_serialises_and_deserialises() {
        let b = make_branch("main", true);
        let json = serde_json::to_string(&b).expect("serialise");
        let decoded: Branch = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(decoded.name, "main");
        assert!(decoded.protected);
        assert_eq!(decoded.commit.sha, "abc1234");
    }

    #[test]
    fn branch_unprotected_roundtrip() {
        let b = make_branch("feature-x", false);
        let json = serde_json::to_string(&b).expect("serialise");
        let decoded: Branch = serde_json::from_str(&json).expect("deserialise");
        assert!(!decoded.protected);
        assert_eq!(decoded.name, "feature-x");
    }

    #[test]
    fn branch_list_roundtrip() {
        let branches = vec![
            make_branch("main", true),
            make_branch("develop", false),
            make_branch("release/1.0", true),
        ];
        let json = serde_json::to_string(&branches).expect("serialise");
        let decoded: Vec<Branch> = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[2].name, "release/1.0");
    }
}
