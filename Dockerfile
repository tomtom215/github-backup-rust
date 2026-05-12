# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F
#
# Multi-stage Docker build for github-backup-rust.
#
# Stage 1 (builder): compiles the release binary using the official Rust image.
#                    The Rust toolchain version must be at least the workspace
#                    `rust-version` declared in Cargo.toml.
# Stage 2 (runtime): minimal Alpine image with only the binary and git.
#
# Usage:
#   docker build -t github-backup .
#   docker run --rm -v /var/backup:/backup \
#     -e GITHUB_TOKEN=ghp_xxx \
#     github-backup octocat --output /backup --all
#
# Quick health-check of a fresh image:
#   docker run --rm -e GITHUB_TOKEN=ghp_xxx github-backup octocat --doctor

# ── Stage 1: Build ───────────────────────────────────────────────────────────
FROM rust:1.88-alpine AS builder

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
COPY crates/github-backup-tui/Cargo.toml     crates/github-backup-tui/Cargo.toml
COPY crates/github-backup/Cargo.toml         crates/github-backup/Cargo.toml

# Create stub source files so `cargo fetch` can resolve the dependency graph
# before the real source code is copied. Any `[[bench]]`, `[[bin]]`,
# `[[test]]`, or `[[example]]` targets declared in the manifests must also be
# stubbed out, otherwise Cargo refuses to parse the manifest.
RUN for crate in github-backup-types github-backup-client github-backup-core \
        github-backup-mirror github-backup-s3 github-backup-tui; do \
      mkdir -p crates/${crate}/src && \
      echo "" > crates/${crate}/src/lib.rs; \
    done && \
    mkdir -p crates/github-backup/src && \
    echo "fn main(){}" > crates/github-backup/src/main.rs && \
    mkdir -p crates/github-backup-types/benches && \
    echo "fn main(){}" > crates/github-backup-types/benches/glob.rs

RUN cargo fetch

# Copy real source and build the release binary.
COPY . .

RUN cargo build --release --package github-backup

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM alpine:3.21 AS runtime

# OCI image metadata.  These propagate to GHCR / Docker Hub so Dependabot,
# Renovate, and humans can find the source from the image alone.
LABEL org.opencontainers.image.title="github-backup-rust" \
      org.opencontainers.image.description="GitHub backup tool: repositories, issues, PRs, releases, gists, wikis, and metadata. Pure-Rust, rustls + hyper, no OpenSSL, no AWS SDK." \
      org.opencontainers.image.url="https://github.com/tomtom215/github-backup-rust" \
      org.opencontainers.image.source="https://github.com/tomtom215/github-backup-rust" \
      org.opencontainers.image.documentation="https://tomtom215.github.io/github-backup-rust/" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.vendor="tomtom215" \
      org.opencontainers.image.base.name="alpine:3.21"

# Runtime dependencies:
# - git           : required for cloning / mirroring repositories.
# - ca-certificates: TLS CA bundle used by rustls-native-certs.
# - tini          : tiny init that reaps zombies and forwards SIGTERM to the
#                   backup process so `docker stop` and Kubernetes pod
#                   eviction terminate cleanly with the right exit code.
RUN apk add --no-cache git ca-certificates tini

# Create a non-root user for running the backup.  Default UID/GID 1000 so
# bind-mounted host directories owned by the typical first user "just work".
RUN addgroup -S -g 1000 backup && \
    adduser  -S -u 1000 -G backup backup

# Copy the compiled binary.
COPY --from=builder \
    /build/target/release/github-backup \
    /usr/local/bin/github-backup

# Install the env-var-aware entrypoint wrapper.  CLI / Compose users
# who pass explicit positional args see no behavioural change; users
# whose launcher only sets env vars (Unraid Community Applications,
# generic web GUIs) get an argv reconstructed from `GITHUB_OWNER`,
# `BACKUP_MODE`, and `BACKUP_FLAGS`.
COPY docker/entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh

# Default backup output directory (mount a volume here).
RUN mkdir -p /backup && chown backup:backup /backup

# Declare /backup as a volume so `docker inspect` shows where output lands.
VOLUME ["/backup"]

USER backup
WORKDIR /backup

# Default log level for cron-style runs.  Operators who want more detail
# can pass `-e RUST_LOG=debug` at run-time.  We deliberately do NOT set
# `NO_COLOR` here — the binary auto-detects TTY and the operator can
# always export it from the host (`-e NO_COLOR=1`) when piping to a file.
ENV RUST_LOG=info

# tini + the entrypoint wrapper makes signals (SIGTERM from
# `docker stop`) propagate correctly and the backup's checkpoint /
# lock cleanup runs.  The wrapper falls through to `github-backup`
# with either the supplied argv (CLI / Compose / Kubernetes) or one
# reconstructed from env vars (Unraid CA WebUI).
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
CMD []
