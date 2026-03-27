// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request, review comment, commit, and review types.

use serde::{Deserialize, Serialize};

use crate::{label::Label, milestone::Milestone, user::User};

/// A GitHub pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// Numeric PR identifier (globally unique).
    pub id: u64,
    /// Repository-scoped PR number.
    pub number: u64,
    /// PR title.
    pub title: String,
    /// PR body (Markdown), or `None` if empty.
    pub body: Option<String>,
    /// State: `"open"`, `"closed"`.
    pub state: String,
    /// Whether the PR has been merged.
    pub merged: Option<bool>,
    /// User who opened the PR.
    pub user: User,
    /// Labels applied to this PR.
    pub labels: Vec<Label>,
    /// Users assigned to this PR.
    pub assignees: Vec<User>,
    /// Milestone associated with this PR, if any.
    pub milestone: Option<Milestone>,
    /// Head branch reference.
    pub head: PullRequestRef,
    /// Base branch reference.
    pub base: PullRequestRef,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// ISO 8601 merge timestamp, or `None` if not merged.
    pub merged_at: Option<String>,
    /// ISO 8601 close timestamp, or `None` if still open.
    pub closed_at: Option<String>,
    /// URL of the PR's GitHub page.
    pub html_url: String,
    /// Number of commits in this PR.
    pub commits: Option<u64>,
    /// Number of changed files.
    pub changed_files: Option<u64>,
    /// Total lines added.
    pub additions: Option<u64>,
    /// Total lines deleted.
    pub deletions: Option<u64>,
}

/// A git ref (branch/commit) as embedded in pull request objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestRef {
    /// Branch or tag label.
    pub label: String,
    /// Branch/ref name.
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// Full commit SHA.
    pub sha: String,
    /// Repository the ref lives in, or `None` if the fork was deleted.
    pub repo: Option<PullRequestRepo>,
}

/// Slim repository descriptor embedded in [`PullRequestRef`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestRepo {
    /// Numeric repository identifier.
    pub id: u64,
    /// `owner/repo` slug.
    pub full_name: String,
    /// HTTPS clone URL.
    pub clone_url: String,
    /// Whether the repository is private.
    pub private: bool,
}

/// An inline review comment on a pull request diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestComment {
    /// Numeric comment identifier.
    pub id: u64,
    /// User who posted the comment.
    pub user: User,
    /// File path the comment is attached to.
    pub path: String,
    /// Comment body (Markdown).
    pub body: Option<String>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// URL of the comment on GitHub.
    pub html_url: String,
}

/// A single commit included in a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestCommit {
    /// Full commit SHA.
    pub sha: String,
    /// Commit details.
    pub commit: CommitDetail,
    /// Author GitHub account, or `None` if not a GitHub user.
    pub author: Option<User>,
    /// Committer GitHub account, or `None` if not a GitHub user.
    pub committer: Option<User>,
}

/// Commit metadata embedded in [`PullRequestCommit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetail {
    /// Commit message.
    pub message: String,
    /// Git author identity.
    pub author: GitIdentity,
    /// Git committer identity.
    pub committer: GitIdentity,
}

/// Git identity (name + email + date) used in commit objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentity {
    /// Display name.
    pub name: String,
    /// Email address.
    pub email: String,
    /// ISO 8601 timestamp.
    pub date: String,
}

/// A pull request review (approve / request changes / comment).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestReview {
    /// Numeric review identifier.
    pub id: u64,
    /// Reviewer.
    pub user: User,
    /// Review body, or `None` if no body was submitted.
    pub body: Option<String>,
    /// Review state: `"APPROVED"`, `"CHANGES_REQUESTED"`, `"COMMENTED"`,
    /// `"DISMISSED"`, `"PENDING"`.
    pub state: String,
    /// ISO 8601 submission timestamp.
    pub submitted_at: Option<String>,
    /// Commit SHA the review was submitted against.
    pub commit_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_user() -> serde_json::Value {
        serde_json::json!({
            "id": 1,
            "login": "octocat",
            "type": "User",
            "avatar_url": "https://github.com/images/error/octocat_happy.gif",
            "html_url": "https://github.com/octocat"
        })
    }

    fn minimal_ref(label: &str, ref_name: &str, sha: &str) -> serde_json::Value {
        serde_json::json!({
            "label": label,
            "ref": ref_name,
            "sha": sha,
            "repo": null
        })
    }

    #[test]
    fn pull_request_deserialise_open_pr_succeeds() {
        let json = serde_json::json!({
            "id": 1,
            "number": 1,
            "title": "Amazing new feature",
            "body": "Please pull these awesome changes.",
            "state": "open",
            "merged": false,
            "user": minimal_user(),
            "labels": [],
            "assignees": [],
            "milestone": null,
            "head": minimal_ref("octocat:new-feature", "new-feature", "abc123"),
            "base": minimal_ref("octocat:main", "main", "def456"),
            "created_at": "2011-01-26T19:01:12Z",
            "updated_at": "2011-01-26T19:01:12Z",
            "merged_at": null,
            "closed_at": null,
            "html_url": "https://github.com/octocat/Hello-World/pull/1",
            "commits": 3,
            "changed_files": 2,
            "additions": 100,
            "deletions": 5
        });
        let pr: PullRequest = serde_json::from_value(json).expect("deserialise");
        assert_eq!(pr.number, 1);
        assert_eq!(pr.state, "open");
        assert_eq!(pr.merged, Some(false));
    }

    #[test]
    fn pull_request_review_deserialise_approved_succeeds() {
        let json = serde_json::json!({
            "id": 80,
            "user": minimal_user(),
            "body": "LGTM",
            "state": "APPROVED",
            "submitted_at": "2019-01-01T00:00:00Z",
            "commit_id": "ecdd80bb57125d7ba9641ffde"
        });
        let review: PullRequestReview = serde_json::from_value(json).expect("deserialise");
        assert_eq!(review.state, "APPROVED");
    }
}
