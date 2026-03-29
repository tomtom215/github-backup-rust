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

**CLI flags always override config file values.**

This means you can define a base configuration in the file and override specific values per-run:

```bash
# Config: owner = "octocat", concurrency = 4
# Override concurrency for this run only:
github-backup --config config.toml --concurrency 16
```

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
# clone_type = "mirror"  # mirror | bare | full | shallow:<depth>
# prefer_ssh  = false
# lfs         = false
# no_prune    = false

# ── Backup categories ──────────────────────────────────────────────────────

# Enable everything (except clone_starred and action_runs which are opt-in)
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

# ── Organisation-specific ───────────────────────────────────────────────────
org_members      = false
org_teams        = false

# ── Reporting ─────────────────────────────────────────────────────────────
# report = "/var/log/github-backup/report.json"

# ── Mirror to Gitea/Codeberg ───────────────────────────────────────────────
# mirror_to      = "https://codeberg.org"
# mirror_token   = "cb_token"         # or use MIRROR_TOKEN env var
# mirror_owner   = "alice"
# mirror_private = false

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

A production-ready config for nightly scheduled backups:

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
| `report` | path | — | Write JSON summary report to this file |

### Clone Behaviour

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `clone_type` | string | `mirror` | `mirror`, `bare`, `full`, or `shallow:<depth>` |
| `prefer_ssh` | bool | `false` | Use SSH clone URLs instead of HTTPS |
| `lfs` | bool | `false` | Enable Git LFS when cloning |
| `no_prune` | bool | `false` | Do not prune deleted remote refs |

### Backup Categories

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `all` | bool | `false` | Enable all categories (excluding opt-in) |
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
| `org_members` | bool | `false` | Back up org member list |
| `org_teams` | bool | `false` | Back up org team list |
| `actions` | bool | `false` | Back up GitHub Actions workflow metadata |
| `action_runs` | bool | `false` | Back up workflow run history (opt-in; can be large) |
| `environments` | bool | `false` | Back up deployment environment configurations |
| `include_repos` | string array | `[]` | Only back up repos matching these glob patterns |
| `exclude_repos` | string array | `[]` | Exclude repos matching these glob patterns |
| `since` | string | — | ISO 8601 timestamp: only fetch issues/PRs updated after this |

### Mirror Destination

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `mirror_to` | string | — | Push mirrors to this Gitea/Codeberg/Forgejo base URL |
| `mirror_token` | string | — | API token for the mirror host (prefer `MIRROR_TOKEN` env var) |
| `mirror_owner` | string | — | Owner name at the mirror destination |
| `mirror_private` | bool | `false` | Create repos as private at the mirror destination |

### S3-Compatible Storage

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `s3_bucket` | string | — | S3 bucket name (required to enable S3 sync) |
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

## Incremental Backup Config

```toml
owner = "octocat"
output = "/var/backup/github"
issues = true
pulls  = true
# Only fetch issues/PRs updated after this date (update each run)
since  = "2026-01-01T00:00:00Z"
```

## Repository Filter Config

```toml
owner        = "octocat"
output       = "/var/backup/github"
repositories = true

# Only back up repos whose names start with "rust-" or equal "my-tool"
include_repos = ["rust-*", "my-tool"]

# But exclude archived repos regardless
exclude_repos = ["*archived*"]
```
