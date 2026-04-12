# CLI Reference

Complete reference for all `github-backup` command-line flags.

## Synopsis

```
github-backup [OPTIONS] [OWNER]
github-backup [OWNER] --tui
github-backup --config <FILE> [OPTIONS]
github-backup --completions <SHELL>
```

## Arguments

| Argument | Description |
|---------|-------------|
| `[OWNER]` | GitHub username or organisation name. May be omitted when supplied via `--config`. |

## Authentication

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `-t, --token <TOKEN>` | `GITHUB_TOKEN` | — | Personal access token (classic or fine-grained) |
| `--device-auth` | — | `false` | Use GitHub OAuth device flow (interactive) |
| `--oauth-client-id <ID>` | `GITHUB_OAUTH_CLIENT_ID` | — | OAuth App client ID (required with `--device-auth`) |
| `--oauth-scopes <SCOPES>` | — | `repo gist read:org` | OAuth scopes (space-separated) |

## GitHub Enterprise Server

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--api-url <URL>` | `GITHUB_API_URL` | `https://api.github.com` | Override the GitHub API base URL for GitHub Enterprise Server |
| `--clone-host <HOST>` | `GITHUB_CLONE_HOST` | *(from API)* | Override the hostname used in git clone URLs |

For GHES instances the API is typically at `https://github.example.com/api/v3`.

```bash
# Back up a GitHub Enterprise Server instance
github-backup myorg \
  --token $GITHUB_TOKEN \
  --api-url https://github.example.com/api/v3 \
  --output /backup --org --all

# Split API / clone hostnames (separate load balancers)
github-backup myorg \
  --token $GITHUB_TOKEN \
  --api-url https://github-api.example.com/api/v3 \
  --clone-host github-git.example.com \
  --output /backup --org --repositories
```

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config <FILE>` | — | Path to TOML config file; CLI flags override config values |

## Output

| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output <DIR>` | `.` | Root directory for backup artefacts |
| `--report <FILE>` | — | Write a JSON summary report to this path |

## Target Type

| Flag | Default | Description |
|------|---------|-------------|
| `--org` | `false` | Treat OWNER as a GitHub organisation |

## Broad Selectors

| Flag | Description |
|------|-------------|
| `--all` | Enable all backup categories (conflicts with individual category flags) |

## Repository Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--repositories` | — | `false` | Clone/mirror repositories |
| `--forks` | `-F` | `false` | Include forked repositories |
| `--private` | `-P` | `false` | Include private repositories |
| `--prefer-ssh` | — | `false` | Use SSH URLs instead of HTTPS |
| `--clone-type <TYPE>` | — | `mirror` | Clone mode: `mirror`, `bare`, `full`, `shallow:<n>` |
| `--lfs` | — | `false` | Enable Git LFS |
| `--no-prune` | — | `false` | Skip pruning deleted remote refs |

## Issue Options

| Flag | Default | Description |
|------|---------|-------------|
| `--issues` | `false` | Back up issue metadata |
| `--issue-comments` | `false` | Back up issue comment threads |
| `--issue-events` | `false` | Back up issue timeline events |

## Pull Request Options

| Flag | Default | Description |
|------|---------|-------------|
| `--pulls` | `false` | Back up pull request metadata |
| `--pull-comments` | `false` | Back up PR review comments |
| `--pull-commits` | `false` | Back up PR commit lists |
| `--pull-reviews` | `false` | Back up PR reviews |

## Repository Metadata

| Flag | Default | Description |
|------|---------|-------------|
| `--labels` | `false` | Back up repository labels |
| `--milestones` | `false` | Back up repository milestones |
| `--releases` | `false` | Back up release metadata |
| `--release-assets` | `false` | Download release binary assets (requires `--releases`) |
| `--hooks` | `false` | Back up webhook configurations (requires admin token) |
| `--security-advisories` | `false` | Back up published security advisories |
| `--wikis` | `false` | Clone repository wikis |
| `--topics` | `false` | Back up repository topics (tags) |
| `--branches` | `false` | Back up branch list and protection status |
| `--deploy-keys` | `false` | Back up deploy keys (requires admin access) |
| `--collaborators` | `false` | Back up collaborator list with permissions (requires admin access) |

