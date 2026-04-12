# Architecture

## Workspace Layout

```
github-backup-rust/
├── crates/
│   ├── github-backup-types/    # GitHub API types + backup configuration
│   ├── github-backup-client/   # Async HTTP client (GitHub API + OAuth)
│   ├── github-backup-core/     # Backup engine: orchestration, storage, git
│   ├── github-backup-mirror/   # Push-mirror to Gitea / GitLab
│   ├── github-backup-s3/       # S3-compatible storage backend
│   ├── github-backup-tui/      # Ratatui TUI front-end (--tui flag)
│   └── github-backup/          # CLI binary (main entry point)
├── Dockerfile
├── docker-compose.yml
└── deny.toml                   # cargo-deny: license + ban policy
```

## Crate Responsibilities

### `github-backup-types`

Pure data: GitHub API response structs, backup configuration types.  No I/O.
All types implement `Serialize + Deserialize` (serde).

Key types:
- `BackupOptions` — what to back up (all selection flags + `CloneType`)
- `CloneType` — mirror / bare / full / shallow
- `OutputConfig` — derives backup directory paths from a root
- GitHub response types: `Repository`, `Issue`, `PullRequest`, …

### `github-backup-client`

Async HTTP client for the GitHub REST API v3.

- `GitHubClient` — hyper + rustls, automatic pagination, rate-limit back-off,
  5xx retry
- `BackupClient` trait — object-safe interface enabling mock substitution in tests
- `oauth` module — GitHub OAuth Device Flow for browser-based auth
- `endpoints/write.rs` — mutating endpoints used by `--restore`:
  `create_label`, `create_milestone`, `create_issue`

### `github-backup-core`

The backup engine and its abstractions.

```
BackupEngine<S: Storage, G: GitRunner>
  ├── GitHubClient           (API calls)
  ├── S: Storage             (write JSON/bytes to a sink)
  └── G: GitRunner           (git subprocess: clone, fetch, push)
```

Key traits:
- `Storage` — write JSON and binary files (production: `FsStorage`)
- `GitRunner` — git operations (production: `ProcessGitRunner`)

Both traits have test stubs (`MemStorage`, `SpyGitRunner`) enabling full
coverage without network or filesystem access.

Backup modules (`crates/github-backup-core/src/backup/`), one file per category:
- `repository.rs` — git clone dispatching on `CloneType`
- `issue.rs`, `pull_request.rs`, `release.rs` — JSON metadata
- `gist.rs`, `wiki.rs` — secondary git clones
- `user_data.rs` — starred, watched, followers, following
- `labels.rs`, `milestones.rs` — repository metadata
- `hooks.rs`, `security_advisories.rs` — admin metadata (graceful 403/404)
- `topics.rs`, `branches.rs` — repository topology; `branches.rs` also saves
  `branch_protections.json` for protected branches (graceful 403/404 per branch)
- `deploy_keys.rs`, `collaborators.rs` — access control metadata
- `actions.rs`, `environments.rs` — GitHub Actions and deployments
- `discussion.rs`, `project.rs`, `package.rs` — Discussions, Classic Projects, Packages
- `starred_repos.rs` — durable-queue starred-repo cloning

### `github-backup-mirror`

Post-processing: push cloned repositories to a secondary Git host.

- `GiteaClient` — Gitea REST API v1 (repo existence check, creation)
- `runner::push_mirrors` — walks local `*.git` dirs, ensures repos exist,
  runs `git push --mirror`
- Compatible with Codeberg, Gitea, Forgejo, and any Gitea API v1 host

### `github-backup-s3`

Post-processing: upload backup artefacts to S3-compatible object stores.

- `signing::Signer` — AWS Signature Version 4 (pure Rust, no AWS SDK)
- `S3Client` — PutObject / HeadObject / multipart upload using hyper + rustls
- `sync::sync_to_s3` — concurrent incremental directory sync (up to 8 parallel
  uploads via Tokio JoinSet + Semaphore; skips already-uploaded objects)
- `encrypt` — AES-256-GCM at-rest encryption; wire format `[12-byte nonce |
  ciphertext | 16-byte GCM tag]`; exposed as public `encrypt` / `decrypt` fns
- Supports AWS S3, Backblaze B2, MinIO, Cloudflare R2, DigitalOcean Spaces

### `github-backup-tui`

