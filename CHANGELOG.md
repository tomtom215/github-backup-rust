# Changelog

All notable changes to `github-backup` are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added (Unraid)

- **Community Applications template** at `unraid/github-backup.xml`
  authored against the **Unraid v7.2.x DockerMan schema** and the
  current (2026) CA submission guidelines.  Surfaces every important
  option as a WebUI form field: `GITHUB_TOKEN` (masked), output
  volume, `GITHUB_OWNER`, run-mode dropdown (`--all`, `--doctor`,
  `--check`, `--list-scopes`, `--verify`, `--tui`,
  `--print-config-template`), free-form `BACKUP_FLAGS`, GHES URL
  overrides, OAuth client ID, AES-256-GCM encryption key (also
  masked), webhook URL, `RUST_LOG`, and `HTTPS_PROXY`.  Advanced
  fields hidden behind the "Advanced View" toggle.
- **`docker/entrypoint.sh` wrapper** keeps the existing CLI / Compose
  / Kubernetes invocation contract (positional argv passed through
  verbatim) while *also* reconstructing argv from env vars when none
  are supplied — the workflow Unraid CA uses.  Whitelists
  `BACKUP_MODE`, rejects shell metacharacters in `BACKUP_FLAGS`, and
  refuses unknown mode tokens with a clear error.
- **`unraid/ca_profile.xml`** developer profile picked up by CA so
  the "by tomtom215" link on the template lands on the project repo.
- **`unraid/README.md`** walks through installation, first-run
  diagnostic, scheduled-backup pattern via the User Scripts plugin,
  restore, verify, local testing, and the CA submission flow.
- **`unraid/icon.png`** placeholder 256×256 PNG (regeneratable via
  `unraid/make_icon.py`, stdlib-only).
- **Dockerfile updated** to install the new entrypoint wrapper and
  use it (`ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]`).
  Existing `docker run … github-backup OWNER --all` invocations are
  unaffected.

### Added (Docker / Compose)

- **`.dockerignore`** prunes `target/`, `.git/`, `.env`, editor /
  IDE clutter, and local backups from the build context.  Previously
  every `docker build` uploaded the entire workspace (often several
  GiB) to the daemon, making the build slow by minutes.
- **OCI image labels** baked into the Dockerfile
  (`org.opencontainers.image.source`, `…title`, `…description`,
  `…licenses`, `…vendor`, `…base.name`).  Registries, Dependabot, and
  Renovate use these to link back to the source repository.
- **Tini as PID 1** in the runtime image so `docker stop` /
  Kubernetes pod eviction delivers SIGTERM cleanly to the backup
  process — which writes its checkpoint, releases its lock, and
  exits with the conventional 143 code instead of being SIGKILLed.
- **`VOLUME ["/backup"]`** declaration so `docker inspect` shows
  exactly where output lands; the existing non-root `backup` user
  (now pinned to UID/GID 1000 for predictable bind-mount semantics)
  already owns that directory.
- **`--profile doctor`** Compose service runs the pre-flight check
  inside the image: confirms git, network, TLS, and the token before
  the first scheduled cron run.
- **`--profile tui`** Compose service with `tty: true` +
  `stdin_open: true` so ratatui renders correctly under Compose.
- **`--profile verify`** Compose service for SHA-256 manifest checks.
- **`--profile gitlab`** Compose service for GitLab.com / self-hosted
  GitLab mirror push (matching the existing `codeberg` profile).
- **`init: true`** on every backup service as a belt-and-braces
  injection of Docker's bundled tini in case the image's own tini is
  bypassed on older Docker versions.

### Changed (Docker / Compose)

- **`config.toml` is no longer auto-mounted** by the default
  Compose service.  Previously the bind mount `./config.toml` failed
  on a fresh checkout because the file did not exist.  Mount it
  explicitly when you need it:
  `docker compose run --rm -v $PWD/config.toml:/etc/github-backup/config.toml:ro backup --config /etc/github-backup/config.toml`
- **Every Compose service propagates the full env-var set** —
  `GITHUB_API_URL`, `GITHUB_CLONE_HOST`, `GITHUB_OAUTH_CLIENT_ID`,
  `BACKUP_NOTIFY_WEBHOOK`, `BACKUP_ENCRYPT_KEY`,
  `GITHUB_BACKUP_RESTORE_YES`.  All optional; unset values are
  forwarded as empty strings which clap ignores.
- **`compose.example.env` is fully annotated** and documents every
  variable, including the new restore-confirmation env var, the
  optional GHES variables, and the AES-256-GCM encryption key.