## GitHub Actions

| Flag | Default | Description |
|------|---------|-------------|
| `--actions` | `false` | Back up Actions workflow metadata (id, name, path, state, badge URL) |
| `--action-runs` | `false` | Back up workflow run history (requires `--actions`; can be very large) |

## Deployment Environments

| Flag | Default | Description |
|------|---------|-------------|
| `--environments` | `false` | Back up deployment environment configs (protection rules, branch policies) |

## Discussions, Classic Projects, and Packages

| Flag | Default | Description |
|------|---------|-------------|
| `--discussions` | `false` | Back up GitHub Discussions threads and their comments |
| `--projects` | `false` | Back up Classic Projects (v1) and their column structure |
| `--packages` | `false` | Back up GitHub Packages metadata for the target user (requires `read:packages`) |

## Organisation Data

| Flag | Default | Description |
|------|---------|-------------|
| `--org-members` | `false` | Back up organisation member list (requires `--org`) |
| `--org-teams` | `false` | Back up organisation team list (requires `--org`) |

## User / Org Data

| Flag | Default | Description |
|------|---------|-------------|
| `--starred` | `false` | Back up starred repositories |
| `--clone-starred` | `false` | Clone every starred repository as a bare mirror (durable queue, opt-in) |
| `--watched` | `false` | Back up watched repositories |
| `--followers` | `false` | Back up follower list |
| `--following` | `false` | Back up following list |
| `--gists` | `false` | Back up owned gists |
| `--starred-gists` | `false` | Back up starred gists |

## Repository Filters

| Flag | Default | Description |
|------|---------|-------------|
| `--include-repos <PATTERN>` | *(all)* | Only back up repos matching glob (repeat or comma-separate) |
| `--exclude-repos <PATTERN>` | *(none)* | Exclude repos matching glob (takes precedence over `--include-repos`) |

Pattern syntax: `*` matches any sequence, `?` matches one character. Matching is case-insensitive.

```bash
# Only repos whose name starts with "rust-"
github-backup octocat --token $TOKEN --output /backup --repositories \
  --include-repos "rust-*"

# All repos except archived ones
github-backup octocat --token $TOKEN --output /backup --repositories \
  --exclude-repos "*archived*,*deprecated*"
```

## Incremental Filter

| Flag | Default | Description |
|------|---------|-------------|
| `--since <DATETIME>` | — | Only fetch issues/PRs updated at or after this ISO 8601 timestamp |

```bash
# Incremental: only issues/PRs updated since 2026-01-01
github-backup octocat --token $TOKEN --output /backup \
  --issues --pulls --since "2026-01-01T00:00:00Z"
```

## Push-Mirror Options

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--mirror-to <URL>` | — | — | Mirror destination base URL (e.g. `https://codeberg.org` or `https://gitlab.com`) |
| `--mirror-type <TYPE>` | — | `gitea` | `gitea` (Gitea / Codeberg / Forgejo) or `gitlab` |
| `--mirror-token <TOKEN>` | `MIRROR_TOKEN` | — | API token for the mirror destination |
| `--mirror-owner <OWNER>` | — | Same as OWNER | Username, org, or namespace at the mirror destination |
| `--mirror-private` | — | `false` | Create repositories as private at the destination |

