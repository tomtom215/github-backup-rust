# Security

The security design of `github-backup`, how credentials are handled, and
recommendations for production.  Each statement here was checked against the
code; where something is a limitation it says so.

---

## Credential Handling

### Personal access tokens

- **API requests** carry `Authorization: Bearer <token>` (never a URL
  parameter) and go only to the API host you configured.
- **Release-asset downloads** send the token to GitHub's API host only: a
  redirect to another host, port or scheme is followed **without** the
  `Authorization` header, and a redirect from HTTPS to plain HTTP is refused.
- **Git over HTTPS**: the token is handed to the `git` child process in an
  environment variable and answered by an inline credential helper that is
  limited to the clone URL's scheme, host and port (`-c credential.helper=`
  first clears helpers you configured, so none can store the token).  No file is
  written and the token is never part of an argument list, a remote URL or the
  repository configuration.  A redirect, a submodule or a `.lfsconfig` that points
  at another host is never offered the token.  The token is in the **environment
  of the git process**: the same user, and root, can read it from
  `/proc/<pid>/environ` while git runs.
- **Mirror pushes** use the same mechanism for the destination token.
- **Logs and error messages** are scrubbed of the token (and of well-known token
  prefixes) before they are printed, stored in `--report`, or sent to a webhook;
  the webhook payload carries no error message text for individual failures, only
  scope and step.  `-v` and `-vv` do not print the token or request headers.
- **Command line**: a token passed with `--token`, an encryption key with
  `--encrypt-key` and S3 keys with `--s3-access-key` / `--s3-secret-key` are
  visible to every local user in the process list.  Use the environment
  variables.  The tool warns about `--encrypt-key` on the command line, not about
  the others.
- **`Debug` output** of the credential type is redacted.

### GitHub App

- The **private key** is read from a file (`--app-key`) or from
  `GITHUB_APP_PRIVATE_KEY`, never from the command line or the config file.
  The PEM text and the decoded key bytes are wiped from memory when they are
  dropped; the parsed key lives inside `aws-lc` (the library rustls already
  uses), whose allocator clears memory it frees.  Signing (RS256) is aws-lc's.
- The **JWT** (valid for 9 minutes) and the **installation tokens** (one
  hour) are sent only to the API host, as `Authorization: Bearer`, and are never
  logged or written to a file.  Installation tokens reach `git` exactly as
  personal tokens do (above), and are scrubbed from log and error text by
  their `ghs_` prefix.

### Environment variables

The recommended way to supply secrets: `GITHUB_TOKEN`, `AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `MIRROR_TOKEN`,
`BACKUP_ENCRYPT_KEY`.  They are visible to the same user and root through
`/proc`; use a `0600` environment file (systemd `EnvironmentFile=`,
`docker --env-file`) or your secret manager.

### Config files

A `token` (or S3 or mirror secret) in a TOML config file works but is the least
safe option.  Mode `0600` is recommended; the tool logs a warning when the file is
readable by group or others but does not refuse to use it.

---

## Minimum Token Permissions

The scopes and fine-grained permissions per category are in
[Authentication](../getting-started/authentication.md#what-each-category-needs).
In short: a classic token with `repo`, `gist`, `read:org`, `read:packages`,
`read:project` and `admin:repo_hook` covers every category of `--all`; a fine-grained token is preferable when you do
not need gists or packages.  Fine-grained tokens **can** access an organisation's
data when the organisation is the token's resource owner (and approves it), so
organisation backups do not require a classic token.

---

## Network Security

- **TLS only for GitHub and the mirror hosts**: the GitHub API client (which
  also serves `--doctor`), the OAuth device flow and the Gitea and GitLab API
  clients refuse plain `http://` URLs; `--api-url` must be `https://`.
- **Exceptions you control**: an `http://` S3 endpoint (for example a local
  MinIO) is allowed and logs a warning unless it is a loopback address; an
  `http://` webhook URL is allowed and logs a warning.  Both carry data in
  plaintext.
- **TLS stack**: `rustls` with certificates from the operating system's store
  (`rustls-native-certs`; `SSL_CERT_FILE` / `SSL_CERT_DIR` add trust).  No
  OpenSSL; the dependency policy bans `openssl`, `openssl-sys`, `native-tls` and
  `reqwest`.  There is no option to disable certificate verification.  The crypto
  provider (`aws-lc-rs`) contains C code.
