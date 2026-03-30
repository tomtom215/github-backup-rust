// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Branch-protection rule types returned by the GitHub Branch Protection API.
//!
//! See `GET /repos/{owner}/{repo}/branches/{branch}/protection`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Detailed branch-protection rules for a single branch.
///
/// Stored as part of `branch_protections.json` in the per-repository metadata
/// directory.  Fetched only when `opts.branches` is enabled and the token
/// has admin access to the repository (403 responses are silently skipped).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchProtection {
    /// GitHub API URL for this protection resource.
    pub url: String,

    /// Status-check requirements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_status_checks: Option<RequiredStatusChecks>,

    /// Whether branch-protection rules apply to administrators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_admins: Option<AdminEnforcement>,

    /// Pull-request review requirements before merging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_pull_request_reviews: Option<RequiredPullRequestReviews>,

    /// Push-access restrictions (users, teams, apps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restrictions: Option<Restrictions>,

    /// Requires a linear commit history (no merge commits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_linear_history: Option<SimpleEnabled>,

    /// Whether force-pushes are allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_force_pushes: Option<SimpleEnabled>,

    /// Whether branch deletion is allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_deletions: Option<SimpleEnabled>,

    /// Whether direct pushes to matching branches are blocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_creations: Option<SimpleEnabled>,

    /// Whether open comments must be resolved before merging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_conversation_resolution: Option<SimpleEnabled>,

    /// Whether the branch is read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_branch: Option<SimpleEnabled>,

    /// Whether forks can sync with the upstream repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fork_syncing: Option<SimpleEnabled>,
}

/// A feature flag that is simply enabled or disabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleEnabled {
    /// Whether the feature is currently enabled.
    pub enabled: bool,
}

/// Enforcement setting for administrators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminEnforcement {
    /// GitHub API URL for this enforcement resource.
    pub url: String,
    /// Whether branch-protection rules also apply to administrators.
    pub enabled: bool,
}

/// Required status checks that must pass before a branch can be merged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredStatusChecks {
    /// Whether branches must be up-to-date before merging.
    pub strict: bool,
    /// Status check context names that must pass.
    pub contexts: Vec<String>,
    /// Fine-grained status check requirements (GitHub-defined objects).
    #[serde(default)]
    pub checks: Vec<Value>,
}

/// Required pull-request review settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredPullRequestReviews {
    /// Whether approved reviews are dismissed when a new commit is pushed.
    pub dismiss_stale_reviews: bool,
    /// Whether the most-recent push approval is required from someone
    /// other than the person who pushed.
    #[serde(default)]
    pub require_last_push_approval: bool,
    /// Whether code-owner review is required.
    pub require_code_owner_reviews: bool,
    /// Minimum number of approving reviews required.
    pub required_approving_review_count: u32,
}

/// Push-access restrictions for a branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Restrictions {
    /// GitHub API URL for this restrictions resource.
    pub url: String,
    /// Users that are allowed to push to the branch.
    #[serde(default)]
    pub users: Vec<Value>,
    /// Teams that are allowed to push to the branch.
    #[serde(default)]
    pub teams: Vec<Value>,
    /// GitHub Apps that are allowed to push to the branch.
    #[serde(default)]
    pub apps: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_protection_minimal_roundtrip() {
        let json = serde_json::json!({
            "url": "https://api.github.com/repos/owner/repo/branches/main/protection",
            "enforce_admins": {"url": "...", "enabled": true},
            "required_linear_history": {"enabled": true},
            "allow_force_pushes": {"enabled": false},
            "allow_deletions": {"enabled": false}
        });
        let bp: BranchProtection = serde_json::from_value(json).expect("deserialise");
        assert_eq!(
            bp.url,
            "https://api.github.com/repos/owner/repo/branches/main/protection"
        );
        assert!(bp.enforce_admins.as_ref().unwrap().enabled);
        assert!(bp.required_status_checks.is_none());
        let re = serde_json::to_string(&bp).expect("serialise");
        assert!(re.contains("enforce_admins"));
    }

    #[test]
    fn branch_protection_with_status_checks() {
        let json = serde_json::json!({
            "url": "https://api.github.com/repos/owner/repo/branches/main/protection",
            "required_status_checks": {
                "strict": true,
                "contexts": ["ci/tests", "ci/lint"],
                "checks": []
            }
        });
        let bp: BranchProtection = serde_json::from_value(json).expect("deserialise");
        let checks = bp.required_status_checks.unwrap();
        assert!(checks.strict);
        assert_eq!(checks.contexts, ["ci/tests", "ci/lint"]);
    }

    #[test]
    fn simple_enabled_roundtrip() {
        let on = SimpleEnabled { enabled: true };
        let json = serde_json::to_string(&on).unwrap();
        let decoded: SimpleEnabled = serde_json::from_str(&json).unwrap();
        assert!(decoded.enabled);
    }
}
