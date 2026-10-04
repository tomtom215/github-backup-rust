# Architecture

`github-backup` is a Rust workspace of seven crates.  This page describes how
they fit together.  (It is also the book page *Development / Architecture*.)

## Workspace Layout

```
github-backup-rust/
├── crates/
│   ├── github-backup-types/    # GitHub API models, Raw<T>/Page<T>, configuration types
│   ├── github-backup-client/   # Async HTTP client (GitHub API, OAuth, proxy support)
│   ├── github-backup-core/     # Backup engine: orchestration, storage, git, locks
│   ├── github-backup-mirror/   # Push mirrors to Gitea / Forgejo / Codeberg / GitLab
│   ├── github-backup-s3/       # S3-compatible client, sync, AES-256-GCM encryption
│   ├── github-backup-tui/      # Ratatui front-end (--tui)
│   └── github-backup/          # CLI binary (clap), run modes, reporting
├── docker/entrypoint.sh        # container entrypoint wrapper
├── Dockerfile, docker-compose.yml
├── unraid/                     # Unraid Community Applications template
├── docs/                       # mdBook sources
└── deny.toml                   # cargo-deny: licences, bans, advisories, sources
```

## Crate Responsibilities

### `github-backup-types`

Pure data, no I/O.

- GitHub models (`Repository`, `Issue`, `PullRequest`, ...).  Fields GitHub can
  send as `null` are `Option`s.
