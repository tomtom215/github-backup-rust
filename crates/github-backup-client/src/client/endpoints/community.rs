// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Owner-level Projects (v2) and GitHub Packages endpoints.
//!
//! - **Projects (v2)** – `GET /users/{username}/projectsV2` and
//!   `GET /orgs/{org}/projectsV2`, each project's `fields` and `items`.  The
//!   routes are in GitHub's OpenAPI description for API version 2022-11-28
//!   (the version this client requests) as well as 2026-03-10; they paginate
//!   with cursors carried in the `Link` header.
//! - **Packages** – require the `read:packages` scope.  An organisation's
//!   packages are listed under `/orgs/{org}`; the token owner's own packages
//!   under `/user`, which includes private ones; anybody else's under
//!   `/users/{username}`.

use tracing::info;

use github_backup_types::{
    Package, PackageVersion, Page, ProjectV2, ProjectV2Field, ProjectV2Item,
};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};
use super::encode_segment;

/// The most field ids one item listing may name (`fields` has `maxItems: 50`).
pub(crate) const MAX_PROJECT_FIELDS: usize = 50;

impl GitHubClient {
    // ── Projects (v2) ─────────────────────────────────────────────────────

    /// `{api}/users/{owner}` or `{api}/orgs/{owner}`.
    fn owner_base(&self, owner: &str, org: bool) -> String {
        let api = self.api();
        if org {
            format!("{api}/orgs/{owner}")
        } else {
            format!("{api}/users/{owner}")
        }
    }

    /// Lists the Projects (v2) of a user (`org == false`) or an organisation.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_projects(
        &self,
        owner: &str,
        org: bool,
    ) -> Result<Page<ProjectV2>, ClientError> {
        let base = self.owner_base(owner, org);
        let url = format!("{base}/projectsV2?per_page={PER_PAGE}");
        let all = self.get_all_pages(&url).await?;
        info!(owner, count = all.len(), "fetched projects");
        Ok(all)
    }

    /// Lists the fields of one project.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_project_fields(
        &self,
        owner: &str,
        org: bool,
        project_number: u64,
    ) -> Result<Page<ProjectV2Field>, ClientError> {
        let base = self.owner_base(owner, org);
        let url = format!("{base}/projectsV2/{project_number}/fields?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists the items of one project with the values of `field_ids` (at most
    /// [`MAX_PROJECT_FIELDS`]; without `fields` GitHub returns only the title).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_project_items(
        &self,
        owner: &str,
        org: bool,
        project_number: u64,
        field_ids: &[u64],
    ) -> Result<Page<ProjectV2Item>, ClientError> {
        let base = self.owner_base(owner, org);
        let mut url = format!("{base}/projectsV2/{project_number}/items?per_page={PER_PAGE}");
        if !field_ids.is_empty() {
            let ids: Vec<String> = field_ids
                .iter()
                .take(MAX_PROJECT_FIELDS)
                .map(u64::to_string)
                .collect();
            // `fields=1,2,3`: the comma is encoded as %2C.
            url.push_str("&fields=");
            url.push_str(&encode_segment(&ids.join(",")));
        }
        self.get_all_pages(&url).await
    }

    // ── GitHub Packages ───────────────────────────────────────────────────

    /// The URL prefix of the packages of `owner`: `/orgs/{org}` for an
    /// organisation, `/user` for the token owner (so private packages are
    /// included), `/users/{username}` for anybody else.
    async fn packages_base(&self, owner: &str, org: bool) -> Result<String, ClientError> {
        let api = self.api();
        Ok(if org {
            format!("{api}/orgs/{owner}")
        } else if self.is_authenticated_user(owner).await? {
            format!("{api}/user")
        } else {
            format!("{api}/users/{owner}")
        })
    }

    /// Lists the packages of one ecosystem (`package_type`) owned by a user
    /// (`org == false`) or an organisation.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_packages(
        &self,
        owner: &str,
        org: bool,
        package_type: &str,
    ) -> Result<Page<Package>, ClientError> {
        let base = self.packages_base(owner, org).await?;
        let url = format!("{base}/packages?package_type={package_type}&per_page={PER_PAGE}");
        let all = self.get_all_pages(&url).await?;
        info!(owner, package_type, count = all.len(), "fetched packages");
        Ok(all)
    }

    /// Lists the versions of one package.  The package name is
    /// percent-encoded: container names such as `team/image` contain `/`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_package_versions(
        &self,
        owner: &str,
        org: bool,
        package_type: &str,
        package_name: &str,
    ) -> Result<Page<PackageVersion>, ClientError> {
        let base = self.packages_base(owner, org).await?;
        let name = encode_segment(package_name);
        let url = format!("{base}/packages/{package_type}/{name}/versions?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
