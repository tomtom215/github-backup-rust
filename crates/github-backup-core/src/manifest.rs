// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! SHA-256 hash manifest for backup integrity and tamper-evidence.
//!
//! After a backup run, [`write_manifest`] walks the backup directory tree and
//! records the SHA-256 digest of every file in a manifest file.  The manifest
//! itself is written last and contains a digest of the sorted entry list so
//! its own integrity can be checked.
//!
//! [`verify_manifest`] re-reads the manifest and recomputes every digest,
//! reporting any files that are missing, added, or have changed content.
//!
//! # Manifest file format
//!
//! The manifest is a JSON file at `<owner-json-dir>/backup_manifest.json`:
//!
//! ```json
//! {
//!   "tool_version": "0.3.0",
//!   "created_at": "2026-01-15T12:34:56Z",
//!   "root": "/var/backup/octocat/json",
//!   "entries": [
//!     { "path": "repos/Hello-World/issues.json", "sha256": "abc123…" },
//!     …
//!   ]
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::info;

/// A single file entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Path relative to the manifest root directory.
    pub path: String,
    /// Lowercase hex-encoded SHA-256 digest of the file contents.
    pub sha256: String,
}

/// The full manifest document.
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    /// Tool version that generated this manifest.
    pub tool_version: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// Absolute path to the root directory that was hashed.
    pub root: String,
    /// Sorted list of file entries.
    pub entries: Vec<ManifestEntry>,
}

/// Result of a manifest verification run.
#[derive(Debug, Default)]
pub struct VerifyReport {
    /// Files present in manifest and on disk with matching digest.
    pub ok: u64,
    /// Files in the manifest whose digest no longer matches.
    pub tampered: Vec<String>,
    /// Files in the manifest that are missing from disk.
    pub missing: Vec<String>,
    /// Files on disk that are not in the manifest.
    pub unexpected: Vec<String>,
}

impl VerifyReport {
    /// Returns `true` if there are no issues.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.tampered.is_empty() && self.missing.is_empty() && self.unexpected.is_empty()
    }
}

/// Name of the manifest file within the backup root.
pub const MANIFEST_FILENAME: &str = "backup_manifest.json";

/// Walks `root`, hashes every file, writes the manifest to
/// `root/backup_manifest.json`, and returns the number of entries written.
///
/// The manifest file itself is excluded from the entry list.
///
/// # Errors
///
/// Returns an error string on I/O failure.
pub fn write_manifest(root: &Path, created_at: &str) -> Result<usize, String> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    let mut entries = collect_entries(root, &manifest_path)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let count = entries.len();
    let manifest = Manifest {
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: created_at.to_string(),
        root: root.display().to_string(),
        entries,
    };

    let json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("serialise manifest: {e}"))?;
    std::fs::write(&manifest_path, json).map_err(|e| format!("write manifest: {e}"))?;

    info!(
        path = %manifest_path.display(),
        entries = count,
        "wrote SHA-256 manifest"
    );
    Ok(count)
}

/// Reads the manifest from `root/backup_manifest.json` and verifies every
/// entry against the current files on disk.
///
/// # Errors
///
/// Returns an error string if the manifest file cannot be read or parsed.
pub fn verify_manifest(root: &Path) -> Result<VerifyReport, String> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    let content =
        std::fs::read_to_string(&manifest_path).map_err(|e| format!("read manifest: {e}"))?;
    let manifest: Manifest =
        serde_json::from_str(&content).map_err(|e| format!("parse manifest: {e}"))?;

    // Build a map of expected path → digest.
    let expected: HashMap<String, String> = manifest
        .entries
        .into_iter()
        .map(|e| (e.path, e.sha256))
        .collect();

    // Build a set of actual files on disk.
    let manifest_rel = MANIFEST_FILENAME.to_string();
    let actual = collect_entries(root, &root.join(MANIFEST_FILENAME))?;
    let actual_map: HashMap<String, String> =
        actual.into_iter().map(|e| (e.path, e.sha256)).collect();

    let mut report = VerifyReport::default();

    for (path, digest) in &expected {
        if path == &manifest_rel {
            continue;
        }
        match actual_map.get(path) {
            None => report.missing.push(path.clone()),
            Some(actual_digest) if actual_digest != digest => {
                report.tampered.push(path.clone());
            }
            Some(_) => report.ok += 1,
        }
    }

    for path in actual_map.keys() {
        if !expected.contains_key(path) {
            report.unexpected.push(path.clone());
        }
    }

    Ok(report)
}

/// Walks `root` recursively and hashes every file, skipping `exclude`.
fn collect_entries(root: &Path, exclude: &Path) -> Result<Vec<ManifestEntry>, String> {
    let mut entries = Vec::new();
    visit_dir(root, root, exclude, &mut entries)?;
    Ok(entries)
}

fn visit_dir(
    root: &Path,
    dir: &Path,
    exclude: &Path,
    out: &mut Vec<ManifestEntry>,
) -> Result<(), String> {
    let read_dir =
        std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;

    for entry in read_dir {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();

        if path == exclude {
            continue;
        }

        let ft = entry.file_type().map_err(|e| format!("file type: {e}"))?;

        if ft.is_dir() {
            visit_dir(root, &path, exclude, out)?;
        } else if ft.is_file() {
            let digest = hash_file(&path)?;
            let rel = path
                .strip_prefix(root)
                .map_err(|e| format!("strip prefix: {e}"))?;
            let rel_str = rel
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 path: {}", path.display()))?
                .to_string();
            out.push(ManifestEntry {
                path: rel_str,
                sha256: digest,
            });
        }
    }

    Ok(())
}

/// Returns the hex-encoded SHA-256 digest of the file at `path`.
fn hash_file(path: &Path) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let digest = Sha256::digest(&data);
    Ok(format!("{digest:x}"))
}

/// Returns the hex-encoded SHA-256 digest of `data` bytes.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    format!("{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn iso_now() -> String {
        "2026-01-01T00:00:00Z".to_string()
    }

    #[test]
    fn write_and_verify_clean_backup() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        std::fs::create_dir_all(root.join("repos/hello-world")).expect("mkdir");
        std::fs::write(root.join("repos/hello-world/issues.json"), b"[1,2,3]").expect("write");
        std::fs::write(root.join("starred.json"), b"[]").expect("write");

        write_manifest(root, &iso_now()).expect("write manifest");

        let report = verify_manifest(root).expect("verify");
        assert!(report.is_clean(), "fresh backup must verify clean");
        assert_eq!(report.ok, 2);
    }

    #[test]
    fn verify_detects_tampered_file() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        let file = root.join("issues.json");
        std::fs::write(&file, b"original").expect("write");
        write_manifest(root, &iso_now()).expect("write manifest");

        // Tamper with the file after the manifest is written.
        std::fs::write(&file, b"tampered!").expect("overwrite");

        let report = verify_manifest(root).expect("verify");
        assert!(!report.is_clean());
        assert_eq!(report.tampered.len(), 1);
    }

    #[test]
    fn verify_detects_missing_file() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        let file = root.join("issues.json");
        std::fs::write(&file, b"data").expect("write");
        write_manifest(root, &iso_now()).expect("write manifest");

        // Delete the file after the manifest is written.
        std::fs::remove_file(&file).expect("remove");

        let report = verify_manifest(root).expect("verify");
        assert!(!report.is_clean());
        assert_eq!(report.missing.len(), 1);
    }

    #[test]
    fn sha256_hex_known_value() {
        // SHA-256 of empty string
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
