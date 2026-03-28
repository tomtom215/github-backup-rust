# github-backup

**github-backup** is a comprehensive, production-ready GitHub backup tool written in Rust. It backs up repositories, issues, pull requests, releases, gists, wikis, and relationship data for any GitHub user or organisation — with zero OpenSSL dependencies, minimal transitive packages, and first-class S3 storage support.

## Feature Highlights

| Feature | Details |
|---------|---------|
| **Repository backup** | Mirror, bare, full, or shallow clone |
| **Issue & PR backup** | Full JSON: metadata, comments, reviews, events |
| **Release backup** | Metadata + optional binary asset download |
| **Gist backup** | Owned and starred gists |
| **Wiki backup** | Repository wiki clones |
| **User/org data** | Starred repos, watched repos, followers, following |
| **S3 sync** | AWS S3, Backblaze B2, MinIO, Cloudflare R2, Wasabi |
| **Git mirroring** | Push to Gitea, Codeberg, Forgejo |
| **Authentication** | Personal access token or OAuth device flow |
| **Config file** | TOML config file with CLI override |
| **Concurrency** | Configurable parallel repository backup |
| **Dry-run mode** | Preview what would be backed up |
| **Docker** | Multi-stage Alpine image ≈15 MB |

## Design Principles

- **Minimal dependencies** — 14 direct runtime crates; no OpenSSL, no reqwest
- **Zero unsafe code** — `#![deny(unsafe_op_in_unsafe_fn)]` workspace-wide
- **Trait-based design** — `Storage`, `GitRunner`, and `BackupClient` traits enable full unit testing without network or filesystem
- **RAII credential cleanup** — `GIT_ASKPASS` scripts are auto-deleted even on panic
- **Rate-limit aware** — automatic backoff on GitHub API rate limits
- **Pure-Rust SigV4** — AWS Signature V4 implemented from scratch (sha2 + hmac)

## Quick Links

- [Installation](getting-started/installation.md)
- [Quick Start](getting-started/quick-start.md)
- [CLI Reference](configuration/cli-reference.md)
- [Docker](docker.md)
- [Architecture](development/architecture.md)
- [GitHub Repository](https://github.com/tomtom215/github-backup-rust)
