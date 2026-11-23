#!/bin/sh
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F
#
# Install the github-backup binary from GitHub Releases (Linux and macOS).
#
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/install.sh | sh
#
# or, to read it first:
#
#   curl -fsSLO https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/install.sh
#   less install.sh
#   sh install.sh [--version X.Y.Z] [--dir DIR]
#
# Downloads the release binary for this OS and CPU, checks it against the
# release's SHA256SUMS.txt and installs it as DIR/github-backup (default
# ~/.local/bin).  No sudo: pick a DIR you can write to.  The same options can
# be given as GITHUB_BACKUP_VERSION and GITHUB_BACKUP_INSTALL_DIR.

set -eu

repo=tomtom215/github-backup-rust
version=${GITHUB_BACKUP_VERSION:-latest}
dir=${GITHUB_BACKUP_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}

die() {
    echo "install.sh: $*" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version) [ "$#" -ge 2 ] || die "--version needs a value"; version=$2; shift 2 ;;
        --dir)     [ "$#" -ge 2 ] || die "--dir needs a value"; dir=$2; shift 2 ;;
        -h|--help)
            echo "usage: install.sh [--version X.Y.Z] [--dir DIR]"
            exit 0
            ;;
        *) die "unknown argument: $1" ;;
    esac
done

case "$(uname -s)" in
    Linux)  os=linux ;;
    Darwin) os=macos ;;
    *) die "unsupported OS $(uname -s); on Windows use install.ps1" ;;
esac

case "$(uname -m)" in
    x86_64|amd64)  arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) die "no release binary for CPU $(uname -m); build from source instead" ;;
esac

asset="github-backup-$os-$arch"
version=${version#v}
if [ "$version" = latest ]; then
    base="https://github.com/$repo/releases/latest/download"
else
    base="https://github.com/$repo/releases/download/v$version"
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q --https-only -O "$2" "$1"; }
else
    die "needs curl or wget"
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    die "needs sha256sum or shasum to verify the download"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM

echo "Downloading $asset ($version) ..."
fetch "$base/$asset" "$tmp/$asset" || die "download failed: $base/$asset"
fetch "$base/SHA256SUMS.txt" "$tmp/SHA256SUMS.txt" || die "download failed: $base/SHA256SUMS.txt"

expected=$(awk -v f="$asset" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS.txt")
[ -n "$expected" ] || die "$asset is not listed in SHA256SUMS.txt"
actual=$(sha256 "$tmp/$asset")
[ "$actual" = "$expected" ] || die "checksum mismatch for $asset (expected $expected, got $actual)"
echo "Checksum OK."

mkdir -p "$dir"
chmod 0755 "$tmp/$asset"
mv -f "$tmp/$asset" "$dir/github-backup"
echo "Installed $dir/github-backup"
"$dir/github-backup" --version

case ":${PATH:-}:" in
    *":$dir:"*) ;;
    *) echo "Note: $dir is not on your PATH; add it, or run $dir/github-backup" ;;
esac