- **`DOCKER.md` rewritten** with a profile matrix, a pre-flight
  workflow, a Kubernetes CronJob example, and `--doctor` /
  `--check` / `--list-scopes` troubleshooting recipes.

### Changed (binary)

- **`NO_COLOR` now follows the no-color.org spec literally**: the
  variable must be **set and non-empty** to disable ANSI.  An empty
  value (a common pattern for declaring "pass through" variables in
  Dockerfiles and systemd unit files) no longer accidentally
  suppresses colour.  Affects both `init_tracing` and the
  banner-rendering helpers.

### Added

- **`--doctor` self-diagnostic** runs every prerequisite check (git
  binary + version, output writability, credential type, network
  reachability of the configured API host, system TLS roots) and
  prints a colour-coded pass/fail report. Exit status is `0` when
  every blocking check passes, `1` otherwise. The fastest way to
  confirm a fresh install will succeed before scheduling cron / CI.
- **`--check` configuration-validation mode** is a superset of
  `--doctor` that additionally echoes the resolved configuration
  (owner, output, api_url, concurrency, computed OAuth scope set).
  Performs no backup work and writes no files.
- **`--list-scopes`** prints exactly which OAuth scopes the current
  flag set needs, formatted as a copy-paste-able list for
  https://github.com/settings/tokens/new. Replaces the "guess and
  iterate" workflow new users typically follow.
- **Friendly quickstart** is now printed when `github-backup` is run
  with no arguments at all, in place of the old one-line error.
  Walks the user through token creation, env export, and a working
  command line. Detects ANSI / NO_COLOR.
- **Pre-run plan banner** previews owner, output, concurrency,
  enabled categories, and — when a backup-history file exists — an
  ETA based on the last successful run. Skipped under `--quiet` so
  cron / journald output stays machine-friendly.
- **End-of-run summary banner** with colour/icon-coded pass/warn/fail
  status, repo/issue/PR counters, and a formatted elapsed time. When
  zero repositories were processed, an inline hint suggests checking
  the OWNER spelling, scope, and category flags.
- **Inline examples** in `--help` via clap's `after_help`: complete,
  copy-paste-able invocations for the six most common scenarios
  (full user backup, TUI, doctor, scopes, check, Codeberg mirror).
- **`--print-config-template`** (from the previous unreleased entry)
  remains. The template is unit-tested to parse and to keep flagging
  every REQUIRED field.

### Changed

- **GitHub token format is now validated** at the doctor / check
  level: classic PAT (`ghp_`), fine-grained PAT (`github_pat_`),
  OAuth (`gho_`), and server-to-server (`ghu_` / `ghs_` / `ghr_`)
  are recognised explicitly. Unknown-prefix tokens emit a warning
  but are not rejected (custom GHES installations sometimes use
  bespoke prefixes).
- **`git` binary detection runs at doctor-time** with a
  platform-specific install hint (`brew install git` on macOS,
  package-manager string on Linux, `winget install Git.Git` on
  Windows). Saves new users from a several-minutes-in stall.
- **Error messages now include an actionable hint** when the raw
  error text matches a well-known failure pattern: 401 (token
  rejected), 403 (missing scope, with a pointer to `--list-scopes`),
  404 (wrong target), rate-limit exhaustion, git-missing, network
  unreachable, TLS, disk-full. Hints are emitted as a second `error!`
  line tagged `hint:`.

### Security

- **Token redaction in error bodies**: any string the binary is
  about to display via `error!()` is now passed through
  `redact_secrets()`, which replaces every recognised GitHub token
  prefix with `<prefix>_<redacted>`. Defence in depth — the rest of
  the codebase already takes care to keep tokens out of error
  strings, but a misbehaving proxy that echoes a request URL could
  in principle still surface a token in `--verbose` output. This
  scrubber catches the last hop.

### Added (previous unreleased entry below)

- **`--print-config-template`** flag prints an annotated TOML configuration
  template to stdout and exits, so new operators can bootstrap a working
  config without hunting through documentation. The template is bundled
  with the binary and unit-tested to remain parseable and to document
  every required field.
- **`GITHUB_BACKUP_RESTORE_YES=1`** environment variable is now accepted
  as an alternative to `--restore-yes`, so CI pipelines that cannot easily
  add flags can still authorise non-interactive restore. Non-interactive
  mode now prints both escape hatches in its rejection message.
