// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Directory-to-S3 synchronisation.
//!
//! After a local backup run completes, this module uploads the backup
//! artefacts to S3.  Only files that do not yet exist in the bucket are
//! uploaded (checked via `HeadObject`); this makes re-runs incremental.

use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::client::{S3Client, MULTIPART_THRESHOLD_BYTES};
use crate::config::S3Config;
use crate::encrypt;
use crate::error::S3Error;

/// Statistics from a sync run.
#[derive(Debug, Default, Clone)]
pub struct SyncStats {
    /// Number of files uploaded to S3.
    pub uploaded: usize,
    /// Number of files skipped (already exist in S3).
    pub skipped: usize,
    /// Number of files that failed to upload.
    pub errored: usize,
}

impl std::fmt::Display for SyncStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "uploaded={} skipped={} errored={}",
            self.uploaded, self.skipped, self.errored
        )
    }
}

/// Synchronises the JSON metadata portion of a backup to S3.
///
/// Walks the `backup_root` directory tree recursively, uploading each file
/// to the configured S3 bucket under the configured prefix.  The S3 key for
/// each file is derived from its path relative to `backup_root`.
///
/// Binary release assets can be large; set `include_binary_assets = false`
/// to skip files outside `json/` subdirectories.
///
/// When `encrypt_key` is `Some`, every file is encrypted with AES-256-GCM
/// before upload.  The S3 key gains a `.enc` suffix to distinguish encrypted
/// objects from plaintext ones.  The wire format is
/// `[12-byte nonce][ciphertext + 16-byte tag]` — see [`encrypt`] for details.
///
/// # Errors
///
/// Returns [`S3Error`] on configuration or TLS errors.  Per-file upload
/// failures are logged as warnings and counted in `stats.errored`.
pub async fn sync_to_s3(
    client: &S3Client,
    config: &S3Config,
    backup_root: &Path,
    include_binary_assets: bool,
    encrypt_key: Option<&[u8; 32]>,
) -> Result<SyncStats, S3Error> {
    let mut stats = SyncStats::default();

    let files = walk_files(backup_root);
    if files.is_empty() {
        info!(dir = %backup_root.display(), "no files to sync to S3");
        return Ok(stats);
    }

    info!(
        count = files.len(),
        bucket = %config.bucket,
        "syncing backup to S3"
    );

    for file_path in &files {
        // Optionally skip binary assets (large files in release_assets/).
        if !include_binary_assets && is_binary_asset(file_path) {
            debug!(path = %file_path.display(), "skipping binary asset");
            continue;
        }

        // Compute the S3 key relative to the backup root.
        let relative = match file_path.strip_prefix(backup_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        // Encrypted objects get a `.enc` suffix so they can be distinguished
        // from plaintext objects and so that the plaintext S3 key is never
        // inadvertently treated as already-uploaded when the encryption key
        // changes.
        let keyed_relative = if encrypt_key.is_some() {
            format!("{relative}.enc")
        } else {
            relative
        };
        let s3_key = config.full_key(&keyed_relative);

        match upload_file(client, file_path, &s3_key, encrypt_key).await {
            Ok(UploadOutcome::Uploaded) => {
                stats.uploaded += 1;
            }
            Ok(UploadOutcome::Skipped) => {
                stats.skipped += 1;
            }
            Err(e) => {
                stats.errored += 1;
                warn!(
                    path = %file_path.display(),
                    key = %s3_key,
                    error = %e,
                    "failed to upload file to S3"
                );
            }
        }
    }

    info!(%stats, "S3 sync complete");
    Ok(stats)
}

/// Outcome of a single-file upload attempt.
#[derive(Debug)]
enum UploadOutcome {
    Uploaded,
    Skipped,
}

/// Uploads a single file to S3 if it does not already exist.
///
/// When `encrypt_key` is `Some`, the file bytes are encrypted with
/// AES-256-GCM before upload.  The content type is always
/// `application/octet-stream` for encrypted blobs.
async fn upload_file(
    client: &S3Client,
    local_path: &Path,
    s3_key: &str,
    encrypt_key: Option<&[u8; 32]>,
) -> Result<UploadOutcome, S3Error> {
    // Skip files that already exist in S3.
    if client.object_exists(s3_key).await? {
        debug!(key = %s3_key, "object already exists in S3, skipping");
        return Ok(UploadOutcome::Skipped);
    }

    let plaintext = std::fs::read(local_path)?;

    let (data, content_type) = if let Some(key) = encrypt_key {
        let ciphertext = encrypt::encrypt(key, &plaintext)?;
        (ciphertext, "application/octet-stream")
    } else {
        let ct = guess_content_type(local_path);
        (plaintext, ct)
    };

    debug!(
        path = %local_path.display(),
        key = %s3_key,
        bytes = data.len(),
        encrypted = encrypt_key.is_some(),
        "uploading to S3"
    );

    // Use multipart upload for large objects (original plaintext size).
    let original_size = if encrypt_key.is_some() {
        // The encrypted blob is slightly larger than the plaintext; use the
        // plaintext size to decide whether to use multipart.
        std::fs::metadata(local_path).map(|m| m.len()).unwrap_or(0)
    } else {
        data.len() as u64
    };

    if original_size >= MULTIPART_THRESHOLD_BYTES {
        client.multipart_upload(s3_key, &data, content_type).await?;
    } else {
        client.put_object(s3_key, &data, content_type).await?;
    }

    Ok(UploadOutcome::Uploaded)
}

/// Recursively walks `dir`, returning all regular file paths.
fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    walk_files_inner(dir, &mut result);
    result
}

fn walk_files_inner(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files_inner(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Returns `true` if the file is a binary release asset (not JSON metadata).
fn is_binary_asset(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "release_assets")
        && path.extension().is_none_or(|e| e != "json")
}

/// Guesses the `Content-Type` for a file based on its extension.
fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => "application/json",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn walk_files_finds_nested_files() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub/deep")).unwrap();
        fs::write(dir.path().join("a.json"), b"{}").unwrap();
        fs::write(dir.path().join("sub/b.json"), b"{}").unwrap();
        fs::write(dir.path().join("sub/deep/c.json"), b"{}").unwrap();

        let files = walk_files(dir.path());
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn is_binary_asset_returns_true_for_release_assets() {
        let path = PathBuf::from("/backup/owner/json/repos/my-repo/release_assets/v1.0/app.zip");
        assert!(is_binary_asset(&path));
    }

    #[test]
    fn is_binary_asset_returns_false_for_json() {
        let path = PathBuf::from("/backup/owner/json/repos/my-repo/info.json");
        assert!(!is_binary_asset(&path));
    }

    #[test]
    fn guess_content_type_json() {
        assert_eq!(
            guess_content_type(Path::new("data.json")),
            "application/json"
        );
    }

    #[test]
    fn guess_content_type_binary() {
        assert_eq!(
            guess_content_type(Path::new("archive.tar.gz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn sync_stats_display() {
        let s = SyncStats {
            uploaded: 5,
            skipped: 3,
            errored: 1,
        };
        assert_eq!(s.to_string(), "uploaded=5 skipped=3 errored=1");
    }
}
