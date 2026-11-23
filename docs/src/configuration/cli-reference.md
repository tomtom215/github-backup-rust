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

With no options at all, `github-backup` backs up **everything** the token can
reach for the account the token belongs to, into the current directory.  The
modes `--tui`, `--list-scopes`, `--doctor`, `--check`, `--verify`, `--decrypt`,
`--restore`, `--every`, `--print-config-template` and `--completions` exclude
one another.

## Arguments

| Argument | Description |
|---------|-------------|
| `[OWNER]` | GitHub username or organisation name.  Defaults to `owner` from the config file, or else to the account the token belongs to (`GET /user`; the log says which).  An organisation is recognised as one without `--org`.  It is used as a directory name under `--output`, so `..`, `/`, `\` and control characters are rejected; a name GitHub does not know stops the run with status `1`. |

## Authentication

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `-t, --token <TOKEN>` | `GITHUB_TOKEN`, `GH_TOKEN` | see below | Personal access token (classic or fine-grained).  Prefer a variable: a command-line value is visible in `ps`, and the tool warns about it. |
| `--device-auth` | none | `false` | Use the GitHub OAuth device flow (interactive).  Needs `--oauth-client-id`. |
| `--oauth-client-id <CLIENT_ID>` | `GITHUB_OAUTH_CLIENT_ID` | none | OAuth App client ID.  The variable is ignored unless `--device-auth` is given; the flag without `--device-auth` is an error. |
| `--oauth-scopes <SCOPES>` | none | `repo gist read:org read:packages read:project` | Scopes requested by the device flow (space-separated).  They cover every category of the default backup except webhooks (`admin:repo_hook`; without it webhooks are skipped with a warning); see [Authentication](../getting-started/authentication.md). |

The token is taken from the first of: `--token`, `GITHUB_TOKEN`, `GH_TOKEN`,
`token` in the config file, and the GitHub CLI's login (`gh auth token
--hostname <host>`, where the host is `github.com` or the host of `--api-url`;
at most 5 seconds; the token is never logged).  `--device-auth` skips the
GitHub CLI.

With no credential at all the tool runs unauthenticated (public data, 60
requests per hour).  It refuses to start, with exit status `1`, when the
categories it would back up (after the default, `--all`, the config file and
`--skip`) include one that anonymous requests cannot read: private
repositories, hooks, deploy keys, collaborators, organisation members or teams,
Actions, packages, discussions or projects.  Select public categories
explicitly (for example `--repositories --issues`) to run anonymously.

## GitHub Enterprise Server

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--api-url <URL>` | `GITHUB_API_URL` | `https://api.github.com` | API base URL.  Must be `https://`. |
| `--clone-host <HOST>` | `GITHUB_CLONE_HOST` | from the API | Replace the hostname in every clone URL. |

See [GitHub Enterprise Server](github-enterprise.md).

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config <FILE>` | see below | TOML config file.  Values in the file are defaults; see [Config File](config-file.md#precedence) for exactly how the command line and the file combine. |
| `--print-config-template` | `false` | Print an annotated config template and exit.  Needs no owner or token. |
| `--completions <SHELL>` | none | Print a shell completion script and exit (`bash`, `zsh`, `fish`, `powershell`, `elvish`). |

Without `--config`, the file `$XDG_CONFIG_HOME/github-backup/config.toml`
(`~/.config/github-backup/config.toml` when `XDG_CONFIG_HOME` is unset, also on
macOS; `%APPDATA%\github-backup\config.toml` on Windows) is loaded when it
exists, and the log says so.  Use `--config /dev/null` to ignore it once.

## Output

| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output <DIR>` | `.` | Root directory for everything the tool writes. |
| `--report <FILE>` | none | Write a JSON summary after the run (also after a failed run).  See [Monitoring](../monitoring.md). |

## Target Type

| Flag | Default | Description |
|------|---------|-------------|
| `--org` | `false` | Treat OWNER as an organisation.  Rarely needed: the run looks OWNER up and treats an organisation as one (and ignores `--org` for a user, with a warning). |

