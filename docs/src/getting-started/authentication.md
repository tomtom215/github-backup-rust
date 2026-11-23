# Authentication

`github-backup` supports four ways to authenticate: a **personal access
token** (PAT, or the token of the GitHub CLI), a **GitHub App** (for unattended
organisation backups without a personal account), the **GitHub OAuth device
flow**, and **no credential** (public data only).

## Where the Token Comes From

The first of these that is set is used:

1. `--token` (avoid it: the value is visible in `ps`; the tool warns)
2. `GITHUB_TOKEN`
3. `GH_TOKEN` (the GitHub CLI's variable)
4. `token` in the config file
5. the GitHub CLI's login: `gh auth token --hostname github.com` (or the host
   of `--api-url` for GitHub Enterprise Server), if `gh` is installed and logged
   in.  The log says `using the GitHub CLI's login`; the token itself is never
   logged.

`--device-auth` replaces step 5 with the interactive device flow.  With none
of them the run is anonymous.  A [GitHub App](#github-app) (`--app-id`) is used
instead of all of them: configuring an App together with a token (steps 1 to 4)
or `--device-auth` is an error, and the GitHub CLI is not asked.

The simplest setup is therefore `gh auth login` once; its token has the `repo`,
`read:org` and `gist` scopes, enough for everything but packages and projects
(`gh auth refresh -s read:packages,read:project` adds those).

---

## Personal Access Token (Recommended)

The simplest and most reliable method for scheduled backups.

### Creating a Token

#### Classic PAT

1. Open [Settings → Developer settings → Personal access tokens → Tokens (classic)](https://github.com/settings/tokens).
2. **Generate new token (classic)**.
3. Select the scopes from [What each category needs](#what-each-category-needs).
   For a complete backup (the default, or `--all`; organisation or personal)
   select `repo`, `gist`, `read:org`, `read:packages`, `read:project` and
   `admin:repo_hook` (webhooks; leave it out with `--skip hooks`).
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
github-backup octocat --output /backup
```

Other ways, in order of preference: a config file with mode `0600`
(`token = "..."`; the tool warns if the file is readable by others) and the
`--token` flag.  A command-line value is visible in `ps` to every user of the
machine and ends up in the shell history, so the tool warns whenever a secret
(`--token`, `--mirror-token`, `--s3-secret-key`, `--s3-session-token`,
`--encrypt-key`, `--notify-webhook`) is given on the command line.

The token is only ever sent to the GitHub API host you configured, as
`Authorization: Bearer ...`, to `git` through a credential helper limited to
the clone URL's host, and, with `--attachments` against github.com, to
`https://github.com` as HTTP Basic authentication (see
[Attachments of private repositories](#attachments-of-private-repositories)).  It is not written to disk by the tool and is removed from
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
| `--projects` | `read:project` | Projects: read (organisation permission) |
| `--discussions` | `repo` (private); a token is always needed (GraphQL) | Discussions: read |
| commit comments (`--all`) | `repo` (private) | Contents: read |
| `--attachments` | `repo` (private) | reported not to work for private repositories, see below |
| `--restore` | `repo` (write) | Issues: read and write |

Notes:

* A user's **private repositories and secret gists** are listed only when the
  token belongs to that user, see
  [Private repositories](../backup-categories.md#private-repositories).
* For organisation targets (`--org`) the token's owner must be able to see the
  repositories.  If the organisation enforces SAML single sign-on, authorise the
  token for it (Settings → token → *Configure SSO*), otherwise GitHub answers 403.
* When GitHub refuses a category (HTTP 403), a category you asked for with its
  own flag is a failure naming the missing permission (exit status `3`); under
  `--all` it is skipped with one warning per category per run.  A category that
  does not exist for a repository (404/410) is skipped without a failure; see
  [When GitHub answers 403, 404 or 410](../backup-categories.md#when-github-answers-403-404-or-410).  A repository
  that cannot be listed or cloned at all is a failure (exit status `3`).
* **Attachments** have their own rules, see the next section.
* `github-backup --list-scopes <flags>` prints the classic scopes for the
  categories the flags select (with none selected, or `--all`: `admin:repo_hook
  gist read:org read:packages read:project repo`).

### Attachments of private repositories

Files attached to issues, comments and release notes (`--attachments`) are not
served by the API but by `github.com` itself, which redirects to a signed
storage URL.  Since
[May 2023](https://github.blog/changelog/2023-05-08-more-secure-private-attachments)
new attachments of private repositories need a signed-in user; older ones and
those of public repositories are reachable by their unguessable URL.

The project has not tested this against live accounts.  What is known comes
from [python-github-backup](https://github.com/josegonzalez/python-github-backup)
([issue #477](https://github.com/josegonzalez/python-github-backup/issues/477)):
a **classic token with the `repo` scope**, sent as HTTP Basic authentication
(`<token>:x-oauth-basic`), downloads them; **fine-grained and GitHub App
tokens** are answered `404 Not Found` for many attachments of private
repositories.  `github-backup` sends the token in that same form, to
`https://github.com` only, and only when it talks to github.com's API (a GitHub
Enterprise Server token never goes to github.com); the storage host gets no
credential.  A 404 cannot be told apart from a deleted attachment, so it is
recorded as missing, with a warning, rather than as a failure; after switching
to a classic token, run once with `--full` to ask for those again.  A 401, 403
or a sign-in page is a refusal: a failure with `--attachments`, one warning
per run under `--all`.

---

## GitHub App

A GitHub App lets an organisation run unattended backups without a personal
account or a long-lived personal token:

* the App acts as its **installation** on the organisation (or user), with only
  the permissions granted to it;
* its rate limit is the installation's: at least 5,000 requests per hour, rising
  with the number of repositories and users to 12,500 (15,000 on GitHub
  Enterprise Cloud organisations);
* no token is stored: each run signs a short JWT with the App's private key and
  exchanges it for an installation token that expires after one hour.  The tool
  renews that token five minutes before it expires, for API requests and for
  every `git` process, so a run may last as long as it needs.  Concurrent
  workers share one renewal.

### Create the App

1. Open the organisation's *Settings → Developer settings → GitHub Apps → New
   GitHub App* (or your account's, for a personal backup).
2. Give it a name and a homepage URL; untick **Webhook → Active** (the tool
   needs no events).
3. Grant the permissions below, all **Read-only**.
4. *Where can this GitHub App be installed?*: **Only on this account**.
5. Create it, note its **Client ID** (`Iv23...`; the numeric **App ID** works
   too), and under *Private keys* **Generate a private key**: the browser
   downloads a `.pem` file.  Keep it like a password.

| Flags | GitHub App permission |
|-------|-----------------------|
| `--repositories`, `--wikis`, `--releases`, `--release-assets`, `--branches`, `--topics`, commit comments | Repository: Contents, Metadata |
| `--issues`, `--issue-comments`, `--issue-events`, `--labels`, `--milestones` | Repository: Issues (pull request items also Pull requests) |
| `--pulls`, `--pull-comments`, `--pull-commits`, `--pull-reviews` | Repository: Pull requests |
| `--branches` protection rules, `--deploy-keys`, `--collaborators` | Repository: Administration |
| `--hooks` | Repository: Webhooks |
| `--security-advisories` | Repository: Repository security advisories |
| `--actions`, `--action-runs`, `--environments` | Repository: Actions (environments: Actions/Administration as GitHub documents) |
| `--org-members`, `--org-teams` | Organization: Members |
| `--projects` | Organization: Projects |
| `--gists`, `--starred-gists` | none: an App sees public gists only |
| `--packages` | GitHub documents the packages endpoints for classic tokens; expect a refusal (under `--all`, one warning) |
| `--restore` | Repository: Issues, **Read and write** |

These are the fine-grained permissions of [the table
above](#what-each-category-needs) (GitHub Apps and fine-grained tokens share
them); like that table, they come from GitHub's documentation and have not been
exercised against live accounts by the project.  `--doctor` lists what the
installation was granted.

### Install the App

On the App's page choose **Install App** and install it on the organisation (or
account), with **All repositories** or a selection.  An installation limited to
selected repositories backs up only those: an App lists its repositories with
`GET /installation/repositories`, which returns every repository the
installation can access, private ones included.

### Configure the Tool

```bash
export GITHUB_APP_ID=Iv23li0123456789abcd
github-backup my-org --app-key /etc/github-backup/app.pem --output /backup
```

| Setting | Command line | Environment | Config file |
|---------|--------------|-------------|-------------|
| Client ID or App ID | `--app-id` | `GITHUB_APP_ID` | `app_id` |
| Private key file | `--app-key <PATH>` | `GITHUB_APP_PRIVATE_KEY_PATH` | `app_key_path` |
| Private key itself (PEM text) | never on the command line | `GITHUB_APP_PRIVATE_KEY` | never in the file |
| Installation | `--app-installation-id` | `GITHUB_APP_INSTALLATION_ID` | `app_installation_id` |

* Give the key either as a file or in `GITHUB_APP_PRIVATE_KEY` (for containers
  that cannot mount a file), not both.  A one-line value with literal `\n`
  sequences is accepted.  The key must be the unencrypted PEM GitHub issued
  (`BEGIN RSA PRIVATE KEY`); unencrypted PKCS#8 (`BEGIN PRIVATE KEY`) works
  too.
* Without `--app-installation-id` the installation is looked up on OWNER, as an
  organisation (`GET /orgs/{org}/installation`) and then as a user (`GET
  /users/{username}/installation`); whether OWNER is an organisation is taken
  from the installation.  With it, OWNER may be left out: it is the
  installation's account, and naming another OWNER is an error.
* `--tui` and `--device-auth` cannot be combined with an App.

`github-backup my-org --app-id ... --app-key ... --doctor` checks, in order,
that GitHub accepts the App's JWT, that the installation is found (and belongs
to OWNER), and that an installation token can be minted and is accepted; it
prints the installation's permissions.

The private key is read once at start-up and wiped from memory when the run
ends; the JWT and the installation tokens are sent only to the API host, and
installation tokens reach `git` the way personal tokens do (an environment
variable and a host-scoped credential helper).  Installation tokens start with
`ghs_` and are removed from log and error text by that prefix.

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
  --output /backup
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
the token.  A lost connection or a server error while polling is retried until
the code expires.  If you do not authorise in time the flow ends with an
"expired" error; just run the command again.  Any other refusal is reported
with GitHub's own description (for example when the app does not have
**Enable Device Flow** ticked).  `--oauth-client-id` is only valid together
with `--device-auth`.

The flow honours the [proxy variables](../configuration/environment.md#proxy).
With `--api-url` it signs in on that GitHub Enterprise Server instead of
github.com (see [GitHub Enterprise](../configuration/github-enterprise.md#authentication)).

### Scopes

The default `--oauth-scopes` is `"repo gist read:org read:packages
read:project"`: every category of the default backup except webhooks, which
need the administrative `admin:repo_hook` scope (without it, webhooks are
skipped with a warning).  Add that scope when you want webhooks; narrow the list
if you back up fewer categories (see the table above).

---

## Unauthenticated Access (Public Data Only)

Omit the token to back up public data:

```bash
github-backup octocat --output /backup --repositories --issues --releases
```

At startup the tool warns:

```
WARN no GitHub credential supplied — running unauthenticated. Limited to public data and 60 requests / hour. Set GITHUB_TOKEN, run `gh auth login`, or use --device-auth for a full backup.
```

Unauthenticated requests are limited to **60 per hour per IP address**, which
accounts with many repositories or issues exhaust quickly.  Without a token the
categories must be chosen explicitly: when the categories to back up (after the
default, `--all`, the config file and `--skip`) include one that cannot work
anonymously (private repositories, hooks, deploy keys, collaborators,
organisation members or teams, Actions, packages, discussions, projects), the
tool refuses to start (exit status `1`).  An anonymous run also needs an OWNER:
without a token there is no account to default to.

---

## Verify the Token

```bash
github-backup octocat --doctor
```

`--doctor` asks GitHub whether the token is accepted (`GET /rate_limit`, which
costs no quota) and, without OWNER, which account it belongs to.  It does not
check scopes or permissions.  For a GitHub App it checks the JWT, the
installation and an installation token instead, and lists the installation's
permissions (see [GitHub App](#github-app)).

---

## Security Best Practices

1. Use `gh auth login`, or pass the token in `GITHUB_TOKEN` (or a `0600`
   config file), not with `--token`.
2. Prefer a fine-grained token limited to what you need, or a classic token with
   only the scopes in the table above.  For an organisation, prefer a GitHub
   App with read-only permissions.
3. Rotate tokens regularly and revoke one immediately if it may have leaked.
4. `chmod 600 /etc/github-backup/config.toml`.
5. Never commit tokens to version control.
