# Docker Guide

The repository ships a multi-stage `Dockerfile`.  The runtime image is based on
`alpine:3.23` and contains the `github-backup` binary (a static musl build),
`git`, `git-lfs`, `openssh-client`, the CA bundle and `tini`.  It is **not** a
minimal or distroless image: it has a shell (`/bin/sh`) and the Alpine package
manager.  Its size has not been measured for the current Dockerfile; build it
and run `docker images` if it matters to you.

> **Version note.**  Multi-architecture images (`linux/amd64`, `linux/arm64`)
> are published to GHCR for every release: `latest` (most recent stable
> release), `X.Y.Z` and `X.Y`.  `:latest` is the last **published release**; the
> entrypoint wrapper, `--doctor` and the other behaviour described on this page
> are not in v0.3.2.  Until the next release, build the image from the
> repository (`docker build -t github-backup .`) and use that tag in the
> examples below.  Pin a version in production instead of `:latest`.

```
ghcr.io/tomtom215/github-backup-rust:latest     # latest stable release
ghcr.io/tomtom215/github-backup-rust:X.Y.Z      # an exact release
ghcr.io/tomtom215/github-backup-rust:X.Y        # latest patch of X.Y
```

> Running on **Unraid**?  A Community Applications template is bundled: see
> [unraid/README.md](https://github.com/tomtom215/github-backup-rust/blob/main/unraid/README.md).
> It uses the same image; the rest of this guide still applies.

## Quick Start

```sh
# Pull the image (or build it: docker build -t github-backup .)
docker pull ghcr.io/tomtom215/github-backup-rust:latest

# Check the setup first: git, output directory, network, token
docker run --rm -e GITHUB_TOKEN \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --doctor

# Run a backup
docker run --rm -e GITHUB_TOKEN \
  -v "$PWD/backups:/backup" \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all
```

`-e GITHUB_TOKEN` without a value forwards the variable from your shell; the
token then never appears in the `docker` command line or your shell history
(`-e GITHUB_TOKEN=$GITHUB_TOKEN` would put it into the process list of the host).
The container's working directory is `/backup`, and `--output` defaults to the
working directory, so `--output /backup` is optional.

The container exit status is the tool's: `0` complete, `3` finished but
incomplete (something could not be backed up), `1` could not run, `143` stopped
by `docker stop`.  See [Exit Codes](https://tomtom215.github.io/github-backup-rust/configuration/cli-reference.html#exit-codes).

## Users and Permissions

The image runs as the numeric user `1000:1000` (`backup`).  A second user,
`99:100` (`nobody:users`, the owner of Unraid shares), also has an account in
the image because `ssh` refuses to run for a UID without one.  The `/backup`
volume is owned by `1000:1000`.

* A **bind mount** must be writable by the user the container runs as.  Either
  `chown 1000:1000` the host directory, or run as the directory's owner:
  `docker run --user "$(id -u):$(id -g)" ...` (Compose: `BACKUP_UID` and
  `BACKUP_GID` in `.env`).
* Files get the mode implied by `umask` 022 by default.  Set `-e UMASK=000`
  (octal, three or four digits) if other users must write into the share.
* `git` refuses repositories owned by another user ("dubious ownership").  The
  tool trusts exactly the path it works on, and the image trusts `/backup` and
  `/backup/*` system-wide, so a volume that was populated by another UID still
  updates.  With another `--output` path add it:
  `-e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0='/other/path/*'`.
* On **Kubernetes** set `securityContext.runAsUser: 1000` and
  `fsGroup: 1000`, otherwise most volume drivers hand you a directory the user
  cannot write.

## How Arguments Reach the Binary

The entrypoint is `tini` followed by a small POSIX shell wrapper
(`docker/entrypoint.sh`):

* **Arguments given** (`docker run IMAGE octocat --all`, Compose `command:`,
  Kubernetes `args:`): passed to `github-backup` unchanged.
* **No arguments**: the wrapper builds them from `GITHUB_OWNER` (the owner),
  `BACKUP_MODE` (one of `--all`, `--doctor`, `--check`, `--list-scopes`,
  `--verify`, `--tui`, `--print-config-template`, or another flag starting
  with `--`) and `BACKUP_FLAGS` (more flags, split on spaces: quotes are not
  interpreted, so write `--include-repos rust-*` and not
  `--include-repos 'rust-*'`).  This is the Unraid path.
* **No arguments and none of those variables**: `github-backup --help`.
* The wrapper refuses `;`, backticks, `$` and parentheses in `BACKUP_FLAGS` and
  shell metacharacters in `BACKUP_MODE`, and a malformed `UMASK`, with exit
  status `64`.
* A variable that is **set but empty** (`VAR=`, which Compose and Unraid pass for
  every blank field) is removed for these: `GITHUB_TOKEN`, `GITHUB_API_URL`,
  `GITHUB_CLONE_HOST`, `GITHUB_OAUTH_CLIENT_ID`, `MIRROR_TOKEN`,
  `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `BACKUP_ENCRYPT_KEY`,
  `BACKUP_NOTIFY_WEBHOOK`, `GITHUB_BACKUP_RESTORE_YES` and the proxy variables.

## Docker Compose

`docker-compose.yml` defines one service per scenario; the non-default ones sit
behind `--profile`.  Copy the template once:

```sh
cp compose.example.env .env
chmod 600 .env
$EDITOR .env            # at least GITHUB_TOKEN
```

Fixed flags of a profile (`--s3-bucket ...`) are part of the service's
`entrypoint:`, so whatever you add after the service name is **appended**:

### Profile matrix

| Profile | Service | What it does |
|---------|---------|--------------|
| _default_ | `backup` | Local backup, output under `./backups/` (or `BACKUP_DIR`) |
| `doctor` | `doctor` | Run the `--doctor` pre-flight checks only |
| `tui` | `tui` | The interactive TUI (needs a terminal: use `run`, not `up`) |
| `verify` | `verify` | Verify the SHA-256 manifest of an existing backup (`--manifest` must have been used) |
| `s3` | `backup-s3` | Backup + sync of the JSON metadata to AWS S3 |
| `b2` | `backup-b2` | Backup + sync to Backblaze B2 |
| `minio` | `backup-minio` | Backup + sync to a MinIO or other S3-compatible server **you run** (`MINIO_ENDPOINT`) |
| `codeberg` | `backup-codeberg` | Backup + mirror push to Codeberg / Forgejo / Gitea |
| `gitlab` | `backup-gitlab` | Backup + mirror push to GitLab.com or self-managed GitLab |

```sh
docker compose run --rm backup octocat --all                          # local
docker compose --profile doctor run --rm doctor octocat              # pre-flight
docker compose --profile tui run --rm tui octocat                    # TUI
docker compose --profile verify run --rm verify octocat              # verify
docker compose --profile s3 run --rm backup-s3 octocat --all         # S3
docker compose --profile b2 run --rm backup-b2 octocat --all         # B2
docker compose --profile minio run --rm backup-minio octocat --all   # MinIO
docker compose --profile codeberg run --rm backup-codeberg octocat --all
docker compose --profile gitlab run --rm backup-gitlab octocat --all
```

Notes:

* The compose file does not bundle an S3 server (`MINIO_ENDPOINT` points at one
  you run, with the bucket created beforehand; the tool never creates buckets).
  Do not publish a MinIO server's ports on all interfaces with default
  credentials.
* Only the JSON metadata goes to S3, and only metadata reaches B2/MinIO; the
  clones stay in `./backups/`.  To copy the code off-site use the mirror
  profiles or a file-level tool.
* Compose does not forward `HTTPS_PROXY`; add `HTTPS_PROXY:` to the `x-env`
  block if the container needs a proxy (a host-side `127.0.0.1` proxy is not
  reachable from the container).
* A config file is not mounted by default; mount it for the run:

  ```sh
  docker compose run --rm -v "$PWD/config.toml:/etc/github-backup/config.toml:ro" \
    backup --config /etc/github-backup/config.toml
  ```

## Environment Variables

Everything in `compose.example.env` is also accepted by a plain `docker run`:

| Variable | Purpose |
|----------|---------|
| `GITHUB_TOKEN` | GitHub personal access token |
| `GITHUB_API_URL`, `GITHUB_CLONE_HOST` | GitHub Enterprise Server API URL and clone host |
| `GITHUB_OAUTH_CLIENT_ID` | OAuth App client ID for `--device-auth` |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` | S3 / B2 / MinIO credentials (used only with `--s3-bucket`) |
| `MIRROR_TOKEN` | Token for the mirror destination (used only with `--mirror-to`) |
| `BACKUP_ENCRYPT_KEY` | 64 hex characters: AES-256-GCM key for the S3 upload |
| `BACKUP_NOTIFY_WEBHOOK` | URL that receives the JSON status of each run |
| `GITHUB_BACKUP_RESTORE_YES` | `1` confirms `--restore` without a prompt |
| `GITHUB_OWNER`, `BACKUP_MODE`, `BACKUP_FLAGS`, `UMASK` | Read by the entrypoint wrapper only (see above) |
| `RUST_LOG` | Log filter; the image sets `info` |
| `NO_COLOR`, `CLICOLOR_FORCE` | Disable / force colour |
| `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` | Proxy for the GitHub API, release assets and the webhook (HTTP proxies only; S3 and the mirror API ignore them; `git` reads them itself) |

Compose forwards only the variables of the `x-env` block of `docker-compose.yml` and
the credentials of each profile; add any other variable (for example
`AWS_SESSION_TOKEN` or a proxy) to that block.

## Security Notes

* **Non-root**: the image runs as `1000:1000` (numeric, so Kubernetes
  `runAsNonRoot: true` can verify it).
* **A shell is present**: this is an Alpine image with `apk`, `sh` and `ssh`.
  Nothing is removed from it.
* **Secrets**: pass them as environment variables (`-e GITHUB_TOKEN`,
  `--env-file`, Compose `.env`, Kubernetes secrets), never baked into the image
  or on a command line.  The tool hands the token to `git` through an
  environment variable and a host-scoped credential helper; it is not written to
  a file and not placed in any argument list.
* **SSH**: `--prefer-ssh` needs keys and a `known_hosts` file; the image ships
  none.  Mount them and point `GIT_SSH_COMMAND` at them.
* **tini as PID 1** forwards `SIGTERM` (`docker stop`, pod eviction) to the
  backup, which stops its `git` processes, releases its locks and exits with `143`
  within a few seconds, well inside Docker's default 10 s grace period.  The
  checkpoint it leaves lets the next run resume (within 6 hours).
* **Atomic outputs**: reports, metrics and JSON files are written to a temporary
  name and renamed, so a monitor never reads a half-written file.
* **Not covered**: the image is not signed or attested (only the release
  binaries are), and it has not been scanned by the project.

## Scheduled Backups

The image is a one-shot tool: it runs to completion and exits.  Use a scheduler
that starts a new container per run.

### Plain cron

```sh
# /etc/cron.d/github-backup   (the token comes from a 0600 env file)
0 2 * * * backup docker run --rm --env-file /etc/github-backup/env \
  -v /var/backup/github:/backup \
  ghcr.io/tomtom215/github-backup-rust:X.Y.Z \
  octocat --output /backup --all >> /var/log/github-backup.log 2>&1
```

### systemd timer

```ini
# /etc/systemd/system/github-backup.timer
[Unit]
Description=Daily GitHub backup

[Timer]
OnCalendar=*-*-* 02:00:00
Persistent=true
RandomizedDelaySec=15m

[Install]
WantedBy=timers.target
```

```ini
# /etc/systemd/system/github-backup.service
[Unit]
Description=GitHub backup
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service
OnFailure=notify-github-backup-failure@%n.service

[Service]
Type=oneshot
ExecStart=/usr/bin/docker run --rm --env-file /etc/github-backup/env -v /var/backup/github:/backup ghcr.io/tomtom215/github-backup-rust:X.Y.Z octocat --output /backup --all
```

Keep the secret in `/etc/github-backup/env` (mode `0600`, one `NAME=value` per
line).  systemd does not expand shell `\` continuations in `ExecStart=`, so
keep that line whole.  A run that exits `3` marks the unit failed and triggers
`OnFailure=`.

### Kubernetes CronJob

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: github-backup
spec:
  schedule: "0 2 * * *"
  concurrencyPolicy: Forbid          # one backup at a time
  failedJobsHistoryLimit: 3
  successfulJobsHistoryLimit: 1
  jobTemplate:
    spec:
      backoffLimit: 2                # a failed run is retried at most twice
      template:
        spec:
          restartPolicy: Never
          securityContext:
            runAsUser: 1000
            runAsGroup: 1000
            fsGroup: 1000
            runAsNonRoot: true
          containers:
            - name: backup
              image: ghcr.io/tomtom215/github-backup-rust:X.Y.Z
              args: ["octocat", "--output", "/backup", "--all"]
              env:
                - name: GITHUB_TOKEN
                  valueFrom:
                    secretKeyRef:
                      name: github-backup-token
                      key: token
              volumeMounts:
                - name: backup
                  mountPath: /backup
              resources:
                requests: { cpu: "100m", memory: "128Mi" }
                limits: { cpu: "1", memory: "512Mi" }
          volumes:
            - name: backup
              persistentVolumeClaim:
                claimName: github-backup-pvc
```

A job whose backup exits `3` (incomplete) counts as failed and is retried up to
`backoffLimit` times; each retry resumes and retries only what failed.

## Troubleshooting

```sh
# What the doctor sees inside the container
docker compose --profile doctor run --rm doctor octocat

# Print the OAuth scopes a flag set needs (note: it does not expand --all)
docker run --rm -e GITHUB_TOKEN ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --org --repositories --issues --list-scopes

# Validate a config file and connectivity without running a backup
docker run --rm -e GITHUB_TOKEN \
  -v "$PWD/config.toml:/etc/github-backup/config.toml:ro" \
  ghcr.io/tomtom215/github-backup-rust:latest \
  --config /etc/github-backup/config.toml --check
```

More: [Troubleshooting](https://tomtom215.github.io/github-backup-rust/development/troubleshooting.html)
and the [Operations Runbook](https://tomtom215.github.io/github-backup-rust/ops-runbook.html).