## S3 Storage Options

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--s3-bucket <BUCKET>` | — | — | S3 bucket name |
| `--s3-region <REGION>` | — | `us-east-1` | AWS region |
| `--s3-prefix <PREFIX>` | — | *(empty)* | Object key prefix |
| `--s3-endpoint <URL>` | — | — | Custom S3-compatible endpoint |
| `--s3-access-key <KEY>` | `AWS_ACCESS_KEY_ID` | — | AWS access key ID |
| `--s3-secret-key <SECRET>` | `AWS_SECRET_ACCESS_KEY` | — | AWS secret access key |
| `--s3-include-assets` | — | `false` | Upload binary release assets to S3 |
| `--s3-delete-stale` | — | `false` | Delete S3 objects no longer in the local backup (use with caution) |

## At-Rest Encryption

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--encrypt-key <HEX_KEY>` | `BACKUP_ENCRYPT_KEY` | *(none)* | 64-hex-char AES-256-GCM key for at-rest encryption before S3 upload |
| `--decrypt` | — | `false` | Decrypt a single file produced by `--encrypt-key` and exit |
| `--decrypt-input <FILE>` | — | — | Path to the encrypted input file (with `--decrypt`) |
| `--decrypt-output <FILE>` | — | — | Path to write the decrypted plaintext (with `--decrypt`) |

## Restore

| Flag | Default | Description |
|------|---------|-------------|
| `--restore` | `false` | Re-create labels, milestones, and issues from the backup in a target organisation |
| `--restore-target-org <ORG>` | *(source owner)* | Target organisation for `--restore` |
| `--restore-yes` | `false` | Skip the interactive confirmation prompt (required in non-interactive environments) |

See the [Restore guide](../restore.md) for usage details.

## Manifest & Verify

| Flag | Default | Description |
|------|---------|-------------|
| `--manifest` | `false` | Write a SHA-256 hash manifest after the backup completes |
| `--verify` | `false` | Verify an existing manifest instead of running a backup |

## Retention

| Flag | Default | Description |
|------|---------|-------------|
| `--keep-last <N>` | — | Keep only the N most recent dated snapshot directories under `--output` |
| `--max-age-days <DAYS>` | — | Delete dated snapshot directories older than this many days |

## Monitoring

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--prometheus-metrics <FILE>` | — | — | Write Prometheus textfile-format metrics to this path |
| `--diff-with <PREV_JSON_DIR>` | — | — | Compare the current backup with a previous backup directory |
| `--notify-webhook <URL>` | `BACKUP_NOTIFY_WEBHOOK` | — | POST a JSON status payload to this URL after the backup |
| `--history-size <N>` | — | `20` | Maximum number of entries kept in `backup_history.json` |

## Execution Options

| Flag | Default | Description |
|------|---------|-------------|
| `--concurrency <N>` | `4` | Max repositories backed up in parallel (config file can set this; CLI always wins) |
| `--dry-run` | `false` | Log actions without writing files or running git |

## Logging

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--quiet` | `-q` | `false` | Suppress all non-error output |
| `--verbose` | `-v` | 0 | Increase verbosity (`-v` = debug, `-vv` = trace) |

## Special Commands

### Interactive TUI

`--tui` launches the full-screen Ratatui interface instead of running
non-interactively.  Any flags passed alongside `--tui` are pre-loaded into
the Configure screen.

```bash
# Launch with pre-seeded owner and token
github-backup octocat --token "$GITHUB_TOKEN" --tui

# Launch with full pre-seed (output dir, API URL, etc.)
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /var/backup/github \
  --api-url https://github.example.com/api/v3 \
  --tui
```

See the [Interactive TUI guide](../tui.md) for screen layouts and the full
key reference.

### Shell Completions

`--completions <SHELL>` prints a completion script and exits immediately — no token or network access required.

Supported shells: `bash`, `zsh`, `fish`, `powershell`, `elvish`.

```bash
# Quick one-liner — pipe directly into your shell's completion directory
github-backup --completions bash >> ~/.bash_completion
github-backup --completions zsh > ~/.zfunc/_github-backup
github-backup --completions fish > ~/.config/fish/completions/github-backup.fish
github-backup --completions powershell >> $PROFILE
github-backup --completions elvish > ~/.config/elvish/lib/github-backup.elv
```

See the [Installation guide](../getting-started/installation.md#shell-completions) for per-shell setup details (Zsh `fpath`, Elvish `rc.elv`, etc.).

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Failure (authentication error, fatal API error, etc.) |

Per-repository errors are non-fatal: they are logged as warnings and the backup
continues.  A `1` exit code indicates that the entire backup failed to start
or that a post-processing step (S3 sync, mirror push) failed.
