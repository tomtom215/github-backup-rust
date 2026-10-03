// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request, review comment, commit, and review types.

use serde::{Deserialize, Serialize};

use crate::{
    label::Label,
    milestone::Milestone,
    user::{deserialize_user_or_empty, User},
};

/// A GitHub pull request as returned by the pull request *list*
/// (`GET /repos/{owner}/{repo}/pulls`, schema `pull-request-simple`).
///
/// The list objects carry no merge statistics (`merged`, `commits`,
/// `additions`, ...); those exist only on the single-pull-request endpoint,
/// which the backup does not call, so they are deliberately not modelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// Numeric PR identifier (globally unique).
    pub id: u64,
    /// Repository-scoped PR number.
    ///
    /// Pull requests and issues share one number space per repository.
    pub number: u64,
    /// PR title.
    pub title: String,
    /// PR body (Markdown), or `None` if empty.
    pub body: Option<String>,
    /// State: `"open"`, `"closed"`.
    pub state: String,
    /// User who opened the PR; `None` when the account no longer exists.
    pub user: Option<User>,
    /// Labels applied to this PR.
    pub labels: Vec<Label>,
    /// Users assigned to this PR.
    #[serde(default)]
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
    /// User who posted the comment; `None` when the account no longer exists.
    pub user: Option<User>,
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
    /// Author GitHub account, or `None` if not a GitHub user (`null` or `{}`).
    #[serde(default, deserialize_with = "deserialize_user_or_empty")]
    pub author: Option<User>,
    /// Committer GitHub account, or `None` if not a GitHub user (`null` or `{}`).
    #[serde(default, deserialize_with = "deserialize_user_or_empty")]
    pub committer: Option<User>,
}

/// Commit metadata embedded in [`PullRequestCommit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetail {
    /// Commit message.
    pub message: String,
    /// Git author identity; `None` when GitHub sends `null`.
    pub author: Option<GitIdentity>,
    /// Git committer identity; `None` when GitHub sends `null`.
    pub committer: Option<GitIdentity>,
}

/// Git identity (name + email + date) used in commit objects.
///
/// None of the properties is required by the OpenAPI description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentity {
    /// Display name (empty when omitted).
    #[serde(default)]
    pub name: String,
    /// Email address (empty when omitted).
    #[serde(default)]
    pub email: String,
    /// ISO 8601 timestamp (empty when omitted).
    #[serde(default)]
    pub date: String,
}

/// A pull request review (approve / request changes / comment).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestReview {
    /// Numeric review identifier.
    pub id: u64,
    /// Reviewer; `None` when the account no longer exists.
    pub user: Option<User>,
    /// Review body, or `None` if no body was submitted.
    pub body: Option<String>,
    /// Review state: `"APPROVED"`, `"CHANGES_REQUESTED"`, `"COMMENTED"`,
    /// `"DISMISSED"`, `"PENDING"`.
    pub state: String,
    /// ISO 8601 submission timestamp.
    pub submitted_at: Option<String>,
    /// Commit SHA the review was submitted against; `None` when `null`.
    pub commit_id: Option<String>,
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

    fn minimal_pr() -> serde_json::Value {
        serde_json::json!({
            "id": 1,
            "number": 1,
            "title": "Amazing new feature",
            "body": "Please pull these awesome changes.",
            "state": "open",
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
        })
    }

    #[test]
    fn pull_request_deserialise_open_pr_succeeds() {
        let pr: PullRequest = serde_json::from_value(minimal_pr()).expect("deserialise");
        assert_eq!(pr.number, 1);
        assert_eq!(pr.state, "open");
        assert!(pr.merged_at.is_none());
    }

    #[test]
    fn pull_request_serialises_without_fields_the_list_never_returns() {
        let pr: PullRequest = serde_json::from_value(minimal_pr()).expect("deserialise");

        let value = serde_json::to_value(&pr).expect("serialise");
        let object = value.as_object().expect("object");

        // These used to be invented as explicit `null`s in every backup.
        for key in ["merged", "commits", "changed_files", "additions", "deletions"] {
            assert!(!object.contains_key(key), "{key} must not be invented");
        }
    }

    #[test]
    fn pull_request_accepts_null_user_and_missing_assignees() {
        let mut value = minimal_pr();
        value["user"] = serde_json::Value::Null;
        value.as_object_mut().expect("object").remove("assignees");

        let pr: PullRequest = serde_json::from_value(value).expect("deserialise");

        assert!(pr.user.is_none());
        assert!(pr.assignees.is_empty());
    }

    #[test]
    fn pull_request_accepts_a_deleted_head_fork() {
        // `head.repo` is null once the fork is deleted; the user may be gone too.
        let mut value = minimal_pr();
        value["user"] = serde_json::Value::Null;
        value["head"]["repo"] = serde_json::Value::Null;

        let pr: PullRequest = serde_json::from_value(value).expect("deserialise");

        assert!(pr.head.repo.is_none());
        assert!(pr.user.is_none());
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

    #[test]
    fn pull_request_review_accepts_null_user_and_null_commit_id() {
        let json = serde_json::json!({
            "id": 81,
            "user": null,
            "body": "",
            "state": "DISMISSED",
            "submitted_at": null,
            "commit_id": null
        });

        let review: PullRequestReview = serde_json::from_value(json).expect("deserialise");

        assert!(review.user.is_none());
        assert!(review.commit_id.is_none());
    }

    #[test]
    fn pull_request_comment_accepts_a_null_user() {
        let json = serde_json::json!({
            "id": 9,
            "user": null,
            "path": "src/lib.rs",
            "body": "nit",
            "created_at": "2011-04-14T16:00:49Z",
            "updated_at": "2011-04-14T16:00:49Z",
            "html_url": "https://github.com/octocat/Hello-World/pull/1#discussion_r9"
        });

        let comment: PullRequestComment = serde_json::from_value(json).expect("deserialise");

        assert!(comment.user.is_none());
    }

    fn commit_json(author: serde_json::Value, git_author: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "sha": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
            "commit": {
                "message": "Fix all the bugs",
                "author": git_author,
                "committer": null
            },
            "author": author,
            "committer": {}
        })
    }

    #[test]
    fn pull_request_commit_accepts_empty_object_and_null_identities() {
        let json = commit_json(serde_json::Value::Null, serde_json::Value::Null);

        let commit: PullRequestCommit = serde_json::from_value(json).expect("deserialise");

        assert!(commit.author.is_none(), "null GitHub user");
        assert!(commit.committer.is_none(), "empty-object GitHub user");
        assert!(commit.commit.author.is_none(), "null git author");
        assert!(commit.commit.committer.is_none(), "null git committer");
    }

    #[test]
    fn pull_request_commit_parses_a_linked_account_and_partial_git_identity() {
        let json = commit_json(minimal_user(), serde_json::json!({"name": "Mona"}));

        let commit: PullRequestCommit = serde_json::from_value(json).expect("deserialise");

        assert_eq!(commit.author.map(|u| u.login), Some("octocat".to_string()));
        let identity = commit.commit.author.expect("git author");
        assert_eq!(identity.name, "Mona");
        assert!(identity.email.is_empty() && identity.date.is_empty());
    }
}
