# Authentication

`github-backup` supports two authentication methods: a **Personal Access Token** (PAT) and the **GitHub OAuth device flow**.

---

## Personal Access Token (Recommended)

The simplest and most reliable method for automated and scheduled backups.

### Creating a Token

#### Classic PAT

1. Go to [GitHub Settings → Developer settings → Personal access tokens → Tokens (classic)](https://github.com/settings/tokens)
2. Click **Generate new token (classic)**
3. Select scopes:
   - `repo` — access private repositories
   - `gist` — back up gists
   - `read:org` — back up organisation repositories
4. Copy the generated token

#### Fine-grained PAT (Recommended for security)

1. Go to [GitHub Settings → Developer settings → Personal access tokens → Fine-grained tokens](https://github.com/settings/tokens?type=beta)
2. Click **Generate new token**
3. Set repository access to **All repositories** or specific repos
4. Grant these repository permissions:
   - **Contents**: Read
   - **Issues**: Read
   - **Pull requests**: Read
   - **Metadata**: Read (mandatory)
5. For webhooks: add **Webhooks: Read**
6. For security advisories: add **Security advisories: Read**

### Using the Token

Via CLI flag:
```bash
github-backup octocat --token ghp_xxx --output /backup --all
```

Via environment variable (preferred — keeps the token out of shell history):
```bash
export GITHUB_TOKEN=ghp_xxx
github-backup octocat --output /backup --all
```

Via config file (restrict file permissions to `0600`):
```toml
# /etc/github-backup/config.toml
token = "ghp_xxx"
```

---

## OAuth Device Flow

The interactive OAuth device flow is useful when you want to authenticate without
creating a long-lived PAT — for example on a new machine or in a CI environment
where you interact manually.

### Prerequisites

1. Create an [OAuth App on GitHub](https://github.com/settings/developers):
   - **Application name**: anything (e.g. `github-backup`)
   - **Homepage URL**: any valid URL
   - **Authorization callback URL**: `http://localhost` (not actually used by device flow)
2. Copy the **Client ID** (a string like `Iv1.xxxx`)

### Running Device Flow

```bash
github-backup octocat \
  --device-auth \
  --oauth-client-id Iv1.xxxx \
  --oauth-scopes "repo gist read:org" \
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

Open the URL in a browser, enter the code, and authorise the app. `github-backup` polls for the token automatically.

### Scopes

The default scope string `"repo gist read:org"` is sufficient for a complete backup.  Narrow it if you only need specific categories:

| Scope | Needed for |
|-------|-----------|
| `repo` | Private repositories, pull requests, releases, wikis |
| `gist` | Gists |
| `read:org` | Organisation repositories |
| `admin:repo_hook` | Webhooks (`--hooks`) |

---

## Security Best Practices

1. **Use environment variables** rather than `--token` CLI flags to keep tokens out of shell history and process listings.
2. **Use fine-grained PATs** scoped to only the repositories and permissions you need.
3. **Rotate tokens regularly** — if a token is compromised, rotate it immediately in GitHub settings.
4. **Restrict config file permissions**: `chmod 600 /etc/github-backup/config.toml`
5. **Never commit tokens** to version control.
