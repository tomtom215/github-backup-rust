// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Discussions API types.

use serde::{Deserialize, Serialize};

use crate::user::User;

/// A GitHub Discussion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discussion {
    /// Numeric discussion ID.
    pub number: u64,
    /// Discussion title.
    pub title: String,
    /// Markdown body.
    #[serde(default)]
    pub body: String,
    /// Whether the discussion is locked.
    #[serde(default)]
    pub locked: bool,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// HTML URL on GitHub.
    pub html_url: String,
    /// Author of the discussion.
    pub user: User,
    /// Number of comments on the discussion.
    pub comments: u64,
    /// Discussion category (if provided by the API).
    #[serde(default)]
    pub category: Option<DiscussionCategory>,
    /// Whether this discussion has been answered.
    #[serde(default)]
    pub answered: bool,
}

/// A discussion category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionCategory {
    /// Numeric ID of the category.
    pub id: u64,
    /// Category name (e.g. "General", "Q&A").
    pub name: String,
    /// Category description.
    #[serde(default)]
    pub description: String,
    /// Whether this category supports "answered" discussions.
    #[serde(default)]
    pub is_answerable: bool,
}

/// A comment on a GitHub Discussion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionComment {
    /// Numeric comment ID.
    pub id: u64,
    /// Comment body.
    #[serde(default)]
    pub body: String,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// HTML URL on GitHub.
    pub html_url: String,
    /// Comment author.
    pub user: User,
}
