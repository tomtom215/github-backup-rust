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

# File-creation mask for everything the backup writes.  Default 022 (dirs
# 0755, files 0644).  Unraid users who want shares writable by `nobody`
# over SMB can set `UMASK=000` (the community convention) - see unraid/README.md.
case "${UMASK:-022}" in
    [0-7][0-7][0-7]|[0-7][0-7][0-7][0-7]) umask "${UMASK:-022}" ;;
    *)
        echo "github-backup entrypoint: UMASK must be an octal mask like 022 (got '$UMASK')" >&2
        exit 64
        ;;
esac

# Drop set-but-empty optional variables.  Compose (`VAR=` in .env, or `${VAR-}`)
# and Unraid's DockerMan hand the container `NAME=` for every blank field.
# clap's `env = "..."` treats a *set but empty* variable as a value, so a blank
# BACKUP_ENCRYPT_KEY aborts the run ("must be exactly 64 hex characters") and a
# blank GITHUB_API_URL makes every API call fail.  Treat empty as unset.
# Variables with real values, and variables not listed here, are untouched.
for v in GITHUB_TOKEN GITHUB_API_URL GITHUB_CLONE_HOST GITHUB_OAUTH_CLIENT_ID \
         MIRROR_TOKEN AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY \
         BACKUP_ENCRYPT_KEY BACKUP_NOTIFY_WEBHOOK GITHUB_BACKUP_RESTORE_YES \
         HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy \
         NO_PROXY no_proxy; do
    eval "val=\${$v-__unset__}"
    if [ -z "$val" ]; then
        unset "$v"
    fi
done

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
# Pathname expansion is switched off so a pattern such as `rust-*` reaches the
# binary literally instead of being expanded against the working directory.
# Note there is no quote processing: write `--include-repos rust-*`, not
# `--include-repos 'rust-*'`.
set -f
# shellcheck disable=SC2086
exec "$BIN" $ARGS
