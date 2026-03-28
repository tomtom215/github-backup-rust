# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F
#
# Multi-stage Docker build for github-backup-rust.
#
# Stage 1 (builder): compiles the release binary using the official Rust image.
# Stage 2 (runtime): minimal Alpine image with only the binary and git.
#
# Usage:
#   docker build -t github-backup .
#   docker run --rm -v /var/backup:/backup \
#     -e GITHUB_TOKEN=ghp_xxx \
#     github-backup octocat --output /backup --all

# ── Stage 1: Build ───────────────────────────────────────────────────────────
FROM rust:1.85-alpine AS builder

# Build dependencies
RUN apk add --no-cache musl-dev pkgconf

WORKDIR /build

# Cache dependencies by copying manifests first.
COPY Cargo.toml Cargo.lock ./
COPY crates/github-backup-types/Cargo.toml   crates/github-backup-types/Cargo.toml
COPY crates/github-backup-client/Cargo.toml  crates/github-backup-client/Cargo.toml
COPY crates/github-backup-core/Cargo.toml    crates/github-backup-core/Cargo.toml
COPY crates/github-backup-mirror/Cargo.toml  crates/github-backup-mirror/Cargo.toml
COPY crates/github-backup-s3/Cargo.toml      crates/github-backup-s3/Cargo.toml
COPY crates/github-backup/Cargo.toml         crates/github-backup/Cargo.toml

# Create placeholder source files so `cargo fetch` can resolve the dependency
# graph without the actual source code.
RUN for crate in github-backup-types github-backup-client github-backup-core \
        github-backup-mirror github-backup-s3; do \
      mkdir -p crates/${crate}/src && \
      echo "// placeholder" > crates/${crate}/src/lib.rs; \
    done && \
    mkdir -p crates/github-backup/src && \
    echo "fn main(){}" > crates/github-backup/src/main.rs

RUN cargo fetch

# Copy real source and build the release binary.
COPY . .

RUN cargo build --release --package github-backup

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM alpine:3.21 AS runtime

# git is required for cloning/mirroring repositories.
# ca-certificates provides the TLS CA bundle used by rustls-native-certs.
RUN apk add --no-cache git ca-certificates

# Create a non-root user for running the backup.
RUN addgroup -S backup && adduser -S backup -G backup

# Copy the compiled binary.
COPY --from=builder \
    /build/target/release/github-backup \
    /usr/local/bin/github-backup

# Default backup output directory (mount a volume here).
RUN mkdir -p /backup && chown backup:backup /backup

USER backup
WORKDIR /backup

ENTRYPOINT ["github-backup"]
CMD ["--help"]
