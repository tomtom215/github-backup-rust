// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Release and release-asset types.

use serde::{Deserialize, Serialize};

use crate::user::User;

/// A GitHub release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Numeric release identifier.
    pub id: u64,
    /// Git tag this release is associated with.
    pub tag_name: String,
    /// Release title.
    pub name: Option<String>,
    /// Release notes (Markdown), or `None` if empty.
    pub body: Option<String>,
    /// Whether this is a draft release (not publicly visible).
    pub draft: bool,
    /// Whether this is a pre-release.
    pub prerelease: bool,
    /// User who created the release.
    pub author: User,
    /// Binary assets attached to this release.
    pub assets: Vec<ReleaseAsset>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 publication timestamp, or `None` for drafts.
    pub published_at: Option<String>,
    /// URL of the release on GitHub.
    pub html_url: String,
    /// URL of the source-code tarball.
    pub tarball_url: Option<String>,
    /// URL of the source-code zip.
    pub zipball_url: Option<String>,
}

/// A binary file attached to a [`Release`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    /// Numeric asset identifier.
    pub id: u64,
    /// File name of the asset.
    pub name: String,
    /// MIME content type.
    pub content_type: String,
    /// Asset state: `"uploaded"` or `"open"`.
    pub state: String,
    /// File size in bytes.
    pub size: u64,
    /// Download count.
    pub download_count: u64,
    /// API URL to download the asset (requires `Accept: application/octet-stream`).
    pub url: String,
    /// Direct browser download URL.
    pub browser_download_url: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_deserialise_with_assets_succeeds() {
        let json = serde_json::json!({
            "id": 1,
            "tag_name": "v1.0.0",
            "name": "v1.0.0",
            "body": "Release notes",
            "draft": false,
            "prerelease": false,
            "author": {
                "id": 1,
                "login": "octocat",
                "type": "User",
                "avatar_url": "https://github.com/images/error/octocat_happy.gif",
                "html_url": "https://github.com/octocat"
            },
            "assets": [{
                "id": 2,
                "name": "app-linux.tar.gz",
                "content_type": "application/gzip",
                "state": "uploaded",
                "size": 4096,
                "download_count": 10,
                "url": "https://api.github.com/repos/octocat/Hello-World/releases/assets/2",
                "browser_download_url": "https://github.com/octocat/Hello-World/releases/download/v1.0.0/app-linux.tar.gz",
                "created_at": "2013-02-27T19:35:32Z",
                "updated_at": "2013-02-27T19:35:32Z"
            }],
            "created_at": "2013-02-27T19:35:32Z",
            "published_at": "2013-02-27T19:35:32Z",
            "html_url": "https://github.com/octocat/Hello-World/releases/tag/v1.0.0",
            "tarball_url": "https://api.github.com/repos/octocat/Hello-World/tarball/v1.0.0",
            "zipball_url": "https://api.github.com/repos/octocat/Hello-World/zipball/v1.0.0"
        });

        let release: Release = serde_json::from_value(json).expect("deserialise");
        assert_eq!(release.tag_name, "v1.0.0");
        assert!(!release.draft);
        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].name, "app-linux.tar.gz");
    }
}
