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
- **Trait-based**: `Storage` and `GitRunner` traits enable full unit-test coverage without network or filesystem access
- **Zero unsafe code**: `#![deny(unsafe_op_in_unsafe_fn)]` workspace-wide
- **Zero clippy warnings**: enforced with `-D warnings`

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

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `github-backup-types` | GitHub API response types + backup configuration |
| `github-backup-client` | Async HTTP client (hyper + rustls), pagination, rate limiting |
| `github-backup-core` | Backup engine: orchestration, `Storage` and `GitRunner` traits |
| `github-backup` | CLI binary (clap) |

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

MIT — Copyright 2026 Tom F
