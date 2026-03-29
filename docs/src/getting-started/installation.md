# Installation

## Pre-built Binaries

Download a pre-built binary from the [GitHub Releases page](https://github.com/tomtom215/github-backup-rust/releases):

```bash
# Linux (x86_64)
curl -sSL https://github.com/tomtom215/github-backup-rust/releases/latest/download/github-backup-x86_64-unknown-linux-musl.tar.gz \
  | tar xz && sudo mv github-backup /usr/local/bin/

# macOS (Apple Silicon)
curl -sSL https://github.com/tomtom215/github-backup-rust/releases/latest/download/github-backup-aarch64-apple-darwin.tar.gz \
  | tar xz && sudo mv github-backup /usr/local/bin/
```

## From Source (Cargo)

Requires Rust 1.85 or later. Install Rust via [rustup](https://rustup.rs):

```bash
cargo install --git https://github.com/tomtom215/github-backup-rust github-backup
```

Or clone and build locally:

```bash
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust
cargo build --release
sudo cp target/release/github-backup /usr/local/bin/
```

## Docker

Pull and run without installing anything locally:

```bash
docker pull ghcr.io/tomtom215/github-backup:latest

docker run --rm \
  -e GITHUB_TOKEN=ghp_xxx \
  -v /var/backup/github:/backup \
  ghcr.io/tomtom215/github-backup:latest \
  octocat --output /backup --all
```

See the [Docker guide](../docker.md) for compose examples, scheduled backups, and S3 integration.

## Verify Installation

```bash
github-backup --version
# github-backup 0.2.0

github-backup --help
```

## Shell Completions

`github-backup` generates tab-completion scripts for all major shells via `--completions <SHELL>`.
Run the one-time setup below, then **open a new terminal** (or source your shell's config file) to activate completions.

### Bash

```bash
github-backup --completions bash >> ~/.bash_completion
```

If you use a distribution-managed completions directory you can instead write to `/etc/bash_completion.d/github-backup` (requires sudo).

### Zsh

```zsh
mkdir -p ~/.zfunc
github-backup --completions zsh > ~/.zfunc/_github-backup
```

Add the following lines to `~/.zshrc` **once** (before any `compinit` call):

```zsh
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

Then reload: `exec zsh`.

### Fish

```fish
github-backup --completions fish > ~/.config/fish/completions/github-backup.fish
```

Fish auto-loads files from `~/.config/fish/completions/` — no further configuration needed.

### PowerShell

```powershell
github-backup --completions powershell >> $PROFILE
```

Reload your profile with `. $PROFILE` or start a new PowerShell session.

### Elvish

```elvish
github-backup --completions elvish > ~/.config/elvish/lib/github-backup.elv
```

Then add `use github-backup` to your `~/.config/elvish/rc.elv`.

## System Requirements

| Requirement | Details |
|------------|---------|
| **OS** | Linux, macOS, Windows (x86_64, aarch64) |
| **Rust MSRV** | 1.85 |
| **git** | Any recent version (`git` must be on `$PATH`) |
| **git-lfs** | Only required if using `--lfs` |
| **Disk space** | Depends on repository sizes |
