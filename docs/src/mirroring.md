# Push to Gitea / Codeberg

After the primary backup, `github-backup` can push every cloned repository as a mirror to a Gitea-compatible self-hosted git instance.

## Supported Hosts

| Host | URL | Notes |
|------|-----|-------|
| **Codeberg** | `https://codeberg.org` | Forgejo-based public instance |
| **Forgejo** (self-hosted) | Your URL | Community fork of Gitea |
| **Gitea** (self-hosted) | Your URL | Original upstream |

Any service that implements the Gitea REST API v1 works.

## Basic Usage

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /backup \
  --repositories \
  --mirror-to https://codeberg.org \
  --mirror-token "$CODEBERG_TOKEN" \
  --mirror-owner your_codeberg_username
```

This will:
1. Clone/update all repositories from GitHub locally
2. Create any missing repositories on Codeberg (named identically)
3. Push all refs (`git push --mirror`) to Codeberg

## Getting a Codeberg Token

1. Log in at [codeberg.org](https://codeberg.org)
2. Go to **Settings → Applications → Manage Access Tokens**
3. Create a token with **write:repository** permission
4. Export: `export CODEBERG_TOKEN=your_token`

## Mirror Flags Reference

| Flag | Env Var | Description |
|------|---------|-------------|
| `--mirror-to <URL>` | — | Base URL of the Gitea-compatible instance |
| `--mirror-token <TOKEN>` | `MIRROR_TOKEN` | API token for the destination |
| `--mirror-owner <OWNER>` | — | Username or org to create repos under |
| `--mirror-private` | — | Create repos as private at the destination |

`--mirror-owner` defaults to the GitHub `OWNER` argument if not specified.

## How it Works

For each repository in the backup:

1. `GET /api/v1/repos/<owner>/<repo>` — check if the repo exists
2. If not: `POST /api/v1/user/repos` — create it (public or private)
3. `git push --mirror <gitea_repo_url>` — push all refs

Credentials for the git push are injected via `GIT_ASKPASS` (not embedded in the URL), keeping the token out of git's reflog.

## Self-Hosted Gitea Example

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /backup \
  --repositories \
  --mirror-to https://git.example.com \
  --mirror-token "$GITEA_TOKEN" \
  --mirror-owner octocat \
  --mirror-private
```

## Docker Compose with Codeberg Mirror

```yaml
services:
  backup:
    image: ghcr.io/tomtom215/github-backup:latest
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
      MIRROR_TOKEN: "${CODEBERG_TOKEN}"
    command: >
      octocat
      --output /backup
      --repositories
      --mirror-to https://codeberg.org
      --mirror-owner your_username
    volumes:
      - backup_data:/backup
```

See the full [Docker guide](docker.md) for more examples.

## Limitations

- The tool uses the **Gitea API** to create repositories.  GitLab and GitHub are not supported as mirror destinations through this mechanism (though you can always use `git push --mirror` manually on the backed-up clones).
- Mirroring is one-way: GitHub → mirror destination.  Changes made at the destination are overwritten on the next run.
- Large repositories may take a long time to push on first run.
