# Authentication

`github-backup` supports three ways to authenticate: a **personal access
token** (PAT), the **GitHub OAuth device flow**, and **no credential** (public
data only).

---

## Personal Access Token (Recommended)

The simplest and most reliable method for scheduled backups.

### Creating a Token

#### Classic PAT

1. Open [Settings → Developer settings → Personal access tokens → Tokens (classic)](https://github.com/settings/tokens).
2. **Generate new token (classic)**.
3. Select the scopes from [What each category needs](#what-each-category-needs).
   For a complete backup (`--all`, organisation or personal) select
   `repo`, `gist`, `read:org` and `read:packages`.
4. Copy the token.

#### Fine-grained PAT

1. Open [Settings → Developer settings → Personal access tokens → Fine-grained tokens](https://github.com/settings/tokens?type=beta).
2. Choose the **resource owner** (your account, or the organisation whose
   repositories you back up; the organisation may need to approve the token) and
   repository access (**All repositories** or a selection).
3. Grant the repository permissions listed in the table below.
4. Fine-grained tokens cannot read some account-level resources
   (gists and packages are classic-only according to GitHub's documentation);
   use a classic token if you need `--gists`, `--starred-gists` or `--packages`.

### Using the Token

Environment variable (preferred: the token stays out of shell history and
the process list):

```bash
export GITHUB_TOKEN=ghp_dummy_token_for_illustration
github-backup octocat --output /backup --all
```

Other ways, in order of preference: a config file with mode `0600`
(`token = "..."`; the tool warns if the file is readable by others) and the
`--token` flag.  A command-line value is visible in `ps` to every user of the
machine and ends up in the shell history (the tool warns about that for
`--encrypt-key`, not for `--token`), so avoid `--token` on shared hosts.

The token is only ever sent to the GitHub API host you configured, as
`Authorization: Bearer ...`, and to `git` through a credential helper limited to
the clone URL's host.  It is not written to disk by the tool and is removed from
error messages.

---

## What Each Category Needs

The table maps each flag to the classic scope and the fine-grained permission
GitHub documents for the endpoint it uses.  It is derived from GitHub's
documentation and has **not** been exercised against live accounts by the
project: check with `--doctor` (token accepted) and a trial run, and look at the
failures it reports.

| Flags | Classic scope | Fine-grained permission |
|-------|---------------|-------------------------|
| `--repositories`, `--wikis` | `repo` for private repositories, none for public | Contents: read, Metadata: read |
| `--issues`, `--issue-comments`, `--issue-events`, `--labels`, `--milestones` | `repo` (private) | Issues: read (pull request items also Pull requests: read) |
| `--pulls`, `--pull-comments`, `--pull-commits`, `--pull-reviews` | `repo` (private) | Pull requests: read |
| `--releases`, `--release-assets`, `--branches`, `--topics` | `repo` (private) | Contents: read, Metadata: read |
| `--branches` protection rules, `--deploy-keys`, `--collaborators` | `repo`, and admin rights on the repository | Administration: read |
| `--hooks` | `repo` (or `admin:repo_hook`), and admin rights | Webhooks: read |
| `--security-advisories` | `repo` | Repository security advisories: read |
| `--actions`, `--action-runs`, `--environments` | `repo` | Actions: read (environments: Actions/Administration as GitHub documents) |
| `--org-members`, `--org-teams` | `read:org` | Organization: Members: read |
| `--starred`, `--watched` | none (public) or `repo` | Starring: read, Watching: read |
| `--followers`, `--following` | none (public data) | none |
| `--gists`, `--starred-gists` | `gist` | not available to fine-grained tokens |
| `--packages` | `read:packages` | not available to fine-grained tokens |
| `--discussions`, `--projects` | n/a | n/a: nothing is backed up (no REST endpoint) |
| `--restore` | `repo` (write) | Issues: read and write |

Notes:

* A user's **private repositories and secret gists** are listed only when the
  token belongs to that user, see
  [Private repositories](../backup-categories.md#private-repositories).
* For organisation targets (`--org`) the token's owner must be able to see the
  repositories.  If the organisation enforces SAML single sign-on, authorise the
  token for it (Settings → token → *Configure SSO*), otherwise GitHub answers 403.
* When a category is not permitted (HTTP 403 or 404) the tool skips that item
  with an `INFO` line instead of failing; see
  [Repository Metadata](../backup-categories.md#repository-metadata).  A repository
  that cannot be listed or cloned at all is a failure (exit status `3`).
* `github-backup --list-scopes <flags>` prints suggested scopes for the flags you
  give, but it does not expand `--all` (it prints only `public_repo repo`) and it
  suggests `user:follow` and `admin:public_key` for `--followers` and
  `--deploy-keys`, which only read.  Use the table above rather than that output
  for `--all`.

---

## OAuth Device Flow

The device flow signs in interactively without creating a long-lived PAT; the
token is **not stored**, so every run needs the login.  It is unsuitable for
unattended runs.

### Prerequisites

1. Create an [OAuth App](https://github.com/settings/developers):
   - **Application name**: anything (for example `github-backup`)
   - **Homepage URL**: any valid URL
   - **Authorization callback URL**: `http://localhost` (not used by the device flow)
   - tick **Enable Device Flow** in the app's settings
2. Copy the **Client ID**.

### Running Device Flow

```bash
github-backup octocat \
  --device-auth \
  --oauth-client-id Iv1.xxxx \
  --oauth-scopes "repo gist read:org read:packages" \
  --output /backup \
  --all
```

You will see:

```
──────────────────────────────────────────────────────
  GitHub OAuth device authorisation
──────────────────────────────────────────────────────
  1. Open:  https://github.com/login/device
  2. Enter: ABCD-1234
──────────────────────────────────────────────────────
  Waiting for authorisation…
```

Open the URL, enter the code and authorise the app; `github-backup` polls for
the token.  If you do not authorise in time the flow ends with an
"expired" error; just run the command again.  `--oauth-client-id` is only valid
together with `--device-auth`.

### Scopes

The default `--oauth-scopes` is `"repo gist read:org"`.  It does **not** cover
`--packages` (add `read:packages`).  Narrow it if you back up fewer categories;
see the table above.

---

## Unauthenticated Access (Public Data Only)

Omit the token to back up public data:

```bash
github-backup octocat --output /backup --repositories --issues --releases
```

At startup the tool warns:

```
WARN no GitHub credential supplied — running unauthenticated. Limited to public data and 60 requests / hour. Set GITHUB_TOKEN, pass --token, or use --device-auth for a full backup.
```

Unauthenticated requests are limited to **60 per hour per IP address**, which
accounts with many repositories or issues exhaust quickly.  If you name a flag
that cannot work anonymously (`--private`, `--hooks`, `--deploy-keys`,
`--collaborators`, `--org-members`, `--org-teams`, `--actions`, `--action-runs`,
`--packages`, `--discussions`, `--projects`) the tool refuses to start (exit
status `1`).  That check does not look inside `--all`, so do not use an
anonymous `--all`.

---

## Verify the Token

```bash
github-backup octocat --doctor
```

`--doctor` asks GitHub whether the token is accepted (`GET /rate_limit`, which
costs no quota).  It does not check scopes or permissions.

---

## Security Best Practices

1. Pass the token in `GITHUB_TOKEN` (or a `0600` config file), not with `--token`.
2. Prefer a fine-grained token limited to what you need, or a classic token with
   only the scopes in the table above.
3. Rotate tokens regularly and revoke one immediately if it may have leaked.
4. `chmod 600 /etc/github-backup/config.toml`.
5. Never commit tokens to version control.
