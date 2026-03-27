// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Release metadata and asset download backup.

use std::path::Path;

use tracing::{info, warn};

use github_backup_client::GitHubClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

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
    client: &GitHubClient,
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
            storage.write_bytes(&asset_path, &data)?;
        }
    }

    Ok(())
}
