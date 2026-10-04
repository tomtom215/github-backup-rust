# CLI Reference

Reference for every `github-backup` command-line flag.  `github-backup --help`
is the authoritative list for the binary you have installed; this page is
checked against it, and the **Env var** and **Default** columns are taken from
it.

## Synopsis

```
github-backup [OPTIONS] [OWNER]
github-backup [OWNER] --tui
github-backup --config <FILE> [OPTIONS]
github-backup --completions <SHELL>
github-backup --print-config-template
```

`--completions <SHELL>` is not listed by `--help` (it is handled before the
normal argument parsing); `--help` and `--version` (`-h`, `-V`) behave as
usual.

## Arguments

| Argument | Description |
|---------|-------------|
| `[OWNER]` | GitHub username or organisation name.  May be omitted when `--config` supplies `owner`.  It is used as a directory name under `--output`, so `..`, `/`, `\` and control characters are rejected. |

## Authentication

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `-t, --token <TOKEN>` | `GITHUB_TOKEN` | none | Personal access token (classic or fine-grained).  Prefer the variable: a command-line value is visible in `ps`. |
| `--device-auth` | none | `false` | Use the GitHub OAuth device flow (interactive).  Needs `--oauth-client-id`. |
| `--oauth-client-id <CLIENT_ID>` | `GITHUB_OAUTH_CLIENT_ID` | none | OAuth App client ID.  The variable is ignored unless `--device-auth` is given; the flag without `--device-auth` is an error. |
| `--oauth-scopes <SCOPES>` | none | `repo gist read:org` | Scopes requested by the device flow (space-separated).  These do **not** cover every category (for example `--packages` needs `read:packages`); see [Authentication](../getting-started/authentication.md). |

With no credential at all the tool runs unauthenticated (public data, 60
requests per hour).  It refuses to start, with exit status `1`, when one of
`--private`, `--hooks`, `--deploy-keys`, `--collaborators`, `--org-members`,
`--org-teams`, `--actions`, `--action-runs`, `--packages`, `--discussions` or
`--projects` is given explicitly.  That check does not look inside `--all`: an
anonymous `--all` starts, and everything that needs a token fails or comes back
empty.

## GitHub Enterprise Server

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--api-url <URL>` | `GITHUB_API_URL` | `https://api.github.com` | API base URL.  Must be `https://`. |
| `--clone-host <HOST>` | `GITHUB_CLONE_HOST` | from the API | Replace the hostname in every clone URL. |

See [GitHub Enterprise Server](github-enterprise.md).

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config <FILE>` | none | TOML config file.  Values in the file are defaults; see [Config File](config-file.md#precedence) for exactly how the command line and the file combine. |
| `--print-config-template` | `false` | Print an annotated config template and exit.  Needs no owner or token. |

## Output

| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output <DIR>` | `.` | Root directory for everything the tool writes. |
| `--report <FILE>` | none | Write a JSON summary after the run (also after a failed run).  See [Monitoring](../monitoring.md). |

## Target Type

| Flag | Default | Description |
|------|---------|-------------|
| `--org` | `false` | Treat OWNER as an organisation. |

## Broad Selector

