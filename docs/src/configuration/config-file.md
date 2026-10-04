# Config File (TOML)

`github-backup` supports a TOML configuration file, making it easy to manage complex backup configurations without long command lines.

## Loading a Config File

```bash
github-backup --config /etc/github-backup/config.toml
```

Or with the short form:

```bash
github-backup -c /etc/github-backup/config.toml
```

## Precedence

The file supplies defaults; the command line and the environment are applied
on top.  The exact rules, because "the command line always wins" is not true
for every kind of setting:

| Kind of setting | Rule |
|-----------------|------|
| Single values (`owner`, `token`, `output`, `concurrency`, `api_url`, `clone_host`, `report`, `mirror_*` values, `s3_*` values, `since`) | A value given on the command line (or in its environment variable) wins; the file is used only when the command line gave none. |
| `clone_type` | A `--clone-type` given on the command line wins, including `--clone-type mirror`.  The file applies only when the flag was not given. |
| Switches (`org`, `all`, `repositories`, `issues`, ..., `lfs`, `prune`, `prefer_ssh`, `mirror_private`, `mirror_public`, `s3_include_assets`) | **Either one turns it on.**  A switch that is `true` in the file cannot be turned off from the command line; set it to `false` (or remove it) in the file instead. |
| Lists (`include_repos`, `exclude_repos`) | The patterns of the file and of the command line are **combined**. |

```bash
# Config: owner = "octocat", concurrency = 4
# Override concurrency for this run only:
github-backup --config config.toml --concurrency 16
```

A config file that cannot be read or contains an unknown key is an error
(exit status `1`): every key is checked, so a typo such as `issuez = true` is
reported instead of silently ignored.

## Full Config File Example

```toml
# /etc/github-backup/config.toml

# ── Identity ───────────────────────────────────────────────────────────────
owner = "octocat"

# Authentication (prefer GITHUB_TOKEN environment variable instead)
# token = "ghp_xxx"

# Output directory
output = "/var/backup/github"

# Parallelism
concurrency = 8

# Target type (default: user)
# org = true

# ── Clone behaviour ────────────────────────────────────────────────────────
# clone_type = "mirror"  # "mirror", "bare", "full", "shallow:<depth>" or { shallow = <depth> }
# prefer_ssh  = false
# lfs         = false
# prune       = false   # true: delete refs that were deleted on GitHub

# ── Backup categories ──────────────────────────────────────────────────────

# Enable the categories of `--all` (clone_starred and action_runs stay opt-in)
# all = true

# Or enable individually:
repositories     = true
forks            = false
private          = true

issues           = true
issue_comments   = true
issue_events     = false

pulls            = true
pull_comments    = true
pull_commits     = false
pull_reviews     = true

labels           = true
milestones       = true
releases         = true
release_assets   = false

hooks            = false
security_advisories = true
wikis            = true

starred          = true
watched          = false
followers        = false
following        = false
gists            = true
starred_gists    = false

# ── GitHub Actions ─────────────────────────────────────────────────────────
actions          = true
# action_runs   = false  # opt-in; can be large for active repos

# ── Deployment environments ─────────────────────────────────────────────────
environments     = true

# ── Packages ───────────────────────────────────────────────────────────────
# (`discussions` and `projects` are accepted but back up nothing: GitHub's
#  REST API has no endpoint for them.)
packages         = false  # requires the read:packages OAuth scope

# ── Organisation-specific ───────────────────────────────────────────────────
org_members      = false
org_teams        = false

# ── Reporting ─────────────────────────────────────────────────────────────
# report = "/var/log/github-backup/report.json"

# ── Mirror to Gitea/Codeberg ───────────────────────────────────────────────
# mirror_to      = "https://codeberg.org"   # Gitea-type only: mirror_type cannot be set in the file
# mirror_token   = "cb_token"         # or use MIRROR_TOKEN env var
# mirror_owner   = "alice"
# mirror_public  = false   # true: mirrors of public repositories are public
# mirror_private = false   # explicit form of the default

# ── S3-compatible storage ──────────────────────────────────────────────────
# s3_bucket       = "my-github-backup"
# s3_region       = "us-east-1"
# s3_prefix       = "github/"
# s3_endpoint     = ""  # Leave blank for AWS; set for B2/MinIO/R2/etc.
# s3_access_key   = ""  # or use AWS_ACCESS_KEY_ID env var
# s3_secret_key   = ""  # or use AWS_SECRET_ACCESS_KEY env var
# s3_include_assets = false
```

