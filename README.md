# github-backup

[![CI](https://github.com/tomtom215/github-backup-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/tomtom215/github-backup-rust/actions/workflows/ci.yml)
[![Book](https://github.com/tomtom215/github-backup-rust/actions/workflows/pages.yml/badge.svg)](https://tomtom215.github.io/github-backup-rust/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV: 1.88](https://img.shields.io/badge/MSRV-1.88-orange.svg)](Cargo.toml)

A GitHub backup tool written in Rust.

Backs up repositories (mirror / bare / full / shallow), issues, pull requests,
releases, gists, wikis, topics, branches, and relationship data for any GitHub
user or organisation. Uses `rustls` (no OpenSSL), pure-Rust S3 SigV4 (no AWS
SDK), and ships an optional full-screen interactive TUI.

> **Full documentation** → **[tomtom215.github.io/github-backup-rust](https://tomtom215.github.io/github-backup-rust/)**

---

## Install

`github-backup` is **not published to crates.io**. Releases ship as
pre-built binaries on GitHub Releases, multi-arch container images on
GHCR, and source install from Git. Pick whichever suits your host.

### 1. Pre-built binary (recommended)

Download the latest binary for your platform from the
[**Releases page**](https://github.com/tomtom215/github-backup-rust/releases),
verify the SHA-256 checksum, and place the binary on your `$PATH`.

```bash
# Linux x86_64 example (replace VERSION and target as needed)
VERSION=0.3.2
TARGET=linux-x86_64
curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}"
curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}.sha256"
sha256sum -c "github-backup-${TARGET}.sha256"
install -m 0755 "github-backup-${TARGET}" /usr/local/bin/github-backup
```

Supported targets: `linux-x86_64`, `linux-aarch64`, `macos-x86_64`,
`macos-aarch64`, `windows-x86_64.exe`. The Linux binaries are static (musl)
and run on any distribution.  From v0.3.3 each binary carries a signed
GitHub build-provenance attestation ([SLSA v1.0 Build Level 2](https://slsa.dev/)
as GitHub documents it; earlier releases have none); verify with:

```bash
gh attestation verify "github-backup-${TARGET}" \
  --repo tomtom215/github-backup-rust
```

### 2. Docker / Docker Compose

Multi-arch images (`linux/amd64`, `linux/arm64`) are published to GHCR
on every release.

```bash
docker pull ghcr.io/tomtom215/github-backup-rust:latest

docker run --rm \
  -e GITHUB_TOKEN="$GITHUB_TOKEN" \
  -v "$PWD/backups:/backup" \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all
```

For a persistent, reproducible setup, use the bundled Compose file:

```bash
cp compose.example.env .env
$EDITOR .env                                # set GITHUB_TOKEN etc.
docker compose run --rm backup octocat --all
```

The Compose file ships profiles for S3 (`--profile s3`), Backblaze B2
(`--profile b2`), a self-hosted MinIO / S3-compatible server (`--profile minio`, you run the
server), Codeberg / Forgejo / Gitea mirroring (`--profile
codeberg`), GitLab mirroring (`--profile gitlab`), pre-flight checks
(`--profile doctor`), the interactive TUI (`--profile tui`), and
integrity verification (`--profile verify`).  See `docker-compose.yml`
for the full list.

### 3. Unraid (Community Applications)

A Community Applications template is bundled at `unraid/github-backup.xml`.
Inside the Unraid WebUI, add the template URL under *Settings →
Community Applications → Settings → Add Container → Template URL*:

```
https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/unraid/github-backup.xml
```

The template exposes every important option as a form field (token
masked, output volume, run mode dropdown, GHES URLs, encryption key,
webhook), and the image's wrapper entrypoint auto-builds argv from
those env vars while keeping the regular `docker run … OWNER --all`
invocation working unchanged.  Schedule recurring backups via the
User Scripts plugin — see `unraid/README.md` for the full walkthrough.

### 4. Build from source

Requires a Rust toolchain meeting the MSRV in `Cargo.toml`
(currently **1.88**).

```bash
cargo install --git https://github.com/tomtom215/github-backup-rust \
  --tag v0.3.2 \
  github-backup
```

Omit `--tag` to track `main`. The binary lands in `$CARGO_HOME/bin`
(by default `~/.cargo/bin`), which should already be on your `$PATH`
after a standard `rustup` install.

## Quick Start

```bash
# Launch the interactive TUI (recommended first-run experience)
export GITHUB_TOKEN=ghp_your_token_here
github-backup octocat --tui

# Or run non-interactively
github-backup octocat --output /var/backup/github --all
```

## Interactive TUI

Pass `--tui` to launch a full-screen terminal interface built with [Ratatui](https://ratatui.rs) 0.30.
Flags given with `--tui` (`--all`, `--private`, `--dry-run`, `--org`, `--full`, `--since`,
`--concurrency`, `--manifest`, category flags, ...) pre-fill the Configure screen.

```
 github-backup v0.3.2  [1]Dashboard  [2]Configure  [3]Run  [4]Verify  [5]Results
┌ Status ──────────────────────────────────────────────────────────────────────┐
│Owner:  octocat                        Last run: 2026-10-03T23:51:37Z         │
│Output: /var/backup/github             Repos:    312                          │
│Token:  configured                     Result:   INCOMPLETE (1 failure)       │
└──────────────────────────────────────────────────────────────────────────────┘
┌ Actions ─────────────────────────────────────────────────────────────────────┐
│> [r] Run Backup                                                              │
│  [c] Configure                                                               │
│  [v] Verify Integrity                                                        │
│  [q] Quit                                                                    │
└──────────────────────────────────────────────────────────────────────────────┘
r run backup  q quit  c configure  v verify  j/k move  Enter select  1-5 screens
```

### TUI Screens

| Screen | Key | Purpose |
|--------|-----|---------|
| Dashboard | `1` | Last run (with result and failure count); start a backup or verify |
| Configure | `2` | Six tabs: Auth, Target (incl. **Full backup**), Categories, Clone, Filter, Output |
| Run | `3` | Live progress; shows `INCOMPLETE: N failed` as soon as something fails |
| Verify | `4` | Integrity check of the JSON manifest (not the cloned repositories) |
| Results | `5` | `COMPLETE` / `INCOMPLETE` / `CANCELLED` / `FAILED`, counters, scrollable list of failures (scope, step, message) |

A run in which anything failed is always reported as **INCOMPLETE**, never as complete.

### Keys

| Key | Action |
|-----|--------|
| `1`-`5` | Switch screens (not while a backup runs or a field is edited) |
| `Ctrl+C` | Cancel the running backup; elsewhere quit |
| `Tab` / `Shift+Tab` | Configure: next / previous tab |
| `j` / `k` | Move (fields, lists) |
| `Space` / `Enter` | Toggle / edit a field |
| `Enter` / `Esc` while editing | Save / discard the edit |
| `A` (Categories) | Select all, or none if all are on |
| `s` / `F5` | Configure: start the backup |
| `g` / `G`, `PgUp` / `PgDn` | Run: log oldest / newest, scroll |
| `r` | Results: run again |
| `Esc` | Back to the Dashboard (Configure, Verify, Results) |

### What the TUI cannot do

Mirror push, S3 sync, JSON report, Prometheus metrics, webhook notification,
device-flow sign-in, `--config` files and `--clone-host` are command-line only;
the Output tab says so.  See the [TUI guide](docs/src/tui.md) for details.

## Feature Summary

| Feature | Details |
|---------|---------|
| Interactive TUI | Full-screen Ratatui interface with live progress (`--tui`) |
| Repositories | Mirror, bare, full, or shallow clone |
| Issues & PRs | Full JSON: metadata, comments, reviews, events |
| Releases | Metadata + optional binary asset download |
| Gists | Owned and starred gists |
| Wikis | Repository wiki clones |
| Topics & branches | Repository tags and branch list with protection status |
| Deploy keys & collaborators | Per-repository key and permission metadata |
| GitHub Actions | Workflow metadata + optional run history (`--actions`, `--action-runs`) |
| Environments | Deployment environments with protection rules (`--environments`) |
| Discussions / Projects / Packages | `--discussions`, `--projects`, `--packages` |
| User / org data | Starred, watched, followers, following, org members, teams |
| Repo filters | `--include-repos` / `--exclude-repos` glob patterns |
| Incremental | `--since` to limit issues/PR fetching by date |
| S3 sync | AWS S3, B2, MinIO, R2, Spaces, Wasabi |
| At-rest encryption | AES-256-GCM encryption before S3 upload (`--encrypt-key`) |
| Git mirroring | Push to Gitea, Codeberg, Forgejo, or GitLab |
| Restore mode | Re-create labels, milestones, and issues in target org (`--restore`) |
| Auth | PAT or OAuth device flow |
| GitHub Enterprise | `--api-url` + `--clone-host` for GHES instances |
| Config file | TOML config with CLI override |
| Concurrency | Configurable parallel backup |
| Dry-run | Preview without writing |
| Report | JSON summary with duration, counters, timestamps |
| Docker | Multi-stage Alpine image |

## Design Principles

- **No OpenSSL, no reqwest, no AWS SDK** — TLS via `rustls`, HTTP via `hyper`
- **`unsafe` is restricted to a single FFI call** (`kill(2)` for stale-lock detection); the workspace denies `unsafe_op_in_unsafe_fn`
- **RAII credential cleanup** — `GIT_ASKPASS` scripts are removed even on panic
- **Pure-Rust SigV4** — S3 authentication implemented from `sha2` + `hmac`
- **Rate-limit aware** — automatic backoff on GitHub API limits

## Documentation

The full documentation is in the **[GitHub Book](https://tomtom215.github.io/github-backup-rust/)**:

| Topic | Link |
|-------|------|
| Installation | [getting-started/installation](https://tomtom215.github.io/github-backup-rust/getting-started/installation.html) |
| Quick Start | [getting-started/quick-start](https://tomtom215.github.io/github-backup-rust/getting-started/quick-start.html) |
| TUI Guide | [tui](https://tomtom215.github.io/github-backup-rust/tui.html) |
| Authentication | [getting-started/authentication](https://tomtom215.github.io/github-backup-rust/getting-started/authentication.html) |
| Backup Categories | [backup-categories](https://tomtom215.github.io/github-backup-rust/backup-categories.html) |
| CLI Reference | [configuration/cli-reference](https://tomtom215.github.io/github-backup-rust/configuration/cli-reference.html) |
| Config File | [configuration/config-file](https://tomtom215.github.io/github-backup-rust/configuration/config-file.html) |
| S3 Storage | [storage/s3](https://tomtom215.github.io/github-backup-rust/storage/s3.html) |
| At-Rest Encryption | [storage/encryption](https://tomtom215.github.io/github-backup-rust/storage/encryption.html) |
| Restore from Backup | [development/restore](https://tomtom215.github.io/github-backup-rust/development/restore.html) |
| Mirroring | [mirroring](https://tomtom215.github.io/github-backup-rust/mirroring.html) |
| Monitoring & Reporting | [monitoring](https://tomtom215.github.io/github-backup-rust/monitoring.html) |
| Docker | [docker](https://tomtom215.github.io/github-backup-rust/docker.html) |
| Security | [development/security](https://tomtom215.github.io/github-backup-rust/development/security.html) |
| Troubleshooting | [development/troubleshooting](https://tomtom215.github.io/github-backup-rust/development/troubleshooting.html) |
| Architecture | [development/architecture](https://tomtom215.github.io/github-backup-rust/development/architecture.html) |

## Common Examples

```bash
# GitHub Enterprise Server (standard)
github-backup myorg --token $GITHUB_TOKEN \
  --api-url https://github.example.com/api/v3 \
  --output /backup --org --all

# GitHub Enterprise Server (split API / clone hostnames)
github-backup myorg --token $GITHUB_TOKEN \
  --api-url https://github-api.example.com/api/v3 \
  --clone-host github-git.example.com \
  --output /backup --org --repositories
```


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

# Only back up repos starting with "rust-"
github-backup octocat --token $GITHUB_TOKEN --output /backup --repositories \
  --include-repos "rust-*"

# Incremental: only fetch issues/PRs updated since last run
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --issues --pulls --since "2026-01-01T00:00:00Z"
```

## Shell Completions

`github-backup` ships built-in tab completion for every major shell.
Run the one-time setup for your shell, then restart your session:

```bash
# Bash — append to the system completion file (or your own ~/.bash_completion)
github-backup --completions bash >> ~/.bash_completion

# Zsh — write to a fpath directory, then rebuild the completion cache
mkdir -p ~/.zfunc
github-backup --completions zsh > ~/.zfunc/_github-backup
# Add to ~/.zshrc if not already present:
#   fpath=(~/.zfunc $fpath)
#   autoload -Uz compinit && compinit

# Fish
github-backup --completions fish > ~/.config/fish/completions/github-backup.fish

# PowerShell — append to your profile
github-backup --completions powershell >> $PROFILE

# Elvish
github-backup --completions elvish > ~/.config/elvish/lib/github-backup.elv
```

Once installed, `github-backup <Tab>` completes flags, sub-commands, and enum values (e.g. `--mirror-type`, `--completions <shell>`).

## Workspace Layout

```
crates/
├── github-backup-types/     Pure data types (GitHub API models, config)
├── github-backup-client/    Async GitHub API client (hyper + rustls)
├── github-backup-core/      Backup engine, Storage and GitRunner traits
├── github-backup-mirror/    Gitea push-mirror integration
├── github-backup-s3/        S3-compatible storage (pure-Rust SigV4)
├── github-backup-tui/       Ratatui TUI front-end (--tui flag)
└── github-backup/           CLI binary (clap)
docs/                        mdBook documentation source
```

## License

MIT — see [LICENSE](LICENSE).
