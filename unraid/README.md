# Unraid deployment

`github-backup-rust` ships an [Unraid](https://unraid.net) [Community
Applications](https://docs.unraid.net/unraid-os/using-unraid-to/run-docker-containers/community-applications/)
template so users can install and configure it from the Unraid WebUI
without touching the terminal.

```
unraid/
├── github-backup.xml     ← the template (this is what you submit to CA)
├── icon.png              ← 256×256 PNG referenced by <Icon> in the template
├── make_icon.py          ← regenerates icon.png (stdlib only)
└── README.md             ← this file

ca_profile.xml            ← CA developer profile; CA requires it in the repo ROOT
```

The template has been authored against the v7.2.x DockerMan / CA conventions
documented at <https://docs.unraid.net/unraid-os/using-unraid-to/run-docker-containers/>.
It sets no `<MinVer>`; it has not been tested on Unraid 6.x.

**Permissions.** The template passes `--user 99:100` (`nobody:users`, the owner of
Unraid shares), so files the backup writes belong to the same user SMB clients
act as.  If you ran an earlier version of this template (the container ran as
uid 1000), fix ownership of the existing output once:
`chown -R 99:100 /mnt/user/backups/github`.  The `umask` field (default `022`)
sets the mode of new files; use `000` only if other users must write into the share.
The template no longer passes `--rm`: the container must survive its own exit
so that the User Scripts pattern below (`docker start github-backup`) works.

## What the template gives the user

| WebUI field                       | Variable / mount        | Default                          | Notes |
|-----------------------------------|--------------------------|----------------------------------|-------|
| Output Directory                  | `/backup` (Path)         | `/mnt/user/backups/github/`      | Required. Point at an Unraid share. |
| GitHub Owner                      | `GITHUB_OWNER`           | _(empty)_                        | Required. User / org to back up. |
| GitHub Token                      | `GITHUB_TOKEN`           | _(empty)_                        | Required, **masked**. |
| Run Mode                          | `BACKUP_MODE` (dropdown) | `--all`                          | `--doctor`, `--check`, `--list-scopes`, `--verify`, `--print-config-template` also available.  `--tui` needs a terminal: run `docker run -it --rm --user 99:100 ghcr.io/tomtom215/github-backup-rust:latest --tui` from the Unraid terminal. |
| Extra CLI Flags                   | `BACKUP_FLAGS`           | _(empty)_                        | e.g. `--org --concurrency 8 --include-repos rust-*` (split on spaces, quotes are not interpreted). Shell metacharacters refused. |
| GitHub API URL (GHES)             | `GITHUB_API_URL`         | _(empty)_                        | Advanced. |
| GitHub Clone Host (split GHES)    | `GITHUB_CLONE_HOST`      | _(empty)_                        | Advanced. |
| OAuth App Client ID               | `GITHUB_OAUTH_CLIENT_ID` | _(empty)_                        | Advanced. Pair with `--device-auth` in Extra CLI Flags. |
| At-Rest Encryption Key            | `BACKUP_ENCRYPT_KEY`     | _(empty)_                        | Advanced, **masked**. 32-byte hex; generate with `openssl rand -hex 32`. |
| Notification Webhook              | `BACKUP_NOTIFY_WEBHOOK`  | _(empty)_                        | Advanced. JSON POST on completion. |
| umask                             | `UMASK`                  | `022`                            | Advanced. Octal file-creation mask. |
| Log Level                         | `RUST_LOG`               | `info`                           | Advanced. `info`/`debug`/`trace`/`warn`/`error`. |
| HTTPS Proxy                       | `HTTPS_PROXY`            | _(empty)_                        | Advanced. Honoured by both the API client and git. |

## How the env-var workflow works

CLI / Compose / Kubernetes users invoke the binary directly:

    docker run --rm -e GITHUB_TOKEN ghcr.io/tomtom215/github-backup-rust:latest octocat --all

That continues to work unchanged.

Unraid CA, however, fills in env vars from a form — it does not let
the user supply positional arguments. The image therefore ships a tiny
POSIX shell wrapper (`docker/entrypoint.sh`) which behaves as follows:

- **If any positional arguments are supplied → exec verbatim.**
  The CLI / Compose / Kubernetes contract is preserved.
- **Otherwise → reconstruct argv from `GITHUB_OWNER`, `BACKUP_MODE`,
  and `BACKUP_FLAGS`.** This is the Unraid path.
- **Both empty → `github-backup --help`.**

`BACKUP_MODE` is restricted to a whitelist of known run modes plus
any flag starting with `--` (so future modes work without an image
rebuild); shell metacharacters are rejected up-front.

## First run

1. **Install** via Community Applications: search "github-backup" or
   add the template URL directly under *Settings → Community
   Applications → Settings → Add Container → Template URL*:
   `https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/unraid/github-backup.xml`
2. **Fill the form**: at minimum `GitHub Owner`, `GitHub Token`, and
   keep `Run Mode = --doctor` for the first run.
3. **Start the container**. The pre-flight diagnostic runs in a few
   seconds; review the colour-coded output in the container's log
   (Docker tab → click the github-backup icon → Logs).
4. **Change `Run Mode` to `--all`** and start the container again.
   The backup runs to completion and exits.

The container is one-shot: it exits when the backup finishes.  The exit code
is visible with `docker ps -a` (0 on success); the Docker tab only shows
started/stopped.  Blank form fields are passed to the container as empty
variables; the entrypoint treats an empty optional variable as unset.

## Scheduling recurring backups

Unraid does not currently support a built-in scheduler for docker
containers. The community-standard pattern is:

1. Install the **User Scripts** plugin (already in Community
   Applications: <https://forums.unraid.net/topic/48286-plugin-ca-user-scripts/>).
2. Create a new script named e.g. `github-backup-daily`:

   ```sh
   #!/bin/bash
   docker start github-backup
   ```

3. Set the schedule to a cron expression — daily at 02:00 is `0 2 * * *`.

The script returns immediately; the container runs in the background
and writes structured progress to its Docker log. A *successful* run
exits with code 0; *failure* exits with the error category's code
(usually 1); read the log or `docker ps -a` to see which.

## Restore

`--restore` is not exposed as a `BACKUP_MODE` option on purpose: it
*writes* to GitHub and we don't want a stray click to recreate
hundreds of issues against the wrong org. To run it, supply explicit
arguments via the *Post Arguments* field on the WebUI edit page (or
exec from the Console):

    --restore --restore-target-org my-other-org --restore-yes

Set `GITHUB_BACKUP_RESTORE_YES=1` if you'd prefer the env-var form.

## Verify a previous backup

Switch `Run Mode` to `--verify` and start the container. It reads the
SHA-256 manifest under the configured output directory and exits 0
when every file matches, non-zero when anything is missing, tampered,
or unexpected.

## Submission to Community Applications

CA submission goes through the portal at <https://ca.unraid.net/submit>
(validate and scan steps; the CA docs under `ca.unraid.net/submit/help/`
describe the fields).  Checklist for this repository:

1. `ca_profile.xml` is in the repository root with a non-empty `<Profile>`
   (CA blocks submission otherwise).
2. `<Support>` points at the GitHub issue tracker (CA only asks for a support
   URL; a forum thread is optional).
3. `<Category>` uses CA's `Main:` / `Main:Sub` syntax separated by spaces
   (currently `Backup: Tools:Utilities`); confirm it in the portal's Validate step.
4. `<Repository>` pulls from GHCR (`release.yml` publishes multi-arch images on
   each tagged release).
5. Open source (MIT, per `LICENSE`).

Not verified from this repository: that the portal accepts the template as-is.
Run its Validate step before submitting.

## Local testing without submitting

You can install the template directly from a local file without
touching the registry:

1. Copy the XML to your Unraid box's `/boot/config/plugins/dockerMan/templates-user/`
   (e.g. via `scp` or the SMB share).
2. Open *Docker → Add Container* in the WebUI; the template will
   appear under "User Templates".
3. Click **Apply**, fill in `GITHUB_TOKEN` + `GITHUB_OWNER`, and
   start.

Any subsequent edit you make in the WebUI is written back to the same
file, so you can diff it against this version to see how DockerMan
re-emits the template.

## Icon

`icon.png` is a square 256×256 PNG (opaque two-colour "GB" monogram),
regenerated with `python3 unraid/make_icon.py`.  The image is hot-linked from
the template via its raw GitHub URL.  Replace the placeholder with a designed
icon before submitting to CA.

## References

- [Unraid Docs — Community Applications](https://docs.unraid.net/unraid-os/using-unraid-to/run-docker-containers/community-applications/)
- [Selfhosters — writing a CA-compatible template](https://selfhosters.net/docker/templating/templating/)
- [Unraid wiki — DockerTemplateSchema](https://wiki.unraid.net/DockerTemplateSchema)
- [Unraid 7.2 release notes](https://docs.unraid.net/unraid-os/release-notes/7.2.0/)
- Reference templates studied:
  [binhex-rclone](https://raw.githubusercontent.com/binhex/docker-templates/master/binhex/rclone.xml),
  [cmccambridge/mosquitto](https://raw.githubusercontent.com/cmccambridge/unraid-templates/master/cmccambridge/mosquitto-unraid.xml),
  [ibracorp/unraid-templates](https://github.com/ibracorp/unraid-templates)
