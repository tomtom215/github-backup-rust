// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Release metadata and asset download backup.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tracing::{info, warn};

use github_backup_client::{AssetSink, BackupClient, ClientError};
use github_backup_types::config::BackupOptions;
use github_backup_types::ReleaseAsset;

use crate::{
    error::CoreError,
    storage::{PendingFile, Storage},
};

/// Backs up all releases for a repository, optionally downloading binary assets.
///
/// Writes:
/// - `meta_dir/releases.json` – all release metadata, as the API returned it
/// - `meta_dir/release_assets/<tag>/<filename>` – binary assets (if enabled)
/// - `meta_dir/release_assets/<tag>/<filename>.sha256` – checksum sidecar
///
/// Assets are streamed to a temporary file next to their destination and
/// renamed into place once complete, so an interrupted run never leaves a
/// truncated asset.  The download is checked against the size the API reports
/// and, when the API provides one, its `sha256:<hex>` digest.
///
/// An asset that is already on disk is skipped only when it is complete: its
/// size equals the API's `size`, and its content matches the API digest (or,
/// without one, the checksum in its `.sha256` sidecar).  Anything else is
/// downloaded again.
///
/// One asset that cannot be fetched does not stop the others: every asset is
/// attempted and the first failure is returned at the end.
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

    let mut first_error: Option<CoreError> = None;
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

            // Both come from the API: neither may leave the assets directory.
            let asset_path = meta_dir
                .join("release_assets")
                .join(crate::paths::nested(&release.tag_name))
                .join(crate::paths::file_name(&asset.name));

            if let Err(e) = backup_asset(client, asset, &asset_path, storage).await {
                warn!(
                    asset = %asset.name,
                    tag = %release.tag_name,
                    error = %e,
                    "release asset could not be backed up"
                );
                first_error.get_or_insert(e);
            }
        }
    }

    first_error.map_or(Ok(()), Err)
}

/// Downloads one asset unless a complete, verified copy is already on disk.
async fn backup_asset(
    client: &impl BackupClient,
    asset: &ReleaseAsset,
    asset_path: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    let api_digest = asset.digest.as_deref().and_then(parse_sha256_digest);
    let sidecar_path = sidecar_path(asset_path);

    if let Some(existing) = storage.file_size(asset_path) {
        if existing == asset.size {
            let expected = match &api_digest {
                Some(digest) => Some(digest.clone()),
                None => read_sidecar_digest(storage, &sidecar_path),
            };
            match expected {
                // Size matches and there is nothing to compare the content to.
                None => {
                    info!(asset = %asset.name, "asset already downloaded, skipping");
                    return Ok(());
                }
                Some(expected) => match sha256_of_stored(storage, asset_path)? {
                    Some(actual) if actual == expected => {
                        info!(asset = %asset.name, "asset already downloaded and verified, skipping");
                        return Ok(());
                    }
                    _ => warn!(
                        asset = %asset.name,
                        "stored asset does not match its checksum; downloading it again"
                    ),
                },
            }
        } else {
            warn!(
                asset = %asset.name,
                stored_bytes = existing,
                expected_bytes = asset.size,
                "stored asset has the wrong size (truncated or replaced); downloading it again"
            );
        }
    }

    info!(asset = %asset.name, size = asset.size, "downloading release asset");
    let mut sink = HashingSink {
        file: storage.begin_write(asset_path)?,
        hasher: Sha256::new(),
    };
    let received = client.download_release_asset(&asset.url, &mut sink).await?;
    let digest = format!("{:x}", sink.hasher.clone().finalize());

    if received != asset.size {
        return Err(integrity_error(
            asset_path,
            format!(
                "received {received} bytes but the API reports {} (download incomplete)",
                asset.size
            ),
        ));
    }
    if let Some(expected) = &api_digest {
        if *expected != digest {
            return Err(integrity_error(
                asset_path,
                format!("sha256 is {digest} but GitHub reports {expected}"),
            ));
        }
    }

    // Moves the finished file into place atomically; on any error above the
    // temporary file is discarded when `sink` is dropped.
    sink.file.finish()?;

    // The sidecar lets the download be verified later without GitHub.
    storage.write_bytes(
        &sidecar_path,
        format!("{digest}  {}\n", asset.name).as_bytes(),
    )?;
    info!(
        asset = %asset.name,
        sha256 = %&digest[..16],
        "asset downloaded and checksum recorded"
    );
    Ok(())
}

