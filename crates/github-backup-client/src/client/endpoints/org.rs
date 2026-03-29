// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Organisation data listing endpoints.
//!
//! Covers organisation member lists and team rosters. Only meaningful when
//! the backup target is an organisation.

use github_backup_types::{Team, User};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── Organisation data ─────────────────────────────────────────────────

    /// Lists members of a GitHub organisation.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_members(&self, org: &str) -> Result<Vec<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/orgs/{org}/members?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists teams in a GitHub organisation.
    ///
    /// Requires the authenticated user to be an organisation member.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_teams(&self, org: &str) -> Result<Vec<Team>, ClientError> {
        let api = self.api();
        let url = format!("{api}/orgs/{org}/teams?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
