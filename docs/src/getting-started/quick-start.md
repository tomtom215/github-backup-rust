# Quick Start

## 1. Get a GitHub Token

Create a [Personal Access Token](https://github.com/settings/tokens) with the following scopes:

| Token type | Recommended scopes |
|-----------|-------------------|
| Classic PAT | `repo`, `gist`, `read:org` |
| Fine-grained | Repository: `Contents (read)`, `Issues (read)`, `Pull requests (read)`, `Metadata (read)` |

Export it as an environment variable:

```bash
export GITHUB_TOKEN=ghp_your_token_here
```

## 2. Run Your First Backup

Back up everything for a user:

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /var/backup/github \
  --all
```

Back up only repositories and issues for an organisation:

```bash
github-backup my-org \
  --token "$GITHUB_TOKEN" \
  --output /var/backup/github \
  --org \
  --repositories \
  --issues
```

## 3. Explore the Output

```
/var/backup/github/
└── octocat/
    ├── git/
    │   ├── repos/
    │   │   ├── Hello-World.git/      ← bare mirror clone
    │   │   └── Spoon-Fork.git/
    │   ├── wikis/
    │   │   └── Hello-World.wiki.git/
    │   └── gists/
    │       └── abc123.git/
    └── json/
        ├── starred.json
        ├── watched.json
        ├── followers.json
        ├── following.json
        └── repos/
            └── Hello-World/
                ├── issues.json
                ├── issue_comments.json
                ├── pulls.json
                ├── releases.json
                ├── labels.json
                └── milestones.json
```

## 4. Common Recipes

### Selective backup with high concurrency

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /backup \
  --repositories \
  --issues \
  --pulls \
  --releases \
  --concurrency 8
```

### Shallow clone (saves disk space)

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /backup \
  --repositories \
  --clone-type shallow:10
```

### Dry-run (preview without writing)

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /backup \
  --all \
  --dry-run
```

### Using a config file

```bash
# Create /etc/github-backup/config.toml
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

github-backup --config /etc/github-backup/config.toml --token "$GITHUB_TOKEN"
```

### Mirror to Codeberg after backup

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /backup \
  --repositories \
  --mirror-to https://codeberg.org \
  --mirror-token "$CODEBERG_TOKEN" \
  --mirror-owner your_codeberg_username
```

### S3 sync after backup

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /backup \
  --all \
  --s3-bucket my-backup-bucket \
  --s3-region us-east-1 \
  --s3-access-key "$AWS_ACCESS_KEY_ID" \
  --s3-secret-key "$AWS_SECRET_ACCESS_KEY"
```

## Next Steps

- [Authentication options](authentication.md) — PAT vs. OAuth device flow
- [Backup categories](../backup-categories.md) — what each flag backs up
- [CLI Reference](../configuration/cli-reference.md) — all flags explained
- [Docker](../docker.md) — containerised and scheduled backups
