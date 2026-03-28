# github-backup-rust

A comprehensive, async GitHub backup tool written in Rust with minimal
dependencies.

Backs up repositories (with full clone-type selection), issues, pull requests,
releases, release assets, gists, wikis, labels, milestones, webhooks, security
advisories, followers, following, starred repositories, and watched repositories.

After the primary backup, optionally:
- **Push mirrors** to Gitea, Codeberg, Forgejo, or any self-hosted Git service.
- **Sync metadata** to AWS S3, Backblaze B2, MinIO, Cloudflare R2, or any
  S3-compatible object store.

## Features

| Category | What is backed up |
|----------|------------------|
| **Repositories** | Configurable clone type: mirror, bare, full, or shallow |
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

### Storage Backends

| Backend | How to enable |
|---------|--------------|
| Local filesystem (default) | `--output /path/to/backup` |
| AWS S3 | `--s3-bucket my-bucket --s3-region us-east-1` |
| Backblaze B2 | `--s3-bucket b2-bucket --s3-endpoint https://s3.us-west-004.backblazeb2.com` |
| MinIO (self-hosted) | `--s3-bucket my-bucket --s3-endpoint http://minio:9000` |
| Cloudflare R2 | `--s3-bucket r2-bucket --s3-endpoint https://<account>.r2.cloudflarestorage.com` |

### Mirror Destinations

| Destination | How to enable |
|-------------|--------------|
| Codeberg | `--mirror-to https://codeberg.org --mirror-token TOKEN --mirror-owner USER` |
| Self-hosted Gitea | `--mirror-to https://git.example.com --mirror-token TOKEN --mirror-owner USER` |
| Forgejo | `--mirror-to https://forge.example.com --mirror-token TOKEN --mirror-owner USER` |
| Any Gitea API v1 host | Same as above |

## Design

