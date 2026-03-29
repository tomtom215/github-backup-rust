// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository deployment environment types.

use serde::{Deserialize, Serialize};

/// A repository deployment environment.
///
/// Environments model deployment targets such as `staging` or `production`.
/// They can have protection rules (required reviewers, wait timers, branch
/// policies) that gate automated deployments.  Backing up environment metadata
/// makes it possible to audit and reproduce deployment gate configurations.
///
/// # API reference
///
/// `GET /repos/{owner}/{repo}/environments`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// Numeric environment identifier.
    pub id: u64,
    /// Unique node ID.
    pub node_id: String,
    /// Human-readable environment name (e.g. `"production"`, `"staging"`).
    pub name: String,
    /// GitHub API URL for this environment.
    pub url: String,
    /// GitHub HTML URL for viewing this environment in a browser.
    pub html_url: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// Ordered list of protection rules applied to this environment.
    ///
    /// Rules are evaluated in order; a deployment is blocked until **all**
    /// rules pass.
    #[serde(default)]
    pub protection_rules: Vec<EnvironmentProtectionRule>,
    /// Deployment branch (or tag) policy, if configured.
    #[serde(default)]
    pub deployment_branch_policy: Option<DeploymentBranchPolicy>,
}

/// A single protection rule on a deployment environment.
///
/// GitHub currently supports three rule types: `required_reviewers`,
/// `wait_timer`, and `branch_policy`.  Unknown future types are preserved
/// as raw JSON via the `raw` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProtectionRule {
    /// Numeric rule identifier.
    pub id: u64,
    /// Unique node ID.
    pub node_id: String,
    /// Rule type string: `"required_reviewers"`, `"wait_timer"`, or
    /// `"branch_policy"`.
    #[serde(rename = "type")]
    pub rule_type: String,
    /// Wait timer duration in minutes (only set for `"wait_timer"` rules).
    #[serde(default)]
    pub wait_timer: Option<u32>,
    /// Accounts/teams required to approve deployments (for
    /// `"required_reviewers"` rules).
    #[serde(default)]
    pub reviewers: Vec<EnvironmentReviewer>,
}

/// A reviewer (user or team) required to approve a deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentReviewer {
    /// Reviewer type: `"User"` or `"Team"`.
    #[serde(rename = "type")]
    pub reviewer_type: String,
    /// The reviewer's login name (for users) or slug (for teams).
    #[serde(default)]
    pub reviewer: Option<serde_json::Value>,
}

/// Deployment branch and tag policy for an environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentBranchPolicy {
    /// When `true`, only branches matching the configured patterns can deploy.
    pub protected_branches: bool,
    /// When `true`, custom branch name patterns have been configured.
    pub custom_branch_policies: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_deserialise_minimal_succeeds() {
        let json = serde_json::json!({
            "id": 526860367,
            "node_id": "EN_kwDOI6WqtM4B4cOP",
            "name": "production",
            "url": "https://api.github.com/repos/octocat/hello-world/environments/production",
            "html_url": "https://github.com/octocat/hello-world/deployments/activity_log?environments_filter=production",
            "created_at": "2022-06-15T19:06:04.000Z",
            "updated_at": "2022-06-16T00:04:22.000Z",
            "protection_rules": [],
            "deployment_branch_policy": null
        });

        let env: Environment = serde_json::from_value(json).expect("deserialise");
        assert_eq!(env.name, "production");
        assert!(env.protection_rules.is_empty());
        assert!(env.deployment_branch_policy.is_none());
    }

    #[test]
    fn environment_deserialise_with_protection_rules() {
        let json = serde_json::json!({
            "id": 1,
            "node_id": "abc",
            "name": "staging",
            "url": "https://api.github.com/repos/o/r/environments/staging",
            "html_url": "https://github.com/o/r/deployments/activity_log",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "protection_rules": [
                {
                    "id": 10,
                    "node_id": "def",
                    "type": "wait_timer",
                    "wait_timer": 30
                }
            ],
            "deployment_branch_policy": {
                "protected_branches": false,
                "custom_branch_policies": true
            }
        });

        let env: Environment = serde_json::from_value(json).expect("deserialise");
        assert_eq!(env.protection_rules.len(), 1);
        assert_eq!(env.protection_rules[0].rule_type, "wait_timer");
        assert_eq!(env.protection_rules[0].wait_timer, Some(30));

        let policy = env.deployment_branch_policy.as_ref().unwrap();
        assert!(policy.custom_branch_policies);
        assert!(!policy.protected_branches);
    }
}
