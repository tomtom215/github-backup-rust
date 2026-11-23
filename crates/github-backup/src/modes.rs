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
