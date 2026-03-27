# github-backup-rust

A comprehensive, async GitHub backup tool written in Rust with minimal dependencies.

Backs up repositories (bare mirror clones), issues, pull requests, releases,
release assets, gists, wikis, labels, milestones, webhooks, security advisories,
followers, following, starred repositories, and watched repositories.

## Features

| Category | What is backed up |
|----------|------------------|
| **Repositories** | Bare mirror clones (`git clone --mirror`) |
| **Issues** | Metadata, comments, timeline events |
| **Pull Requests** | Metadata, review comments, commits, reviews |
| **Releases** | Metadata + optional binary asset download |
| **Gists** | Metadata + git clones |
| **Wikis** | Bare mirror clones |
| **Labels** | Repository label JSON |
| **Milestones** | Repository milestone JSON |
| **Hooks** | Webhook configuration JSON (requires admin token) |
| **Security Advisories** | Published advisory JSON |
| **User data** | Starred, watched, followers, following |

## Design

- **No OpenSSL**: uses [rustls](https://crates.io/crates/rustls) + system CA bundle
- **No reqwest**: uses [hyper](https://crates.io/crates/hyper) directly
- **Minimal dependencies**: 12 direct runtime dependencies
- **Async throughout**: built on [tokio](https://crates.io/crates/tokio)
- **Trait-based**: `BackupClient`, `Storage`, and `GitRunner` traits enable full unit-test coverage without network or filesystem access
- **Zero unsafe code**: `#![deny(unsafe_op_in_unsafe_fn)]` workspace-wide
- **Zero clippy warnings**: enforced with `-D warnings`
- **Transient resilience**: up to 3 retries with exponential back-off on 5xx responses
- **RAII cleanup**: `GIT_ASKPASS` helper scripts are removed by Drop even on panic

## Installation

```sh
cargo install --path crates/github-backup
```

## Usage

```
github-backup <OWNER> --token <TOKEN> [OPTIONS]
```

### Authentication

Provide a personal access token (classic or fine-grained) via `--token` or
the `GITHUB_TOKEN` environment variable.

```sh
export GITHUB_TOKEN=ghp_xxxxxxxxxxxx
github-backup octocat --output /var/backup/github --all
```

### Common options

```
-o, --output <DIR>     Root backup directory [default: .]
--all                  Enable all backup categories
--repositories         Clone/mirror repositories
--forks                Include forked repositories
--private              Include private repositories
--issues               Back up issues
--issue-comments       Back up issue comment threads
--issue-events         Back up issue timeline events
--pulls                Back up pull requests
--pull-comments        Back up PR review comments
--pull-commits         Back up PR commit lists
--pull-reviews         Back up PR reviews
--labels               Back up labels
--milestones           Back up milestones
--releases             Back up releases
--release-assets       Download release binary assets
--hooks                Back up webhooks (requires admin scope)
--security-advisories  Back up security advisories
--wikis                Clone wikis
--starred              Back up starred repositories
--watched              Back up watched repositories
--followers            Back up follower list
--following            Back up following list
--gists                Back up gists
--starred-gists        Back up starred gists
--prefer-ssh           Use SSH URLs for cloning
--lfs                  Use Git LFS
-q, --quiet            Suppress non-error output
-v, --verbose          Increase log verbosity (-v = debug, -vv = trace)
```

### Output directory layout

```
<output>/
└── <owner>/
    ├── git/
    │   ├── repos/
    │   │   └── <repo-name>.git/       # bare mirror
    │   ├── wikis/
    │   │   └── <repo-name>.wiki.git/  # bare mirror
    │   └── gists/
    │       └── <gist-id>.git/         # bare mirror
    └── json/
        ├── repos/
        │   └── <repo-name>/
        │       ├── info.json
        │       ├── topics.json
        │       ├── issues.json
        │       ├── issue_comments/<number>.json
        │       ├── issue_events/<number>.json
        │       ├── pulls.json
        │       ├── pull_comments/<number>.json
        │       ├── pull_commits/<number>.json
        │       ├── pull_reviews/<number>.json
        │       ├── releases.json
        │       ├── release_assets/<tag>/<filename>
        │       ├── labels.json
        │       ├── milestones.json
        │       ├── hooks.json
        │       └── security_advisories.json
        ├── gists/
        │   ├── index.json
        │   ├── <gist-id>.json
        │   └── starred_index.json
        ├── starred.json
        ├── watched.json
        ├── followers.json
        └── following.json
```

### Run summary

After a successful backup run the tool logs a structured summary:

```
INFO backup complete repos=42 backed_up=40 skipped=1 errored=1 gists=5
```

The same counters are also emitted as a human-readable `Display` line at
`INFO` level:

```
repos: 40 backed up, 1 skipped, 1 errored; gists: 5 backed up
```

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `github-backup-types` | GitHub API response types + backup configuration |
| `github-backup-client` | Async HTTP client (hyper + rustls), pagination, rate-limit handling, 5xx retry; exposes the `BackupClient` trait |
| `github-backup-core` | Backup engine: orchestration, `Storage` and `GitRunner` traits, `BackupStats` |
| `github-backup` | CLI binary (clap) |

### `BackupClient` trait

All HTTP operations are hidden behind an object-safe `BackupClient` trait
defined in `github-backup-client`:

```rust
pub trait BackupClient: Send + Sync {
    fn list_user_repos<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>>;
    // … 20 more methods
}
```

`BoxFuture<'a, T>` is `Pin<Box<dyn Future<Output = T> + Send + 'a>>`.
`GitHubClient` in `github-backup-client` implements the trait; unit tests
in `github-backup-core` use an in-process `MockBackupClient` that returns
pre-loaded fixture data without touching the network.

### `BackupStats`

`BackupStats` uses lock-free `AtomicU64` counters wrapped in an `Arc` so
each concurrently spawned tokio task can increment counters without
contention:

```rust
let stats = BackupStats::default();
let handle = stats.handle(); // cheap Arc clone
tokio::spawn(async move { handle.inc_repos_backed_up(); });
println!("{stats}"); // "repos: 1 backed up, 0 skipped, 0 errored; gists: 0 backed up"
```

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo doc --no-deps --workspace
```

### Architecture notes

- `github-backup-client/src/client/` is split into `mod.rs` (HTTP machinery,
  pagination, rate-limit retry, 5xx exponential back-off) and `endpoints.rs`
  (one method per API endpoint), keeping each file well under 500 lines.
- `github-backup-core/src/backup/` has one file per backup domain
  (`issue.rs`, `pull_request.rs`, `release.rs`, `gist.rs`, `user_data.rs`)
  each with its own `#[cfg(test)]` suite driven by `MockBackupClient`.
- `GIT_ASKPASS` scripts are managed by the `AskpassScript` RAII guard in
  `github-backup-core/src/git.rs`; the temp file is removed by `Drop` even
  if the git command panics.

## License

MIT — Copyright 2026 Tom F
