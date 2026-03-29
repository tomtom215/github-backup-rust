// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Release metadata and asset download backup.

use std::path::Path;

use tracing::{info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, manifest::sha256_hex, storage::Storage};

/// Backs up all releases for a repository, optionally downloading binary assets.
///
/// Writes:
/// - `meta_dir/releases.json` – all release metadata
/// - `meta_dir/release_assets/<tag>/<filename>` – binary assets (if enabled)
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_releases(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.releases {
        return Ok(());
    }

    info!(owner, repo = repo_name, "fetching releases");
    let releases = client.list_releases(owner, repo_name).await?;
    storage.write_json(&meta_dir.join("releases.json"), &releases)?;

    if !opts.release_assets {
        return Ok(());
    }

    for release in &releases {
        for asset in &release.assets {
            if asset.state != "uploaded" {
                warn!(
                    asset = %asset.name,
                    state = %asset.state,
                    "skipping asset not in 'uploaded' state"
                );
                continue;
            }

            let asset_path = meta_dir
                .join("release_assets")
                .join(&release.tag_name)
                .join(&asset.name);

            // Skip if already downloaded (idempotent re-runs).
            if storage.exists(&asset_path) {
                info!(asset = %asset.name, "asset already downloaded, skipping");
                continue;
            }

            info!(asset = %asset.name, size = asset.size, "downloading release asset");
            let data = client.download_release_asset(&asset.url).await?;

            // Verify the download is non-empty before persisting.
            if data.is_empty() {
                warn!(
                    asset = %asset.name,
                    "downloaded asset is empty; skipping"
                );
                continue;
            }

            storage.write_bytes(&asset_path, &data)?;

            // Write a SHA-256 sidecar so the download can be verified later
            // without re-downloading.
            let digest = sha256_hex(&data);
            let sha_path = asset_path.with_extension(format!(
                "{}.sha256",
                asset_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("bin")
            ));
            storage.write_bytes(&sha_path, format!("{digest}  {}\n", asset.name).as_bytes())?;
            info!(
                asset = %asset.name,
                sha256 = %&digest[..16],
                "asset downloaded and checksum recorded"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::release::{Release, ReleaseAsset};
    use github_backup_types::user::User;
    use std::path::PathBuf;

    fn make_user() -> User {
        User {
            id: 1,
            login: "octocat".to_string(),
            user_type: "User".to_string(),
            avatar_url: String::new(),
            html_url: String::new(),
        }
    }

    fn make_release(tag: &str, assets: Vec<ReleaseAsset>) -> Release {
        Release {
            id: 1,
            tag_name: tag.to_string(),
            name: Some(tag.to_string()),
            body: None,
            draft: false,
            prerelease: false,
            author: make_user(),
            assets,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            published_at: Some("2024-01-01T00:00:00Z".to_string()),
            html_url: format!("https://github.com/octocat/repo/releases/tag/{tag}"),
            tarball_url: None,
            zipball_url: None,
        }
    }

    fn make_asset(name: &str, state: &str) -> ReleaseAsset {
        ReleaseAsset {
            id: 1,
            name: name.to_string(),
            content_type: "application/octet-stream".to_string(),
            state: state.to_string(),
            size: 1024,
            download_count: 0,
            url: "https://api.github.com/repos/octocat/repo/releases/assets/1".to_string(),
            browser_download_url: String::new(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn backup_releases_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // releases = false

        backup_releases(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_releases");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_releases_enabled_writes_releases_json() {
        let release = make_release("v1.0.0", vec![]);
        let client = MockBackupClient::new().with_releases(vec![release]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            releases: true,
            ..Default::default()
        };

        backup_releases(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_releases");

        assert!(storage.get(&PathBuf::from("/meta/releases.json")).is_some());
    }

    #[tokio::test]
    async fn backup_releases_downloads_uploaded_assets() {
        let asset = make_asset("binary.tar.gz", "uploaded");
        let release = make_release("v1.0.0", vec![asset]);
        let client = MockBackupClient::new()
            .with_releases(vec![release])
            .with_asset_bytes(b"asset-data".to_vec());
        let storage = MemStorage::default();
        let opts = BackupOptions {
            releases: true,
            release_assets: true,
            ..Default::default()
        };

        backup_releases(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_releases");

        let asset_path = PathBuf::from("/meta/release_assets/v1.0.0/binary.tar.gz");
        let data = storage.get(&asset_path).expect("asset should be saved");
        assert_eq!(data, b"asset-data");
    }

    #[tokio::test]
    async fn backup_releases_skips_non_uploaded_assets() {
        let pending_asset = make_asset("pending.tar.gz", "open");
        let release = make_release("v1.0.0", vec![pending_asset]);
        let client = MockBackupClient::new().with_releases(vec![release]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            releases: true,
            release_assets: true,
            ..Default::default()
        };

        backup_releases(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_releases");

        // Only releases.json; no asset file for pending asset
        assert_eq!(storage.len(), 1);
    }

    #[tokio::test]
    async fn backup_releases_skips_already_downloaded_asset() {
        let asset = make_asset("binary.tar.gz", "uploaded");
        let release = make_release("v1.0.0", vec![asset]);
        let client = MockBackupClient::new()
            .with_releases(vec![release])
            .with_asset_bytes(b"new-data".to_vec());

        // Pre-populate the storage with the asset to simulate a prior download
        let storage = MemStorage::default();
        let asset_path = PathBuf::from("/meta/release_assets/v1.0.0/binary.tar.gz");
        storage
            .write_bytes(&asset_path, b"old-data")
            .expect("pre-populate");

        let opts = BackupOptions {
            releases: true,
            release_assets: true,
            ..Default::default()
        };

        backup_releases(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_releases");

        // The old data should not be overwritten
        let stored = storage.get(&asset_path).expect("asset");
        assert_eq!(
            stored, b"old-data",
            "existing asset should not be re-downloaded"
        );
    }
}
