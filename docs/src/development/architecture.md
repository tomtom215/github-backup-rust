# Architecture

`github-backup` is a Rust workspace of six focused crates. This page describes how they fit together.

## Workspace Layout

```
crates/
├── github-backup-types/     Pure data types (no I/O)
├── github-backup-client/    Async GitHub API client
├── github-backup-core/      Backup engine and storage traits
├── github-backup-mirror/    Gitea push-mirror integration
├── github-backup-s3/        S3-compatible storage backend
└── github-backup/           CLI binary (entry point)
```

## Crate Responsibilities

### `github-backup-types`

Strongly-typed models for the GitHub REST API v3 responses.

- All types implement `Serialize + Deserialize` (serde)
- Pure data: no I/O, no async, no network
- Also contains `BackupOptions`, `OutputConfig`, and `ConfigFile`
- Used by every other crate as a vocabulary type library

### `github-backup-client`

Async HTTP client built on `hyper` + `rustls` (no OpenSSL, no reqwest).

- `GitHubClient` — paginated API calls, rate limit awareness, retry on 5xx
- `BackupClient` — trait abstracting the API (enables mock clients in tests)
- `OAuthClient` — GitHub device flow authentication
- `RateLimitTracker` — respects `X-RateLimit-*` response headers

### `github-backup-core`

The backup orchestration engine and key abstractions.

- `BackupEngine<S, G>` — generic over `Storage` and `GitRunner`; orchestrates all backup categories
- `Storage` trait — `write_json()`, `write_bytes()`, `exists()`; `FsStorage` is the production implementation; `MemStorage` is the test stub
- `GitRunner` trait — `mirror_clone()`, `bare_clone()`, `full_clone()`, `shallow_clone()`, `lfs_clone()`, `push_mirror()`; `ProcessGitRunner` shells out to git; `SpyGitRunner` is the test stub
- Per-category modules: `backup/repository`, `backup/issue`, `backup/pull_request`, `backup/release`, `backup/gist`, `backup/wiki`, `backup/user_data`
- `BackupStats` — lock-free `AtomicU64` counters shared across spawned tasks
- `CoreError` — all error variants for the backup engine

### `github-backup-mirror`

Gitea REST API v1 client for push-mirror integration.

- `GiteaClient` — create/check repositories via API
- `push_mirrors()` — iterates local mirror clones and pushes each to Gitea
- Supports Gitea, Forgejo, and Codeberg

### `github-backup-s3`

Pure-Rust S3-compatible storage client.

- `S3Client` — `put_object()`, `head_object()` with SigV4 signing
- `signing.rs` — AWS Signature Version 4 from scratch (sha2 + hmac)
- `sync_to_s3()` — incremental sync: skip existing objects via `HeadObject`
- Works with AWS S3, Backblaze B2, Cloudflare R2, MinIO, DigitalOcean Spaces, Wasabi

### `github-backup` (binary)

CLI entry point using `clap`.

- `cli/args.rs` — `Args` struct with all flags; `merge_config()` for TOML overlay
- `cli/clone_type.rs` — `CliCloneType` parser (`mirror`, `bare`, `full`, `shallow:N`)
- `main.rs` — orchestration: auth → backup → report → mirror → S3

## Data Flow

```
CLI args + config file
       │
       ▼
   Args::merge_config()
       │
       ▼
   BackupEngine::run(owner)
       │
       ├── fetch_repos() → GitHubClient
       │
       ├── backup_gists() ─────────────────────────────────┐
       │                                                    │
       └── for each repo (concurrent, semaphore-limited):   │
               │                                           │
               ├── backup_repository() → GitRunner         │
               ├── backup_wiki()       → GitRunner         │
               ├── backup_issues()     → Storage           │
               ├── backup_pull_requests() → Storage        │
               ├── backup_releases()   → Storage           │
               └── backup_user_data()  → Storage           │
                                                           │
   BackupStats (AtomicU64) ◄──────────────────────────────┘
       │
       ▼
  [optional] write_report()
  [optional] push_mirrors()   → GiteaClient
  [optional] sync_to_s3()     → S3Client
```

## Concurrency Model

Repository backups run as `tokio::spawn` tasks behind a `Semaphore`:

```rust
let sem = Arc::new(Semaphore::new(opts.concurrency));

for repo in repos {
    let permit = sem.clone().acquire_owned().await?;
    tokio::spawn(async move {
        let _permit = permit; // dropped when task completes
        backup_one_repo(...).await
    });
}
```

This allows up to `concurrency` (default: 4) repositories to be backed up simultaneously, with the API client, storage, and git runner each being `Send + Sync`.

`BackupStats` counters use `AtomicU64` with `Relaxed` ordering — fully lock-free.

## Credential Security

HTTPS token credentials are **never embedded in git URLs or command arguments**. Instead, a small shell script is written to a uniquely-named temp file:

```sh
#!/bin/sh
echo 'ghp_your_token_here'
```

The `GIT_ASKPASS` environment variable points git at this script. The file has `0700` permissions on Unix and is **deleted by a RAII guard** immediately after the git subprocess exits — even on panic.

## Dependency Policy

Maintained in `deny.toml`:

| Banned crate | Why | Alternative |
|-------------|-----|-------------|
| `openssl-sys` | Build complexity, licensing | `rustls` |
| `reqwest` | Too many transitive deps | `hyper` + `hyper-rustls` |
| `native-tls` | Platform-specific | `rustls` + `rustls-native-certs` |

Direct runtime dependencies: 14 crates. No OpenSSL. No AWS SDK.

## Testing Strategy

| Layer | Mechanism |
|-------|----------|
| Types | `proptest` round-trip tests (JSON serialisation) |
| Client | Unit tests with `MockBackupClient` |
| Core | `SpyGitRunner` + `MemStorage` in-memory stubs |
| S3 | Unit tests for SigV4 signing; integration tests with mock responses |
| Mirror | Unit tests with mock Gitea client |
| CLI | `Args::try_parse_from()` parse tests |
