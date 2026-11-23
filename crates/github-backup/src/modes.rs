// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The non-backup run modes: `--verify`, `--decrypt` and `--doctor`/`--check`.

use std::process::ExitCode;

use tracing::{error, info, warn};

use github_backup_core::verify_manifest;

use crate::cli::Args;
use crate::{account, doctor, scopes, ui};

/// Runs the verify-only mode: checks the SHA-256 manifest in `json_dir`.
pub(crate) fn run_verify(json_dir: &std::path::Path) -> ExitCode {
    info!(dir = %json_dir.display(), "verifying backup integrity");
    match verify_manifest(json_dir) {
        Err(e) => {
            error!("manifest verification failed: {e}");
            ExitCode::FAILURE
        }
        Ok(report) => {
            if report.is_clean() {
                info!(
                    ok = report.ok,
                    "backup integrity verified — all files match"
                );
                ExitCode::SUCCESS
            } else {
                if !report.tampered.is_empty() {
                    error!(files = ?report.tampered, "TAMPERED: digest mismatch");
                }
                if !report.missing.is_empty() {
                    error!(files = ?report.missing, "MISSING: files in manifest but not on disk");
                }
                if !report.unexpected.is_empty() {
                    warn!(files = ?report.unexpected, "UNEXPECTED: files on disk not in manifest");
                }
                ExitCode::FAILURE
            }
        }
    }
}

