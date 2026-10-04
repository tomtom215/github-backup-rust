# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F
#
# Multi-stage Docker build for github-backup-rust.
#
# Stage 1 (builder): compiles a static musl release binary using the official
#                    Rust image.  The Rust toolchain version must be at least
#                    the workspace `rust-version` declared in Cargo.toml.
# Stage 2 (runtime): minimal Alpine image with the binary, git, git-lfs and ssh.
#
# Usage:
#   docker build -t github-backup .
#   docker run --rm -v /var/backup:/backup \
#     -e GITHUB_TOKEN \
#     github-backup octocat --output /backup --all
#
# (`-e GITHUB_TOKEN` without a value forwards the variable from your shell, so
# the secret never appears in the docker client's argv or shell history.)
#
# Quick health-check of a fresh image:
#   docker run --rm -e GITHUB_TOKEN github-backup octocat --doctor

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

# Create stub source files so the dependency graph can be resolved *and
# compiled* before the real source code is copied.  Any `[[bench]]`, `[[bin]]`,
# `[[test]]`, or `[[example]]` targets declared in the manifests must also be
# stubbed out, otherwise Cargo refuses to parse the manifest.
#
# Building the stubs (not just `cargo fetch`) puts every third-party crate,
# including the C/asm build of aws-lc-sys, into this cached layer, so an edit
# to workspace source only recompiles the workspace crates.
RUN for crate in github-backup-types github-backup-client github-backup-core \
        github-backup-mirror github-backup-s3 github-backup-tui; do \
      mkdir -p crates/${crate}/src && \
      echo "" > crates/${crate}/src/lib.rs; \
    done && \
    mkdir -p crates/github-backup/src && \
    echo "fn main(){}" > crates/github-backup/src/main.rs && \
    mkdir -p crates/github-backup-types/benches && \
    echo "fn main(){}" > crates/github-backup-types/benches/glob.rs && \
    cargo build --release --locked --package github-backup

# Copy the real source and build the release binary.  The stub artefacts of
# the workspace crates are discarded first so a source file whose mtime is
# older than its stub's cannot be skipped by Cargo's freshness check.
# Third-party dependencies stay cached.
COPY . .

RUN cargo clean --release \
        --package github-backup --package github-backup-types \
        --package github-backup-client --package github-backup-core \
        --package github-backup-mirror --package github-backup-s3 \
        --package github-backup-tui && \
    cargo build --release --locked --package github-backup

# ── Optional stage: export the bare binary ───────────────────────────────────
# Used by the release workflow to produce the static Linux release binaries with
# exactly the toolchain that builds the image:
#   docker build --target export --output type=local,dest=out .
# It is not the last stage, so a plain `docker build .` still yields the runtime image.
FROM scratch AS export
COPY --from=builder /build/target/release/github-backup /github-backup

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM alpine:3.23 AS runtime

# OCI image metadata.  These propagate to GHCR / Docker Hub so Dependabot,
# Renovate, and humans can find the source from the image alone.
LABEL org.opencontainers.image.title="github-backup-rust" \
      org.opencontainers.image.description="GitHub backup tool: repositories, issues, PRs, releases, gists, wikis, and metadata. Rust, rustls + hyper (aws-lc-rs crypto), no OpenSSL, no AWS SDK." \
      org.opencontainers.image.url="https://github.com/tomtom215/github-backup-rust" \
      org.opencontainers.image.source="https://github.com/tomtom215/github-backup-rust" \
      org.opencontainers.image.documentation="https://tomtom215.github.io/github-backup-rust/" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.vendor="tomtom215" \
      org.opencontainers.image.base.name="alpine:3.23"

