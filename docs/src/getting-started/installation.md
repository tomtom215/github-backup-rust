# Installation

`github-backup` is distributed in three ways:

1. [Pre-built binary](#1-pre-built-binary) from the GitHub Releases page
2. [Docker / Docker Compose](#2-docker--docker-compose) from GHCR
3. [Source install via `cargo install --git`](#3-build-from-source)

> **Not on crates.io.** This project ships as an application, not a
> library.  Every workspace crate is marked `publish = false`, so
> `cargo install github-backup` from the default registry will not work.

> **Which version does this documentation describe?**  It describes the
> current development branch (`main`).  The latest published release, v0.3.2,
> predates a large set of changes; among others it has no `--doctor`,
> `--check`, `--list-scopes`, `--print-config-template`, `--full`, no exit
> status `3` and no per-repository incremental state; in v0.3.2 the second and
> later runs applied the previous run's timestamp automatically, which could
> overwrite `issues.json` with only the issues that had changed.  The complete list is the `Unreleased`
> section of the [changelog](../development/changelog.md).  Until the next
> release is published, use a source build from `main` (below) to get what
> these pages describe.  `github-backup --version` prints the same number for
> a release and for a build from `main` before its version is bumped, so check
> the changelog if in doubt.

## 1. Pre-built binary

Every release publishes binaries for five targets, each with a `.sha256`
checksum file, plus a combined `SHA256SUMS.txt`:

| Target | Asset | Notes |
|---|---|---|
| Linux, x86_64 | `github-backup-linux-x86_64` | static (musl); runs on any distribution |
| Linux, aarch64 | `github-backup-linux-aarch64` | static (musl) |
| macOS, Intel | `github-backup-macos-x86_64` | |
| macOS, Apple Silicon | `github-backup-macos-aarch64` | |
| Windows, x86_64 | `github-backup-windows-x86_64.exe` | built by the release workflow; the project's CI does not run its tests on Windows |

(The static Linux binaries are built by the release workflow, which checks with
`file` and `ldd` that they have no dynamic dependencies.  The v0.3.2
Linux x86_64 binary, by contrast, was linked against glibc and needed glibc 2.39
or newer.)

```bash
# Linux x86_64 example: set VERSION to a version from the Releases page
VERSION=X.Y.Z
TARGET=linux-x86_64

curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}"
curl -LO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}.sha256"

# Verify the SHA-256
sha256sum -c "github-backup-${TARGET}.sha256"

# Install into /usr/local/bin
install -m 0755 "github-backup-${TARGET}" /usr/local/bin/github-backup
```

On macOS use `shasum -a 256 -c` instead of `sha256sum -c`.

### Verify build provenance (optional)

Releases published after v0.3.2 (the first built by the current release
workflow) attach a signed GitHub **build-provenance attestation** to each of the
five binaries.  With the [GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify "github-backup-${TARGET}" \
  --repo tomtom215/github-backup-rust
```

This proves that GitHub Actions built that file from this repository.  It
covers the five binaries only: not the `.sha256` files, `SHA256SUMS.txt` or the
container images (the images are not attested).  There is no SBOM, and v0.3.2 and
earlier releases have no attestation.  The checksum files only protect against
corrupted downloads; an attacker who can replace the binary can replace its
checksum, which is what the attestation is for.

## 2. Docker / Docker Compose

Multi-architecture images (`linux/amd64`, `linux/arm64`) are published to GHCR
for every release:

```bash
docker pull ghcr.io/tomtom215/github-backup-rust:latest   # latest stable release
docker pull ghcr.io/tomtom215/github-backup-rust:X.Y.Z    # a specific release
docker pull ghcr.io/tomtom215/github-backup-rust:X.Y      # latest patch of X.Y
```

(`latest` and `X.Y` are only moved by stable releases; a pre-release has just its
own tag.)  **Note:** `:latest` is the last *published release*, so until the next
release it is v0.3.2 and lacks the entrypoint wrapper, the diagnostics and the
other changes described in this documentation.  Build the image from `main`
(`docker build -t github-backup .`) to try them.

### Ad-hoc run

```bash
docker run --rm \
  -e GITHUB_TOKEN \
  -v "$PWD/backups:/backup" \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all
```

`-e GITHUB_TOKEN` (no value) passes the variable from your shell without
putting the token into the `docker` command line.

### Docker Compose

The repository ships a `docker-compose.yml` with profiles for local backups,
AWS S3, Backblaze B2, a MinIO or other S3-compatible server you run, and
Codeberg / Forgejo / Gitea and GitLab mirroring, plus `doctor`, `tui` and
`verify`.  It reads secrets from a `.env` file next to it.

```bash
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust

cp compose.example.env .env
chmod 600 .env
$EDITOR .env            # set GITHUB_TOKEN and what the profile you use needs

# Local filesystem backup to ./backups/
docker compose run --rm backup octocat --all

# AWS S3 backup
docker compose --profile s3 run --rm backup-s3 octocat --all

# Codeberg mirror
docker compose --profile codeberg run --rm backup-codeberg octocat --all
```

The default `backup` service mounts one volume, `./backups` (or `BACKUP_DIR`)
at `/backup`.  No config file is mounted; to use one, mount it for the run:

```bash
docker compose run --rm -v "$PWD/config.toml:/etc/github-backup/config.toml:ro" \
  backup --config /etc/github-backup/config.toml
```

The [Docker guide](../docker.md) has the details (user IDs, profiles, scheduling,
Kubernetes).

## 3. Build from source

Requires a Rust toolchain meeting the MSRV in `Cargo.toml` (currently **1.88**)
and `git` at run time.  Install via [rustup](https://rustup.rs).

```bash
# Track main (what this documentation describes)
cargo install --locked --git https://github.com/tomtom215/github-backup-rust \
  github-backup

# Or pin to a released tag (see the Releases page)
cargo install --locked --git https://github.com/tomtom215/github-backup-rust \
  --tag vX.Y.Z \
  github-backup
```

The binary lands in `$CARGO_HOME/bin` (by default `~/.cargo/bin`), which a
standard `rustup` install puts on your `$PATH`.

Or clone and build manually:

```bash
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust
cargo build --release --locked -p github-backup
sudo install -m 0755 target/release/github-backup /usr/local/bin/
```

(A glibc-linked build from source runs on the system it was built on.  For a
portable static binary, build with the Dockerfile's `export` stage:
`docker build --target export --output type=local,dest=out .`.)

## Verify installation

```bash
github-backup --version
github-backup --help
github-backup octocat --doctor      # checks git, the output directory, network and token
```

## Shell completions

`github-backup` generates tab-completion scripts for all major shells via
`--completions <SHELL>`.  Run the one-time setup below, then **open a new
terminal** (or source your shell's config file).

### Bash

```bash
github-backup --completions bash >> ~/.bash_completion
```

Or write to `/etc/bash_completion.d/github-backup` (needs sudo) if your
distribution loads completions from there.

### Zsh

```zsh
mkdir -p ~/.zfunc
github-backup --completions zsh > ~/.zfunc/_github-backup
```

Add these lines to `~/.zshrc` **once** (before any `compinit` call):

```zsh
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

Then reload: `exec zsh`.

### Fish

```fish
github-backup --completions fish > ~/.config/fish/completions/github-backup.fish
```

Fish loads files from `~/.config/fish/completions/` automatically.

### PowerShell

```powershell
github-backup --completions powershell >> $PROFILE
```

Reload your profile with `. $PROFILE` or start a new session.

### Elvish

```elvish
github-backup --completions elvish > ~/.config/elvish/lib/github-backup.elv
```

Then add `use github-backup` to `~/.config/elvish/rc.elv`.

## System requirements

| Requirement | Details |
|---|---|
| **OS** | Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64).  Linux and macOS are tested in CI; Windows is built but not tested there. |
| **git** | `git` on the `PATH`; `--doctor` warns below 2.20.  The container image includes it. |
| **git-lfs** | Only for `--lfs`.  The container image includes it. |
| **Rust** | MSRV 1.88, only for building from source |
| **Disk space** | Depends on the repositories; the tool does not check free space |
