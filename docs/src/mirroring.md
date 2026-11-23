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

1. skips it if its backup failed in this run, if it is a shallow clone, or if
   it has no branch or tag (see [What Is Skipped](#what-is-skipped));
2. checks whether the repository exists at the destination and creates it if not;
3. verifies that it may write to it (see [Safety](#safety));
4. pushes all branches and tags (`git push` with the refspecs
   `+refs/heads/*:refs/heads/*` and `+refs/tags/*:refs/tags/*`); with
   `--prune` it also deletes destination branches and tags that no longer
   exist locally (`git push --prune`).

This happens after the local backup of the same run.  Under `--dry-run` it is
skipped.

> **A mirror is a replica, not a second backup.**  It holds what the local
> clone holds: a branch that was force-pushed on GitHub is force-updated in
> the mirror too, and with `--prune` a branch or tag deleted on GitHub is
> deleted there as well.  Without `--prune` (the default, as for the local
> clones) deleted branches and tags are kept in the mirror.  For history that
> must survive a mistake on GitHub, keep snapshots of the backup directory
> (restic, borg, ZFS).

## What Is and Is Not Pushed

* **Branches and tags**: yes, force-updated; deletions are mirrored only with
  `--prune`.
* **`refs/pull/*`** and other namespaces a GitHub mirror clone contains: **no**.
  GitHub-style hosts reject them, which makes a plain `git push --mirror` fail as
  a whole.
* **Only directories ending in `.git` under `git/repos/`**: mirror and bare
  clones and `--lfs`.  **Shallow clones cannot be mirrored** (see below).
  Working-tree clones (`--clone-type full`, stored as `<repo>/`) are **not**
  pushed.  Wikis, gists and starred clones are not pushed either.
* **LFS objects** are not pushed.
* **Issues and other metadata** are not pushed; only git data.

## What Is Skipped

These repositories are not pushed, and this is **not** a failure of the run:

* **Repositories whose backup failed in this run.**  Their clone may be stale
  or incomplete, and pushing it could rewind the mirror.  (The backup failure
  itself already makes the run incomplete.)  They are pushed again by the next
  run whose backup succeeds.
* **Shallow clones** (`--clone-type shallow:<n>`, or any clone with a `shallow`
  file).  Git servers refuse a push that would make the receiving repository
  shallow (`receive.shallowUpdate` is off by default, and Gitea, Forgejo and
  GitLab do not turn it on), so these can never be mirrored.  One warning per
  run lists them; use `--clone-type mirror` or `bare` for repositories you
  want mirrored.
* **Clones without any branch or tag** (an empty GitHub repository, or a
  damaged clone).  There is nothing to push, and with `--prune` the push
  would delete everything at the destination.

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
* **HTTPS only.**  The API token is sent in a request header, so `--mirror-to`
  must be an `https://` URL; an `http://` URL is rejected at start-up.  A
  self-hosted instance with a private certificate authority works when the CA
  is in the system store (or `SSL_CERT_FILE` / `SSL_CERT_DIR` point to it).
* **Settings are checked before the backup starts**: a missing token, an
  `http://` URL or a missing owner ends the run at once (exit status `1`)
  instead of after the backup.

## Mirror Flags

| Flag | Env var | Description |
|------|---------|-------------|
| `--mirror-to <URL>` | none | Base URL of the destination |
| `--mirror-type <TYPE>` | none | `gitea` (default) or `gitlab`; command line only (no config-file key) |
| `--mirror-token <TOKEN>` | `MIRROR_TOKEN` | API token (the variable is ignored unless `--mirror-to` is given) |
| `--mirror-owner <OWNER>` | none | User, organisation (Gitea) or namespace (GitLab) to create repositories under; default: the GitHub OWNER |
| `--mirror-private` | none | Create every repository private (the default) |
| `--mirror-public` | none | Create mirrors of public repositories as public |
| `--prune` | none | Also delete destination branches and tags that no longer exist locally (the same flag prunes the local clones) |

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

`--mirror-owner` is the namespace path: a user, a group or a subgroup
(`group/subgroup`).  It is resolved to its id with `GET /api/v4/namespaces/<path>`
and the project is created there with `POST /api/v4/projects` (`namespace_id`,
`path`); a namespace that does not exist, or that the token cannot see, is an
error for every repository.  The token must be allowed to create projects there
(a personal or group access token with the `api` scope).

## Failures

A destination that cannot be reached, a refused push or a foreign repository
is logged per repository and **recorded as a failure** of the run (step
`mirror push`, exit status `3`); the remaining repositories are still
mirrored.  An error answer from the API is quoted in the message (for example
Forgejo's `422` for a repository name it rejects).  API calls that fail with a
`5xx` status, a dropped connection or a timeout are tried up to three times.
The destination's own API calls do not use `HTTPS_PROXY` (the `git push`
itself follows git's proxy settings).

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
   