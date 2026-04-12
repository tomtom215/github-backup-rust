# Contributing

Contributions are welcome!  Please follow these guidelines.

## Development Setup

### Prerequisites

- Rust 1.88+ (install via [rustup](https://rustup.rs))
- `git` on `$PATH`
- Optional: `cargo-deny` for licence checking

```bash
# Install cargo-deny
cargo install cargo-deny
```

### Clone and Build

```bash
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust
cargo build --workspace
cargo test --workspace
```

### Quality Gates

All of these must pass before merging:

```bash
# Formatting
cargo fmt --all -- --check

# Linting (zero warnings)
cargo clippy --workspace --all-targets -- -D warnings

# Tests
cargo test --workspace

# MSRV check
cargo +1.88 build --workspace

# Documentation
cargo doc --workspace --no-deps

# Security audit
cargo audit

# License/ban check
cargo deny check
```

## Project Structure

See [Architecture](architecture.md) for a full description of the crate layout.

## Coding Conventions

- **`unsafe` is restricted** — the workspace denies `unsafe_op_in_unsafe_fn`,
  and the only `unsafe` block in the codebase is a single FFI call to POSIX
  `kill(2)` for stale-lock detection.  New `unsafe` requires explicit
  justification in review.
- **No clippy warnings** — every warning is treated as an error in CI
- **Document public APIs** — `#![warn(missing_docs)]` is workspace-wide
- **Prefer small modules** — split large files into focused sub-modules when
  doing so improves navigability
- **Test new code** — add unit tests for new functions; use `SpyGitRunner`
  and `MemStorage` for engine tests
- **No OpenSSL** — use `rustls` for TLS; enforced by `deny.toml`
- **No reqwest** — use `hyper` + `hyper-rustls` directly

## Adding a New Backup Category

1. Add the new field(s) to `BackupOptions` in
   `crates/github-backup-types/src/config/options.rs`.
2. Add the matching field to `ConfigFile` in
   `crates/github-backup-types/src/config/file.rs`.
3. Add the CLI flag(s) to `crates/github-backup/src/cli/args.rs`.
4. Update `Args::merge_config()` and `Args::into_backup_options()` in
   `crates/github-backup/src/cli/args_impl.rs`.
5. Add the API method to the `BackupClient` trait in
   `crates/github-backup-client/src/api_client/` and implement it under
   `crates/github-backup-client/src/client/endpoints/`.
6. Implement the backup function in a new module under
   `crates/github-backup-core/src/backup/`.
7. Wire it in `crates/github-backup-core/src/engine.rs`.
8. Update `MockBackupClient` in
   `crates/github-backup-core/src/backup/mock_client.rs` for tests.
9. Add unit tests using the mock client and `MemStorage`.
10. Document the new flag in `docs/src/backup-categories.md` and
    `docs/src/configuration/cli-reference.md`.

## Pull Requests

1. Fork the repository and create a feature branch
2. Make your changes with tests
3. Run the full quality-gate suite (see above)
4. Open a PR against `main` with a clear description

## Reporting Issues

Open an issue at [github.com/tomtom215/github-backup-rust/issues](https://github.com/tomtom215/github-backup-rust/issues).

Please include:
- `github-backup --version` output
- The command you ran (redact any tokens)
- The full error message / log output
- OS and Rust version