/// Receives the download chunk by chunk: writes it to the pending file and
/// hashes it on the way.
struct HashingSink {
    file: Box<dyn PendingFile>,
    hasher: Sha256,
}

impl AssetSink for HashingSink {
    fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        self.file.write_chunk(chunk)?;
        self.hasher.update(chunk);
        Ok(())
    }
}

/// An integrity failure, reported like any other failed download.
fn integrity_error(path: &Path, what: String) -> CoreError {
    CoreError::Client(ClientError::ApiError {
        status: 0,
        body: format!(
            "release asset {} failed verification: {what}",
            path.display()
        ),
    })
}

/// `<asset>.sha256`: the checksum sidecar of `asset_path`.
fn sidecar_path(asset_path: &Path) -> PathBuf {
    let mut name = asset_path.file_name().unwrap_or_default().to_os_string();
    name.push(".sha256");
    asset_path.with_file_name(name)
}

/// Extracts the hex digest from a GitHub `sha256:<hex>` digest string
/// (lowercased); `None` for other algorithms or malformed values.
fn parse_sha256_digest(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    is_sha256_hex(hex).then(|| hex.to_ascii_lowercase())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reads the digest recorded in a `.sha256` sidecar (`<hex>  <name>`), if the
/// file exists and is well formed.
fn read_sidecar_digest(storage: &impl Storage, path: &Path) -> Option<String> {
    let mut reader = storage.open_read(path).ok().flatten()?;
    let mut text = String::new();
    reader.by_ref().take(4096).read_to_string(&mut text).ok()?;
    let first = text.split_whitespace().next()?;
    is_sha256_hex(first).then(|| first.to_ascii_lowercase())
}

/// SHA-256 (lowercase hex) of the stored file, read in chunks; `None` if the
/// storage cannot read it back.
fn sha256_of_stored(storage: &impl Storage, path: &Path) -> Result<Option<String>, CoreError> {
    let Some(mut reader) = storage.open_read(path)? else {
        return Ok(None);
    };
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| CoreError::io(path.display(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
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
            digest: None,
            download_count: 0,
            url: "https://api.github.com/repos/octocat/repo/releases/assets/1".to_string(),
            browser_download_url: String::new(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn sha256_hex(data: &[u8]) -> String {
        format!("{:x}", Sha256::digest(data))
    }

    fn opts() -> BackupOptions {
        BackupOptions {
            releases: true,
            release_assets: true,
            ..Default::default()
        }
    }

    const META: &str = "/meta";
    const ASSET: &str = "/meta/release_assets/v1.0.0/binary.tar.gz";
    const SIDECAR: &str = "/meta/release_assets/v1.0.0/binary.tar.gz.sha256";

    async fn run(client: &MockBackupClient, storage: &MemStorage) -> Result<(), CoreError> {
        backup_releases(
            client,
            "octocat",
            "Hello-World",
            &opts(),
            &PathBuf::from(META),
            storage,
        )
        .await
    }

    /// A release with one 10-byte asset, served by a mock holding `bytes`.
    fn one_asset(size: u64, digest: Option<String>, bytes: &[u8]) -> MockBackupClient {
        let mut asset = make_asset("binary.tar.gz", "uploaded");
        asset.size = size;
        asset.digest = digest;
        MockBackupClient::new()
            .with_releases(vec![make_release("v1.0.0", vec![asset])])
            .with_asset_bytes(bytes.to_vec())
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
    async fn backup_releases_downloads_uploaded_assets_with_a_sidecar() {
        let client = one_asset(10, None, b"asset-data");
        let storage = MemStorage::default();

        run(&client, &storage).await.expect("backup_releases");

        let data = storage
            .get(&PathBuf::from(ASSET))
            .expect("asset should be saved");
        assert_eq!(data, b"asset-data");
        let sidecar = storage.get(&PathBuf::from(SIDECAR)).expect("sidecar");
        assert_eq!(
            String::from_utf8(sidecar).expect("utf-8"),
            format!("{}  binary.tar.gz\n", sha256_hex(b"asset-data"))
        );
    }

    #[tokio::test]
    async fn sidecar_sits_next_to_extensionless_assets_too() {
        let mut asset = make_asset("tool", "uploaded");
        asset.size = 3;
        let client = MockBackupClient::new()
            .with_releases(vec![make_release("v1", vec![asset])])
            .with_asset_bytes(b"abc".to_vec());
        let storage = MemStorage::default();

        run(&client, &storage).await.expect("backup");

        assert!(storage
            .get(&PathBuf::from("/meta/release_assets/v1/tool"))
            .is_some());
        assert!(storage
            .get(&PathBuf::from("/meta/release_assets/v1/tool.sha256"))
            .is_some());
    }

    #[tokio::test]
    async fn backup_releases_skips_non_uploaded_assets() {
        let pending_asset = make_asset("pending.tar.gz", "open");
        let release = make_release("v1.0.0", vec![pending_asset]);
        let client = MockBackupClient::new().with_releases(vec![release]);
        let storage = MemStorage::default();

        run(&client, &storage).await.expect("backup_releases");

        // Only releases.json; no asset file for pending asset
        assert_eq!(storage.len(), 1);
    }

    #[tokio::test]
    async fn complete_existing_asset_is_not_downloaded_again() {
        let client = one_asset(8, None, b"new-data");
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from(ASSET), b"old-data")
            .expect("pre-populate");

        run(&client, &storage).await.expect("backup_releases");

        assert_eq!(
            storage.get(&PathBuf::from(ASSET)).expect("asset"),
            b"old-data",
            "an existing asset of the right size must not be re-downloaded"
        );
    }

    #[tokio::test]
    async fn truncated_existing_asset_is_downloaded_again() {
        // The API says 10 bytes; only 5 are on disk (a crash or a full disk).
        let client = one_asset(10, None, b"asset-data");
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from(ASSET), b"asset")
            .expect("pre-populate");

        run(&client, &storage).await.expect("backup_releases");

        assert_eq!(
            storage.get(&PathBuf::from(ASSET)).expect("asset"),
            b"asset-data",
            "a truncated asset must be replaced, not trusted because it exists"
        );
    }

    #[tokio::test]
    async fn same_size_but_wrong_content_is_caught_by_the_api_digest() {
        let digest = format!("sha256:{}", sha256_hex(b"good-data"));
        let client = one_asset(9, Some(digest), b"good-data");
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from(ASSET), b"evil-data") // 9 bytes, corrupt
            .expect("pre-populate");

        run(&client, &storage).await.expect("backup_releases");

        assert_eq!(
            storage.get(&PathBuf::from(ASSET)).expect("asset"),
            b"good-data"
        );
    }

    #[tokio::test]
    async fn matching_api_digest_means_skip() {
        let digest = format!("sha256:{}", sha256_hex(b"good-data"));
        let client = one_asset(9, Some(digest), b"WOULD-REPLACE");
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from(ASSET), b"good-data")
            .expect("pre-populate");

        run(&client, &storage).await.expect("backup_releases");

        assert_eq!(
            storage.get(&PathBuf::from(ASSET)).expect("asset"),
            b"good-data"
        );
    }

    #[tokio::test]
    async fn the_sidecar_is_honoured_when_the_api_has_no_digest() {
        let client = one_asset(9, None, b"good-data");
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from(ASSET), b"evil-data")
            .expect("asset");
        storage
            .write_bytes(
                &PathBuf::from(SIDECAR),
                format!("{}  binary.tar.gz\n", sha256_hex(b"good-data")).as_bytes(),
            )
            .expect("sidecar");

        run(&client, &storage).await.expect("backup_releases");

        assert_eq!(
            storage.get(&PathBuf::from(ASSET)).expect("asset"),
            b"good-data",
            "the content no longer matches its sidecar, so it is fetched again"
        );
    }

    #[tokio::test]
    async fn a_download_with_the_wrong_digest_is_rejected_and_leaves_nothing() {
        let digest = format!("sha256:{}", sha256_hex(b"expected!"));
        let client = one_asset(9, Some(digest), b"tampered!");
        let storage = MemStorage::default();

        let err = run(&client, &storage)
            .await
            .expect_err("must fail verification");

        assert!(err.to_string().contains("failed verification"), "{err}");
        assert!(
            storage.get(&PathBuf::from(ASSET)).is_none(),
            "no file for a bad download"
        );
        assert!(storage.get(&PathBuf::from(SIDECAR)).is_none());
    }

    #[tokio::test]
    async fn a_short_download_is_rejected_and_leaves_nothing() {
        // The API announces 10 bytes, the server delivers 4.
        let client = one_asset(10, None, b"abcd");
        let storage = MemStorage::default();

        let err = run(&client, &storage).await.expect_err("incomplete");

        assert!(err.to_string().contains("incomplete"), "{err}");
        assert!(storage.get(&PathBuf::from(ASSET)).is_none());
    }

    #[tokio::test]
    async fn one_bad_asset_does_not_stop_the_others() {
        let mut bad = make_asset("bad.bin", "uploaded");
        bad.size = 99; // the mock delivers 10 bytes
        let mut good = make_asset("good.bin", "uploaded");
        good.size = 10;
        let client = MockBackupClient::new()
            .with_releases(vec![make_release("v1", vec![bad, good])])
            .with_asset_bytes(b"asset-data".to_vec());
        let storage = MemStorage::default();

        let result = run(&client, &storage).await;

        assert!(result.is_err(), "the failure is still reported");
        assert!(storage
            .get(&PathBuf::from("/meta/release_assets/v1/bad.bin"))
            .is_none());
        assert_eq!(
            storage
                .get(&PathBuf::from("/meta/release_assets/v1/good.bin"))
                .expect("good"),
            b"asset-data"
        );
    }

    #[test]
    fn digest_parsing_accepts_only_sha256_hex() {
        let hex = "ab".repeat(32);
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{hex}")),
            Some(hex.clone())
        );
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{}", hex.to_uppercase())),
            Some(hex.clone())
        );
        assert_eq!(parse_sha256_digest(&format!("sha1:{hex}")), None);
        assert_eq!(parse_sha256_digest("sha256:abc"), None);
        assert_eq!(parse_sha256_digest(&hex), None);
    }

    /// A hostile or buggy API (GitHub Enterprise Server, a proxy) must not be
    /// able to place files outside the assets directory through a tag name or
    /// an asset name.
    #[tokio::test]
    async fn tag_and_asset_names_cannot_escape_the_assets_directory() {
        let mut asset = make_asset("../../../../outside/pwned.txt", "uploaded");
        asset.size = 10;
        let client = MockBackupClient::new()
            .with_releases(vec![make_release("../../tagdir", vec![asset])])
            .with_asset_bytes(b"asset-data".to_vec());
        let storage = MemStorage::default();

        run(&client, &storage).await.expect("backup_releases");

        let root = PathBuf::from("/meta/release_assets");
        let written: Vec<PathBuf> = storage
            .written_paths()
            .into_iter()
            .filter(|p| p.to_string_lossy().contains("pwned"))
            .collect();
        assert!(
            !written.is_empty(),
            "the asset is still backed up, safely named"
        );
        for path in written {
            assert!(path.starts_with(&root), "{path:?} escaped {root:?}");
            assert!(
                !path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
                "{path:?}"
            );
        }
    }
}
