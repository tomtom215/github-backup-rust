// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Directory-to-S3 synchronisation.
//!
//! After a local backup run completes, this module uploads the backup
//! artefacts to S3.  Only files that do not yet exist in the bucket are
//! uploaded (checked via `HeadObject`); this makes re-runs incremental.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

/// AES-256-GCM nonce (12 bytes) + GCM tag (16 bytes) overhead added to every
/// encrypted file.
const ENCRYPT_OVERHEAD: u64 = 12 + 16;

use crate::client::{S3Client, MULTIPART_THRESHOLD_BYTES};
use crate::config::S3Config;
use crate::encrypt;
use crate::error::S3Error;

/// Maximum number of concurrent S3 upload tasks.
const S3_UPLOAD_CONCURRENCY: usize = 8;
/// Log a progress line every time this many percent of files complete.
const PROGRESS_INTERVAL_PCT: usize = 10;

/// Statistics from a sync run.
#[derive(Debug, Default, Clone)]
pub struct SyncStats {
    /// Number of files uploaded to S3.
    pub uploaded: usize,
    /// Number of files skipped (already exist in S3 with matching size).
    pub skipped: usize,
    /// Number of files that failed to upload.
    pub errored: usize,
    /// Number of stale S3 objects deleted (only when `delete_stale = true`).
    pub deleted: usize,
}

