# Changelog

All notable changes to `github-backup` are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.3.0] — 2026-03-29

### Added (this session)

- **`--clone-host <HOST>`** (`GITHUB_CLONE_HOST` env var / `clone_host` in config): overrides the
  hostname in every git clone URL returned by the API.  Intended for GitHub Enterprise Server
  deployments where the API endpoint and the git clone endpoint are on separate hosts (e.g.
  behind different load balancers).  Applied consistently to repository clones, wiki clones, and
  gist clones.  Includes unit tests for HTTPS, `git@host:path`, and `ssh://` URL forms.

- **`--concurrency` now truly optional**: changed `Args::concurrency` from `usize` (with
  `default_value = "4"`) to `Option<usize>`.  Previously, a config file value of `concurrency = 8`
  was silently ignored when the user ran `--concurrency 4` explicitly, because the code couldn't
  distinguish "user passed 4" from "still at default".  Now the default is applied at
  `into_backup_options()` time so CLI always wins over the config file, regardless of the value.

- **`BackupStats::add_gists(n)`**: O(1) batch increment replacing the previous `for _ in 0..n`
  loop in the engine.

- **`repos_discovered` in `BackupStats::Display`**: the summary line now shows
  `N/M backed up` (backed-up / discovered) so users can immediately see if any repositories were
  skipped or errored without reading individual log lines.

### Fixed

- **Dead code removed**: `FsStorage::write_bytes_owned` and the unused `use bytes::Bytes` import
  in `storage.rs` have been deleted.  The method was never called outside the module.

- **`run_git` signature simplified**: removed the confusing `in_cwd: bool` parameter.  All callers
  now pass the actual working directory directly (`dest` when updating an existing repo, `.` when
  running a fresh clone command).

### Internal — Refactoring & Tech Debt

- **Module extraction**: all inline metadata backup blocks in `engine.rs`
  (`labels`, `milestones`, `hooks`, `security_advisories`, `topics`, `branches`)
  extracted into dedicated modules under `backup/` — each with unit tests.
  All 18 source files in `backup/` now follow a single consistent pattern.
- **`endpoints/` directory**: `client/endpoints.rs` (649 lines) split into eight
  focused submodules (`actions`, `issues`, `keys`, `org`, `pulls`, `repo_meta`,
  `repos`, `social`).
- **`api_client/` directory**: `api_client.rs` (546 lines) split into `mod.rs`
  (trait definition) + `impl_github.rs` (blanket `impl BackupClient for GitHubClient`).
- **`config/` directory**: `config.rs` (566 lines) split into `credential`,
  `output`, `clone_type`, `options`, and `file` submodules with a separate
  `tests` module.
- **`report.rs`**: `write_report`, `unix_to_iso8601`, and `is_valid_iso8601`
  extracted from `main.rs` into a dedicated module with 13 unit tests; fixed
  wrong Unix timestamp in `known_timestamp_formats_correctly`.
- **Test extraction**: `starred_repos.rs` tests moved to `starred_repos_tests.rs`
  via `#[path]` attribute; source file trimmed from 564 → 318 lines.
- **Broken intra-doc link** fixed in `api_client/mod.rs`.
- All 326+ tests pass; zero clippy warnings; rustdoc builds cleanly.

### Added

- **GitHub Actions workflow backup** (`--actions`, `--action-runs`): new
  `Workflow` and `WorkflowRun` types added to `github-backup-types`.  Two new
  client endpoints (`list_workflows`, `list_workflow_runs`) and a dedicated
  backup module (`backup/actions.rs`) in `github-backup-core`.  The engine
  writes `workflows.json` per repository when `--actions` is set, and optionally
  `workflow_runs_<id>.json` per workflow when `--action-runs` is also set.
  Both endpoints handle 403/404 gracefully (Actions disabled, token scope).
  `BackupStats` now tracks `workflows_fetched` and the JSON report includes the
  counter.  `--action-runs` is intentionally excluded from `--all` due to its
  potentially large output.

- **Deployment environment backup** (`--environments`): new `Environment`,
  `EnvironmentProtectionRule`, and `DeploymentBranchPolicy` types added to
  `github-backup-types`.  New client endpoint (`list_environments`) and backup
  module (`backup/environments.rs`) write `environments.json` per repository.
  404/403 responses (no environments or insufficient permissions) are logged
  and skipped gracefully.

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

- **GitHub Enterprise Server** support via `--api-url <URL>` (or
  `GITHUB_API_URL` environment variable / `api_url` config file key).  Pass
  the GHES API base URL (e.g. `https://github.example.com/api/v3`) and all API
  requests are directed there.  New `GitHubClient::with_api_url()` constructor
  added to `github-backup-client`.
- **Extended backup stats**: `BackupStats` now tracks `issues_fetched` and
  `prs_fetched` across all repositories.  Both counters appear in the log
  output, the `Display` summary, and the JSON report (`--report`).
- **`--since` format validation**: the ISO 8601 value is now validated before
  the backup starts, producing a clear error for malformed timestamps.
- **`dry_run` gap fixed**: `backup_gists` and `backup_user_data` now respect
  `opts.dry_run` and skip all I/O in dry-run mode (previously only
  per-repository operations were skipped).
- **Modular code**: `config.rs` split into `config.rs` + `glob.rs`; `args.rs`
  split into `args.rs` (struct) + `args_impl.rs` (`merge_config` / `into_backup_options`).

### Changed

- `owner` positional argument is now optional; it can be supplied via the
  `owner` key in the config file instead.
- `--output` flag now defaults to `.` when not specified via CLI or config.
- `BackupClient::list_issues` and `BackupClient::list_pull_requests` now
  accept an optional `since: Option<&str>` parameter (used by `--since`).
- `BackupOptions::all()` now also enables `topics` and `branches`.
- `BackupStats::Display` now includes elapsed time, issues fetched, and PRs
  fetched.
- `backup_issues` and `backup_pull_requests` return `u64` (count of items
  fetched) instead of `()`.  The engine uses these to populate `BackupStats`.

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

[0.3.0]: https://github.com/tomtom215/github-backup-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/tomtom215/github-backup-rust/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/github-backup-rust/releases/tag/v0.1.0
