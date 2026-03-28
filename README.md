# github-backup

[![CI](https://github.com/tomtom215/github-backup-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/tomtom215/github-backup-rust/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/github-backup.svg)](https://crates.io/crates/github-backup)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV: 1.85](https://img.shields.io/badge/MSRV-1.85-orange.svg)](Cargo.toml)

A comprehensive, production-ready GitHub backup tool written in Rust.

Backs up repositories (mirror / bare / full / shallow), issues, pull requests,
releases, gists, wikis, and relationship data for any GitHub user or
organisation — with zero OpenSSL dependencies and first-class S3 storage
support.

> **Full documentation** → **[tomtom215.github.io/github-backup-rust](https://tomtom215.github.io/github-backup-rust/)**

---

## Quick Start

```bash
# Install
cargo install --git https://github.com/tomtom215/github-backup-rust github-backup

# Back up everything for a user
export GITHUB_TOKEN=ghp_your_token_here
github-backup octocat --output /var/backup/github --all

# Or using Docker
docker run --rm \
  -e GITHUB_TOKEN="$GITHUB_TOKEN" \
  -v /var/backup/github:/backup \
  ghcr.io/tomtom215/github-backup:latest \
  octocat --output /backup --all
```

## Feature Summary

| Feature | Details |
|---------|---------|
| Repositories | Mirror, bare, full, or shallow clone |
| Issues & PRs | Full JSON: metadata, comments, reviews, events |
| Releases | Metadata + optional binary asset download |
| Gists | Owned and starred gists |
| Wikis | Repository wiki clones |
| User / org data | Starred, watched, followers, following |
| **S3 sync** | AWS S3, B2, MinIO, R2, Spaces, Wasabi |
| **Git mirroring** | Push to Gitea, Codeberg, Forgejo |
| **Auth** | PAT or OAuth device flow |
| **Config file** | TOML config with CLI override |
| **Concurrency** | Configurable parallel backup |
| **Dry-run** | Preview without writing |
| **Docker** | ~15 MB Alpine image |

## Design Principles

- **Minimal dependencies** — 14 direct runtime crates; no OpenSSL, no reqwest, no AWS SDK
- **Zero unsafe code** — `#![deny(unsafe_op_in_unsafe_fn)]` workspace-wide
- **RAII credential cleanup** — `GIT_ASKPASS` scripts auto-deleted even on panic
- **Pure-Rust SigV4** — S3 authentication without any AWS SDK
- **Rate-limit aware** — automatic backoff on GitHub API limits

## Documentation

The full documentation is in the **[GitHub Book](https://tomtom215.github.io/github-backup-rust/)**:

| Topic | Link |
|-------|------|
| Installation | [docs/installation](https://tomtom215.github.io/github-backup-rust/getting-started/installation.html) |
| Quick Start | [docs/quick-start](https://tomtom215.github.io/github-backup-rust/getting-started/quick-start.html) |
| Authentication | [docs/authentication](https://tomtom215.github.io/github-backup-rust/getting-started/authentication.html) |
| Backup Categories | [docs/backup-categories](https://tomtom215.github.io/github-backup-rust/backup-categories.html) |
| CLI Reference | [docs/cli-reference](https://tomtom215.github.io/github-backup-rust/configuration/cli-reference.html) |
| Config File | [docs/config-file](https://tomtom215.github.io/github-backup-rust/configuration/config-file.html) |
| S3 Storage | [docs/s3](https://tomtom215.github.io/github-backup-rust/storage/s3.html) |
| Mirroring | [docs/mirroring](https://tomtom215.github.io/github-backup-rust/mirroring.html) |
| Docker | [docs/docker](https://tomtom215.github.io/github-backup-rust/docker.html) |
| Architecture | [docs/architecture](https://tomtom215.github.io/github-backup-rust/development/architecture.html) |

## Common Examples

```bash
# Config file (recommended for repeated use)
github-backup --config /etc/github-backup/config.toml

# Organisation backup with 8 parallel workers
github-backup my-org --token $GITHUB_TOKEN --output /backup --org --all --concurrency 8

# S3 sync after backup
github-backup octocat --token $GITHUB_TOKEN --output /backup --all \
  --s3-bucket my-bucket --s3-region us-east-1

# Mirror to Codeberg
github-backup octocat --token $GITHUB_TOKEN --output /backup --repositories \
  --mirror-to https://codeberg.org --mirror-token $CODEBERG_TOKEN --mirror-owner alice

# Write a JSON summary report
github-backup octocat --token $GITHUB_TOKEN --output /backup --all \
  --report /var/log/github-backup-report.json
```

## Workspace Layout

```
crates/
├── github-backup-types/     Pure data types (GitHub API models, config)
├── github-backup-client/    Async GitHub API client (hyper + rustls)
├── github-backup-core/      Backup engine, Storage and GitRunner traits
├── github-backup-mirror/    Gitea push-mirror integration
├── github-backup-s3/        S3-compatible storage (pure-Rust SigV4)
└── github-backup/           CLI binary (clap)
docs/                        mdBook documentation source
```

## License

MIT — see [LICENSE](LICENSE).
