# Installation

`github-backup` is distributed in three ways, in recommended order:

1. [Pre-built binary](#1-pre-built-binary) from the GitHub Releases page
2. [Docker / Docker Compose](#2-docker--docker-compose) from GHCR
3. [Source install via `cargo install --git`](#3-build-from-source)

> **Not on crates.io.** This project ships as an application, not a
> library. Every workspace crate is marked `publish = false`, so
> `cargo install github-backup` from the default registry will not
> work — use one of the three methods below instead.

## 1. Pre-built binary

Every release publishes statically-linkable binaries for five targets.
Each binary is uploaded alongside a `.sha256` checksum and signed with
[SLSA Level 2 build provenance](https://slsa.dev/).

| Target | Artefact |
|---|---|
| Linux, x86_64 (glibc) | `github-backup-linux-x86_64` |
| Linux, aarch64 (glibc) | `github-backup-linux-aarch64` |
| macOS, Intel | `github-backup-macos-x86_64` |
| macOS, Apple Silicon | `github-backup-macos-aarch64` |
| Windows, x86_64 | `github-backup-windows-x86_64.exe` |

```bash
# Linux x86_64 example
VERSION=0.3.2
TARGET=linux-x86_64

curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}"
curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}.sha256"

# Verify the SHA-256
sha256sum -c "github-backup-${TARGET}.sha256"

# Install into /usr/local/bin
install -m 0755 "github-backup-${TARGET}" /usr/local/bin/github-backup
```

For macOS, replace `sha256sum` with `shasum -a 256 -c`.

### Verify SLSA provenance (optional)

If you have the [GitHub CLI](https://cli.github.com/), you can verify
that the binary was built by this repository's release workflow:

```bash
gh attestation verify "github-backup-${TARGET}" \
  --repo tomtom215/github-backup-rust
```

## 2. Docker / Docker Compose

Multi-arch images (`linux/amd64`, `linux/arm64`) are published to GHCR
on every release under the repository name:

```bash
docker pull ghcr.io/tomtom215/github-backup-rust:latest
# or pin to a specific version
docker pull ghcr.io/tomtom215/github-backup-rust:0.3.2
```

### Ad-hoc run

```bash
docker run --rm \
  -e GITHUB_TOKEN=ghp_xxx \
  -v "$PWD/backups:/backup" \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all
```

### Docker Compose (recommended)

The repository ships a `docker-compose.yml` at the root with profiles
for local backups, AWS S3, Backblaze B2, self-hosted MinIO, and
Codeberg / Forgejo / Gitea mirroring. It reads secrets from a `.env`
file in the same directory.

```bash
# Clone (or just download docker-compose.yml and compose.example.env)
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust

# Copy the template and fill in your GITHUB_TOKEN (and any other
# credentials you need for the profile you plan to use).
cp compose.example.env .env
$EDITOR .env

# Local filesystem backup to ./backups/
docker compose run --rm backup octocat --all

# AWS S3 backup
docker compose --profile s3 run --rm backup-s3 octocat --all

# Codeberg mirror
docker compose --profile codeberg run --rm backup-codeberg octocat --all
```

The default `backup` service mounts:

| Host path | Container path | Purpose |
|---|---|---|
| `./backups` | `/backup` | backup output tree |
| `./config.toml` | `/etc/github-backup/config.toml` (ro) | optional config file |

If `./config.toml` does not exist, create an empty one or remove that
line from the service definition. See the
[Configuration → Config File](../configuration/config-file.md) chapter
for the full schema.

## 3. Build from source

Requires a Rust toolchain meeting the MSRV in `Cargo.toml` (currently
**1.88**). Install via [rustup](https://rustup.rs) if you don't have
one.

```bash
# Pin to a released tag (recommended)
cargo install --git https://github.com/tomtom215/github-backup-rust \
  --tag v0.3.2 \
  github-backup

# Or track main
cargo install --git https://github.com/tomtom215/github-backup-rust \
  github-backup
```

The binary lands in `$CARGO_HOME/bin` (by default `~/.cargo/bin`),
which a standard `rustup` install puts on your `$PATH`.

Alternatively, clone and build manually:

```bash
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust
cargo build --release -p github-backup
sudo install -m 0755 target/release/github-backup /usr/local/bin/
```

## Verify installation

```bash
github-backup --version
github-backup --help
```

## Shell completions

`github-backup` generates tab-completion scripts for all major shells
via `--completions <SHELL>`. Run the one-time setup below, then
**open a new terminal** (or source your shell's config file) to
activate completions.

### Bash

```bash
github-backup --completions bash >> ~/.bash_completion
```

If you use a distribution-managed completions directory you can
instead write to `/etc/bash_completion.d/github-backup` (requires
sudo).

### Zsh

```zsh
mkdir -p ~/.zfunc
github-backup --completions zsh > ~/.zfunc/_github-backup
```

Add the following lines to `~/.zshrc` **once** (before any `compinit`
call):

```zsh
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

Then reload: `exec zsh`.

### Fish

```fish
github-backup --completions fish > ~/.config/fish/completions/github-backup.fish
```

Fish auto-loads files from `~/.config/fish/completions/` — no further
configuration needed.

### PowerShell

```powershell
github-backup --completions powershell >> $PROFILE
```

Reload your profile with `. $PROFILE` or start a new PowerShell
session.

### Elvish

```elvish
github-backup --completions elvish > ~/.config/elvish/lib/github-backup.elv
```

Then add `use github-backup` to your `~/.config/elvish/rc.elv`.

## System requirements

| Requirement | Details |
|---|---|
| **OS** | Linux, macOS, Windows (x86_64, aarch64) |
| **Rust MSRV** | 1.88 (only required for source install) |
| **git** | Any recent version (`git` must be on `$PATH`) |
| **git-lfs** | Only required if using `--lfs` |
| **Disk space** | Depends on repository sizes |
