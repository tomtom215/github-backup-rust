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

# GitHub username or organisation to back up
owner = "octocat"

# Authentication (prefer GITHUB_TOKEN environment variable instead)
# token = "ghp_xxx"

# Output directory
output = "/var/backup/github"

# Parallelism
concurrency = 8

# Target type (default: user)
# org = true

# ── Backup categories ──────────────────────────────────────────────────────

# Enable everything
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

## Security

- Set file permissions to `0600` to prevent other users reading your token:

  ```bash
  chmod 600 /etc/github-backup/config.toml
  ```

- Prefer the `GITHUB_TOKEN` environment variable over storing the token in the config file, especially in multi-user environments.

## Config File Schema

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `owner` | string | — | GitHub username or org |
| `token` | string | — | Personal access token |
| `api_url` | string | `https://api.github.com` | GitHub API base URL (for GitHub Enterprise Server) |
| `output` | path | `.` | Output root directory |
| `concurrency` | integer | `4` | Parallel repository backup count |
| `org` | bool | `false` | Treat owner as org |
| `all` | bool | `false` | Enable all categories |
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
| `hooks` | bool | `false` | Back up webhooks |
| `security_advisories` | bool | `false` | Back up advisories |
| `wikis` | bool | `false` | Clone wikis |
| `starred` | bool | `false` | Back up starred repos |
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
