# CLI Reference

Complete reference for all `github-backup` command-line flags.

## Synopsis

```
github-backup [OPTIONS] [OWNER]
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

## User / Org Data

| Flag | Default | Description |
|------|---------|-------------|
| `--starred` | `false` | Back up starred repositories |
| `--watched` | `false` | Back up watched repositories |
| `--followers` | `false` | Back up follower list |
| `--following` | `false` | Back up following list |
| `--gists` | `false` | Back up owned gists |
| `--starred-gists` | `false` | Back up starred gists |

## Push-Mirror Options

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--mirror-to <URL>` | — | — | Gitea-compatible base URL |
| `--mirror-token <TOKEN>` | `MIRROR_TOKEN` | — | API token for mirror destination |
| `--mirror-owner <OWNER>` | — | Same as OWNER | Username/org at mirror destination |
| `--mirror-private` | — | `false` | Create repos as private at destination |

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

## Execution Options

| Flag | Default | Description |
|------|---------|-------------|
| `--concurrency <N>` | `4` | Max repositories backed up in parallel |
| `--dry-run` | `false` | Log actions without writing files or running git |

## Logging

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--quiet` | `-q` | `false` | Suppress all non-error output |
| `--verbose` | `-v` | 0 | Increase verbosity (`-v` = debug, `-vv` = trace) |

## Special Commands

```bash
# Generate shell completions (no auth required)
github-backup --completions bash
github-backup --completions zsh
github-backup --completions fish
github-backup --completions powershell
github-backup --completions elvish
```

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Failure (authentication error, fatal API error, etc.) |

Per-repository errors are non-fatal: they are logged as warnings and the backup
continues.  A `1` exit code indicates that the entire backup failed to start
or that a post-processing step (S3 sync, mirror push) failed.
