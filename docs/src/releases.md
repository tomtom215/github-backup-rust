# Releases & Assets

## Backup Release Metadata

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --releases
```

This saves a JSON array for each repository, each release exactly as GitHub returns it:
- Tag name and `target_commitish`
- Release title and body (markdown)
- Author and timestamps
- List of assets (filename, size, `digest`, download count, URL, uploader)
- Whether the release is a draft or prerelease

### Output

```
json/repos/<repo>/releases.json
```

The file accumulates: it is merged by release `id` with what earlier runs
stored, so a release deleted on GitHub stays in the backup.

### JSON schema excerpt

```json
[
  {
    "id": 12345,
    "tag_name": "v1.0.0",
    "name": "Version 1.0.0",
    "body": "## What's Changed\n...",
    "draft": false,
    "prerelease": false,
    "created_at": "2023-06-01T00:00:00Z",
    "published_at": "2023-06-01T12:00:00Z",
    "author": { "login": "octocat" },
    "assets": [
      {
        "name": "app-linux-x86_64.tar.gz",
        "size": 5242880,
        "download_count": 1234,
        "browser_download_url": "https://github.com/..."
      }
    ]
  }
]
```

---

## Download Release Assets

> **Warning**: Binary release assets can be very large. Assess disk space before enabling.

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --releases \
  --release-assets
```

`--release-assets` requires `--releases` to be set.

Assets are downloaded and stored alongside the JSON metadata, and the
SHA-256 of each one is recorded in a sidecar file in a directory of its own:

```
json/repos/<repo>/
├── releases.json
├── release_assets/
│   └── v1.0.0/
│       ├── app-linux-x86_64.tar.gz
│       ├── app-linux-x86_64.tar.gz.sha256   ← an asset the release itself publishes
│       └── checksums.txt
└── release_checksums/
    └── v1.0.0/
        ├── app-linux-x86_64.tar.gz.sha256
        ├── app-linux-x86_64.tar.gz.sha256.sha256
        └── checksums.txt.sha256
```

A sidecar holds one line, `<sha256>  <asset name>`.  Keeping the sidecars out
of `release_assets/` means a release that publishes its own `<file>.sha256`
is backed up as it is.  Earlier versions wrote the sidecar next to the asset;
on the next run such a file is moved to `release_checksums/`, and the old one
is removed only when its content is exactly that sidecar line (anything else
at that path, such as an asset GitHub no longer lists, is kept).

A release or asset the tool's model cannot read is still written to
`releases.json` as received; the assets of such a release are still read one
by one, and an asset that cannot be read at all is reported as a failure.

### How downloads are made safe

- **Streamed, not buffered.** Each asset is written to disk as it arrives
  (to `.<file>.part-<pid>-<n>` in the same directory) and renamed into place when
  complete, so memory use does not depend on the asset size and a crash,
  kill or full disk never leaves a truncated file under the real name.
- **Verified.** The byte count must equal the `size` GitHub reports, and the
  SHA-256 must equal GitHub's `digest` when the release provides one.  A
  download that fails either check is discarded and reported; the remaining
  assets are still attempted.
- **Resumable by re-running.** An asset already on disk is skipped only if its
  size equals GitHub's `size` and its content matches the API digest (or, if
  GitHub has none, the checksum in its sidecar).  A truncated or altered file
  is downloaded again.
- **Token stays with GitHub.** GitHub redirects asset downloads to its storage
  host.  The `Authorization` header is sent to GitHub only; a redirect to
  another host, port or scheme is followed without the token, and a redirect
  from HTTPS to plain HTTP is refused.

### Combining with S3

For large assets, sync them to S3 after backup:

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --releases --release-assets \
  --s3-bucket my-bucket \
  --s3-include-assets
```

By default, `--s3-bucket` only syncs JSON metadata (and the small checksum
sidecars under `release_checksums/`).  Add `--s3-include-assets` to also upload
the binary assets.

### Storage Estimates

| Repository type | Typical releases JSON | Typical assets |
|----------------|----------------------|----------------|
| Small library | < 100 KB | < 10 MB |
| Desktop app | < 1 MB | 50–500 MB per release |
| Large project (many versions) | 1–10 MB | GBs |
