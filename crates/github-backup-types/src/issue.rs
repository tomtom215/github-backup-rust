// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue, issue comment, and issue event types.

use serde::{Deserialize, Serialize};

use crate::{label::Label, milestone::Milestone, user::User};

/// A GitHub issue (note: pull requests also appear as issues in the issues API).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// Numeric issue identifier (globally unique).
    pub id: u64,
    /// Repository-scoped issue number.
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// Issue body (Markdown), or `None` if empty.
    pub body: Option<String>,
    /// State: `"open"` or `"closed"`.
    pub state: String,
    /// User who opened the issue.
    pub user: User,
    /// Labels applied to this issue.
    pub labels: Vec<Label>,
    /// Users assigned to this issue.
    pub assignees: Vec<User>,
    /// Milestone associated with this issue, if any.
    pub milestone: Option<Milestone>,
    /// Whether this issue is actually a pull request.
    ///
    /// GitHub's Issues API returns PRs as issues; this field being `Some`
    /// distinguishes them.
    pub pull_request: Option<IssuePullRequestRef>,
    /// Number of comments on this issue.
    pub comments: u64,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// ISO 8601 closed timestamp, or `None` if still open.
    pub closed_at: Option<String>,
    /// URL of the issue's GitHub page.
    pub html_url: String,
}

impl Issue {
    /// Returns `true` if this issue is actually a pull request.
    #[must_use]
    pub fn is_pull_request(&self) -> bool {
        self.pull_request.is_some()
    }
}

/// Stub reference present on issue objects that are actually pull requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuePullRequestRef {
    /// API URL of the pull request.
    pub url: String,
    /// HTML URL of the pull request.
    pub html_url: String,
}

/// A comment on a GitHub issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    /// Numeric comment identifier.
    pub id: u64,
    /// User who posted the comment.
    pub user: User,
    /// Comment body (Markdown).
    pub body: Option<String>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// URL of the comment on GitHub.
    pub html_url: String,
}

/// An event in a GitHub issue's timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueEvent {
    /// Numeric event identifier.
    pub id: u64,
    /// Actor who caused the event, or `None` for system events.
    pub actor: Option<User>,
    /// Event type (e.g. `"closed"`, `"labeled"`, `"assigned"`).
    pub event: String,
    /// ISO 8601 timestamp of the event.
    pub created_at: String,
    /// Label involved in a `labeled`/`unlabeled` event, if applicable.
    pub label: Option<EventLabel>,
    /// Assignee involved in an `assigned`/`unassigned` event, if applicable.
    pub assignee: Option<User>,
    /// Milestone involved in a `milestoned`/`demilestoned` event, if applicable.
    pub milestone: Option<EventMilestone>,
}

/// Label reference embedded in label-related issue events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLabel {
    /// Label name.
    pub name: String,
    /// Hex colour string without the leading `#`.
    pub color: String,
}

/// Milestone reference embedded in milestone-related issue events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMilestone {
    /// Milestone title.
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_issue_json() -> &'static str {
        r#"{
            "id": 1,
            "number": 1347,
            "title": "Found a bug",
            "body": "I'm having a problem with this.",
            "state": "open",
            "user": {
                "id": 1,
                "login": "octocat",
                "type": "User",
                "avatar_url": "https://github.com/images/error/octocat_happy.gif",
                "html_url": "https://github.com/octocat"
            },
            "labels": [],
            "assignees": [],
            "milestone": null,
            "pull_request": null,
            "comments": 0,
            "created_at": "2011-04-22T13:33:48Z",
            "updated_at": "2011-04-22T13:33:48Z",
            "closed_at": null,
            "html_url": "https://github.com/octocat/Hello-World/issues/1347"
        }"#
    }

    #[test]
    fn issue_deserialise_returns_correct_fields() {
        let issue: Issue = serde_json::from_str(sample_issue_json()).expect("deserialise");
        assert_eq!(issue.number, 1347);
        assert_eq!(issue.title, "Found a bug");
        assert_eq!(issue.state, "open");
        assert!(!issue.is_pull_request());
    }

    #[test]
    fn issue_is_pull_request_true_when_pull_request_field_present() {
        let mut json = sample_issue_json().to_string();
        json = json.replace(
            r#""pull_request": null"#,
            r#""pull_request": {"url": "https://api.github.com/repos/octocat/Hello-World/pulls/1347", "html_url": "https://github.com/octocat/Hello-World/pull/1347"}"#,
        );
        let issue: Issue = serde_json::from_str(&json).expect("deserialise");
        assert!(issue.is_pull_request());
    }

    #[test]
    fn issue_comment_deserialise_succeeds() {
        let json = r#"{
            "id": 1,
            "user": {
                "id": 1,
                "login": "octocat",
                "type": "User",
                "avatar_url": "https://github.com/images/error/octocat_happy.gif",
                "html_url": "https://github.com/octocat"
            },
            "body": "Me too",
            "created_at": "2011-04-14T16:00:49Z",
            "updated_at": "2011-04-14T16:00:49Z",
            "html_url": "https://github.com/octocat/Hello-World/issues/1347#issuecomment-1"
        }"#;
        let comment: IssueComment = serde_json::from_str(json).expect("deserialise");
        assert_eq!(comment.id, 1);
        assert_eq!(comment.body.as_deref(), Some("Me too"));
    }

    #[test]
    fn issue_event_deserialise_closed_event_succeeds() {
        let json = r#"{
            "id": 6430295168,
            "actor": {
                "id": 1,
                "login": "octocat",
                "type": "User",
                "avatar_url": "https://github.com/images/error/octocat_happy.gif",
                "html_url": "https://github.com/octocat"
            },
            "event": "closed",
            "created_at": "2024-01-01T00:00:00Z",
            "label": null,
            "assignee": null,
            "milestone": null
        }"#;
        let event: IssueEvent = serde_json::from_str(json).expect("deserialise");
        assert_eq!(event.event, "closed");
    }
}