/// Decrypts `input_path` with `key` and writes plaintext to `output_path`.
///
/// A version-1 object is decrypted a segment at a time, so memory stays
/// bounded whatever its size.  The plaintext goes to a temporary file next to
/// `output_path`, renamed into place only once the whole object has
/// authenticated: a truncated or tampered object leaves no output file.
pub(crate) fn run_decrypt(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    key: &[u8; 32],
) -> ExitCode {
    match decrypt_file(input_path, output_path, key) {
        Ok(bytes) => {
            info!(
                input = %input_path.display(),
                output = %output_path.display(),
                bytes,
                "decryption complete"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The work of [`run_decrypt`]; returns the plaintext size.
fn decrypt_file(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    key: &[u8; 32],
) -> Result<u64, String> {
    use std::io::{BufReader, BufWriter, Write as _};

    let input = std::fs::File::open(input_path).map_err(|e| {
        format!(
            "failed to read encrypted file {}: {e}",
            input_path.display()
        )
    })?;
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create output directory {}: {e}",
                    parent.display()
                )
            })?;
        }
    }
    let tmp = github_backup_types::atomic_file::temp_path(output_path, "part");
    let result = (|| {
        let mut out = BufWriter::new(
            std::fs::File::create_new(&tmp)
                .map_err(|e| format!("failed to write decrypted output {}: {e}", tmp.display()))?,
        );
        let bytes = github_backup_s3::encrypt::decrypt_to(key, BufReader::new(input), &mut out)
            .map_err(|e| format!("decryption failed: {e}"))?;
        let write_failed =
            |e: std::io::Error| format!("failed to write decrypted output {}: {e}", tmp.display());
        out.flush().map_err(write_failed)?;
        let file = out.into_inner().map_err(|e| write_failed(e.into_error()))?;
        file.sync_all().map_err(write_failed)?;
        drop(file);
        std::fs::rename(&tmp, output_path).map_err(|e| {
            format!(
                "failed to write decrypted output {}: {e}",
                output_path.display()
            )
        })?;
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Runs the `--doctor` / `--check` pre-flight diagnostic and exits.
///
/// Returns `ExitCode::SUCCESS` when every blocking check passed (warnings
/// are non-blocking), `ExitCode::FAILURE` otherwise.  The whole report is
/// printed to stdout so users can pipe it into a bug report.
pub(crate) async fn run_doctor(args: &Args) -> ExitCode {
    let mut report = doctor::Report::default();
    report.push(doctor::check_git_binary());
    report.push(doctor::check_output_dir(args.output.as_deref()));
    report.push(doctor::check_credential(args));
    report.push(doctor::check_categories(args));
    if let Some(check) = doctor::check_destinations(args) {
        report.push(check);
    }
    let api_checks = doctor::check_api(args).await;
    let api_ok = api_checks.iter().all(|c| c.status == doctor::Status::Pass);
    for check in api_checks {
        report.push(check);
    }
    if api_ok {
        let credential = args
            .token
            .clone()
            .map_or(github_backup_types::config::Credential::Anonymous, |t| {
                github_backup_types::config::Credential::Token(t)
            });
        if let Ok(api) = account::Api::new(&credential, args.api_url.as_deref()) {
            report.push(doctor::check_owner(args, &api).await);
        }
    }

    let ansi = ui::use_ansi_on(std::io::stdout());
    let label = if args.check { "check" } else { "doctor" };
    let mut out = format!(
        "github-backup {label} v{}\n\n{}\n\n",
        env!("CARGO_PKG_VERSION"),
        report.render(ansi)
    );

    // `--check` echoes the resolved configuration so the user can confirm
    // the categories really are what they expect.
    if args.check {
        use std::fmt::Write as _;
        let _ = writeln!(out, "Resolved configuration:");
        let _ = writeln!(
            out,
            "  owner            {}",
            args.owner.as_deref().unwrap_or("(the token's account)")
        );
        let _ = writeln!(
            out,
            "  output           {}",
            args.output
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ". (current directory)".to_string())
        );
        let _ = writeln!(
            out,
            "  api_url          {}",
            args.api_url
                .as_deref()
                .unwrap_or("https://api.github.com (default)")
        );
        let _ = writeln!(
            out,
            "  concurrency      {}",
            args.concurrency.unwrap_or(4).max(1)
        );
        let _ = writeln!(
            out,
            "  token scopes     {}",
            scopes::recommended_scopes(args).join(" ")
        );
        let _ = writeln!(out);
    }

    let failures = report.failures();
    let warnings = report.warnings();
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let (summary, code) = if failures > 0 {
        (
            format!(
                "Summary: {failures} blocking issue{} and {warnings} warning{} — backup will not start.",
                plural(failures),
                plural(warnings),
            ),
            ExitCode::FAILURE,
        )
    } else if warnings > 0 {
        (
            format!(
                "Summary: ready, but {warnings} warning{} to consider.",
                plural(warnings)
            ),
            ExitCode::SUCCESS,
        )
    } else {
        (
            "Summary: ready — every check passed.".to_string(),
            ExitCode::SUCCESS,
        )
    };
    out.push_str(&summary);
    out.push('\n');
    ui::out(&out);
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    /// The names in `dir`, sorted.
    fn names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_multi_segment_object_is_decrypted_to_the_output_file() {
        let dir = tempfile::tempdir().unwrap();
        let plaintext: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let input = dir.path().join("asset.bin.enc");
        std::fs::write(
            &input,
            github_backup_s3::encrypt::encrypt(&KEY, &plaintext).unwrap(),
        )
        .unwrap();
        let output = dir.path().join("out").join("asset.bin");
        assert_eq!(
            decrypt_file(&input, &output, &KEY).unwrap(),
            plaintext.len() as u64
        );
        assert_eq!(std::fs::read(&output).unwrap(), plaintext);
        assert_eq!(names(&dir.path().join("out")), ["asset.bin"]);
    }

    /// Truncation and tampering are found only at the end of the object,
    /// after earlier segments authenticated: no output file may appear, an
    /// existing one is left alone, and no temporary file is left behind.
    #[test]
    fn a_truncated_or_tampered_object_leaves_no_output_file() {
        let plaintext = vec![3u8; 200_000];
        let blob = github_backup_s3::encrypt::encrypt(&KEY, &plaintext).unwrap();
        let mut tampered = blob.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        for (what, bad) in [
            ("truncated", blob[..blob.len() - 1000].to_vec()),
            ("tampered", tampered),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("in.enc");
            std::fs::write(&input, &bad).unwrap();
            let fresh = dir.path().join("fresh.bin");
            let err = decrypt_file(&input, &fresh, &KEY).unwrap_err();
            assert!(err.contains("decryption failed"), "{what}: {err}");
            assert!(!fresh.exists(), "{what}");
            let existing = dir.path().join("existing.bin");
            std::fs::write(&existing, b"keep me").unwrap();
            assert!(decrypt_file(&input, &existing, &KEY).is_err());
            assert_eq!(std::fs::read(&existing).unwrap(), b"keep me", "{what}");
            assert_eq!(names(dir.path()), ["existing.bin", "in.enc"], "{what}");
        }
    }
}
