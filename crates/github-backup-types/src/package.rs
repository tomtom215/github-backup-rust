// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Packages API types.

use serde::{Deserialize, Serialize};

use crate::user::User;

/// A GitHub Package.
///
/// Packages include container images, npm packages, Maven artifacts, etc.
/// hosted on GitHub Packages (pkg.github.com / ghcr.io).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Numeric package ID.
    pub id: u64,
    /// Package name (e.g. `my-image`, `@owner/my-npm-package`).
    pub name: String,
    /// Package type: `container`, `npm`, `maven`, `rubygems`, `nuget`, `docker`.
    pub package_type: String,
    /// Visibility: `"public"` or `"private"`.
    pub visibility: String,
    /// Number of versions.
    #[serde(default)]
    pub version_count: u64,
    /// HTML URL on GitHub.
    pub html_url: String,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// Package owner.
    pub owner: User,
    /// Associated repository (name), if any.
    #[serde(default)]
    pub repository: Option<PackageRepository>,
}

/// A stub for the repository associated with a package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRepository {
    /// Repository name.
    pub name: String,
    /// Repository full name (owner/name).
    pub full_name: String,
    /// Whether the repository is private.
    pub private: bool,
}

/// A specific version of a GitHub Package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersion {
    /// Numeric version ID.
    pub id: u64,
    /// Version name / tag (e.g. `"v1.0.0"`, `"sha256:abc123"`).
    pub name: String,
    /// HTML URL for this version.
    pub html_url: String,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// Metadata about this version (platform, image digest, etc.).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}