- **`NO_COLOR` / `CLICOLOR_FORCE`** are now honoured by the log
  formatter, following the standard observability conventions. The
  default is "ANSI on TTY, off otherwise" so logs piped to files stay
  plain UTF-8.
- **Owner-name validation** rejects path-traversal payloads (`..`, `/`,
  `\`, NUL, control characters) before any directories are created. The
  authoritative validation still belongs to GitHub's API — this is a
  defense-in-depth net for typos and accidental shell-injection.
- **Up-front writability probe** on the output directory: a tiny marker
  file is created and removed at lock-acquire time so a read-only mount
  or permissions issue is surfaced immediately, not after hundreds of
  API calls.

### Changed

- **HTTP retries** now apply exponential back-off **with jitter**
  (0–999 ms drawn from a deterministic clock-seeded PRNG, no extra
  dependency) so many concurrent workers do not retry in lock-step on a
  shared rate-limit bucket.
- **Back-off caps**: every rate-limit and 5xx retry is now clamped at
  five minutes (`MAX_BACKOFF_SECS`). A pathological `Retry-After` or
  `X-RateLimit-Reset` header can no longer pause a backup for hours.
- **Response body limits**: every JSON API response is capped at 16 MiB
  (`MAX_RESPONSE_BYTES`). Binary release-asset downloads remain
  unbounded as before.
- **OAuth device-flow polling** is deadline-aware: the sleep before
  each poll is clamped to the remaining session lifetime so an
  expiry is surfaced immediately as `OAuthExpired` instead of a vague
  network timeout. A heart-beat log line every 60 s reports
  `seconds_remaining` so the operator knows the session is still
  alive.
- **Report and Prometheus textfile writes are now atomic**
  (`tmp` + `rename`). The node_exporter textfile collector or any other
  consumer can no longer scrape a half-written file when the backup
  process is interrupted mid-write.
- **Anonymous-credential UX**: when no token / device-auth is supplied
  but the requested categories require admin/private scope, the run is
  refused up-front with an actionable error rather than failing
  silently inside the engine.
- **Refactored** the GET/POST retry logic in the HTTP client into a
  single shared `execute_with_retry` helper, removing ~100 lines of
  duplicated code while preserving the existing semantics.

### Security

- **`Credential::Debug` now redacts the token value** — previously the
  auto-derived `Debug` impl would have leaked the literal token through
  any `tracing::debug!("{cred:?}")` call. The `GitHubClient::Debug`
  impl was already redacting at the wrapping level; this closes the
  inner gap.

---

## [0.3.2] — 2026-04-12

Maintenance release focused on the release pipeline, distribution
strategy, supply-chain hardening, mutation-testing coverage, and
CI/docs polish. No runtime behaviour changes for end users; all
existing configurations and command-line flags are unchanged.

### Changed

- **Release distribution: dropped crates.io; releases are now binary +
  Docker only.** Every workspace crate is marked `publish = false`,
  the `publish` / `publish-dry-run` / `package` jobs and the protected
  `crates-io` GitHub Environment have been removed from
  `.github/workflows/release.yml`, and `CARGO_REGISTRY_TOKEN` is no
  longer referenced. Users install via one of three methods, in
  recommended order:
    1. Pre-built binary from the GitHub Releases page (five targets:
       Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64),
       each with a `.sha256` checksum and SLSA Level 2 build
       provenance attestation.
    2. Multi-arch container image from GHCR
       (`ghcr.io/tomtom215/github-backup-rust:<tag>`), wired up with
       a Docker Compose file at the repo root that supports local,
       S3, B2, MinIO, and Codeberg profiles and reads secrets from a
       `.env` file (template: `compose.example.env`).
    3. `cargo install --git https://github.com/tomtom215/github-backup-rust --tag v0.3.2 github-backup`.
  The rationale is that `github-backup` is a CLI application, not a
  reusable library — the workspace split exists for internal code
  organisation, not for third-party consumption. Dropping crates.io
  removes the operational burden of seven name reservations, seven
  docs.rs builds, and per-release intra-workspace version-pin
  synchronisation.
- **Multi-stage release pipeline** (`.github/workflows/release.yml`):
  `validate → ci → security → binaries → github-release → docker`.
  The validate job enforces semver tag format, checks
  `[workspace.package].version` against the tag, verifies every
  intra-workspace dependency line (if any carry `version =` pins),
  and requires a matching `## [X.Y.Z]` CHANGELOG entry before any
  build runs.
- **Intra-workspace `version =` pins dropped** from the root
  `Cargo.toml`. With every member marked `publish = false`, only the
  `path =` source is used to resolve internal dependencies, so there
  is nothing to keep in lock-step every release.
- **Portfolio-grade docs, manifests, and CI polish** (PR #16): the
  root `README.md`, `ARCHITECTURE.md`, every per-crate `Cargo.toml`
  (descriptions, keywords, categories, documentation links), the full
  mdBook (`docs/src/**`), `Dockerfile`, `clippy.toml`, and `deny.toml`
  have been reviewed and aligned. `README.md` and
  `docs/src/getting-started/installation.md` have been rewritten
  around the new binary / Docker / source install story.
- **`docker-compose.yml` now pulls from GHCR by default** via a
  shared YAML anchor (`image: ghcr.io/tomtom215/github-backup-rust`),
  mounts `./backups` → `/backup` and `./config.toml` →
  `/etc/github-backup/config.toml`, and fails fast if required
  secrets are missing from `.env`. `build: .` remains available as a
  commented-out override for local development. A new
  `compose.example.env` template ships at the repo root.
- **`mdBook` pinned to `0.4.40`** in CI and the Pages workflow to
  match the version that the `mdbook-linkcheck` backend is known to
  be compatible with.
- **Pages deployment publishes `docs/book/html/`** instead of
  `docs/book/`. Enabling the `linkcheck` backend causes mdBook to
  emit each backend into its own subdirectory, so the HTML tree is
  now one level deeper.

### Added

- **SLSA Level 2 build provenance attestations** for every pre-built
  binary produced by the release pipeline, verifiable with
  `gh attestation verify`.
- **Mutation-testing configuration** (`.cargo/mutants.toml`): curated
  exclude list for generated / panic-only / `#[cfg(...)]`-gated code
  paths so `cargo mutants` produces actionable survivors only. A
  `workflow_dispatch`-only CI job runs the full mutation suite
  on-demand without blocking every push to main.
- **Pagination malformed-URL tests** in `github-backup-client` and
  **rate-limit edge-case tests** to catch mutants that would
  otherwise silently degrade GitHub API retry/backoff behaviour.
- **`BackupRunHistory::push` regression tests** covering the
  deduplication and ordering mutants flagged by `cargo mutants`.
- **`cargo-audit` configuration** (`.cargo/audit.toml`) with an
  explicit ignore entry and rationale for the informational `rand`
  advisory that does not affect this project.

### Fixed

- **Release workflow `validate` job could never succeed**: the awk
  extractor for `[workspace.package].version` used the greedy regex
  `.*"` to strip the `version = "` prefix, which matched through the
  closing quote and left an empty string. The job then always
  reported "could not parse `[workspace.package].version`" and
  failed before comparing against the tag. Replaced with `^[^"]*"`
  so the strip stops at the first quote.
- **`Deploy Book to GitHub Pages` workflow failed with "The command
  `mdbook-linkcheck` wasn't found"**: `book.toml` enables
  `[output.linkcheck]`, but the Pages workflow only installed
  `mdbook` itself. The link checker is now installed alongside
  `mdBook`, matching the existing `ci.yml` step.
- **`is_process_alive` false positive on macOS**: `kill(0, 0)`
  returns success on BSDs for PID 0 (the kernel swapper), which
  caused the lockfile staleness check to treat a stale lock with
  PID 0 as still held. The check now rejects PID 0 explicitly.
- **macOS CI permissions test** that relied on `chmod 000` being
  honoured when the test runs as root.
- **`cargo-deny` `advisories` check now runs with `contents: read`
  permissions** in CI; the previous implicit default blocked the
  step on pull requests from forks.
- **`BackupEvent::RepoCompleted` missing error field** and related
  `dead_code` clippy errors surfaced by the mutation-testing work.
- **Rustdoc intra-doc link ambiguity** for the `write` module and
  several broken internal mdBook links that surfaced after the
  link-check backend was added.

### Security

- **Dependency audit**: all advisories reviewed and either fixed,
  upgraded away, or explicitly ignored with justification in
  `.cargo/audit.toml`. The release pipeline now runs
  `cargo-deny check licenses bans advisories sources` as a gating
  job before any binary or container image is produced.

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

[Unreleased]: https://github.com/tomtom215/github-backup-rust/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/tomtom215/github-backup-rust/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/tomtom215/github-backup-rust/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/tomtom215/github-backup-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/tomtom215/github-backup-rust/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/github-backup-rust/releases/tag/v0.1.0
