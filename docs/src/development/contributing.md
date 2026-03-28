# Contributing

Contributions are welcome!  Please follow these guidelines.

## Development Setup

### Prerequisites

- Rust 1.85+ (install via [rustup](https://rustup.rs))
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
cargo +1.85 build --workspace

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

- **No unsafe code** — `#![deny(unsafe_op_in_unsafe_fn)]` is workspace-wide
- **No clippy warnings** — every warning is treated as an error in CI
- **Document public APIs** — `#![warn(missing_docs)]` is workspace-wide
- **File size limit** — keep files under ~500 lines for readability; extract sub-modules as needed
- **Test new code** — add unit tests for any new functions; use `SpyGitRunner` and `MemStorage` for engine tests
- **No OpenSSL** — use `rustls` for TLS; see `deny.toml`
- **No reqwest** — use `hyper` + `hyper-rustls` directly

## Adding a New Backup Category

1. Add the new field(s) to `BackupOptions` in `crates/github-backup-types/src/config.rs`
2. Add the CLI flag(s) to `crates/github-backup/src/cli/args.rs`
3. Update `Args::merge_config()` and `Args::into_backup_options()` accordingly
4. Add the corresponding field to `ConfigFile` in `config.rs`
5. Implement the backup function in a new module under `crates/github-backup-core/src/backup/`
6. Wire it in `crates/github-backup-core/src/engine.rs`
7. Add the client method to `crates/github-backup-client/src/client/endpoints.rs` and the `BackupClient` trait
8. Write unit tests using the mock client and `MemStorage`
9. Update `docs/src/backup-categories.md`

## Adding a New API Endpoint

1. Add the method to `BackupClient` in `crates/github-backup-client/src/client/mod.rs`
2. Implement it in `crates/github-backup-client/src/client/endpoints.rs`
3. Add corresponding types in `crates/github-backup-types/src/`
4. Update `MockBackupClient` in `crates/github-backup-core/src/backup/mock_client.rs`

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
