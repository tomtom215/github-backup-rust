# Changelog

All notable changes to `github-backup` are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

_No unreleased changes._

---

## [0.3.1] — 2026-03-29

### Added

- **`--restore` mode (labels, milestones, and issues)**: the `--restore` flag
  reads every repository's `labels.json`, `milestones.json`, and `issues.json`
  from the backup and re-creates them in the target organisation via the
  GitHub REST API.  Pull requests embedded in `issues.json` are skipped.
  Existing resources (HTTP 422) are silently skipped.  Requires
  `--restore-target-org` and a token with repository write access; an
  interactive confirmation banner is printed unless `--restore-yes` is
  supplied.

- **AES-256-GCM at-rest encryption for S3** (`--encrypt-key`): provide a
  32-byte hex key (64 hex chars) and every file is encrypted with AES-256-GCM
  before upload.  The wire format is
  `[12-byte random nonce][ciphertext + 16-byte tag]`.  Encrypted objects
  receive a `.enc` suffix in S3.  The key may also be supplied via the
  `BACKUP_ENCRYPT_KEY` environment variable, and a `--decrypt` mode reverses
  the process locally.

- **`post_process` module**: mirror push, S3 sync, Prometheus metrics, diff,
  and retention logic live in a dedicated `post_process.rs` module in the
  main binary.

- **Write endpoints in `GitHubClient`**: `create_label()`,
  `create_milestone()`, and `create_issue()` use a shared `post_json` helper
  with the same rate-limit and 5xx retry behaviour as the GET path.

- **Interactive TUI** (`--tui`): full-screen terminal interface built with
  [Ratatui](https://ratatui.rs) 0.30.  Five screens — Dashboard, Configure,
  Run, Verify, Results — cover the end-to-end workflow without leaving the
  terminal.  A custom `tracing_subscriber::Layer` routes log lines to the
  Run screen's log panel; a `tokio::sync::oneshot` cancellation channel
  aborts a running backup on `Ctrl+C`.  The TUI crate ships unit tests that
  exercise the full state machine without a real terminal.

- **Config file now covers S3 and mirror settings**: `s3_bucket`,
  `s3_region`, `s3_prefix`, `s3_endpoint`, `s3_access_key`, `s3_secret_key`,
  `s3_include_assets`, `mirror_to`, `mirror_token`, `mirror_owner`, and
  `mirror_private` are valid TOML keys.  All values can be overridden by CLI
  flags.

- **Config file now covers clone behaviour**: `prefer_ssh`, `clone_type`,
  `lfs`, `no_prune`, and `report` are valid TOML keys.

### Changed

- **MSRV raised from 1.85 to 1.88**: `ratatui@0.30` and its transitive
  dependencies require Rust 1.88.  The workspace `rust-version` in
  `Cargo.toml` has been updated accordingly.

- **`deny.toml` allows the `Zlib` licence**: `foldhash@0.2` (a transitive
  dependency of `ratatui-core`) is Zlib-licensed.

- **`s3_region` / `s3_prefix` are now `Option<String>`** in `Args`,
  consistent with `concurrency`.  The defaults (`us-east-1` and `""`) are
  applied at `build_s3_config` time so a config file can supply the values
  when the CLI flags are absent.

### Fixed

- **`org` merge bug**: `merge_config` now applies `cfg.org` when the CLI
  `--org` flag was not passed.  Previously the config-file value was
  silently ignored.

### Internal

- **`repository.rs` split**: inline test module extracted to
  `repository_tests.rs` via the `#[path]` attribute, separating production
  code from its tests.

---

## [0.3.0] — 2026-03-29

### Added

- **`--clone-host <HOST>`** (`GITHUB_CLONE_HOST` env / `clone_host` config
  key): overrides the hostname in every git clone URL returned by the API.
  Intended for GitHub Enterprise Server deployments where the API endpoint
  and the git clone endpoint are on separate hosts.  Applied to repository,
  wiki, and gist clones.

- **`--concurrency` is now truly optional**: `Args::concurrency` is
  `Option<usize>`, so a config-file value such as `concurrency = 8` is no
  longer overridden by the implicit CLI default.

- **`BackupStats::add_gists(n)`**: batch increment replacing the previous
  per-item loop in the engine.

- **`repos_discovered` in `BackupStats::Display`**: the summary line now
  shows `N/M backed up` (backed-up / discovered) so operators can see at a
  glance whether any repositories were skipped or errored.

### Fixed

- **Dead code removed**: `FsStorage::write_bytes_owned` and an unused
  `use bytes::Bytes` import in `storage.rs`.

- **`run_git` signature simplified**: removed the `in_cwd: bool` parameter.
  Callers now pass the working directory directly.

### Internal

- **Module extraction**: inline metadata backup blocks in `engine.rs`
  (`labels`, `milestones`, `hooks`, `security_advisories`, `topics`,
  `branches`) split into dedicated modules under `backup/`.
- **`endpoints/` directory**: `client/endpoints.rs` split into eight focused
  submodules (`actions`, `issues`, `keys`, `org`, `pulls`, `repo_meta`,
  `repos`, `social`).
- **`api_client/` directory**: trait definition (`mod.rs`) split from the
  blanket `impl BackupClient for GitHubClient` (`impl_github.rs`).
- **`config/` directory**: `config.rs` split into `credential`, `output`,
  `clone_type`, `options`, and `file` submodules.
- **`report.rs`**: report-writing helpers extracted from `main.rs` into a
  dedicated module with unit tests.
- **Broken intra-doc link** fixed in `api_client/mod.rs`.

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
- **GitHub Pages deployment** (`pages.yml`): CI workflow that builds the
  mdBook and deploys it to the `github-pages` environment on every push to
  `main`.
- **Full mdBook documentation** in `docs/` covering installation, quick
  start, authentication, all backup categories, storage backends,
  configuration, deployment, monitoring, security, troubleshooting, and the
  workspace architecture.

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
