# Changelog

All notable changes are documented here.  This project follows [Semantic Versioning](https://semver.org).

---

## [0.3.0] — Unreleased

### Added

- **TOML config file** support (`--config` / `-c` flag). All backup options can now be specified in a `config.toml` file; CLI flags take precedence.
- **Backup summary report** (`--report <FILE>`). Writes a JSON file with counters for every backed-up category after the run.
- **Modular CLI** — `cli.rs` split into `cli/args.rs` + `cli/clone_type.rs` for easier maintenance.
- **Modular git runner** — `git.rs` split into `git/mod.rs` + `git/askpass.rs` + `git/spy.rs`.
- Full **mdBook documentation** at `docs/` covering installation, authentication, all backup categories, S3 storage, mirroring, Docker deployment, and architecture.

### Changed

- `owner` positional argument is now optional when `owner` is specified in the config file.
- `--output` flag is now optional; defaults to `.` if not specified anywhere.

---

## [0.2.0] — 2026-01-15

### Added

- **OAuth device flow** (`--device-auth`, `--oauth-client-id`): authenticate interactively via GitHub OAuth without creating a PAT.
- **Gitea/Codeberg/Forgejo mirror push** (`--mirror-to`, `--mirror-token`, `--mirror-owner`, `--mirror-private`): push all cloned repositories as mirrors to a Gitea-compatible instance after backup.
- **S3-compatible storage** (`--s3-bucket` and related flags): sync JSON metadata and release assets to AWS S3, Backblaze B2, Cloudflare R2, MinIO, DigitalOcean Spaces, or Wasabi.  Uses a pure-Rust SigV4 implementation (no AWS SDK).
- **Incremental S3 sync**: uses `HeadObject` to skip already-uploaded objects.
- **Docker support**: multi-stage Alpine image (~15 MB runtime image); `docker-compose.yml` with profiles for S3/B2/MinIO/Codeberg.
- **Shallow clone** support (`--clone-type shallow:<depth>`).
- **LFS support** (`--lfs`).
- **`BackupStats`** with lock-free `AtomicU64` counters shared across concurrent tasks.

### Changed

- Replaced Python reference implementation with a complete Rust rewrite.
- Async I/O throughout (Tokio runtime).

---

## [0.1.0] — 2025-12-01

### Added

- Initial Rust rewrite of the Python `github-backup` reference implementation.
- **Repositories**: mirror, bare, and full clone modes.
- **Issues**: metadata, comments, events.
- **Pull requests**: metadata, comments, commits, reviews.
- **Releases**: metadata + asset download.
- **Gists**: owned and starred.
- **Wikis**: bare mirror clones.
- **User data**: starred, watched, followers, following.
- **Repository metadata**: labels, milestones, hooks, security advisories.
- **Trait-based design**: `Storage`, `GitRunner`, and `BackupClient` traits with in-memory test stubs (`MemStorage`, `SpyGitRunner`, `MockBackupClient`).
- **RAII credential handling**: `GIT_ASKPASS` scripts are auto-deleted even on panic.
- **Rate limit awareness**: automatic backoff on `X-RateLimit-Remaining: 0`.
- **Retry on 5xx**: up to 3 retries with exponential backoff.
- **Concurrent backup**: semaphore-based, configurable with `--concurrency`.
- **Dry-run mode**: `--dry-run` previews actions without executing them.
- **Shell completions**: bash, zsh, fish, powershell, elvish.
- CI: rustfmt, clippy (`-D warnings`), tests (Ubuntu + macOS), MSRV 1.85, `cargo-audit`, `cargo-deny`.
