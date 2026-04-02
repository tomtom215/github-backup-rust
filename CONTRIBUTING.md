# Contributing to github-backup

Thank you for considering a contribution. This document explains how to get
started and what to expect during code review.

## Prerequisites

- Rust stable toolchain (see `rust-version` in `Cargo.toml`)
- `cargo fmt`, `cargo clippy`, `cargo test` must all pass before opening a PR

## Workflow

1. Fork the repository and create a branch from `main`.
2. Make your changes with clear, focused commits.
3. Run the full check suite locally:
   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
4. Open a pull request against `main`. The CI pipeline runs the same checks
   automatically.

## Code style

- Use `cargo fmt` with the default settings (rustfmt.toml is not checked in).
- Clippy warnings are treated as errors in CI — fix them rather than
  suppressing them with `#[allow(...)]`.
- All public items should have rustdoc comments.
- Tests belong in the same file as the code they test (unit tests) or in a
  dedicated `*_tests.rs` file (integration tests).

## Commits

- Use the imperative mood in the subject line: "Add feature" not "Added feature".
- Keep the subject line under 72 characters.
- Reference issues with `Fixes #123` or `Closes #123` in the commit body when
  applicable.

## Security

Please do **not** open a public GitHub issue to report a security
vulnerability. See [SECURITY.md](SECURITY.md) for the responsible disclosure
process.

## Code of Conduct

All contributors are expected to follow the project's
[Code of Conduct](CODE_OF_CONDUCT.md).