## Minimal Config File

```toml
owner = "octocat"
output = "/var/backup/github"
repositories = true
issues = true
```

Then run:

```bash
GITHUB_TOKEN=ghp_xxx github-backup --config config.toml
```

## Full Automated Backup

A complete config for a nightly scheduled backup:

```toml
owner       = "my-org"
org         = true
output      = "/var/backup/github"
concurrency = 8
all         = true

# JSON report for monitoring
report = "/var/log/github-backup/report.json"

# Mirror to Codeberg after each run
mirror_to    = "https://codeberg.org"
mirror_owner = "my-org-mirror"

# Sync JSON metadata to S3
s3_bucket = "my-github-backup"
s3_region = "eu-west-1"
s3_prefix = "nightly/"
```

Run with just:

```bash
GITHUB_TOKEN=ghp_xxx MIRROR_TOKEN=cb_xxx \
  AWS_ACCESS_KEY_ID=AKID AWS_SECRET_ACCESS_KEY=SECRET \
  github-backup --config /etc/github-backup/config.toml
```

## Security

- Set file permissions to `0600` to prevent other users reading your token:

  ```bash
  chmod 600 /etc/github-backup/config.toml
  ```

- Prefer environment variables (`GITHUB_TOKEN`, `MIRROR_TOKEN`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`) over storing secrets in the config file, especially in multi-user environments.

## Config File Schema

### Core

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `owner` | string | — | GitHub username or org |
| `token` | string | — | Personal access token (prefer env var) |
| `api_url` | string | `https://api.github.com` | GitHub API base URL (for GitHub Enterprise Server) |
| `clone_host` | string | *(from API)* | Override git clone hostname (GHES split-hostname) |
| `output` | path | `.` | Output root directory |
| `concurrency` | integer | `4` | Parallel repository backup count |
| `org` | bool | `false` | Treat owner as an organisation |
| `report` | path | — | Write the JSON summary report to this file |

### Clone Behaviour

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `clone_type` | string or table | `mirror` | `"mirror"`, `"bare"`, `"full"`, `"shallow:<depth>"` (for example `"shallow:3"`) or the table form `{ shallow = 3 }` |
| `prefer_ssh` | bool | `false` | Use SSH clone URLs instead of HTTPS |
| `lfs` | bool | `false` | Also fetch Git LFS objects (needs `git-lfs`) |
| `prune` | bool | `false` | Delete branches and tags from the clone when they were deleted on GitHub |
| `no_prune` | bool | — | Deprecated and ignored (not pruning is the default); still accepted so old files load |

### Backup Categories

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `all` | bool | `false` | Enable the categories of `--all` (not `clone_starred`, `action_runs`) |
| `repositories` | bool | `false` | Clone repositories |
| `forks` | bool | `false` | Include forks |
| `private` | bool | `false` | Include private repos |
| `issues` | bool | `false` | Back up issues |
| `issue_comments` | bool | `false` | Back up issue comments |
| `issue_events` | bool | `false` | Back up issue events |
| `pulls` | bool | `false` | Back up pull requests |
| `pull_comments` | bool | `false` | Back up PR comments |
| `pull_commits` | bool | `false` | Back up PR commits |
| `pull_reviews` | bool | `false` | Back up PR reviews |
| `labels` | bool | `false` | Back up labels |
| `milestones` | bool | `false` | Back up milestones |
| `releases` | bool | `false` | Back up releases |
| `release_assets` | bool | `false` | Download release assets |
| `hooks` | bool | `false` | Back up webhooks (admin access required) |
| `security_advisories` | bool | `false` | Back up security advisories |
| `wikis` | bool | `false` | Clone wikis |
| `starred` | bool | `false` | Back up starred repos list (JSON) |
| `clone_starred` | bool | `false` | Clone every starred repo (opt-in; can be large) |
| `watched` | bool | `false` | Back up watched repos |
| `followers` | bool | `false` | Back up followers |
| `following` | bool | `false` | Back up following |
| `gists` | bool | `false` | Back up gists |
| `starred_gists` | bool | `false` | Back up starred gists |
| `topics` | bool | `false` | Back up repository topics |
| `branches` | bool | `false` | Back up branch list |
| `deploy_keys` | bool | `false` | Back up deploy keys (admin access required) |
| `collaborators` | bool | `false` | Back up collaborator list (admin access required) |
| `org_members` | bool | `false` | Back up org member list (organisation targets only) |
| `org_teams` | bool | `false` | Back up org team list (organisation targets only) |
| `actions` | bool | `false` | Back up GitHub Actions workflow metadata |
| `action_runs` | bool | `false` | Back up workflow run history (opt-in; can be large) |
| `environments` | bool | `false` | Back up deployment environment configurations |
| `discussions` | bool | `false` | Accepted, but backs up nothing: GitHub's REST API has no Discussions endpoint |
| `projects` | bool | `false` | Accepted, but backs up nothing: Classic Projects are no longer in GitHub's REST API |
| `packages` | bool | `false` | Back up GitHub Packages metadata for the target user |
| `include_repos` | string array | `[]` | Only back up repos matching these glob patterns |
| `exclude_repos` | string array | `[]` | Exclude repos matching these glob patterns |
| `since` | string | — | Expert override, same as `--since`: a date (`2026-01-01`) or timestamp.  Applied to every run and never stored; leave it unset and the tool tracks its own per-repository watermarks |

### Mirror Destination

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `mirror_to` | string | — | Push mirrors to this base URL.  The destination type is always Gitea/Codeberg/Forgejo from a config file (`--mirror-type gitlab` has no key) |
| `mirror_token` | string | — | API token for the mirror host (prefer `MIRROR_TOKEN` env var) |
| `mirror_owner` | string | — | Owner name at the mirror destination |
| `mirror_private` | bool | `false` | Create every mirror private (the default).  Wins if `mirror_public` is also set |
| `mirror_public` | bool | `false` | Create mirrors of public repositories as public; private ones stay private |

### S3-Compatible Storage

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `s3_bucket` | string | — | S3 bucket name (setting it enables the S3 sync of the JSON metadata) |
| `s3_region` | string | `us-east-1` | AWS region (or equivalent for B2/MinIO/R2) |
| `s3_prefix` | string | `""` | Key prefix for all objects |
| `s3_endpoint` | string | — | Custom endpoint for S3-compatible services |
| `s3_access_key` | string | — | AWS access key ID (prefer `AWS_ACCESS_KEY_ID` env var) |
| `s3_secret_key` | string | — | AWS secret access key (prefer `AWS_SECRET_ACCESS_KEY` env var) |
| `s3_include_assets` | bool | `false` | Also upload release binary assets to S3 |

## GitHub Enterprise Server Config

```toml
owner    = "my-org"
org      = true
output   = "/var/backup/github-enterprise"
api_url  = "https://github.example.com/api/v3"
# Needed only when API host and clone host differ (separate load balancers)
# clone_host = "github-git.example.com"
all      = true
```

## Incremental Backups

Nothing needs to be configured: every run records a watermark per repository
in `<output>/<owner>/json/backup_state.json` and the next run uses it (see
[Incremental runs](../monitoring.md#incremental-runs-and-the-state-file)).  The
`since` key is only an override and should normally stay unset; a fixed
`since` date in a file that is used for every run defeats the watermarks.

## Repository Filter Config

```toml
owner        = "octocat"
output       = "/var/backup/github"
repositories = true

# Only back up repos whose names start with "rust-" or equal "my-tool"
include_repos = ["rust-*", "my-tool"]

# But skip any repo whose name ends in "-archive" (patterns match the repository
# name only; there is no pattern for GitHub's "archived" flag)
exclude_repos = ["*-archive"]
```

## Options That Cannot Be Set in the File

These flags have no config key and must come from the command line (or, where
one exists, the environment): `--config`, `--print-config-template`,
`--doctor`, `--check`, `--list-scopes`, `--device-auth`, `--oauth-client-id`,
`--oauth-scopes`, `--full`, `--mirror-type`, `--s3-session-token`,
`--s3-delete-stale`, `--dry-run`, `--manifest`, `--verify`, `--keep-last`,
`--max-age-days` (deprecated), `--prometheus-metrics`, `--diff-with`,
`--notify-webhook`, `--history-size`, `--restore`, `--restore-target-org`,
`--restore-yes`, `--encrypt-key`, `--decrypt`, `--decrypt-input`,
`--decrypt-output`, `--quiet`, `--verbose` and `--tui`.  A file that tries to
set one of them is rejected as an unknown key.

## The Template

`github-backup --print-config-template` prints an annotated file that lists all
63 keys, commented out.  Some of its comments are outdated: it marks `output`
as required (it defaults to `.`), says that command-line flags always override
the file (see [Precedence](#precedence)), suggests `discussions = true` and
`projects = true` (they have no effect) and says that an unset `since` inherits
the previous run's timestamp (each repository has its own watermark).
