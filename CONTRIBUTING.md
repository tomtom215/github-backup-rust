# Contributing

Thank you for considering a contribution.  This page explains how to get
started and what to expect in review.  (It is also the book page *Development /
Contributing*.)

## Development Setup

### Prerequisites

- Rust 1.88 or newer (`rust-version` in `Cargo.toml`; install via [rustup](https://rustup.rs))
- `git` on the `PATH` (several tests run real `git` against local repositories)
- Optional: `git-lfs`, `cargo-deny` (`cargo install cargo-deny --locked`) and
  `cargo-audit`

### Clone and Build

```bash
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust
cargo build --workspace --locked
cargo test --workspace --locked
```

### Quality Gates

CI runs these; run them before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo +1.88 build --workspace --locked        # MSRV
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo deny check licenses bans advisories sources
cargo audit
```

The documentation book is built in CI with `mdbook` 0.4.40 and
`mdbook-linkcheck` (`mdbook build docs`); a broken internal link fails the build.

## Project Structure

See [Architecture](https://tomtom215.github.io/github-backup-rust/development/architecture.html)
for the crate layout and the engine's failure model.

## Coding Conventions

- **No `unsafe`.**  The workspace's own source has none; `github-backup-core`
  forbids it (`#![forbid(unsafe_code)]`) and four crates deny
  `unsafe_op_in_unsafe_fn`.  New `unsafe` needs explicit justification in review.
- **No clippy warnings**: CI treats them as errors.  Fix them instead of adding
  `#[allow(...)]`.
- **Document public items** (`#![warn(missing_docs)]` is set in the types,
  client, core, mirror and s3 crates; it is not set workspace-wide).
- **Prefer small modules** with one responsibility.
- **Test new code**: unit tests next to the code, or a `*_tests.rs` file; use
  `MockClient`, `MemStorage` and `SpyGitRunner` for engine tests.
- **No OpenSSL, no reqwest**: TLS is `rustls`, HTTP is `hyper`; `deny.toml`
  enforces it.
- **Failures are recorded, not swallowed**: a step that cannot be completed must
  end up in `BackupStats` (the engine's `Steps` do this) so that the run exits
  with status `3` and the report says so.  Do not turn an error into a log line.
- **Never put a secret in an argument list, a URL, a file or a log line.**

## Adding a New Backup Category

1. Add the field to `BackupOptions` (`crates/github-backup-types/src/config/options.rs`),
   and to `BackupOptions::all()` if `--all` should include it.
2. Add the key to `ConfigFile` (`crates/github-backup-types/src/config/file.rs`)
   and to `crates/github-backup/src/config_template.toml`.
3. Add the flag to `crates/github-backup/src/cli/args.rs` (and to the conflict
   list of `--all`).
4. Update `merge_config_inner()` and `into_backup_options()` in
   `crates/github-backup/src/cli/args_impl.rs`.
5. Add the API method to the `BackupClient` trait
   (`crates/github-backup-client/src/api_client/mod.rs`), implement it in
   `crates/github-backup-client/src/api_client/impl_github.rs` and
   `crates/github-backup-client/src/client/endpoints/`, and in the `MockClient`
   used by tests.  Return `Page<T>` (the typed items with their original JSON)
   so that the file written is lossless.
6. Implement the backup function in a new module under
   `crates/github-backup-core/src/backup/`.
7. Run it as a step in `crates/github-backup-core/src/engine/repo.rs` (or
   `engine/mod.rs` for owner-level data).
8. Add unit tests with the mock client and `MemStorage`.
9. Document the flag in `docs/src/backup-categories.md`,
   `docs/src/configuration/cli-reference.md`, `docs/src/configuration/config-file.md`
   and `docs/src/configuration/output-layout.md`, and add it to the changelog.

## Documentation

Every statement in the documentation must be true of the code: check it by
running the binary or reading the code.  Spelling is British English
(organisation, behaviour, artefact), except in identifiers, JSON field names and
GitHub's own terms.  Do not hard-code version numbers in examples (use
`X.Y.Z`); the single source is `Cargo.toml`.

## Commits and Pull Requests

- Use the imperative mood in the subject line ("Add feature", not "Added
  feature") and keep it under 72 characters.
- Reference issues with `Fixes #123` or `Closes #123` in the body when
  applicable.
- Fork, branch from `main`, make focused commits with tests, run the quality gates
  and open a pull request against `main`; CI runs the same checks.

## Reporting Issues

Open an issue at
[github.com/tomtom215/github-backup-rust/issues](https://github.com/tomtom215/github-backup-rust/issues)
with the `github-backup --version` output (say whether it is a release or a
build from `main`), the command you ran (redact tokens), the log output and your
OS.

## Security

Please do **not** open a public GitHub issue to report a security vulnerability.
See [SECURITY.md](https://github.com/tomtom215/github-backup-rust/blob/main/SECURITY.md)
for the responsible disclosure process.

## Code of Conduct

All contributors are expected to follow the project's
[Code of Conduct](https://github.com/tomtom215/github-backup-rust/blob/main/CODE_OF_CONDUCT.md).
