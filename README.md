# github-backup

[![CI](https://github.com/tomtom215/github-backup-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/tomtom215/github-backup-rust/actions/workflows/ci.yml)
[![Book](https://github.com/tomtom215/github-backup-rust/actions/workflows/pages.yml/badge.svg)](https://tomtom215.github.io/github-backup-rust/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV: 1.88](https://img.shields.io/badge/MSRV-1.88-orange.svg)](Cargo.toml)

A GitHub backup tool written in Rust.

Backs up the repositories (mirror / bare / full / shallow), issues, pull
requests, releases, gists, wikis, topics, branches and relationship data of any
GitHub user or organisation.  Uses `rustls` (no OpenSSL), a pure-Rust S3 SigV4
client (no AWS SDK), and ships an optional full-screen interactive TUI.

> **Full documentation** → **[tomtom215.github.io/github-backup-rust](https://tomtom215.github.io/github-backup-rust/)**
>
> **Version note.**  This README and the documentation describe the current
> development branch (`main`).  The latest release, v0.3.2, predates many of the
> behaviours described here (exit status `3`, per-repository incremental state,
> lossless JSON, `--doctor`, ...); the `Unreleased` section of
> [CHANGELOG.md](CHANGELOG.md) lists them.  To get a matching build before the
> next release, use [option 4](#4-build-from-source) below.

---

## Install

`github-backup` is **not published to crates.io**.  Releases ship as pre-built
binaries on GitHub Releases and as multi-arch container images on GHCR; you can
also install from source with Cargo.

### 1. Pre-built binary

Download the binary for your platform from the
[**Releases page**](https://github.com/tomtom215/github-backup-rust/releases),
verify the SHA-256 checksum and put it on your `$PATH`:

```bash
# Linux x86_64 example: set VERSION to a version from the Releases page
VERSION=X.Y.Z
TARGET=linux-x86_64
curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}"
curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}.sha256"
sha256sum -c "github-backup-${TARGET}.sha256"
install -m 0755 "github-backup-${TARGET}" /usr/local/bin/github-backup
```

Targets: `linux-x86_64`, `linux-aarch64`, `macos-x86_64`, `macos-aarch64`,
`windows-x86_64.exe`.  The release workflow builds the Linux binaries as static
(musl) executables with no glibc requirement (the v0.3.2 x86_64 binary was
glibc-linked and needs glibc 2.39 or newer).  Windows is built but its tests are
not run in CI.

Releases after v0.3.2 attach a signed GitHub **build-provenance attestation**
to each of the five binaries (not to the checksum files or the container
images); verify one with:

```bash
gh attestation verify "github-backup-${TARGET}" \
  --repo tomtom215/github-backup-rust
```

### 2. Docker / Docker Compose

Multi-arch images (`linux/amd64`, `linux/arm64`) are published to GHCR for every
release (`latest`, `X.Y.Z`, `X.Y`).  `:latest` is the last *published release*;
until the next release it lacks the behaviour described here, so build from
source for now (`docker build -t github-backup .`).

```bash
docker run --rm \
  -e GITHUB_TOKEN \
  -v "$PWD/backups:/backup" \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all
```

(`-e GITHUB_TOKEN` forwards the variable from your shell without putting the
token on the command line.)  For a persistent setup use the bundled Compose file:

```bash
cp compose.example.env .env
$EDITOR .env                                # set GITHUB_TOKEN etc.
docker compose run --rm backup octocat --all
```

Compose profiles: `s3`, `b2`, `minio` (an S3 server you run), `codeberg`,
`gitlab`, `doctor`, `tui` and `verify`.  See [DOCKER.md](DOCKER.md).

### 3. Unraid (Community Applications)

A Community Applications template is bundled at `unraid/github-backup.xml`.  In
the Unraid WebUI add the template URL under *Docker → Add Container → Template
URL*:

```
https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/unraid/github-backup.xml
```

The template exposes the important options as form fields (masked token, output
volume, run mode, GHES URLs, encryption key, webhook); the image's entrypoint
wrapper turns them into command-line arguments.  Schedule recurring runs with the
User Scripts plugin; see [unraid/README.md](unraid/README.md).  Add the template
by URL as shown above.

### 4. Build from source

Needs a Rust toolchain meeting the MSRV in `Cargo.toml` (**1.88**) and `git`:

```bash
# track main
cargo install --locked --git https://github.com/tomtom215/github-backup-rust github-backup

# or pin a released tag
cargo install --locked --git https://github.com/tomtom215/github-backup-rust \
  --tag vX.Y.Z github-backup
```

The binary lands in `$CARGO_HOME/bin` (by default `~/.cargo/bin`).

## Quick Start

```bash
export GITHUB_TOKEN=ghp_your_token_here

# Check git, the output directory, the network and the token
github-backup octocat --output /var/backup/github --doctor

# Back up everything (private repositories and secret gists of your own account included)
github-backup octocat --output /var/backup/github --all

# Or launch the interactive TUI
github-backup octocat --tui
```

The exit status tells you whether the backup is complete: `0` yes, `3` it
finished but something could not be backed up (the summary lists it), `1` it
could not run.  Run it again to retry only what failed.

## Interactive TUI

Pass `--tui` to launch a full-screen terminal interface built with [Ratatui](https://ratatui.rs) 0.30.

```
 github-backup v0.3.2  [1]Dashboard  [2]Configure  [3]Run  [4]Verify  [5]Results
┌──────────────────────────────────────────────────────────────────────────────┐
│  Owner          octocat                                                      │
│  Output dir     /var/backup/github                                           │
│  Token          ghp_****...****                                              │
│  Last run       2026-03-29 08:14 UTC  (312 repos)                            │
│                                                                              │
│  > Start backup                                                              │
│    Verify integrity                                                          │
│    Configure                                                                 │
└──────────────────────────────────────────────────────────────────────────────┘
 j/k select   Enter run   q quit
```

### TUI Screens

| Screen | Key | Purpose |
|--------|-----|---------|
| Dashboard | `1` | Overview of last run; launch backup or verify |
| Configure | `2` | Edit all 50+ settings across 8 tabbed panels |
| Run | `3` | Live progress: gauge, repo list, log panel |
| Verify | `4` | Integrity check against stored manifests |
| Results | `5` | Post-run statistics table |

### Global Keys

| Key | Action |
|-----|--------|
| `1`–`5` | Switch screens |
| `q` / `Ctrl+C` | Quit (cancel running backup first) |
| `Tab` / `Shift+Tab` | Cycle focus within a screen |
| `Enter` | Confirm / activate |
| `Esc` | Cancel / dismiss modal |

### Configure Screen Keys

| Key | Action |
|-----|--------|
| `h` / `l` or `←` / `→` | Previous / next tab |
| `j` / `k` or `↑` / `↓` | Move field cursor |
| `Enter` | Edit text field / toggle boolean |
| `Esc` | Commit field edit |
| `A` (categories tab) | Select all / deselect all |
| `< >` | Cycle select field options |

### Run Screen Keys

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll repo list |
| `g` / `G` | Scroll log panel top / bottom |
| `Ctrl+C` | Cancel running backup |

## Feature Summary

| Feature | Details |
|---------|---------|
| Interactive TUI | Full-screen Ratatui interface with live progress (`--tui`) |
| Repositories | Mirror, bare, full or shallow clone, updated with `git fetch`; optional LFS objects (`--lfs`) |
| Issues and pull requests | Lists, comments, events, commits and reviews as GitHub's complete JSON |
| Releases | Metadata plus optional asset download (streamed, verified) |
| Gists and wikis | Owned gists and wikis cloned; starred gists as metadata |
| Metadata | Topics, branches and protection rules, labels, milestones, hooks, deploy keys, collaborators, advisories, Actions, environments, packages |
| User and org data | Starred, watched, followers, following, org members and teams |
| Repo filters | `--include-repos` / `--exclude-repos` glob patterns on repository names |
| Incremental | Per-repository watermarks; lists always fetched in full and merged; `--full` to refresh everything |
| Outcome | Failures never stop the run; exit status `3` = incomplete; report, metrics, webhook and history agree |
| S3 sync | JSON metadata (not clones) to AWS S3, B2, MinIO, R2, Spaces, Wasabi |
| At-rest encryption | AES-256-GCM of the S3 uploads (`--encrypt-key`) |
| Git mirroring | Branches and tags pushed to Gitea, Codeberg, Forgejo or GitLab |
| Restore | Labels, milestones and issues re-created from the local backup (`--restore`) |
| Auth | Personal access token, OAuth device flow, or anonymous |
| GitHub Enterprise | `--api-url` and `--clone-host` for GHES |
| Config file | TOML (the command line wins for single values; switches and lists combine) |
| Dry run | `--dry-run` lists what would be backed up and writes nothing |
| Monitoring | JSON report, Prometheus textfile metrics, webhook, run history |

Not backed up: **Discussions and Projects** (no REST endpoints; the flags do
nothing), Actions secrets, code-scanning and Dependabot alerts, issue
attachments, and repository clones in S3.  See
[the limits](https://tomtom215.github.io/github-backup-rust/restore.html#what-is-not-restored-or-not-backed-up).

## Design Principles

- **No OpenSSL, no reqwest, no AWS SDK**: TLS via `rustls`, HTTP via `hyper`
- **No `unsafe` code** in the workspace's own source
- **Credentials stay out of argument lists, URLs and files**: git gets the token
  through an environment variable and a host-scoped credential helper
- **Pure-Rust SigV4**: S3 authentication built from `sha2` + `hmac`
- **Rate-limit aware**: rate-limit responses are waited out and retried
- **Honest**: an incomplete backup is reported as incomplete

## Documentation

The full documentation is in the **[GitHub Book](https://tomtom215.github.io/github-backup-rust/)**:

| Topic | Link |
|-------|------|
| Introduction | [introduction](https://tomtom215.github.io/github-backup-rust/introduction.html) |
| Installation | [getting-started/installation](https://tomtom215.github.io/github-backup-rust/getting-started/installation.html) |
| Quick Start | [getting-started/quick-start](https://tomtom215.github.io/github-backup-rust/getting-started/quick-start.html) |
| TUI Guide | [tui](https://tomtom215.github.io/github-backup-rust/tui.html) |
| Authentication | [getting-started/authentication](https://tomtom215.github.io/github-backup-rust/getting-started/authentication.html) |
| Backup Categories | [backup-categories](https://tomtom215.github.io/github-backup-rust/backup-categories.html) |
| Output Layout | [configuration/output-layout](https://tomtom215.github.io/github-backup-rust/configuration/output-layout.html) |
| CLI Reference and Exit Codes | [configuration/cli-reference](https://tomtom215.github.io/github-backup-rust/configuration/cli-reference.html) |
| Config File | [configuration/config-file](https://tomtom215.github.io/github-backup-rust/configuration/config-file.html) |
| S3 Storage | [storage/s3](https://tomtom215.github.io/github-backup-rust/storage/s3.html) |
| At-Rest Encryption | [storage/encryption](https://tomtom215.github.io/github-backup-rust/storage/encryption.html) |
| Mirroring | [mirroring](https://tomtom215.github.io/github-backup-rust/mirroring.html) |
| Monitoring and Incremental Runs | [monitoring](https://tomtom215.github.io/github-backup-rust/monitoring.html) |
| Restore | [restore](https://tomtom215.github.io/github-backup-rust/restore.html) |
| Operations Runbook | [ops-runbook](https://tomtom215.github.io/github-backup-rust/ops-runbook.html) |
| Docker | [docker](https://tomtom215.github.io/github-backup-rust/docker.html) |
| systemd / cron | [deployment/systemd](https://tomtom215.github.io/github-backup-rust/deployment/systemd.html), [deployment/cron](https://tomtom215.github.io/github-backup-rust/deployment/cron.html) |
| FAQ | [faq](https://tomtom215.github.io/github-backup-rust/faq.html) |
| Security | [development/security](https://tomtom215.github.io/github-backup-rust/development/security.html) |
| Troubleshooting | [development/troubleshooting](https://tomtom215.github.io/github-backup-rust/development/troubleshooting.html) |
| Architecture | [development/architecture](https://tomtom215.github.io/github-backup-rust/development/architecture.html) |
| Changelog | [development/changelog](https://tomtom215.github.io/github-backup-rust/development/changelog.html) |

## Common Examples

The examples read the token from `GITHUB_TOKEN`; export it first.

```bash
# GitHub Enterprise Server (standard)
github-backup myorg \
  --api-url https://github.example.com/api/v3 \
  --output /backup --org --all

# GitHub Enterprise Server (split API / clone hostnames)
github-backup myorg \
  --api-url https://github-api.example.com/api/v3 \
  --clone-host github-git.example.com \
  --output /backup --org --repositories

# Config file (recommended for repeated use)
github-backup --config /etc/github-backup/config.toml

# Organisation backup with 8 parallel workers
github-backup my-org --output /backup --org --all --concurrency 8

# S3 sync of the JSON metadata after the backup
github-backup octocat --output /backup --all \
  --s3-bucket my-bucket --s3-region us-east-1

# Push mirror to Codeberg (MIRROR_TOKEN holds the Codeberg token)
github-backup octocat --output /backup --repositories \
  --mirror-to https://codeberg.org --mirror-owner alice

# JSON summary report
github-backup octocat --output /backup --all \
  --report /var/log/github-backup-report.json

# Only repositories whose name starts with "rust-"
github-backup octocat --output /backup --repositories \
  --include-repos "rust-*"

# Preview without writing anything
github-backup octocat --output /backup --all --dry-run
```

Runs are incremental on their own: run the same command again and unchanged
issues and pull requests are not re-fetched.  `--full` forces a complete
refresh.

## Shell Completions

`github-backup` ships built-in tab completion for every major shell.  Run the
one-time setup for your shell, then restart your session:

```bash
# Bash: append to the system completion file (or your own ~/.bash_completion)
github-backup --completions bash >> ~/.bash_completion

# Zsh: write to a fpath directory, then rebuild the completion cache
mkdir -p ~/.zfunc
github-backup --completions zsh > ~/.zfunc/_github-backup
# Add to ~/.zshrc if not already present:
#   fpath=(~/.zfunc $fpath)
#   autoload -Uz compinit && compinit

# Fish
github-backup --completions fish > ~/.config/fish/completions/github-backup.fish

# PowerShell: append to your profile
github-backup --completions powershell >> $PROFILE

# Elvish
github-backup --completions elvish > ~/.config/elvish/lib/github-backup.elv
```

Once installed, `github-backup <Tab>` completes flags and enum values (for
example `--mirror-type`).

## Workspace Layout

```
crates/
├── github-backup-types/     GitHub API models, Raw<T>/Page<T>, configuration
├── github-backup-client/    Async GitHub API client (hyper + rustls)
├── github-backup-core/      Backup engine, Storage and GitRunner traits
├── github-backup-mirror/    Push mirrors to Gitea / Forgejo / Codeberg / GitLab
├── github-backup-s3/        S3-compatible storage (pure-Rust SigV4), encryption
├── github-backup-tui/       Ratatui TUI front-end (--tui flag)
└── github-backup/           CLI binary (clap)
docs/                        mdBook documentation source
```

## License

MIT, see [LICENSE](LICENSE).
