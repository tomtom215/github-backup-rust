// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The non-backup run modes: `--verify`, `--decrypt` and `--doctor`/`--check`.

use std::process::ExitCode;

use tracing::{error, info, warn};

use github_backup_core::verify_manifest;

use crate::cli::Args;
use crate::{doctor, scopes, ui};

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
pub(crate) fn run_decrypt(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    key: &[u8; 32],
) -> ExitCode {
    let ciphertext = match std::fs::read(input_path) {
        Ok(b) => b,
        Err(e) => {
            error!(path = %input_path.display(), "failed to read encrypted file: {e}");
            return ExitCode::FAILURE;
        }
    };
    let plaintext = match github_backup_s3::encrypt::decrypt(key, &ciphertext) {
        Ok(p) => p,
        Err(e) => {
            error!("decryption failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!(path = %parent.display(), "failed to create output directory: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    match std::fs::write(output_path, &plaintext) {
        Ok(()) => {
            info!(
                input = %input_path.display(),
                output = %output_path.display(),
                bytes = plaintext.len(),
                "decryption complete"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!(path = %output_path.display(), "failed to write decrypted output: {e}");
            ExitCode::FAILURE
        }
    }
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
    let api_url = args.api_url.as_deref();
    report.push(doctor::check_connectivity(api_url).await);

    let ansi = ui::use_ansi();
    let label = if args.check { "check" } else { "doctor" };
    println!("github-backup {label} v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("{}", report.render(ansi));
    println!();

    // `--check` echoes the resolved configuration so the user can confirm
    // the categories really are what they expect.
    if args.check {
        println!("Resolved configuration:");
        println!(
            "  owner            {}",
            args.owner.as_deref().unwrap_or("(unset)")
        );
        println!(
            "  output           {}",
            args.output
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(unset)".to_string())
        );
        println!(
            "  api_url          {}",
            args.api_url
                .as_deref()
                .unwrap_or("https://api.github.com (default)")
        );
        println!("  concurrency      {}", args.concurrency.unwrap_or(4));
        println!(
            "  enabled scopes   {}",
            scopes::recommended_scopes(args).join(" ")
        );
        println!();
    }

    let failures = report.failures();
    let warnings = report.warnings();
    if failures > 0 {
        println!(
            "Summary: {failures} blocking issue{} and {warnings} warning{} — backup will not start.",
            if failures == 1 { "" } else { "s" },
            if warnings == 1 { "" } else { "s" },
        );
        ExitCode::FAILURE
    } else if warnings > 0 {
        println!(
            "Summary: ready, but {warnings} warning{} to consider.",
            if warnings == 1 { "" } else { "s" }
        );
        ExitCode::SUCCESS
    } else {
        println!("Summary: ready — every check passed.");
        ExitCode::SUCCESS
    }
}
