# Changelog

All notable changes are documented here.  This project follows [Semantic Versioning](https://semver.org).

---

## [0.3.0] — Unreleased

### Added

- **Unauthenticated access**: running without `--token` or `--device-auth` is now valid. The tool backs up public data at GitHub's unauthenticated rate limit (60 req/h) and logs a clear warning rather than refusing to start.
- **Automatic HTTPS proxy support**: `github-backup` now detects `HTTPS_PROXY` (or `https_proxy`) at startup and routes all GitHub API calls through the proxy via HTTP `CONNECT` tunnelling. Credentials embedded in the proxy URL are forwarded automatically. Powered by `hyper-http-proxy` (pure Rust, no OpenSSL).
- **Starred-repository clone** (`--clone-starred`): mirrors every starred repo into `<output>/<owner>/git/starred/<upstream-owner>/<repo>.git` using a crash-safe durable queue (`starred_clone_queue.json`). Features: pause/resume across runs, per-item retry with exponential backoff (5 s → 30 s → 2 min), Ctrl+C graceful shutdown, and structured progress logging (`done`, `pending`, `failed`, `rate_per_min`, `eta_secs`). Not included in `--all` due to potentially large footprint.
- **Deploy keys backup** (`--deploy-keys`): saves `deploy_keys.json` per repository. Requires admin access; gracefully skips on 403/404.
- **Collaborators backup** (`--collaborators`): saves `collaborators.json` per repository with per-user permissions. Requires admin access; gracefully skips on 403/404.
- **Organisation members backup** (`--org-members`): saves `org_members.json` for organisation targets. Silently skipped for user targets.
- **Organisation teams backup** (`--org-teams`): saves `org_teams.json` for organisation targets, including nested parent–child team relationships.
- **`DeployKey` type** in `github-backup-types` with full serde round-trip and unit tests.
- **`Collaborator` / `CollaboratorPermissions` types** in `github-backup-types` with flat layout to avoid `#[serde(flatten)]` + rename conflicts.
- **`Team` / `TeamParent` types** in `github-backup-types` with nested parent support and unit tests.
- **TOML config file** support (`--config` / `-c` flag). All backup options can now be specified in a `config.toml` file; CLI flags take precedence.
- **Backup summary report** (`--report <FILE>`). Writes a JSON file with counters for every backed-up category after the run.
- **Modular CLI** — `cli.rs` split into `cli/args.rs` + `cli/clone_type.rs` for easier maintenance.
- **Modular git runner** — `git.rs` split into `git/mod.rs` + `git/askpass.rs` + `git/spy.rs`.
- Full **mdBook documentation** at `docs/` covering installation, authentication, all backup categories, S3 storage, mirroring, Docker deployment, architecture, restore guide, and GitHub Enterprise Server guide.
- **proptest** property-based round-trip tests now cover all `BackupOptions` fields including the four new ones.

### Changed

- `owner` positional argument is now optional when `owner` is specified in the config file.
- `--output` flag is now optional; defaults to `.` if not specified anywhere.
- `BackupOptions::all()` now enables `deploy_keys`, `collaborators`, `org_members`, and `org_teams`.

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
