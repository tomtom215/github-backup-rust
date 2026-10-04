# Push Mirrors (Gitea, Codeberg, Forgejo, GitLab)

After the local backup, `github-backup` can push every cloned repository to a
second git host, so that a copy of the **code** exists somewhere that is not
your backup disk.  (S3 never receives git data; this is the off-site path for
code.)

## Supported Destinations

| `--mirror-type` | Hosts | API used to create repositories |
|-----------------|-------|---------------------------------|
| `gitea` (default) | Gitea, Forgejo, **Codeberg** | Gitea REST API v1 |
| `gitlab` | GitLab.com and self-managed GitLab CE/EE | GitLab REST API v4 |

GitHub itself is not a supported destination for `--mirror-to`.  (Pushing a
backup to a new GitHub repository is a manual step; see the
[Restore Guide](restore.md#git-data).)

## Basic Usage

```bash
export GITHUB_TOKEN=ghp_dummy_token
export MIRROR_TOKEN=dummy_codeberg_token

github-backup octocat \
  --output /backup \
  --repositories \
  --mirror-to https://codeberg.org \
  --mirror-owner your_codeberg_username
```

For each repository found under `<output>/octocat/git/repos/` the tool:

1. checks whether the repository exists at the destination and creates it if not;
2. verifies that it may write to it (see [Safety](#safety));
3. pushes all branches and tags, and deletes destination branches and tags
   that no longer exist locally (`git push --prune` with the refspecs
   `+refs/heads/*:refs/heads/*` and `+refs/tags/*:refs/tags/*`).

This happens after the local backup of the same run.  Under `--dry-run` it is
skipped.

## What Is and Is Not Pushed

* **Branches and tags**: yes, force-updated, with deletions mirrored.
* **`refs/pull/*`** and other namespaces a GitHub mirror clone contains: **no**.
  GitHub-style hosts reject them, which makes a plain `git push --mirror` fail as
  a whole.
* **Only directories ending in `.git` under `git/repos/`**: mirror, bare and
  shallow clones and `--lfs`.  Working-tree clones (`--clone-type full`, stored
  as `<repo>/`) are **not** pushed.  Wikis, gists and starred clones are not
  pushed either.
* **LFS objects** are not pushed.
* **Issues and other metadata** are not pushed; only git data.

## Safety

* **A repository is created private** by default.  `--mirror-public` makes the
  mirror of a repository public, but only if GitHub's listing (`repos.json`)
  says the source is public: a private source, or one whose visibility is
  unknown (no `repos.json`), is created private whatever the flags say.
  `--mirror-private` states the default explicitly.  An existing repository
  keeps its current visibility.
* **A repository this tool did not create is never pushed into.**  Every
  repository it creates gets the description `GitHub mirror of <owner>/<repo>`.
  An existing destination repository is accepted only if it carries that
  description or is still empty; otherwise the push is refused for that
  repository and recorded as a failure.  (If you want to mirror into an existing,
  non-empty repository, clear it first or set that description yourself.)
* **The token never appears in a command line or a file.**  It is handed to the
  `git` child in an environment variable and answered by a credential helper that
  is limited to the destination's scheme, host and port.

## Mirror Flags

| Flag | Env var | Description |
|------|---------|-------------|
| `--mirror-to <URL>` | none | Base URL of the destination |
| `--mirror-type <TYPE>` | none | `gitea` (default) or `gitlab`; command line only (no config-file key) |
| `--mirror-token <TOKEN>` | `MIRROR_TOKEN` | API token (the variable is ignored unless `--mirror-to` is given) |
| `--mirror-owner <OWNER>` | none | User, organisation (Gitea) or namespace (GitLab) to create repositories under; default: the GitHub OWNER |
| `--mirror-private` | none | Create every repository private (the default) |
| `--mirror-public` | none | Create mirrors of public repositories as public |

`--mirror-to`, `--mirror-token` (or `MIRROR_TOKEN`), `--mirror-owner`,
`--mirror-private` and `--mirror-public` can also be set in the config file; `--mirror-type` cannot.

### Gitea, Forgejo, Codeberg

`--mirror-owner` may be your own user name or an organisation: when it differs
from the login of the token, repositories are created through the
organisation endpoint (`POST /api/v1/orgs/<owner>/repos`), otherwise through
`POST /api/v1/user/repos`.  The token needs permission to create repositories
and to push (on Codeberg, create it under *Settings → Applications* with the
`write:repository` permission, and organisation permissions when
`--mirror-owner` is an organisation).

### GitLab

```bash
github-backup octocat --output /backup --repositories \
  --mirror-to https://gitlab.com \
  --mirror-type gitlab \
  --mirror-owner my-group-or-username
```

`--mirror-owner` is the namespace path (a user or a group).  The token must be
allowed to create projects there (a personal or group access token with the
`api` scope).

## Failures

A destination that cannot be reached, a refused push or a foreign repository
is logged per repository and **recorded as a failure** of the run (step
`mirror push`, exit status `3`); the remaining repositories are still
mirrored.  The destination's own API calls do not use `HTTPS_PROXY` (the `git
push` itself follows git's proxy settings).

## Docker Compose Example

```yaml
services:
  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    environment:
      GITHUB_TOKEN:
      MIRROR_TOKEN:
    command: >
      octocat
      --output /backup
      --repositories
      --mirror-to https://codeberg.org
      --mirror-owner your_username
    volumes:
      - backup_data:/backup

volumes:
  backup_data:
```

Export `GITHUB_TOKEN` and `MIRROR_TOKEN` in the shell (or an `.env` file) that
runs Compose.  See the [Docker guide](docker.md) for the bundled Compose
profiles (`codeberg`, `gitlab`).

## Limitations

* One-way: GitHub to destination.  Anything pushed to a mirror by hand on a
  branch that exists on GitHub is overwritten at the next run.
* The first push of a large repository can take a long time.  The push has no
  stall timeout of its own.
* The behaviour against real Codeberg, Gitea, Forgejo and GitLab servers has not
  been verified by the project; it is tested with mock servers and local git
  remotes.