| Flag | Description |
|------|-------------|
| `--all` | Enable every category listed in `--help` for `--all` (see [The `--all` flag](../backup-categories.md#the---all-flag)).  Conflicts with the individual category flags on the command line, but not with `--forks`, `--private`, `--action-runs`, `--clone-starred`, `--lfs` and the other behaviour flags, which are honoured next to `--all`. |

## Repository Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--repositories` | | `false` | Clone or update repositories. |
| `--forks` | `-F` | `false` | Include forks. |
| `--private` | `-P` | `false` | Include private repositories.  For a user target this needs a token that belongs to that user; see [Private repositories](../backup-categories.md#private-repositories). |
| `--prefer-ssh` | | `false` | Clone over SSH instead of HTTPS (no token is used; git needs working SSH keys). |
| `--clone-type <TYPE>` | | `mirror` | `mirror`, `bare`, `full` or `shallow:<depth>`; see [Clone types](../backup-categories.md#clone-types-explained). |
| `--lfs` | | `false` | Also fetch Git LFS objects (`git lfs fetch --all`) after the mirror update.  Needs `git-lfs` installed; overrides `--clone-type`. |
| `--no-prune` | | `false` | Do not prune refs that were deleted on GitHub. |

## Issue and Pull Request Options

| Flag | Description |
|------|-------------|
| `--issues` | Issue list.  Pull requests appear in it too. |
| `--issue-comments` | Comments of every issue **and pull request**. |
| `--issue-events` | Events of every issue **and pull request**. |
| `--pulls` | Pull request list. |
| `--pull-comments` | Inline review comments, per pull request. |
| `--pull-commits` | Commit list, per pull request (GitHub returns at most 250). |
| `--pull-reviews` | Reviews, per pull request. |

## Repository Metadata

| Flag | Description |
|------|-------------|
| `--labels` | Labels. |
| `--milestones` | Milestones. |
| `--releases` | Release metadata. |
| `--release-assets` | Download release assets.  Requires `--releases`. |
| `--hooks` | Webhook configurations (admin access). |
| `--security-advisories` | Published security advisories. |
| `--wikis` | Clone wikis. |
| `--topics` | Topics. |
| `--branches` | Branch list and, for protected branches, the protection rules (admin access). |
| `--deploy-keys` | Deploy keys (admin access). |
| `--collaborators` | Collaborators with permissions (admin access). |

## GitHub Actions and Environments

| Flag | Description |
|------|-------------|
| `--actions` | Workflow metadata (`workflows.json`). |
| `--action-runs` | Run history per workflow.  Requires `--actions`; can be very large; not part of `--all`. |
| `--environments` | Deployment environments with protection rules. |

## Discussions, Classic Projects, Packages

| Flag | Description |
|------|-------------|
| `--discussions` | **Not functional.**  GitHub has no REST endpoint for Discussions; nothing is saved and a warning is logged.  Still part of `--all`. |
| `--projects` | **Not functional.**  Classic Projects are gone from GitHub's REST API; nothing is saved and a warning is logged.  Still part of `--all`. |
| `--packages` | Package metadata of the target user (needs `read:packages`). |

## Organisation Data

| Flag | Description |
|------|-------------|
| `--org-members` | Member list (organisation targets only; ignored for users). |
| `--org-teams` | Team list (organisation targets only; ignored for users). |

## User and Organisation Data

| Flag | Description |
|------|-------------|
| `--starred` | Starred repositories as a JSON list. |
| `--clone-starred` | Clone every starred repository as a bare mirror through a durable queue.  Not part of `--all`, but honoured next to it. |
| `--watched` | Watched repositories. |
| `--followers` | Followers. |
| `--following` | Accounts followed. |
| `--gists` | Clone gists owned by the target. |
| `--starred-gists` | Metadata of the gists starred by the **authenticated user** (not cloned). |

## Repository Filters

| Flag | Default | Description |
|------|---------|-------------|
| `--include-repos <PATTERN>` | all | Only repositories whose **name** matches (repeat the flag or separate with commas). |
| `--exclude-repos <PATTERN>` | none | Skip repositories whose name matches; wins over `--include-repos`. |

Patterns match the repository name only (not `owner/name`): `*` matches any
sequence, `?` one character, case-insensitively.  There is no pattern for
"archived" repositories; `*archived*` matches names that contain the word.

```bash
github-backup octocat --output /backup --repositories --include-repos "rust-*"
github-backup octocat --output /backup --repositories --exclude-repos "*-old,*-fork"
```

## Incremental Behaviour

Issue and pull request **lists are always fetched in full** and merged into
`issues.json` / `pulls.json`; a run never shrinks them.  What is incremental
is the per-item data (comments, events, commits, reviews): a repository's own
watermark from the previous clean run decides which items can be skipped.  See
[Incremental runs](../monitoring.md#incremental-runs-and-the-state-file).

| Flag | Default | Description |
|------|---------|-------------|
| `--since <DATE>` | none | Expert override: treat everything updated before DATE as already backed up.  Accepts `2024-01-01` (midnight UTC) or an RFC 3339 timestamp with any offset.  Never written to the state file. |
| `--full` | `false` | Ignore all watermarks and fetch every per-item file again.  Conflicts with `--since`. |

## Push Mirror Options

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--mirror-to <URL>` | none | none | Base URL of the destination (`https://codeberg.org`, `https://gitlab.com`, ...). |
| `--mirror-type <TYPE>` | none | `gitea` | `gitea` (Gitea, Codeberg, Forgejo) or `gitlab`. |
| `--mirror-token <TOKEN>` | `MIRROR_TOKEN` | none | API token for the destination.  The variable is ignored unless `--mirror-to` is given. |
| `--mirror-owner <OWNER>` | none | OWNER | User or organisation/namespace at the destination. |
| `--mirror-private` | none | `false` | Create destination repositories as private.  Without it a repository is created private anyway unless GitHub says it is public. |

See [Mirroring](../mirroring.md).

## S3 Options

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--s3-bucket <BUCKET>` | none | none | Bucket (must exist).  Enables S3 sync of the **JSON metadata**; clones are not uploaded. |
| `--s3-region <REGION>` | none | `us-east-1` | Region. |
| `--s3-prefix <PREFIX>` | none | empty | Key prefix; objects are stored as `<prefix>/<owner>/json/<path>`. |
| `--s3-endpoint <URL>` | none | AWS | Custom endpoint with scheme (B2, MinIO, R2, ...). |
| `--s3-access-key <KEY>` | `AWS_ACCESS_KEY_ID` | none | Access key ID.  The variable is ignored unless `--s3-bucket` is given. |
| `--s3-secret-key <SECRET>` | `AWS_SECRET_ACCESS_KEY` | none | Secret access key.  Same rule. |
| `--s3-session-token <TOKEN>` | `AWS_SESSION_TOKEN` | none | Session token for temporary credentials.  Same rule. |
| `--s3-include-assets` | none | `false` | Also upload release assets. |
| `--s3-delete-stale` | none | `false` | Delete remote objects whose local file is gone (guarded; see [S3](../storage/s3.md#deleting-stale-objects---s3-delete-stale)). |

## At-Rest Encryption

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--encrypt-key <HEX_KEY>` | `BACKUP_ENCRYPT_KEY` | none | 64 hexadecimal characters (32 bytes): encrypt files with AES-256-GCM before the S3 upload.  Applies to S3 only. |
| `--decrypt` | none | `false` | Decrypt one file and exit.  Needs the key (`--encrypt-key` or `BACKUP_ENCRYPT_KEY`), no OWNER and no network. |
| `--decrypt-input <FILE>` | none | none | Encrypted input (with `--decrypt`). |
| `--decrypt-output <FILE>` | none | none | Plaintext output (with `--decrypt`). |

## Restore

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--restore` | none | `false` | Re-create labels, milestones and issues from the **local backup** in another organisation.  Makes no backup first and does not contact the source. |
| `--restore-target-org <ORG>` | none | OWNER | Organisation to restore into.  The repositories must already exist there. |
| `--restore-yes` | `GITHUB_BACKUP_RESTORE_YES=1` | `false` | Confirm without the interactive prompt (required when stdin is not a terminal). |

See the [Restore guide](../restore.md).

## Manifest and Verify

| Flag | Default | Description |
|------|---------|-------------|
| `--manifest` | `false` | After the run, write `json/backup_manifest.json` with the SHA-256 of every data file under `json/` (not the history, state, checkpoint and lock files, which change on every run). |
| `--verify` | `false` | Check the manifest of an existing backup and exit.  Needs OWNER (and `--output` unless the backup is in the current directory); no network.  Covers the files under `json/` only, not the git clones.  Conflicts with `--all`. |

## Monitoring

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--prometheus-metrics <FILE>` | none | none | Write node_exporter textfile metrics (also after a failed run). |
| `--notify-webhook <URL>` | `BACKUP_NOTIFY_WEBHOOK` | none | POST a JSON status (`success`, `partial` or `failure`) after the run. |
| `--diff-with <PREV_JSON_DIR>` | none | none | After the run, log how the repository list differs from a previous backup's `<owner>/json` directory (repository names only). |
| `--history-size <N>` | none | `20` | Entries kept in `json/backup_history.json`. |

## Execution Options

| Flag | Default | Description |
|------|---------|-------------|
| `--concurrency <N>` | `4` | Repositories processed in parallel. |
| `--dry-run` | `false` | List what would be backed up; write nothing.  See below. |

### What `--dry-run` does

A dry run fetches the repository list (read-only API calls) and logs which
repositories it would back up.  It writes **nothing**: no files or directories
(not even the lock), no state, report, manifest, metrics, history, no webhook,
no S3 upload and no mirror push, and it runs no `git`.  Owner-level data and
gists are skipped.  With `--restore` it makes no API call at all.

## Deprecated

| Flag | Description |
|------|-------------|
| `--keep-last <N>` | **Ignored.**  A warning is logged after a real run. |
| `--max-age-days <DAYS>` | **Ignored.**  A warning is logged after a real run. |

The tool keeps one continuously updated backup per owner under
`<output>/<owner>/`; it never deletes snapshot directories.  Rotate or expire
copies with a tool built for it (restic, borg, ZFS or LVM snapshots, a
lifecycle rule on the bucket).  `github-backup --help` still describes the two
flags as if they pruned directories; they do not.

## Logging

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--quiet` | `-q` | `false` | Errors only; also hides the plan and summary banners. |
| `--verbose` | `-v` | 0 | `-v` debug, `-vv` trace.  Neither prints the token. |

`RUST_LOG` overrides the level; see [Environment Variables](environment.md#logging).

## Diagnostics

| Flag | Description |
|------|-------------|
| `--doctor` | Run the pre-flight checks and exit (details below). |
| `--check` | `--doctor` plus the resolved configuration (owner, output, API URL, concurrency and the scopes `--list-scopes` would print). |
| `--list-scopes` | Print the classic OAuth scopes recommended for the flags given and exit. |

`--doctor` runs, in order: the `git` binary and its version, whether the output
directory exists or can be created (it is created if missing) and is writable,
the kind of credential
(classic, fine-grained, OAuth or app token, judged by its prefix), whether the
API answers, and whether GitHub accepts the token (`GET /rate_limit`, which
costs no quota).  A rejected token (HTTP 401 or 403) or an unreachable API is a
failure; a server without `/rate_limit` is only a warning.  It does **not**
check free disk space, the token's scopes, S3 access, the mirror destination or
`git-lfs`.  It exits `0` when no check failed and `1` otherwise.

`--list-scopes` maps each enabled flag to the scopes it would need.  With
`--all` it prints only `public_repo repo` (it does not expand `--all`), and it
recommends `user:follow` for `--followers` and `admin:public_key` for
`--deploy-keys`, which only read data.  For a token covering `--all` use the
table in [Authentication](../getting-started/authentication.md#what-each-category-needs).

## Interactive TUI

`--tui` starts the full-screen interface instead of a run.  The owner, token,
output directory and API URL given on the command line pre-fill the form; other
flags are not read.  See the [Interactive TUI guide](../tui.md).

## Shell Completions

`--completions <SHELL>` prints a completion script and exits without a token or
network access.  Supported shells: `bash`, `zsh`, `fish`, `powershell`,
`elvish`.  Per-shell setup is in [Installation](../getting-started/installation.md#shell-completions).

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Everything that was asked for succeeded. |
| `1` | The run could not be carried out: bad configuration or arguments that the tool detects itself, rejected credentials, the repository list could not be fetched, another run holds the lock, the disk is full, a failed `--verify`, `--doctor` or `--decrypt`, or a restore that could not start. |
| `2` | Usage error reported by the argument parser (unknown flag, missing value, conflicting flags). |
| `3` | The run finished but something could not be backed up: a repository, an issue list, an S3 upload, a mirror push, the manifest.  The backup is **incomplete**.  Re-running retries only what failed.  `--restore` also exits `3` when some items could not be restored. |
| `130` / `143` | Interrupted by `SIGINT` / `SIGTERM`. |
| `64` | Container entrypoint only: refused a malformed `UMASK`, `BACKUP_MODE` or `BACKUP_FLAGS`. |

Failures never stop the rest of the run: a repository that cannot be cloned
does not prevent its issues or the other repositories from being backed up.
Each failure is recorded once and shows up in the log, the summary banner, the
`--report` file (`failure_count`, `failures[]`), the metrics
(`github_backup_failures`, `github_backup_success`), `backup_history.json` and
the webhook (`"status": "partial"`); they cannot disagree with the exit code.
A failure of the notification itself (webhook, report or metrics file that
cannot be written) is logged but does not change the exit code.
