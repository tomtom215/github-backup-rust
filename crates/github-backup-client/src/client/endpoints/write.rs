// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Write (mutating) GitHub REST API endpoints.
//!
//! Used exclusively by the `--restore` mode to re-create backed-up metadata
//! (labels, milestones) in a target repository or organisation.
//!
//! All methods are additive — they **never** delete existing data.

use serde::Serialize;

use github_backup_types::{Issue, Label, Milestone};

use crate::error::ClientError;

use super::super::GitHubClient;

/// Request body for `POST /repos/{owner}/{repo}/labels`.
#[derive(Debug, Serialize)]
struct CreateLabelRequest<'a> {
    name: &'a str,
    color: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

/// Request body for `POST /repos/{owner}/{repo}/milestones`.
#[derive(Debug, Serialize)]
struct CreateMilestoneRequest<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    due_on: Option<&'a str>,
}

/// Request body for `POST /repos/{owner}/{repo}/issues`.
#[derive(Debug, Serialize)]
struct CreateIssueRequest<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<&'a str>,
}

impl GitHubClient {
    // ── Write endpoints ───────────────────────────────────────────────────

    /// Creates a label in a repository.
    ///
    /// Returns the newly created [`Label`] as returned by the GitHub API.
    /// If a label with the same name already exists (HTTP 422), the error is
    /// returned as a [`ClientError::ApiError`] with `status = 422`; callers
    /// may choose to ignore it.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn create_label(
        &self,
        owner: &str,
        repo: &str,
        name: &str,
        color: &str,
        description: Option<&str>,
    ) -> Result<Label, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/labels");
        let body = CreateLabelRequest {
            name,
            color,
            description,
        };
        self.post_json(&url, &body).await
    }

    /// Creates an issue in a repository.
    ///
    /// Restores a backed-up issue using `POST /repos/{owner}/{repo}/issues`.
    /// The `labels` slice should contain label **names** (not IDs); the GitHub
    /// API accepts names directly and will attach labels that already exist in
    /// the repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: Option<&str>,
        labels: &[&str],
    ) -> Result<Issue, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/issues");
        let req = CreateIssueRequest {
            title,
            body,
            labels: labels.to_vec(),
        };
        self.post_json(&url, &req).await
    }

    /// Creates a milestone in a repository.
    ///
    /// Returns the newly created [`Milestone`] as returned by the GitHub API.
    /// If a milestone with the same title already exists (HTTP 422), the error
    /// is returned as a [`ClientError::ApiError`] with `status = 422`; callers
    /// may choose to ignore it.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn create_milestone(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        description: Option<&str>,
        state: Option<&str>,
        due_on: Option<&str>,
    ) -> Result<Milestone, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/milestones");
        let body = CreateMilestoneRequest {
            title,
            description,
            state,
            due_on,
        };
        self.post_json(&url, &body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_label_request_skips_none_description() {
        let req = CreateLabelRequest {
            name: "bug",
            color: "d73a4a",
            description: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("description"), "None should be omitted");
        assert!(json.contains("\"name\":\"bug\""));
    }

    #[test]
    fn create_label_request_includes_description_when_some() {
        let req = CreateLabelRequest {
            name: "enhancement",
            color: "a2eeef",
            description: Some("New feature"),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"description\":\"New feature\""));
    }

    #[test]
    fn create_issue_request_includes_title_and_body() {
        let req = CreateIssueRequest {
            title: "Found a bug",
            body: Some("Reproduction steps here"),
            labels: vec!["bug", "help wanted"],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"title\":\"Found a bug\""));
        assert!(json.contains("\"body\":\"Reproduction steps here\""));
        assert!(json.contains("\"labels\":[\"bug\",\"help wanted\"]"));
    }

    #[test]
    fn create_issue_request_skips_none_body_and_empty_labels() {
        let req = CreateIssueRequest {
            title: "Minimal issue",
            body: None,
            labels: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"title\":\"Minimal issue\""));
        assert!(!json.contains("body"), "None body should be omitted");
        assert!(!json.contains("labels"), "empty labels should be omitted");
    }

    #[test]
    fn create_milestone_request_skips_none_fields() {
        let req = CreateMilestoneRequest {
            title: "v1.0",
            description: None,
            state: None,
            due_on: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("description"));
        assert!(!json.contains("state"));
        assert!(!json.contains("due_on"));
    }
}
