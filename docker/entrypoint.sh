#!/bin/sh
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F
#
# github-backup-rust container entrypoint.
#
# Goal: stay 100 % backwards-compatible with the existing
# `docker run … github-backup OWNER --all` invocation style while also
# supporting the env-var-only style that the Unraid Community
# Applications WebUI relies on.
#
# Behaviour:
#   * Any positional arguments → forwarded verbatim to `github-backup`.
#     This is what the CLI, docker-compose, and Kubernetes paths use.
#   * No positional arguments  → argv is reconstructed from a small set
#     of env vars that the Unraid template (or any other "fill the form,
#     hit Apply" launcher) sets:
#         GITHUB_OWNER    – positional OWNER  (required for a backup run)
#         BACKUP_MODE     – one of: --all | --doctor | --check |
#                                   --list-scopes | --verify | --tui
#         BACKUP_FLAGS    – free-form trailing flags
#                           e.g. "--org --concurrency 8 --since 2025-01-01T00:00:00Z"
#   * Both empty → print --help so users discover the CLI quickly.
#
# The script is deliberately POSIX shell (`/bin/sh`) so it runs unchanged
# on Alpine, Debian-slim, or any minimal base.

set -eu

# Strict: drop accidental setuid/setgid bits in /backup.
umask 022

BIN=/usr/local/bin/github-backup

# ── Path: explicit argv ───────────────────────────────────────────────
# Users who set `command:` in Compose, supply trailing args to
# `docker run`, or pass args via Kubernetes `args:` land here.  We do
# not interpret anything and just exec — this is the documented contract.
if [ "$#" -gt 0 ]; then
    exec "$BIN" "$@"
fi

# ── Path: env-var-driven argv (Unraid CA WebUI workflow) ──────────────
ARGS=""

if [ -n "${GITHUB_OWNER:-}" ]; then
    ARGS="$ARGS $GITHUB_OWNER"
fi

if [ -n "${BACKUP_MODE:-}" ]; then
    case "$BACKUP_MODE" in
        # Whitelist of supported modes.  Anything else falls through
        # silently as a flag — useful for forward compatibility with
        # future modes, but flagged unsafe values by quoting.
        --all|--doctor|--check|--list-scopes|--verify|--tui|--print-config-template)
            ARGS="$ARGS $BACKUP_MODE"
            ;;
        "")
            : ;;
        *)
            # Pass through unknown flag-shaped tokens; reject obvious
            # injection attempts (shell metacharacters).
            case "$BACKUP_MODE" in
                *[\;\|\&\`\$\(\)]*)
                    echo "github-backup entrypoint: refusing BACKUP_MODE with shell metacharacters" >&2
                    exit 64
                    ;;
                --*)
                    ARGS="$ARGS $BACKUP_MODE"
                    ;;
                *)
                    echo "github-backup entrypoint: BACKUP_MODE must start with -- (got '$BACKUP_MODE')" >&2
                    exit 64
                    ;;
            esac
            ;;
    esac
fi

if [ -n "${BACKUP_FLAGS:-}" ]; then
    # Refuse obvious shell-injection attempts — env vars set in a
    # public WebUI are easy to typo into something like
    # `--all && curl evil.example.com`.
    case "$BACKUP_FLAGS" in
        *[\;\`\$\(\)]*)
            echo "github-backup entrypoint: refusing BACKUP_FLAGS with shell metacharacters" >&2
            exit 64
            ;;
    esac
    ARGS="$ARGS $BACKUP_FLAGS"
fi

# Empty: print --help so the operator can read it inside the container.
if [ -z "$ARGS" ]; then
    exec "$BIN" --help
fi

# Intentional word-splitting on $ARGS so multi-token BACKUP_FLAGS works.
# shellcheck disable=SC2086
exec "$BIN" $ARGS
