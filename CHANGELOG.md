# Changelog

All notable changes to `github-backup` are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased] — 0.3.0

### Added

- **TOML config file** (`--config` / `-c`): supply any backup option through a
  `config.toml` file; CLI flags always take precedence.  The new `ConfigFile`
  type in `github-backup-types` is parsed with the `toml` crate and merged into
  `Args` before the backup starts.
- **Backup summary report** (`--report <FILE>`): write a machine-readable JSON
  summary of the run to an arbitrary path after the backup completes.  The
  report now includes `tool_version`, `started_at` (ISO 8601), `duration_secs`,
  per-category counters, and a `success` boolean — useful for monitoring and
  alerting integrations.
- **Modular CLI**: `cli.rs` (724 lines) refactored into:
  - `cli/args.rs` — `Args` struct, `merge_config()`, `into_backup_options()`
  - `cli/clone_type.rs` — `CliCloneType` parser
  - `cli/mod.rs` — re-exports
- **Modular git runner**: `git.rs` (600 lines) refactored into:
  - `git/mod.rs` — `CloneOptions`, `GitRunner` trait, `ProcessGitRunner`
  - `git/askpass.rs` — `AskpassScript` RAII guard
  - `git/spy.rs` — `SpyGitRunner` test stub + tests
- **Repository name filters** (`--include-repos` / `--exclude-repos`): back
  up only a subset of repositories using glob patterns (`*` / `?`), matching
  is case-insensitive.  Patterns can be comma-separated or the flag can be
  repeated.  `--exclude-repos` takes precedence over `--include-repos`.
- **`--since <DATETIME>`**: limit issue and pull-request API calls to items
  updated at or after an ISO 8601 timestamp.  Enables efficient incremental
  backups — re-use `started_at` from the previous run's JSON report.
- **Topics backup** (`--topics`): write `topics.json` (repository tags) per
  repository.  Already had a `GitHubClient` endpoint; now wired end-to-end
  through the `BackupClient` trait and the engine.
- **Branch list backup** (`--branches`): write `branches.json` per repository
  containing all branch names, tip SHA-1s, and protection status.  New
  `Branch` / `BranchCommit` types added to `github-backup-types`.
- **`BackupStats::elapsed_secs()`**: wall-clock duration tracking using
  `std::time::Instant`; displayed in the `Display` output and included in the
  JSON report.
- **GitHub Pages deployment** (`pages.yml`): new CI workflow builds the
  mdBook and deploys it to `github-pages` environment on every push to `main`.
- **Full mdBook documentation** in `docs/`:
  - Installation, Quick Start, Authentication
  - Backup Categories, Issues & PRs, Releases, Gists & Wikis, User Data
  - Local Storage, S3 Storage, Mirroring
  - CLI Reference, Config File, Environment Variables, Output Layout
  - Docker, Systemd Timer, Cron
  - **Monitoring & Reporting** (new): JSON report schema, Prometheus/Grafana
    integration, Loki alerting, incremental backup patterns
  - **Security** (new): token scopes, credential handling, TLS policy,
    dependency policy, vulnerability reporting
  - **Troubleshooting** (new): auth errors, rate limits, git failures, S3
    issues, debug logging
  - Architecture, Contributing, Changelog, FAQ

### Changed

- `owner` positional argument is now optional; it can be supplied via the
  `owner` key in the config file instead.
- `--output` flag now defaults to `.` when not specified via CLI or config.
- `BackupClient::list_issues` and `BackupClient::list_pull_requests` now
  accept an optional `since: Option<&str>` parameter (used by `--since`).
- `BackupOptions::all()` now also enables `topics` and `branches`.
- `BackupStats::Display` now includes the elapsed time in seconds.

---

## [0.2.0] — 2026-01-15

### Added

- **OAuth device flow**: `--device-auth` + `--oauth-client-id` enable
  interactive authentication via GitHub's device authorisation flow without
  creating a long-lived PAT.
- **Gitea/Codeberg/Forgejo mirror push**: after the primary backup, push every
  cloned repository as a mirror to a Gitea-compatible instance using
  `--mirror-to`, `--mirror-token`, `--mirror-owner`, and `--mirror-private`.
- **S3-compatible storage sync**: `--s3-bucket` (plus region, prefix, endpoint,
  access-key, secret-key flags) syncs JSON metadata — and optionally binary
  release assets — to any S3-compatible object store.  Uses a pure-Rust SigV4
  implementation; no AWS SDK or OpenSSL required.
- **Incremental S3 sync**: `HeadObject` checks before each `PutObject` so
  already-uploaded objects are skipped on subsequent runs.
- **Shallow clone** support via `--clone-type shallow:<depth>`.
- **Git LFS** support via `--lfs`.
- **Docker**: multi-stage Alpine Dockerfile and `docker-compose.yml` with
  service profiles for S3/B2/MinIO/Codeberg.
- **`BackupStats`**: lock-free `AtomicU64` counters shared across concurrent
  repository backup tasks.
- `ARCHITECTURE.md` and `DOCKER.md` documentation.

### Changed

- `BackupEngine` is now generic over `Storage` and `GitRunner` for compile-time
  dispatch and zero-overhead testability.

---

## [0.1.0] — 2025-12-01

### Added

- Complete Rust rewrite of the Python `github-backup` reference implementation.
- **Repositories**: `mirror`, `bare`, and `full` clone modes.
- **Issues**: metadata, comments, timeline events.
- **Pull requests**: metadata, review comments, commit lists, reviews.
- **Releases**: metadata + optional binary asset download.
- **Gists**: owned and starred.
- **Wikis**: bare mirror clones.
- **User data**: starred repos, watched repos, followers, following.
- **Repository metadata**: labels, milestones, webhooks, security advisories.
- **Trait-based design**: `Storage`, `GitRunner`, and `BackupClient` traits with
  full in-memory test stubs (`MemStorage`, `SpyGitRunner`, `MockBackupClient`).
- **RAII credential cleanup**: `GIT_ASKPASS` temp scripts are deleted even on
  panic, ensuring no tokens are left on disk.
- **Rate-limit awareness**: automatic backoff on `X-RateLimit-Remaining: 0`.
- **Retry on 5xx**: up to 3 retries with exponential backoff.
- **Concurrent backup**: semaphore-based, configurable with `--concurrency`.
- **Dry-run mode**: `--dry-run` previews what would be backed up.
- **Shell completions**: bash, zsh, fish, PowerShell, elvish.
- **145 unit tests** covering all modules.
- **`proptest`** round-trip tests for all serialised types.
- CI: rustfmt, clippy (`-D warnings`), tests (Ubuntu + macOS), MSRV 1.85,
  `cargo-audit`, `cargo-deny`.
- Dependency policy in `deny.toml`: no OpenSSL, no reqwest, no native-tls.

[Unreleased]: https://github.com/tomtom215/github-backup-rust/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/tomtom215/github-backup-rust/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/github-backup-rust/releases/tag/v0.1.0