- `Raw<T>` pairs a typed view with the **original JSON object**, and `Page<T>`
  is a list of them.  The engine writes the original objects (all properties, in
  GitHub's key order; `serde_json` is built with `preserve_order`), so the
  backup is lossless; the typed view is only used to make decisions.  An element
  the typed model cannot read is still written verbatim and reported in the log.
- Configuration: `BackupOptions`, `CloneType`, `ConfigFile` (the TOML schema, with
  `deny_unknown_fields`), `OutputConfig` (all output paths), the persisted
  `BackupState`, `BackupCheckpoint`, `BackupRunHistory` and the starred queue.
- `glob_match()` for `--include-repos` / `--exclude-repos`.

### `github-backup-client`

Async client built on `hyper` + `rustls` (no OpenSSL, no reqwest).

- `GitHubClient` (standard GitHub and, with `with_api_url`, GHES) behind the
  object-safe `BackupClient` trait, which tests replace with `MockClient`.
- **Pagination** with `Link` headers; wrapped responses (`{"workflows": [...]}`)
  are merged into plain lists.
- **Retry policy** (`execute_with_retry`): rate limits (429, or 403 with
  `Retry-After`, `X-RateLimit-Remaining: 0` or a rate-limit message) are waited
  out and retried (up to 6 times, about an hour in total per request); GET
  requests retry 5xx and transport failures 3 times with jittered back-off;
  other statuses fail at once.  Every response body is capped at 16 MiB and
  every request has a 120 s timeout.
- **Own-account listings**: `GET /user` is asked once; when the login equals the
  owner the client lists `/user/repos?affiliation=owner&visibility=all` and
  `/gists`, which include private repositories and secret gists.
- **Release assets** are streamed to disk through an `AssetSink`; the
  `Authorization` header is dropped on any redirect that leaves the origin.
- `proxy`: HTTP `CONNECT` proxy support from `HTTPS_PROXY`, `HTTP_PROXY`,
  `ALL_PROXY`, `NO_PROXY` (no SOCKS), shared with the webhook and `--doctor`.
- `oauth`: the device flow.

### `github-backup-core`

The engine and its abstractions.

```
BackupEngine<C: BackupClient, S: Storage, G: GitRunner>
  ├── C  (API calls)
  ├── S  (write JSON / bytes atomically; production: FsStorage)
  └── G  (git subprocesses; production: ProcessGitRunner)
```

- **Failure model** (`engine/steps.rs`): every unit of work is a *step*.  A failing
  step is recorded in `BackupStats` (scope, step, message) and the run carries on,
  so one bad repository or category never costs the rest.  `run` returns `Err`
  only for things that make continuing pointless (`CoreError::is_fatal`:
  rejected credentials, exhausted rate-limit budget, full disk, cancellation) or
  when the repository list cannot be fetched.  Callers read
  `BackupStats::failures()` and decide the exit status.
- **Run order**: owner-level data, packages, starred clones, gists, then the
  repository list (written to `repos.json`), then repositories concurrently
  (bounded by `--concurrency`); per repository the steps are repository clone,
  wiki, issues, pull requests, releases, labels, milestones, hooks, security
  advisories, topics, branches, deploy keys, collaborators, actions,
  environments (plus the inert discussions and projects steps).
- **Incremental state** (`engine/incremental.rs`): a watermark per repository in
  `backup_state.json`, advanced only for repositories whose every step
  succeeded, trusted only for the categories it was recorded with, set 15
  minutes before the run started.  Lists are always fetched in full and merged
  (`backup/merge.rs`); the watermark only decides which per-item requests
  (comments, events, commits, reviews) can be skipped.  `--full` ignores it; an
  explicit `--since` replaces it and is never stored.
- **Checkpoint**: `backup_checkpoint.json` lists repositories finished in the
  current run; a later run resumes from it only if it is less than 6 hours old.
- **Locking** (`lock.rs`): an operating-system exclusive lock (`fslock`: `flock` /
  `LockFileEx`) on `<owner>/json/.backup.lock`; the CLI takes another on
  `<output>/.github-backup.lock`.  The kernel releases them when the process dies,
  so there are no stale locks and no PID heuristics.
- **Storage** (`storage.rs`): every JSON file is written to a temporary name and
  renamed; release assets are streamed to `.<file>.part` and renamed after the
  size and digest checks.
- **Git** (`git/`): clones are made into a hidden staging directory and renamed
  into place; updates use `fetch` (mirror: `--all --prune`; bare: explicit
  refspecs; shallow: `--depth`; `--lfs`: mirror update plus `git lfs fetch --all`).
  Each `git` runs in its own process group with a **stall** timeout (600 s without
  output), is killed with its helpers on cancellation, and receives the token only
  through an environment variable and a host-scoped credential helper (`credential.rs`).
  `-c safe.directory=<path>` trusts exactly the repository being worked on.
- **Cancellation** (`cancel.rs`): a flag that kills running git processes and makes
  in-flight steps give way; the CLI sets it on `SIGINT` / `SIGTERM`.

### `github-backup-mirror`

Post-processing: push the cloned repositories to another host.

- `GiteaClient` (REST v1) and `GitLabClient` (REST v4): check the destination,
  create it (private unless the source is known to be public), and verify that it
  is ours (description marker `GitHub mirror of <owner>/<repo>`) or empty.
- `push.rs`: `git push --prune` of `refs/heads/*` and `refs/tags/*` only (never
  `--mirror`), with the token in an environment variable and a credential helper
  scoped to the destination origin.
- `runner` / `gitlab_runner`: walk `git/repos/*.git` and report per-repository
  failures.

### `github-backup-s3`

Post-processing: upload the JSON metadata to an S3-compatible store.

- `signing`: AWS Signature V4 built from `sha2` + `hmac` (no AWS SDK).
- `S3Client`: PUT / HEAD / LIST / DELETE and multipart upload over hyper + rustls
  with retries and an idle timeout.  It connects directly (no proxy support).
- `sync_to_s3`: concurrent sync of `<output>/<owner>/json/`; an object is skipped
  only when its stored SHA-256 digest (keyed HMAC for encrypted uploads) and size
  match; guarded `--s3-delete-stale`.
- `encrypt`: AES-256-GCM, wire format `[12-byte nonce | ciphertext | 16-byte tag]`.

### `github-backup-tui`

The Ratatui terminal interface (`--tui`).  It drives the same `BackupEngine`
through an event channel and does not embed its own backup logic.  See the
[Interactive TUI guide](https://tomtom215.github.io/github-backup-rust/tui.html).

### `github-backup` (binary)

| File | Responsibility |
|------|----------------|
| `main.rs` | argument parsing, config merge, mode dispatch, credential resolution |
| `cli/` | the clap `Args`, the config-file merge rules, the `--clone-type` parser |
| `run.rs` | the backup run: engine, post-processing, reports, exit status (0/1/3, 130/143) |
| `post_process.rs` | mirror push, S3 sync, diff |
| `restore.rs` | `--restore` |
| `report.rs`, `metrics.rs`, `notify.rs` | JSON report, Prometheus textfile, webhook |
| `doctor.rs`, `scopes.rs`, `modes.rs` | `--doctor`/`--check`, `--list-scopes`, `--verify`/`--decrypt` |
| `shutdown.rs` | signal handling and the exit watchdog |
| `ui.rs`, `errors.rs`, `setup.rs`, `lock.rs` | banners, error hints and redaction, tracing setup, the output-directory lock |

Flow of a normal run:

1. Parse the arguments and the optional config file (command line wins for
   single values; switches are OR-ed, lists unioned).
2. `--tui`, `--list-scopes`, `--doctor`/`--check`, `--decrypt`, `--verify` and
   `--restore` are handled and return before a backup.
3. Take the output lock, build the client, run `BackupEngine::run`, racing it
   against `SIGINT` / `SIGTERM`.
4. Post-processing, each step's failure being **recorded**, not fatal: manifest,
   diff, mirror push, S3 sync.
5. Write history, report and metrics, send the webhook, exit with `0` or `3`.

A `--dry-run` stops after step 3's listing: it takes no lock and writes nothing.

## Data Flow

```
GitHub API ──► GitHubClient ──► BackupEngine ──► Storage (FsStorage)  ──► <output>/<owner>/json
                                    │
                                    └──► GitRunner (git) ──────────────► <output>/<owner>/git
                                                                              │
                                  local backup ─────┬──► mirror push (git push to Gitea/GitLab)
                                                    └──► S3 sync (json/ only)
```

## Concurrency Model

Repositories are processed as concurrent tasks bounded by a semaphore
(`--concurrency`, default 4); owner-level data, gists and starred clones run first
and sequentially.  The API client, storage and git runner are `Send + Sync`.
`BackupStats` uses atomic counters and a mutex-protected failure list.

## Credential Security

See the [Security page](https://tomtom215.github.io/github-backup-rust/development/security.html).  In short: the token
reaches `git` only through an environment variable and a credential helper scoped
to the clone URL's origin; it is never in an argument list, a URL or a file, and
log and error text is redacted.

## Dependency Policy

`deny.toml` (enforced by `cargo-deny` in CI, in the release workflow and in a
daily workflow) bans `openssl`, `openssl-sys`, `native-tls` and `reqwest`, allows
only MIT, Apache-2.0 (also with the LLVM exception), ISC, BSD-3-Clause,
Unicode-3.0, CC0-1.0 and Zlib, checks RustSec advisories and permits crates.io as
the only source.  TLS is `rustls` with the operating system's certificate store.

## Unsafe Code

The workspace's own source contains no `unsafe`.  `github-backup-core` has
`#![forbid(unsafe_code)]`; the client, mirror, s3 and types crates have
`#![deny(unsafe_op_in_unsafe_fn)]`.

## Testing Strategy

| Layer | Technique |
|-------|-----------|
| Types | golden tests against GitHub's own example payloads, `proptest` round trips |
| Client | local fake HTTP servers: retry, rate limit, redirect, proxy and pagination tests |
| Core | `MockClient`, `MemStorage` and `SpyGitRunner` stubs; real `git` against local repositories for the git runner |
| S3 | an in-process fake S3 server that verifies SigV4 independently; AES-GCM round trips and tamper tests |
| Mirror | real `git push` against local bare repositories with a refusing hook |
| CLI | argument-parsing and config-merge tests; a binary-level test of the environment handling |
| CI | `cargo fmt --check`, `clippy -D warnings`, tests on Linux and macOS, an MSRV (1.88) build, rustdoc, the mdBook build with link checking, `cargo audit` and `cargo deny`, and a Docker build with a smoke test |
| Mutation | `cargo mutants`, started manually (`workflow_dispatch`); not run on every push |