Full-screen terminal user interface built with [Ratatui](https://ratatui.rs) 0.30.

- `run_tui(InitialConfig) -> ExitCode` — public entry point; owns the terminal
- Five screens: Dashboard, Configure, Running, Results, Verify
- `App` state machine: `Screen` enum drives per-screen rendering and key dispatch
- `ConfigState` — mirrors all 50+ `BackupOptions` fields; converts to `BackupOptions`
  via `to_backup_config()`; validates on launch
- `TuiTracingLayer` — intercepts all `tracing` events and routes them to the log
  panel as structured `BackupEvent::LogLine` messages, replacing the default stderr
  logger while the TUI is active
- `event_loop` races terminal input against the `ProgressRx` channel at 60 Hz (16 ms
  tick) using `tokio::select!` over a backup cancellation oneshot
- `run_backup_task` + `run_verify_task` — spawned as Tokio tasks; completion reported
  via `ProgressTx = UnboundedSender<BackupEvent>`

### `github-backup` (CLI binary)

Orchestrates all crates.  Key source files:

| File | Responsibility |
|------|----------------|
| `main.rs` | Entry point; arg parsing, credential resolution, backup orchestration |
| `cli/args.rs` | 50+ clap flags including `--restore`, `--decrypt`, `--restore-yes` |
| `post_process.rs` | Mirror push, S3 sync, retention, diff, Prometheus metrics; typed `PostProcessError` |
| `restore.rs` | `--restore` mode: re-creates labels, milestones, and issues via GitHub API; supports `--dry-run` and `--restore-yes` confirmation gate |
| `report.rs` | JSON summary report generation |

Operational flow:

1. Parse CLI args (`clap`); merge TOML config file
2. If `--tui`: hand off to `github_backup_tui::run_tui()` and return
3. If `--verify`: manifest integrity check and return
4. If `--decrypt`: AES-256-GCM file decryption and return
5. Obtain credential (PAT or OAuth device flow)
6. Run `BackupEngine` (primary backup)
7. Optional post-processing: manifest, Prometheus metrics, diff, restore, mirror push, S3 sync, retention

## Data Flow

```
GitHub API
    │
    ▼
GitHubClient ──► BackupEngine
                    │
                    ├── GitRunner (git clone/fetch)
                    │       └── GIT_ASKPASS RAII script
                    │
                    └── Storage (write JSON/bytes)
                            └── FsStorage (real filesystem)
                                    │
                                    ▼
                              Local backup
                             /            \
                            ▼              ▼
                     GiteaClient      S3Client
                    (push mirror)    (S3 sync)
```

## Concurrency Model

Repositories are backed up concurrently using a Tokio semaphore:

```rust
let sem = Arc::new(Semaphore::new(opts.concurrency)); // default: 4

for repo in repos {
    let permit = sem.clone().acquire_owned().await?;
    tokio::spawn(async move {
        let _permit = permit; // released on drop
        backup_one_repo(…).await
    });
}
```

`BackupStats` uses `Arc<AtomicU64>` for lock-free counter increments across
concurrent tasks.

## Credential Security

HTTPS credentials are never embedded in URLs or passed on the command line.
Instead, a temporary shell script is written to `$TMPDIR` with mode `0700`:

```sh
#!/bin/sh
echo 'ghp_xxxxxxxxx'
```

`GIT_ASKPASS` is set to this script; git calls it to retrieve the password.
The script is deleted by a RAII guard (`AskpassScript::drop`) immediately after
the git subprocess exits, even on panic.

## Dependency Policy

Governed by `deny.toml`:

- **Banned**: `openssl`, `openssl-sys`, `reqwest`, `native-tls`
- **Allowed licenses**: MIT, Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0, CC0-1.0,
  Zlib

TLS is handled exclusively by `rustls` with the platform CA bundle via
`rustls-native-certs`.  Cryptography for S3 SigV4 uses `sha2` + `hmac` from
the RustCrypto project (no OpenSSL).

## Unsafe Code Policy

The workspace denies `unsafe_op_in_unsafe_fn`.  The only `unsafe` block in
the codebase is a single FFI call to POSIX `kill(pid, 0)` in
`crates/github-backup-core/src/lock.rs`, used to detect a stale lock file
left behind by a crashed previous run.  Linux uses `/proc/<pid>` and avoids
the FFI entirely.

## Testing Strategy

| Layer | Technique |
|-------|-----------|
| Unit | `MockBackupClient` + `MemStorage` + `SpyGitRunner` stubs |
| TUI unit | 74 tests in `github-backup-tui::tests` — state machine logic without a real terminal |
| Integration | `tempfile` + real filesystem (storage tests, restore dry-run filesystem test) |
| Property | `proptest` for type round-trip invariants; AES-256-GCM encrypt/decrypt roundtrip + tamper detection |
| Mutation | `cargo mutants` (runs on `main` branch; report uploaded as CI artefact) |
| CI | `cargo test --workspace` on ubuntu-latest + macos-latest |
| Linting | `cargo clippy -D warnings` |
| Formatting | `cargo fmt --check` |
| Security | `cargo audit` + `cargo deny` |
| MSRV | `cargo build` with Rust 1.88 |