- **No OpenSSL**: uses [rustls](https://crates.io/crates/rustls) + system CA bundle
- **No reqwest**: uses [hyper](https://crates.io/crates/hyper) directly
- **No AWS SDK**: S3 backend implements SigV4 signing from scratch (`sha2` + `hmac`)
- **Minimal dependencies**: 14 direct runtime dependencies
- **Async throughout**: built on [tokio](https://crates.io/crates/tokio)
- **Trait-based**: `BackupClient`, `Storage`, and `GitRunner` traits enable full
  unit-test coverage without network or filesystem access
- **Zero unsafe code**: `#![deny(unsafe_op_in_unsafe_fn)]` workspace-wide
- **Zero clippy warnings**: enforced with `-D warnings`
- **Transient resilience**: up to 3 retries with exponential back-off on 5xx
- **RAII credential cleanup**: `GIT_ASKPASS` scripts removed by Drop on panic
- **Incremental S3 sync**: already-uploaded files are skipped via `HeadObject`

## Installation

### From source

```sh
cargo install --path crates/github-backup
```

### Docker

```sh
docker build -t github-backup .
docker run --rm -v /var/backup:/backup \
  -e GITHUB_TOKEN=ghp_xxx \
  github-backup octocat --output /backup --all
```

See [DOCKER.md](DOCKER.md) for full Docker and Docker Compose instructions.

## Usage

```
github-backup <OWNER> --token <TOKEN> [OPTIONS]
```

### Authentication

Provide a personal access token via `--token` or `GITHUB_TOKEN`, **or** use
the GitHub OAuth device flow:

```sh
# PAT (recommended for automation)
export GITHUB_TOKEN=ghp_xxxxxxxxxxxx
github-backup octocat --output /var/backup/github --all

# OAuth device flow (interactive, browser-based)
github-backup octocat --device-auth --oauth-client-id YOUR_APP_ID \
  --output /backup --all
```

For OAuth, create an OAuth App at <https://github.com/settings/developers>.
No callback URL is needed — the device flow does not use redirects.

### Clone types

```sh
# Mirror (default) — complete backup, all refs
github-backup octocat --token ghp_xxx --repositories --clone-type mirror

# Bare clone — no remote-tracking refs
github-backup octocat --token ghp_xxx --repositories --clone-type bare

# Full working tree
github-backup octocat --token ghp_xxx --repositories --clone-type full

# Shallow (last 10 commits per branch)
github-backup octocat --token ghp_xxx --repositories --clone-type shallow:10
```

### Mirror to Codeberg

```sh
github-backup octocat \
  --token ghp_xxx \
  --output /backup \
  --repositories \
  --mirror-to https://codeberg.org \
  --mirror-token CODEBERG_TOKEN \
  --mirror-owner your_username
```

### Sync to S3

```sh
github-backup octocat \
  --token ghp_xxx \
  --output /backup \
  --all \
  --s3-bucket my-github-backups \
  --s3-region us-east-1 \
  --s3-access-key "$AWS_ACCESS_KEY_ID" \
  --s3-secret-key "$AWS_SECRET_ACCESS_KEY"
```

### Sync to Backblaze B2

```sh
github-backup octocat \
  --token ghp_xxx \
  --output /backup \
  --all \
  --s3-bucket my-b2-bucket \
  --s3-region us-west-004 \
  --s3-endpoint https://s3.us-west-004.backblazeb2.com \
  --s3-access-key "$B2_KEY_ID" \
  --s3-secret-key "$B2_APP_KEY"
```

### Common options

```
AUTHENTICATION
  -t, --token <TOKEN>            GitHub PAT [env: GITHUB_TOKEN]
  --device-auth                  Use GitHub OAuth device flow
  --oauth-client-id <ID>         OAuth App client ID [env: GITHUB_OAUTH_CLIENT_ID]

OUTPUT
  -o, --output <DIR>             Root backup directory [default: .]

SELECTION
  --all                          Enable all backup categories
  --repositories                 Clone/mirror repositories
  --forks, -F                    Include forked repositories
  --private, -P                  Include private repositories
  --issues                       Back up issue metadata
  --issue-comments               Back up issue comment threads
  --issue-events                 Back up issue timeline events
  --pulls                        Back up pull requests
  --pull-comments                Back up PR review comments
  --pull-commits                 Back up PR commit lists
  --pull-reviews                 Back up PR reviews
  --labels                       Back up labels
  --milestones                   Back up milestones
  --releases                     Back up releases
  --release-assets               Download release binary assets
  --hooks                        Back up webhooks (requires admin scope)
  --security-advisories          Back up security advisories
  --wikis                        Clone wikis
  --starred                      Back up starred repositories
  --watched                      Back up watched repositories
  --followers                    Back up follower list
  --following                    Back up following list
  --gists                        Back up gists
  --starred-gists                Back up starred gists

CLONE OPTIONS
  --clone-type <TYPE>            mirror|bare|full|shallow:<depth> [default: mirror]
  --prefer-ssh                   Use SSH URLs for cloning
  --lfs                          Use Git LFS
  --no-prune                     Do not prune deleted remote refs

MIRROR (push to another git host after backup)
  --mirror-to <URL>              Gitea/Codeberg base URL
  --mirror-token <TOKEN>         API token for mirror [env: MIRROR_TOKEN]
  --mirror-owner <OWNER>         Owner at mirror destination
  --mirror-private               Create repos as private at mirror

S3 STORAGE (sync metadata to object store after backup)
  --s3-bucket <BUCKET>           S3 bucket name
  --s3-region <REGION>           AWS region [default: us-east-1]
  --s3-prefix <PREFIX>           Key prefix [default: ""]
  --s3-endpoint <URL>            Custom endpoint (B2, MinIO, R2, …)
  --s3-access-key <KEY>          Access key [env: AWS_ACCESS_KEY_ID]
  --s3-secret-key <SECRET>       Secret key [env: AWS_SECRET_ACCESS_KEY]
  --s3-include-assets            Also upload binary release assets

EXECUTION
  --concurrency <N>              Parallel repository limit [default: 4]
  --dry-run                      Log actions without writing files
  -q, --quiet                    Suppress non-error output
  -v, --verbose                  Increase verbosity (-v = debug, -vv = trace)
  --completions <SHELL>          Print shell completions (bash, zsh, fish, …)
```

### Output directory layout

```
<output>/
└── <owner>/
    ├── git/
    │   ├── repos/
    │   │   └── <repo-name>.git/       # bare mirror (or full clone without .git)
    │   ├── wikis/
    │   │   └── <repo-name>.wiki.git/
    │   └── gists/
    │       └── <gist-id>.git/
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

```
INFO backup complete repos=42 backed_up=40 skipped=1 errored=1 gists=5
INFO repos: 40 backed up, 1 skipped, 1 errored; gists: 5 backed up
INFO pushed=40 errored=0 "mirror push complete"
INFO uploaded=1200 skipped=50 errored=0 "S3 sync complete"
```

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `github-backup-types` | GitHub API response types + backup configuration |
| `github-backup-client` | Async HTTP client (hyper + rustls), pagination, rate-limit handling, 5xx retry, OAuth device flow |
| `github-backup-core` | Backup engine, `Storage` + `GitRunner` traits, `BackupStats` |
| `github-backup-mirror` | Push-mirror to Gitea/Codeberg/Forgejo via Gitea REST API v1 |
| `github-backup-s3` | S3/B2/MinIO storage backend with pure-Rust SigV4 signing |
| `github-backup` | CLI binary (clap) |

See [ARCHITECTURE.md](ARCHITECTURE.md) for a deep-dive into the design.

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo doc --no-deps --workspace
```

### MSRV

Minimum Supported Rust Version: **1.85**

## License

MIT — Copyright 2026 Tom F
