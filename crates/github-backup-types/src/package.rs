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
    /// Package owner; `None` when GitHub sends no owner (`null` or absent).
    #[serde(default)]
    pub owner: Option<User>,
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
    ///
    /// Optional in the OpenAPI description (`package_html_url` is the
    /// required one), so `None` when GitHub omits it.
    #[serde(default)]
    pub html_url: Option<String>,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// Metadata about this version (platform, image digest, etc.).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_json() -> serde_json::Value {
        serde_json::json!({
            "id": 197,
            "name": "hello_docker",
            "package_type": "container",
            "visibility": "private",
            "version_count": 1,
            "html_url": "https://github.com/orgs/github/packages/container/package/hello_docker",
            "created_at": "2020-05-19T22:19:11Z",
            "updated_at": "2021-10-05T18:44:39Z",
            "owner": null,
            "repository": null
        })
    }

    #[test]
    fn package_accepts_null_and_absent_owner() {
        // `owner` is optional and nullable in the `package` schema.
        let mut absent = package_json();
        absent.as_object_mut().expect("object").remove("owner");

        let null: Package = serde_json::from_value(package_json()).expect("null owner");
        let missing: Package = serde_json::from_value(absent).expect("absent owner");

        assert!(null.owner.is_none());
        assert!(missing.owner.is_none());
    }

    #[test]
    fn package_version_html_url_is_optional() {
        // The schema requires `package_html_url`; `html_url` may be absent.
        let json = serde_json::json!({
            "id": 836,
            "name": "sha256:b3d3e366b55f9a54599220198b3db5da8f53592acbbb7dc7e4e9878762fc5344",
            "url": "https://api.github.com/users/octocat/packages/container/hello_docker/versions/836",
            "package_html_url": "https://github.com/users/octocat/packages/container/package/hello_docker",
            "created_at": "2020-05-19T22:19:11Z",
            "updated_at": "2021-10-05T18:44:39Z"
        });

        let version: PackageVersion = serde_json::from_value(json).expect("deserialise");

        assert!(version.html_url.is_none());
    }
}
