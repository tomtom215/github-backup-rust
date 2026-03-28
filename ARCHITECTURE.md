# Architecture

## Workspace Layout

```
github-backup-rust/
├── crates/
│   ├── github-backup-types/    # GitHub API types + backup configuration
│   ├── github-backup-client/   # Async HTTP client (GitHub API + OAuth)
│   ├── github-backup-core/     # Backup engine: orchestration, storage, git
│   ├── github-backup-mirror/   # Push-mirror to Gitea/Codeberg/Forgejo
│   ├── github-backup-s3/       # S3/B2/MinIO storage backend
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

Backup modules (`crates/github-backup-core/src/backup/`):
- `repository.rs` — git clone dispatching on `CloneType`
- `issue.rs`, `pull_request.rs`, `release.rs` — JSON metadata
- `gist.rs`, `wiki.rs` — secondary git clones
- `user_data.rs` — starred, watched, followers, following

### `github-backup-mirror`

Post-processing: push cloned repositories to a secondary Git host.

- `GiteaClient` — Gitea REST API v1 (repo existence check, creation)
- `runner::push_mirrors` — walks local `*.git` dirs, ensures repos exist,
  runs `git push --mirror`
- Compatible with Codeberg, Gitea, Forgejo, and any Gitea API v1 host

### `github-backup-s3`

Post-processing: upload backup artefacts to S3-compatible object stores.

- `signing::Signer` — AWS Signature Version 4 (pure Rust, no AWS SDK)
- `S3Client` — PutObject / HeadObject using hyper + rustls
- `sync::sync_to_s3` — incremental directory sync (skips already-uploaded files)
- Supports AWS S3, Backblaze B2, MinIO, Cloudflare R2, DigitalOcean Spaces

### `github-backup` (CLI binary)

Orchestrates all crates:

1. Parse CLI args (`clap`)
2. Obtain credential (PAT or OAuth device flow)
3. Run `BackupEngine` (primary backup)
4. Optional: `push_mirrors` (Gitea mirror)
5. Optional: `sync_to_s3` (S3 upload)

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
- **Allowed licenses**: MIT, Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0, CC0-1.0

TLS is handled exclusively by `rustls` with the platform CA bundle via
`rustls-native-certs`.  Cryptography for S3 SigV4 uses `sha2` + `hmac` from
the RustCrypto project (no OpenSSL).

## Testing Strategy

| Layer | Technique |
|-------|-----------|
| Unit | `MockBackupClient` + `MemStorage` + `SpyGitRunner` stubs |
| Integration | `tempfile` + real filesystem (storage tests) |
| Property | `proptest` for type round-trip invariants |
| CI | `cargo test --workspace` on ubuntu-latest + macos-latest |
| Linting | `cargo clippy -D warnings` |
| Formatting | `cargo fmt --check` |
| Security | `cargo audit` + `cargo deny` |
| MSRV | `cargo build` with Rust 1.85 |