impl std::fmt::Display for SyncStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "uploaded={} skipped={} errored={} deleted={}",
            self.uploaded, self.skipped, self.errored, self.deleted
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
/// Uploads are skipped when an existing S3 object already has the expected
/// `Content-Length`; objects with a mismatched size are re-uploaded.
///
/// When `delete_stale` is `true`, after uploads complete, any S3 objects
/// under the configured prefix that are *not* part of the current local
/// backup are deleted.  This keeps the S3 bucket in sync with the local
/// state and prevents stale objects from accumulating across runs.
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
    delete_stale: bool,
) -> Result<SyncStats, S3Error> {
    let files = walk_files(backup_root);
    if files.is_empty() {
        info!(dir = %backup_root.display(), "no files to sync to S3");
        return Ok(SyncStats::default());
    }

    // Build the list of (local_path, s3_key) pairs, skipping excluded files.
    let candidates: Vec<(PathBuf, String)> = files
        .into_iter()
        .filter_map(|file_path| {
            if !include_binary_assets && is_binary_asset(&file_path) {
                debug!(path = %file_path.display(), "skipping binary asset");
                return None;
            }
            let relative = file_path
                .strip_prefix(backup_root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            // Encrypted objects get a `.enc` suffix so they are never confused
            // with a stale plaintext object from a previous unencrypted run.
            let keyed_relative = if encrypt_key.is_some() {
                format!("{relative}.enc")
            } else {
                relative
            };
            Some((file_path, config.full_key(&keyed_relative)))
        })
        .collect();

    let total = candidates.len();
    info!(
        count = total,
        concurrency = S3_UPLOAD_CONCURRENCY,
        bucket = %config.bucket,
        "syncing backup to S3"
    );

    // Pre-compute the expected S3 key set for stale-deletion (before the
    // `for` loop moves `candidates`).
    let expected_keys: HashSet<String> = if delete_stale {
        candidates.iter().map(|(_, k)| k.clone()).collect()
    } else {
        HashSet::new()
    };

    // Shared state updated by concurrent upload tasks.
    let uploaded = Arc::new(AtomicUsize::new(0));
    let skipped = Arc::new(AtomicUsize::new(0));
    let errored = Arc::new(AtomicUsize::new(0));
    // Tracks the last progress-log threshold crossed (in integer percent buckets).
    let last_logged_pct = Arc::new(AtomicUsize::new(0));

    // Copy the key bytes so each task gets its own owned copy (avoids lifetime
    // issues when spawning independent tasks).
    let key_copy: Option<[u8; 32]> = encrypt_key.copied();

    let sem = Arc::new(Semaphore::new(S3_UPLOAD_CONCURRENCY));
    let mut join_set: JoinSet<()> = JoinSet::new();

    for (file_path, s3_key) in candidates {
        let permit = Arc::clone(&sem)
            .acquire_owned()
            .await
            .expect("semaphore closed");
        let client = client.clone();
        let uploaded = Arc::clone(&uploaded);
        let skipped = Arc::clone(&skipped);
        let errored = Arc::clone(&errored);
        let last_logged_pct = Arc::clone(&last_logged_pct);

        join_set.spawn(async move {
            let _permit = permit;

            match upload_file(&client, &file_path, &s3_key, key_copy.as_ref()).await {
                Ok(UploadOutcome::Uploaded) => {
                    uploaded.fetch_add(1, Ordering::Relaxed);
                }
                Ok(UploadOutcome::Skipped) => {
                    skipped.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    errored.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        path = %file_path.display(),
                        key = %s3_key,
                        error = %e,
                        "failed to upload file to S3"
                    );
                }
            }

            // Emit a progress log line at configurable percentage intervals.
            if total > 0 {
                let done = uploaded.load(Ordering::Relaxed)
                    + skipped.load(Ordering::Relaxed)
                    + errored.load(Ordering::Relaxed);
                let pct = done * 100 / total;
                let bucket = pct / PROGRESS_INTERVAL_PCT;
                let prev = last_logged_pct.fetch_max(bucket, Ordering::Relaxed);
                if bucket > prev {
                    info!(done, total, percent = pct, "S3 sync progress");
                }
            }
        });
    }

    // Drain all tasks.
    while join_set.join_next().await.is_some() {}

    // Optionally delete S3 objects that no longer exist locally.
    let mut deleted = 0usize;
    if delete_stale {
        match client.list_objects(&config.prefix).await {
            Ok(remote_keys) => {
                let stale: Vec<String> = remote_keys
                    .into_iter()
                    .filter(|k| !expected_keys.contains(k))
                    .collect();

                if stale.is_empty() {
                    debug!("no stale S3 objects to delete");
                } else {
                    info!(count = stale.len(), "deleting stale S3 objects");
                    for key in &stale {
                        match client.delete_object(key).await {
                            Ok(()) => {
                                deleted += 1;
                                debug!(key, "deleted stale S3 object");
                            }
                            Err(e) => {
                                warn!(key, error = %e, "failed to delete stale S3 object");
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to list S3 objects for stale-deletion; skipping");
            }
        }
    }

    let stats = SyncStats {
        uploaded: uploaded.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
        errored: errored.load(Ordering::Relaxed),
        deleted,
    };
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
///
/// Takes `encrypt_key` by value (it is `Copy`) so this function can be
/// safely called from spawned tasks without lifetime complications.
async fn upload_file(
    client: &S3Client,
    local_path: &Path,
    s3_key: &str,
    encrypt_key: Option<&[u8; 32]>,
) -> Result<UploadOutcome, S3Error> {
    // Check the S3 object's size.  If it matches the expected upload size the
    // file is already up-to-date and can be skipped.  A size mismatch
    // (e.g. truncated upload from a previous failed run) triggers a re-upload.
    let local_size = std::fs::metadata(local_path)?.len();
    let expected_s3_size = if encrypt_key.is_some() {
        local_size + ENCRYPT_OVERHEAD
    } else {
        local_size
    };

    match client.object_content_length(s3_key).await? {
        Some(s3_size) if s3_size == expected_s3_size => {
            debug!(key = %s3_key, "object already exists in S3 with matching size, skipping");
            return Ok(UploadOutcome::Skipped);
        }
        Some(s3_size) => {
            warn!(
                key = %s3_key,
                local_bytes = expected_s3_size,
                s3_bytes = s3_size,
                "S3 object size mismatch — re-uploading"
            );
        }
        None => {} // Object not found; upload it.
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
            deleted: 2,
        };
        assert_eq!(s.to_string(), "uploaded=5 skipped=3 errored=1 deleted=2");
    }
}
