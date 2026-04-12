# github-backup

**github-backup** is a GitHub backup tool written in Rust. It backs up
repositories, issues, pull requests, releases, gists, wikis, and relationship
data for any GitHub user or organisation. The TLS stack is `rustls` (no
OpenSSL), the S3 client is a pure-Rust SigV4 implementation (no AWS SDK), and
an optional full-screen TUI is available via `--tui`.

## Feature Highlights

| Feature | Details |
|---------|---------|
| Repository backup | Mirror, bare, full, or shallow clone |
| Issue & PR backup | Full JSON: metadata, comments, reviews, events |
| Release backup | Metadata + optional binary asset download |
| Gist backup | Owned and starred gists |
| Wiki backup | Repository wiki clones |
| Topics & branches | Repository topics and branch list with protection status |
| Deploy keys & collaborators | Per-repository key and permission metadata |
| GitHub Actions | Workflow metadata + optional run history (`--actions`, `--action-runs`) |
| Environments | Deployment environments with protection rules (`--environments`) |
| Discussions / Projects / Packages | `--discussions`, `--projects`, `--packages` |
| User / org data | Starred, watched, followers, following, org members & teams |
| Repo filters | `--include-repos` / `--exclude-repos` glob patterns |
| Incremental | `--since` to limit issues/PR fetching by date |
| S3 sync | AWS S3, Backblaze B2, MinIO, Cloudflare R2, Spaces, Wasabi |
| At-rest encryption | AES-256-GCM before S3 upload (`--encrypt-key`) |
| Git mirroring | Push to Gitea / Codeberg / Forgejo or GitLab |
| Restore | Re-create labels, milestones, and issues in a target org |
| Authentication | Personal access token, OAuth device flow, or anonymous |
| GitHub Enterprise | `--api-url` + `--clone-host` for GHES instances |
| Config file | TOML config file with CLI override |
| Concurrency | Configurable parallel repository backup |
| Dry-run | Preview what would be backed up |
| JSON report | Machine-readable summary with counters and timestamps |
| Interactive TUI | Full-screen terminal interface with live progress (`--tui`) |
| Docker | Multi-stage Alpine image |

## Design Principles

- **No OpenSSL, no reqwest, no AWS SDK** — TLS via `rustls`, HTTP via `hyper`
- **`unsafe` is restricted to a single FFI call** (`kill(2)` for stale-lock detection); the workspace denies `unsafe_op_in_unsafe_fn`
- **Trait-based design** — `Storage`, `GitRunner`, and `BackupClient` traits enable unit tests without network or filesystem
- **RAII credential cleanup** — `GIT_ASKPASS` scripts are removed even on panic
- **Rate-limit aware** — automatic backoff on GitHub API rate limits
- **Pure-Rust SigV4** — AWS Signature V4 built from `sha2` + `hmac`

## Quick Links

- [Installation](getting-started/installation.md)
- [Quick Start](getting-started/quick-start.md)
- [Interactive TUI](tui.md)
- [CLI Reference](configuration/cli-reference.md)
- [Config File (TOML)](configuration/config-file.md)
- [Docker](docker.md)
- [Architecture](development/architecture.md)
- [GitHub Repository](https://github.com/tomtom215/github-backup-rust)