## Choosing What to Back Up

**With no category selected** (no category flag on the command line and no
category key in the config file) a run backs up everything, exactly as with
`--all`; the plan banner says so.  Selecting any category (for example
`--issues`, or `issues = true` in the config file) backs up only what is
selected.  `--forks`, `--private`, `--clone-starred` and `--action-runs` do not
count as a selection: `github-backup --action-runs` is everything plus the
workflow run history.

| Flag | Description |
|------|-------------|
| `--all` | Enable every category listed in `--help` for `--all` (see [The `--all` flag](../backup-categories.md#the---all-flag)).  Conflicts with the individual category flags on the command line, but not with `--forks`, `--private`, `--action-runs`, `--clone-starred`, `--lfs` and the other behaviour flags, which are honoured next to `--all`. |
| `--skip <CATEGORY>` | Leave categories out, comma-separated or repeated: `--skip hooks,packages`.  The names are the category flags without `--` (`repositories`, `forks`, `private`, `wikis`, `issues`, `issue-comments`, ..., `packages`); an unknown name is a usage error.  Applies to the default, `--all` and the config file alike, and is the way to turn off a category the config file turns on.  Config key: `skip = ["hooks"]`. |

## Repository Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--repositories` | | `false` | Clone or update repositories. |
| `--forks` | `-F` | `false` | Include forks. |
| `--private` | `-P` | `false` | Include private repositories.  For a user target this needs a token that belongs to that user; see [Private repositories](../backup-categories.md#private-repositories). |
| `--prefer-ssh` | | `false` | Clone over SSH instead of HTTPS (no token is used; git needs working SSH keys). |
| `--clone-type <TYPE>` | | `mirror` | `mirror`, `bare`, `full` or `shallow:<depth>`; see [Clone types](../backup-categories.md#clone-types-explained). |
| `--lfs` | | `false` | Also fetch Git LFS objects (`git lfs fetch --all`) after the mirror update.  Needs `git-lfs` installed; overrides `--clone-type`. |
| `--prune` | | `false` | Delete branches and tags from the clone, and from the `--mirror-to` destination, when they were deleted on GitHub.  Without it they are kept; force-pushed branches are overwritten either way. |
| `--verify-git` | | `false` | Check every repository with `git fsck --no-dangling` after each clone and update.  A repository that fails is recorded as a failure (exit status `3`), and a fresh clone that fails is discarded.  Reads every object, so it is slow on large repositories. |
| `--git-timeout SECS` | | `600` | How long a git command may produce no output before it is stopped as stalled (a limit on silence, not on total time).  The same age makes a git `.lock` file left by a killed run count as stale. |

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
| `--action-runs` | Run history per workflow.  Has an effect only while `--actions` is on (it is by default and with `--all`); can be very large; not part of `--all`. |
| `--environments` | Deployment environments with protection rules. |

## Discussions, Projects, Packages

| Flag | Description |
|------|-------------|
| `--discussions` | Discussions of each repository that has them enabled, with every comment and reply, and the discussion categories (`discussions.json`, `discussion_comments/<n>.json`, `discussion_categories.json`), read through the GraphQL API. |
| `--projects` | The owner's Projects (v2): projects, fields and items with their field values (needs `read:project`). |
| `--packages` | Package metadata of the target user or organisation (needs `read:packages`). |
| `--commit-comments` | Comments made on the commits of each repository (`commit_comments.json`). |

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

Issue and pull request lists are merged into `issues.json` / `pulls.json`;
a run never shrinks them.  A repository's own watermark from the previous
clean run decides what a run can skip: it lists only the issues and pull
requests updated since (when the stored file is intact), skips the per-item
data (comments, events, commits, reviews) of unchanged items, and skips the
`git fetch` of a repository nothing was pushed to (at most for 7 days).  See
[Incremental runs](../monitoring.md#incremental-runs-and-the-state-file).

| Flag | Default | Description |
|------|---------|-------------|
| `--since <DATE>` | none | Expert override: treat everything updated before DATE as already backed up.  Accepts `2024-01-01` (midnight UTC) or an RFC 3339 timestamp with any offset.  Never written to the state file. |
| `--full` | `false` | Ignore all watermarks: list every issue and pull request, fetch every per-item file and `git fetch` every clone.  Conflicts with `--since`. |

## Push Mirror Options

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--mirror-to <URL>` | none | none | Base URL of the destination (`https://codeberg.org`, `https://gitlab.com`, ...). |
| `--mirror-type <TYPE>` | none | `gitea` | `gitea` (Gitea, Codeberg, Forgejo) or `gitlab`; anything else is a usage error.  Config key `mirror_type`. |
| `--mirror-token <TOKEN>` | `MIRROR_TOKEN` | none | API token for the destination.  The variable is ignored unless `--mirror-to` is given. |
| `--mirror-owner <OWNER>` | none | OWNER | User or organisation/namespace at the destination. |
| `--mirror-private` | none | `false` | Create every destination repository as private.  This is the default; the flag states it explicitly.  Cannot be combined with `--mirror-public`. |
| `--mirror-public` | none | `false` | Create mirrors of repositories that are public on GitHub as public.  A private source, or one whose visibility is unknown, is still mirrored privately.  Cannot be combined with `--mirror-private`. |

The mirror options other than `--mirror-to` need a destination, from the
command line or the config file (`mirror_to`); without one they are a usage
error (status `2`).  A destination without a token stops the run at startup
(status `1`), before anything is backed up.

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

The S3 options other than the credentials need a bucket, from the command line
or the config file (`s3_bucket`); without one they are a usage error (status
`2`).  A bucket without an access key and secret stops the run at startup
(status `1`), before anything is backed up.

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
| `--restore-target-org <ORG>` | none | none | Account to restore into; required.  The repositories must already exist there.  The confirmation shows the source and the target. |
| `--restore-into-source` | none | `false` | Allow the target to be OWNER itself (or leave `--restore-target-org` out).  Without it that is refused: issues that still exist in the source would be created a second time. |
| `--restore-yes` | `GITHUB_BACKUP_RESTORE_YES=1` | `false` | Confirm without the interactive prompt (required when stdin is not a terminal). |

See the [Restore guide](../restore.md).

## Manifest and Verify

| Flag | Default | Description |
|------|---------|-------------|
| `--manifest` | `false` | After the run, write `json/backup_manifest.json` with the SHA-256 of every data file under `json/` (not the history, state, checkpoint and lock files, which change on every run, nor temporary files). |
| `--verify` | `false` | Check the manifest of an existing backup and exit.  Needs OWNER (and `--output` unless the backup is in the current directory); no network.  Covers the files under `json/` only, not the git clones. |

## Monitoring

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--prometheus-metrics <FILE>` | none | none | Write node_exporter textfile metrics (also after a failed run). |
| `--notify-webhook <URL>` | `BACKUP_NOTIFY_WEBHOOK` | none | Send the outcome (`success`, `partial`, `failure` or `interrupted`) after each run.  `--help` never shows the variable's value; prefer the variable, since a Slack or Discord URL is a secret. |
| `--notify-format <FORMAT>` | `BACKUP_NOTIFY_FORMAT` | `json` | `json`, `slack`, `discord` or `healthchecks`; see [Monitoring](../monitoring.md#webhook). |
| `--diff-with <PREV_JSON_DIR>` | none | none | After the run, log how the repository list differs from a previous backup's `<owner>/json` directory (repository names only). |
| `--history-size <N>` | none | `20` | Entries kept in `json/backup_history.json`. |

## Execution Options

| Flag | Default | Description |
|------|---------|-------------|
| `--concurrency <N>` | `4` | Repositories processed in parallel (`0` counts as `1`). |
| `--every <INTERVAL>` | none | Keep running: back up now and then every INTERVAL (`30m`, `6h`, `24h`, `7d`; at least `1m`).  See [Built-in schedule](../deployment/cron.md#built-in-schedule---every). |
| `--dry-run` | `false` | List what would be backed up; write nothing.  See below. |

### What `--dry-run` does

A dry run fetches the repository list (read-only API calls) and logs which
repositories it would back up.  It writes **nothing**: no files or directories
(not even the lock), no state, report, manifest, metrics, history, no webhook,
no S3 upload and no mirror push, and it runs no `git`.  Owner-level data and
gists are skipped.  With `--restore` it makes no API call at all.

## Deprecated

These flags are hidden from `--help` and still accepted, so that existing
scheduled jobs keep running:

| Flag | Description |
|------|-------------|
| `--keep-last <N>` | **Ignored.**  A warning is logged at startup. |
| `--max-age-days <DAYS>` | **Ignored.**  A warning is logged at startup. |
| `--no-prune` | **Ignored**: not pruning is the default.  Cannot be combined with `--prune`. |

The tool keeps one continuously updated backup per owner under
`<output>/<owner>/`; it never deletes snapshot directories.  Rotate or expire
copies with a tool built for it (restic, borg, ZFS or LVM snapshots, a
lifecycle rule on the bucket).

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

`--doctor` first validates the configuration exactly as a backup run does
(`--since`, `--encrypt-key`, OWNER's spelling, the flag combinations), so a
configuration it calls ready is one the run accepts.  It then checks, in order:
the `git` binary and its version, whether the output directory exists or can be
created (it is created if missing) and is writable, the kind of credential
(classic, fine-grained, OAuth or app token, judged by its prefix), the
categories that would be backed up (none at all is a failure), the mirror and
S3 settings when they are used (a missing token or key is a failure), whether
the API answers, whether GitHub accepts the token (`GET /rate_limit`, which
costs no quota), and, when the API answered, OWNER: that the account exists and
whether it is an organisation, or, without OWNER, which account the token
belongs to.  A rejected token (HTTP 401 or 403) or an unreachable API is a
failure; a server without `/rate_limit` is only a warning.  It does **not**
check free disk space, the token's scopes, whether the S3 bucket or mirror
destination answers, or `git-lfs`.  It exits `0` when no check failed and `1`
otherwise.

`--list-scopes` maps the categories that would be backed up (after the default,
`--all`, the config file and `--skip`) to the classic scopes they need; with no
category selected, or with `--all`, that is `admin:repo_hook gist read:org
read:packages read:project repo`.  Followers and following
need no scope.  See also
[Authentication](../getting-started/authentication.md#what-each-category-needs).

## Interactive TUI

`--tui` starts the full-screen interface instead of a run.  The command line
and the config file (`--config` or the default one) pre-fill the form: owner,
token (including `GH_TOKEN` and the GitHub CLI's), output directory, API URL,
categories (everything when none is selected), `--dry-run`, `--manifest` and
the clone options.  What the TUI cannot do is an error (status `2`) together
with `--tui`: `--mirror-to`, `--s3-bucket`, `--report`, `--prometheus-metrics`,
`--notify-webhook` on the command line, `--diff-with` and `--device-auth`.  See
the [Interactive TUI guide](../tui.md).

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
| `130` / `143` | Interrupted by `SIGINT` / `SIGTERM`, during the backup or the post-processing (mirror push, S3 sync, notification).  The history, `--report` and metrics are still written, marked unsuccessful, and the webhook is told `interrupted`.  With `--every`, a signal while waiting for the next run exits `0`. |
| `64` | Container entrypoint only: refused a malformed `UMASK`, or a `BACKUP_MODE` that does not start with `--`. |

Failures never stop the rest of the run: a repository that cannot be cloned
does not prevent its issues or the other repositories from being backed up.
Each failure is recorded once and shows up in the log, the summary banner, the
`--report` file (`failure_count`, `failures[]`), the metrics
(`github_backup_failures`, `github_backup_success`), `backup_history.json` and
the webhook (`"status": "partial"`); they cannot disagree with the exit code.
The summary says "backed up completely" only when the git data was cloned;
without `--repositories` it says "metadata only".
A failure of the notification itself (webhook, report or metrics file that
cannot be written) is logged but does not change the exit code.