# Runtime dependencies:
# - git            : required for cloning / mirroring repositories.
# - git-lfs        : required by `--lfs` (the tool runs `git lfs clone/fetch`).
# - openssh-client : required by `--prefer-ssh` (git spawns `ssh`).  The image
#                    ships no key and no known_hosts: mount them and point
#                    GIT_SSH_COMMAND at them (see DOCKER.md).
# - ca-certificates: TLS CA bundle used by rustls-native-certs.
# - tini           : tiny init that reaps zombies and forwards SIGTERM to the
#                    backup process so `docker stop` and Kubernetes pod
#                    eviction terminate cleanly with the right exit code.
RUN apk add --no-cache git git-lfs openssh-client ca-certificates tini

# git refuses to operate on a repository owned by a different uid than the
# current user ("detected dubious ownership", git >= 2.35.2).  That happens
# whenever the backup volume was populated by another uid: a host-side run,
# `--user` changed between runs, a restore done as root, or Unraid's "New
# Permissions" tool (chown to nobody:users).  Every `remote update` would then
# fail for every repository.
#
# The protection exists so that git does not run config/hooks from a directory
# controlled by some *other, untrusted* user.  Here git is only ever pointed at
# repositories under the operator-mounted /backup volume (the documented and
# default output path), which the operator already trusts with a
# token-bearing process.  Trust is therefore scoped to that tree and is
# deliberately NOT the global wildcard '*'.  If you use another `--output`
# path, add it with:
#   -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory \
#   -e GIT_CONFIG_VALUE_0='/other/path/*'
RUN git config --system --add safe.directory /backup && \
    git config --system --add safe.directory '/backup/*'

# Non-root users.  1000:1000 is the default so bind-mounted host directories
# owned by the typical first user "just work".  99:100 (nobody:users) is the
# Unraid share owner; it gets a passwd entry too, because `ssh` refuses to run
# for a uid that has none.  Alpine already ships group 100 ("users").
RUN addgroup -S -g 1000 backup && \
    adduser  -S -u 1000 -G backup -h /home/backup backup && \
    adduser  -S -u 99 -G users -h /home/unraid unraid && \
    mkdir -p /home/backup/.ssh /home/unraid/.ssh && \
    chown backup:backup /home/backup /home/backup/.ssh && \
    chown unraid:users  /home/unraid /home/unraid/.ssh && \
    chmod 0700 /home/backup/.ssh /home/unraid/.ssh

# Copy the compiled binary.
COPY --from=builder \
    /build/target/release/github-backup \
    /usr/local/bin/github-backup

# Install the env-var-aware entrypoint wrapper.  CLI / Compose users
# who pass explicit positional args see no behavioural change; users
# whose launcher only sets env vars (Unraid Community Applications,
# generic web GUIs) get an argv reconstructed from `GITHUB_OWNER`,
# `BACKUP_MODE`, and `BACKUP_FLAGS`.  The wrapper also drops
# set-but-empty optional variables (see docker/entrypoint.sh).
COPY docker/entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh

# Default backup output directory (mount a volume here).
RUN mkdir -p /backup && chown 1000:1000 /backup

# Declare /backup as a volume so `docker inspect` shows where output lands.
VOLUME ["/backup"]

# Numeric on purpose: Kubernetes `runAsNonRoot: true` cannot verify a named user.
USER 1000:1000
WORKDIR /backup

# Default log level for cron-style runs.  Operators who want more detail
# can pass `-e RUST_LOG=debug` at run-time.  We deliberately do NOT set
# `NO_COLOR` here — the binary auto-detects TTY and the operator can
# always export it from the host (`-e NO_COLOR=1`) when piping to a file.
ENV RUST_LOG=info

# Makes tini act as a child subreaper when it is not PID 1 (for example when
# someone also passes `docker run --init` / Compose `init: true`).  That
# silences "Tini is not running as PID 1" and keeps zombie reaping working.
ENV TINI_SUBREAPER=1

# tini + the entrypoint wrapper makes signals (SIGTERM from
# `docker stop`) propagate correctly and the backup's checkpoint /
# lock cleanup runs.  The wrapper falls through to `github-backup`
# with either the supplied argv (CLI / Compose / Kubernetes) or one
# reconstructed from env vars (Unraid CA WebUI).
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
CMD []
