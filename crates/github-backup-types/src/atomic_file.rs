// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Replacing a file so that a reader, a killed process or a power cut sees
//! either the old content or the new content, never a mixture or nothing.
//!
//! Every file the tool writes (JSON metadata, the state, checkpoint, history
//! and starred-queue files) goes through [`write`]:
//!
//! 1. the data is written to a temporary sibling, `.<name>.tmp-<pid>-<n>`,
//!    unique per process and per write;
//! 2. the temporary file is flushed to the disk (`fsync`);
//! 3. it is renamed over the destination, which is atomic.
//!
//! Step 2 is what makes a power cut safe: without it, a filesystem may make
//! the rename durable before the data and leave an empty file under the real
//! name after a crash; merged lists (`issues.json`, `repos.json`, ...) hold
//! items GitHub no longer has, so an empty file would lose them for good.  The
//! directory is not synced: if the rename itself is lost, the previous
//! complete file is still there, which every reader handles.  The cost
//! measured on an ext4 virtual disk was about 5 ms per file, small next to
//! the API request (or several) that produced each file.
//!
//! [`is_temp_name`] recognises the temporary names (also those of older
//! versions) so that leftovers of a killed run can be swept and are never
//! mistaken for backup data.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes the temporary files of one process.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// File names that older versions used for temporary files, next to the
/// state, checkpoint, history and starred-queue files.
const LEGACY_TEMP_NAMES: &[&str] = &[
    "backup_state.json.tmp",
    "backup_checkpoint.tmp",
    "backup_history.tmp",
    "starred_clone_queue.json.tmp",
];

/// A fresh, unique temporary path next to `path` with the given `kind`
/// (`tmp` for [`write`], `part` for a file written piece by piece):
/// `.<name>.<kind>-<pid>-<n>`.
#[must_use]
pub fn temp_path(path: &Path, kind: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{kind}-{}-{n}", std::process::id()))
}

/// `true` if `name` is a temporary file name made by [`temp_path`] (or by an
/// older version of the tool): never backup data, and safe to delete when no
/// run is in progress.
#[must_use]
pub fn is_temp_name(name: &str) -> bool {
    if LEGACY_TEMP_NAMES.contains(&name) {
        return true;
    }
    if !name.starts_with('.') {
        return false;
    }
    ["tmp", "part"].iter().any(|kind| {
        name.rsplit_once(&format!(".{kind}-"))
            .is_some_and(|(base, tail)| base.len() > 1 && is_pid_suffix(tail, *kind == "tmp"))
    })
}

/// `<digits>-<digits>`, or (when `legacy_ok`, for the `.tmp-<pid>` names of
/// older versions) just `<digits>`.
fn is_pid_suffix(tail: &str, legacy_ok: bool) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match tail.split_once('-') {
        Some((pid, n)) => digits(pid) && digits(n),
        None => legacy_ok && digits(tail),
    }
}

/// Replaces the file at `path` with `data` atomically and durably (see the
/// [module documentation](self)), creating parent directories as needed.
///
/// The temporary file is removed again if anything fails.
///
/// # Errors
///
/// Returns the I/O error of the step that failed.
pub fn write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(path, "tmp");
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_replaces_the_file_and_leaves_nothing_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sub").join("a.json");
        write(&path, b"one").expect("first");
        write(&path, b"two").expect("second");
        assert_eq!(std::fs::read(&path).expect("read"), b"two");
        let names: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["a.json"]);
    }

    /// Regression: temporary names were `.x.tmp-<pid>`, so two writes of the
    /// same file in one process (two tasks) shared one temporary file.
    #[test]
    fn temporary_paths_are_unique_within_a_process() {
        let path = Path::new("/backup/o/json/repos.json");
        let a = temp_path(path, "tmp");
        let b = temp_path(path, "tmp");
        assert_ne!(a, b);
        let name = a.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(name.starts_with(".repos.json.tmp-"), "{name}");
        assert!(is_temp_name(name), "{name}");
    }

    #[test]
    fn temporary_names_are_recognised_and_data_names_are_not() {
        for temp in [
            ".issues.json.tmp-123-0",
            ".issues.json.tmp-123",
            ".app.tar.gz.part-77-3",
            "backup_state.json.tmp",
            "backup_checkpoint.tmp",
            "starred_clone_queue.json.tmp",
        ] {
            assert!(is_temp_name(temp), "{temp}");
        }
        for data in [
            "issues.json",
            ".hidden.json",
            "notes.tmp",
            "app.part-1-2",
            ".x.tmp-abc",
            ".x.part-12",
            ".tmp-1-2",
            "report.tmp-1-2",
        ] {
            assert!(!is_temp_name(data), "{data}");
        }
    }
}
