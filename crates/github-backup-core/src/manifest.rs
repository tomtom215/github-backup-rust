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

    #[test]
    fn sha256_hex_known_short_message() {
        // SHA-256 of "abc"
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // ── VerifyReport::is_clean ────────────────────────────────────────────
    //
    // The CI mutation report shows three surviving mutants on
    // `is_clean`: replacing the && operators and the constant return.
    // These tests pin down the boolean composition.

    #[test]
    fn verify_report_default_is_clean() {
        let r = VerifyReport::default();
        assert!(r.is_clean(), "empty report must be clean");
    }

    #[test]
    fn verify_report_with_only_tampered_is_not_clean() {
        let r = VerifyReport {
            ok: 1,
            tampered: vec!["a.json".into()],
            missing: vec![],
            unexpected: vec![],
        };
        assert!(!r.is_clean());
    }

    #[test]
    fn verify_report_with_only_missing_is_not_clean() {
        let r = VerifyReport {
            ok: 1,
            tampered: vec![],
            missing: vec!["b.json".into()],
            unexpected: vec![],
        };
        assert!(!r.is_clean());
    }

    #[test]
    fn verify_report_with_only_unexpected_is_not_clean() {
        let r = VerifyReport {
            ok: 1,
            tampered: vec![],
            missing: vec![],
            unexpected: vec!["c.json".into()],
        };
        assert!(!r.is_clean());
    }

    #[test]
    fn verify_report_with_all_three_categories_is_not_clean() {
        let r = VerifyReport {
            ok: 5,
            tampered: vec!["a".into()],
            missing: vec!["b".into()],
            unexpected: vec!["c".into()],
        };
        assert!(!r.is_clean());
    }

    // ── verify_manifest counts and detection ──────────────────────────────

    #[test]
    fn verify_detects_unexpected_file_added_after_manifest() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        std::fs::write(root.join("a.json"), b"x").expect("write");
        write_manifest(root, &iso_now()).expect("write manifest");

        // Add an unexpected file after the manifest was written.
        std::fs::write(root.join("b.json"), b"y").expect("write");

        let report = verify_manifest(root).expect("verify");
        assert!(!report.is_clean());
        assert_eq!(report.unexpected.len(), 1);
        assert_eq!(report.tampered.len(), 0);
        assert_eq!(report.missing.len(), 0);
        assert_eq!(report.ok, 1);
    }

    #[test]
    fn write_manifest_returns_entry_count() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join("one.json"), b"1").expect("write");
        std::fs::write(root.join("two.json"), b"2").expect("write");
        std::fs::write(root.join("three.json"), b"3").expect("write");

        let count = write_manifest(root, &iso_now()).expect("write manifest");
        assert_eq!(count, 3, "must report exact entry count, not 0/1");
    }

    #[test]
    fn write_manifest_excludes_manifest_file_itself() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join("a.json"), b"x").expect("write");

        let count = write_manifest(root, &iso_now()).expect("write manifest");
        assert_eq!(count, 1, "manifest must not include itself");
        assert!(root.join(MANIFEST_FILENAME).exists());
    }

    #[test]
    fn verify_clean_backup_reports_correct_ok_count() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        for name in &["a.json", "b.json", "c.json", "d.json"] {
            std::fs::write(root.join(name), name.as_bytes()).expect("write");
        }
        write_manifest(root, &iso_now()).expect("write");

        let report = verify_manifest(root).expect("verify");
        assert!(report.is_clean());
        assert_eq!(report.ok, 4, "ok counter must increment per matching file");
    }
}
