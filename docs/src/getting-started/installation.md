# Installation

`github-backup` is distributed in three ways:

1. [Pre-built binary](#1-pre-built-binary) from the GitHub Releases page
2. [Docker / Docker Compose](#2-docker--docker-compose) from GHCR
3. [Source install via `cargo install --git`](#3-build-from-source)

> **Not on crates.io.** This project ships as an application, not a
> library.  Every workspace crate is marked `publish = false`, so
> `cargo install github-backup` from the default registry will not work.

> **Which version does this documentation describe?**  The current
> development branch (`main`).  A feature added since the latest release is
> listed in the `Unreleased` section of the
> [changelog](../development/changelog.md); use a build from `main` (below) to
> get it before the next release.

## 1. Pre-built binary

Every release publishes binaries for five targets, each with a `.sha256`
checksum file, plus a combined `SHA256SUMS.txt`:

| Target | Asset | Notes |
|---|---|---|
| Linux, x86_64 | `github-backup-linux-x86_64` | static (musl); runs on any distribution |
| Linux, aarch64 | `github-backup-linux-aarch64` | static (musl) |
| macOS, Intel | `github-backup-macos-x86_64` | |
| macOS, Apple Silicon | `github-backup-macos-aarch64` | |
| Windows, x86_64 | `github-backup-windows-x86_64.exe` | also runs on Windows on Arm (emulated) |

(The static Linux binaries are built by the release workflow, which checks with
`file` and `ldd` that they have no dynamic dependencies.)

### Install script (Linux, macOS)

`install.sh` picks the binary for your OS and CPU, checks it against the
release's `SHA256SUMS.txt`, and installs it as `~/.local/bin/github-backup`
(no `sudo`):

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/install.sh | sh
```

To read the script before running it (recommended), download it first:

```bash
curl -fsSLO https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/install.sh
less install.sh
sh install.sh                          # latest release into ~/.local/bin
sh install.sh --version X.Y.Z --dir ~/bin
```

`--version` and `--dir` can also be given as the environment variables
`GITHUB_BACKUP_VERSION` and `GITHUB_BACKUP_INSTALL_DIR`.  If the directory is
not on your `PATH`, the script says so.

### Install script (Windows)

In PowerShell (no administrator rights needed):

```powershell
irm https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/install.ps1 | iex
```

or download `install.ps1`, read it, and run
`.\install.ps1 -Version X.Y.Z -Dir C:\Tools\github-backup` (if the execution
policy refuses a downloaded script, run
`powershell -ExecutionPolicy Bypass -File .\install.ps1`).  The default
directory is `%LOCALAPPDATA%\Programs\github-backup`; the script prints the
command that adds it to your user `PATH`.  (With `| iex`, set
`$env:GITHUB_BACKUP_VERSION` / `$env:GITHUB_BACKUP_INSTALL_DIR` instead of the
parameters.)

### cargo-binstall

With [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) the release
binary is downloaded instead of compiled (other targets fall back to a source
build):

```bash
cargo binstall --git https://github.com/tomtom215/github-backup-rust github-backup
```

cargo-binstall does not check the release checksums.

### Manual download

```bash
# Linux x86_64 example: set VERSION to a version from the Releases page
VERSION=X.Y.Z
TARGET=linux-x86_64

curl -fLO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}"
curl -fLO "https://github.com/tomtom215/github-backup-rust/releases/download/v${VERSION}/github-backup-${TARGET}.sha256"

# Verify the SHA-256 (on macOS: shasum -a 256 -c ...)
sha256sum -c "github-backup-${TARGET}.sha256"

# Install into /usr/local/bin (or any directory on your PATH)
install -m 0755 "github-backup-${TARGET}" /usr/local/bin/github-backup
```

On Windows, compare the hash in PowerShell:

```powershell
(Get-FileHash .\github-backup-windows-x86_64.exe -Algorithm SHA256).Hash
Get-Content .\github-backup-windows-x86_64.exe.sha256
```

### Unsigned binaries: macOS and Windows warnings

The binaries are not code-signed or notarised (their integrity is covered by the
checksums and the attestation below instead), so the operating system may warn
about a file downloaded with a **web browser**.  The install scripts and `curl`
downloads do not trigger these warnings.

- **macOS Gatekeeper** ("cannot be opened because the developer cannot be
  verified"): after verifying the checksum, remove the quarantine attribute:
  `xattr -d com.apple.quarantine github-backup-macos-aarch64` (or the x86_64
  file).
- **Windows SmartScreen** ("Windows protected your PC"): after verifying the
  checksum, choose *More info* → *Run anyway*, or unblock the file once with
  `Unblock-File .\github-backup-windows-x86_64.exe`.

### Verify build provenance (optional)

Every release from v0.4.0 on attaches a signed GitHub **build-provenance
attestation** to each of the five binaries and to `SHA256SUMS.txt`.  With the [GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify "github-backup-${TARGET}" \
  --repo tomtom215/github-backup-rust
```

This proves that GitHub Actions built that file from this repository.  The
container images are attested too (see [Docker](#2-docker--docker-compose)).
The `.sha256` files only protect against corrupted downloads; an attacker who
can replace a binary can replace its checksum, which is what the attestation is
for.  Releases before v0.4.0 have no attestation.

## 2. Docker / Docker Compose

Multi-architecture images (`linux/amd64`, `linux/arm64`) are published to GHCR:

```bash
docker pull ghcr.io/tomtom215/github-backup-rust:latest   # latest stable release
docker pull ghcr.io/tomtom215/github-backup-rust:X.Y.Z    # a specific release
docker pull ghcr.io/tomtom215/github-backup-rust:X.Y      # latest patch of X.Y
docker pull ghcr.io/tomtom215/github-backup-rust:edge     # built from main
```

`edge` (and `sha-<commit>`) is rebuilt from `main` whenever the code or the
image changes; it matches this documentation but is not a release.  `latest`
and `X.Y` are only moved by stable releases; a pre-release has just its own tag.
Pin a release (`:X.Y` or `:X.Y.Z`) for unattended production runs.

Release images from v0.4.0 on and every `edge` image carry an SBOM and
build provenance, and a GitHub attestation you can check with
`gh attestation verify oci://ghcr.io/tomtom215/github-backup-rust:TAG --repo tomtom215/github-backup-rust`.
A release image contains exactly the attested Linux release binary.

### Ad-hoc run

```bash
docker run --rm \
  -e GITHUB_TOKEN \
  -v "$PWD/backups:/backup" \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup
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
                        # (GITHUB_BACKUP_TAG selects the image tag; default latest)

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

Requires a Rust toolchain meeting the MSRV in `Cargo.toml` (currently **1.89**)
and `git` at run time.  Install via [rustup](https://rustup.rs).  (Inside a
clone of the repository, rustup switches to the toolchain pinned in
`rust-toolchain.toml` and downloads it if needed; `cargo install --git` uses
your default toolchain.)

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
`docker build --target export --output type=local,dest=out .`; it writes
`out/github-backup`.)

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
| **OS** | Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64).  The test suite runs in CI on Linux, macOS and Windows.  On Windows a name that is a reserved device name (`con`, `aux`, `nul`, `com1`, `lpt1`, ..., also with an extension) cannot be stored: such a repository or gist is reported as a failure of the run, and such a release tag or asset file name is stored as `_`. |
| **git** | `git` on the `PATH`; `--doctor` warns below 2.20.  The container image includes it. |
| **git-lfs** | Only for `--lfs`.  The container image includes it. |
| **Rust** | MSRV 1.89, only for building from source |
| **Disk space** | Depends on the repositories; the tool does not check free space |