- **Proxies**: HTTP proxies only (via `CONNECT`); proxy credentials in the
  variable are sent as `Proxy-Authorization` and are visible in the environment.
  See [Environment Variables](../configuration/environment.md#proxy).

---

## Dependency Policy

`deny.toml` is enforced by `cargo-deny` in CI and in the release workflow; a
daily scheduled workflow (`audit.yml`) also checks the advisories against the
`Cargo.lock` on `main`, so a new advisory is noticed without a code change:

| Policy | Rule |
|--------|------|
| Banned crates | `openssl`, `openssl-sys`, `reqwest`, `native-tls` |
| Allowed licences | MIT, Apache-2.0 (also with the LLVM exception), ISC, BSD-3-Clause, Unicode-3.0, CC0-1.0, Zlib |
| Advisories | `cargo deny check advisories` (RustSec) must pass; `deny.toml` ignores none |
| Sources | only crates.io; no unknown registries or git dependencies |

Dependabot proposes weekly updates for Cargo dependencies (grouped, with a
7-day cooldown) and GitHub Actions.  Every action in the workflows is pinned to
a full commit SHA, and CI runs [zizmor](https://docs.zizmor.sh/) on the
workflows.  The Rust toolchain (`rust-toolchain.toml`) and the Dockerfile's
base images are bumped by hand.

---

## Release and Supply Chain

- Release binaries (five targets) and `SHA256SUMS.txt` carry a GitHub
  **build-provenance attestation** (see [Installation](../getting-started/installation.md#verify-build-provenance-optional));
  the per-file `.sha256` files do not.  The binaries are not code-signed.
- Container images (releases from v0.4.0 on, and `edge`) are packaged from the
  Linux binaries built in the same workflow run, so a release image contains the
  attested release binary.  Each image has an SBOM, max-mode BuildKit
  provenance and a GitHub attestation pushed to the registry.
- A release tag must point to a commit on `main`; the release workflow re-runs
  the tests (Linux, macOS, Windows), clippy, the MSRV check and `cargo-deny`
  before building, restores no build cache, and creates the GitHub release as a
  draft that is published only after the binaries and the image succeeded.

---

## S3 Credential Security

S3 credentials are accepted through `--s3-access-key` / `--s3-secret-key` /
`--s3-session-token` or `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
`AWS_SESSION_TOKEN`.

- Prefer the environment variables (command-line values show up in `ps`).
- Credentials and the session token are kept out of `Debug` output and logs; the
  secret key is wiped from memory when the client is dropped (best effort).
- Use the minimum IAM policy: `s3:ListBucket` on the bucket and `s3:GetObject`,
  `s3:PutObject` (plus `s3:DeleteObject` with `--s3-delete-stale`, and the multipart
  actions for large assets) on `bucket/prefix/*`; see
  [S3](../storage/s3.md#required-permissions).
- Enable bucket versioning where `--s3-delete-stale` is used.
- Encryption protects contents, not names, sizes, or swapping of objects; see
  [At-Rest Encryption](../storage/encryption.md).

## Integrity: What the Manifest Does and Does Not Do

`--manifest` records SHA-256 digests of the data files under `json/` (not the
history, state, checkpoint and lock files, which change on every run, nor
temporary files of an unfinished write);
`--verify` compares them.  It detects accidental damage and casual edits.  It
does not cover the git clones, it is not signed (it sits next to the files it
describes), and it has no digest of its own.

## Output Directory Permissions

A backup can contain private repository code, webhook configuration, deploy key
metadata, collaborator lists and security advisories.  Restrict the directory:

```bash
mkdir -p /var/backup/github
chown backup-user:backup-group /var/backup/github
chmod 700 /var/backup/github
```

The tool does not set restrictive modes itself: new files follow the process
`umask`.  For multi-user systems consider disk encryption (LUKS or similar).  The
`--encrypt-key` feature protects only what is uploaded to S3.

---

## Unsafe Code Policy

The workspace's own source contains **no `unsafe` code**.
`github-backup-core` has `#![forbid(unsafe_code)]`; the client, mirror, S3 and
types crates `#![deny(unsafe_op_in_unsafe_fn)]`; the binary and TUI crates carry
neither attribute but contain no `unsafe` block.  (Third-party dependencies
such as `hyper`, `rustls` and `aws-lc-rs` do use `unsafe`.)  There is no workspace
lint table; the attributes are per crate.

---

## Reporting Security Vulnerabilities

Please do **not** put the details of a vulnerability in a public GitHub issue.
Use GitHub's private vulnerability reporting at
[github.com/tomtom215/github-backup-rust/security/advisories/new](https://github.com/tomtom215/github-backup-rust/security/advisories/new)
if that page offers a form; otherwise open an issue that only asks for a private
contact.  [`SECURITY.md`](https://github.com/tomtom215/github-backup-rust/blob/main/SECURITY.md)
has the details, the disclosure policy, the in-scope and out-of-scope
vulnerability classes and the expected response times.
