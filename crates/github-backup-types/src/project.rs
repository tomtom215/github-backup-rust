// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Projects (classic and v2) API types.

use serde::{Deserialize, Serialize};

use crate::user::User;

/// A GitHub Classic Project (Projects v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassicProject {
    /// Numeric project ID.
    pub id: u64,
    /// Project number within the repository/org.
    pub number: u64,
    /// Project name.
    pub name: String,
    /// Project body / description.
    #[serde(default)]
    pub body: Option<String>,
    /// State: `"open"` or `"closed"`.
    pub state: String,
    /// Creator of the project.
    pub creator: User,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// HTML URL on GitHub.
    pub html_url: String,
    /// Number of open cards across all columns.
    #[serde(default)]
    pub open_issues_count: Option<u64>,
}

/// A column in a Classic Project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectColumn {
    /// Numeric column ID.
    pub id: u64,
    /// Column name (e.g. "To do", "In progress", "Done").
    pub name: String,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// Cards in this column.
    #[serde(default)]
    pub cards: Vec<ProjectCard>,
}

/// A card in a Classic Project column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectCard {
    /// Numeric card ID.
    pub id: u64,
    /// Free-text note (if not linked to an issue or PR).
    #[serde(default)]
    pub note: Option<String>,
    /// Archive state.
    pub archived: bool,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// URL of the content item this card represents (issue or PR URL).
    #[serde(default)]
    pub content_url: Option<String>,
}
