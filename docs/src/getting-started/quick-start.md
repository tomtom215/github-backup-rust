# Quick Start

## 1. Get a GitHub Token

Create a [personal access token](https://github.com/settings/tokens).  For a
complete backup of your own account a classic token with `repo`, `gist`,
`read:org` and `read:packages` covers everything; for public data no token is
needed.  The scopes and fine-grained permissions per category are in
[Authentication](authentication.md#what-each-category-needs).

Export it as an environment variable (not on the command line, where `ps` and
your shell history can see it):

```bash
export GITHUB_TOKEN=ghp_your_token_here
```

## 2. Check the Setup

```bash
github-backup octocat --output /var/backup/github --doctor
```

`--doctor` checks that `git` is installed, that the output directory is
writable, that the API is reachable and that GitHub accepts the token.  It
does not check disk space or scopes.

## 3. Run Your First Backup

### Option A: Interactive TUI

```bash
github-backup octocat --tui
```

The TUI pre-fills the owner (and token and output directory, if given), shows
all settings in tabbed panels and lets you start the backup and watch live
progress.  See the [Interactive TUI guide](../tui.md).

### Option B: Command line

Back up everything for a user:

```bash
github-backup octocat --output /var/backup/github --all
```

(`--all` includes private repositories and secret gists when the token
belongs to `octocat`; see [Private repositories](../backup-categories.md#private-repositories).)

Back up only repositories and issues of an organisation:

```bash
github-backup my-org \
  --output /var/backup/github \
  --org \
  --repositories \
  --issues
```

The command prints a plan, then progress, then a summary.  Check the **exit
status**: `0` means complete, `3` means the backup finished but something
could not be backed up (the summary lists it; run again to retry), `1` means it
could not run at all.  See [Exit Codes](../configuration/cli-reference.md#exit-codes).

## 4. Explore the Output

```
/var/backup/github/
├── .github-backup.lock
└── octocat/
    ├── git/
    │   ├── repos/
    │   │   ├── Hello-World.git/        ← mirror clone
    │   │   └── Spoon-Knife.git/
    │   ├── wikis/
    │   │   └── Hello-World.wiki.git/
    │   └── gists/
    │       └── abc123.git/
    └── json/
        ├── repos.json
        ├── starred.json
        ├── backup_state.json
        ├── backup_history.json
        └── repos/
            └── Hello-World/
                ├── info.json
                ├── issues.json
                ├── issue_comments/1.json
                ├── pulls.json
                ├── releases.json
                ├── labels.json
                └── milestones.json
```

(A selection; which files appear depends on the flags.)  The full listing is on
[Output Directory Layout](../configuration/output-layout.md).

## 5. Run It Again

Run the same command again whenever you like (or schedule it with
[systemd](../deployment/systemd.md) or [cron](../deployment/cron.md)).  Git
clones are updated in place and only changed issues and pull requests are
re-fetched; nothing already captured is lost.  See
[Incremental runs](../monitoring.md#incremental-runs-and-the-state-file).

## 6. Common Recipes

### Selective backup with higher concurrency

```bash
github-backup octocat \
  --output /backup \
  --repositories --issues --pulls --releases \
  --concurrency 8
```

### Shallow clone (saves disk space)

```bash
github-backup octocat \
  --output /backup \
  --repositories \
  --clone-type shallow:10
```

### Dry run (preview, writes nothing)

```bash
github-backup octocat --output /backup --all --dry-run
```

A dry run lists the repositories it would back up and writes nothing: no files,
no lock, no state, no report and no network call except the read-only listing.
Owner-level data and gists are skipped.

### Using a config file

```bash
mkdir -p /etc/github-backup
cat > /etc/github-backup/config.toml <<'EOF'
owner = "octocat"
output = "/var/backup/github"
concurrency = 8
repositories = true
issues = true
pulls = true
releases = true
wikis = true
EOF
chmod 600 /etc/github-backup/config.toml

github-backup --config /etc/github-backup/config.toml
```

(The token comes from `GITHUB_TOKEN`.)

### Push mirror to Codeberg after the backup

```bash
export MIRROR_TOKEN=your_codeberg_token
github-backup octocat \
  --output /backup \
  --repositories \
  --mirror-to https://codeberg.org \
  --mirror-owner your_codeberg_username
```

### S3 sync of the metadata

```bash
export AWS_ACCESS_KEY_ID=...  AWS_SECRET_ACCESS_KEY=...
github-backup octocat \
  --output /backup \
  --all \
  --s3-bucket my-backup-bucket \
  --s3-region us-east-1
```

The bucket must exist.  Only the JSON metadata goes to S3, not the repository
clones; see [S3](../storage/s3.md).

## Next Steps

- [Interactive TUI](../tui.md): full-screen interface with live progress
- [Authentication](authentication.md): tokens, scopes and the device flow
- [Backup categories](../backup-categories.md): what each flag backs up, and what it cannot
- [CLI Reference](../configuration/cli-reference.md): all flags and the exit codes
- [Monitoring](../monitoring.md): get told when a backup fails
- [Docker](../docker.md): containers and scheduled backups
