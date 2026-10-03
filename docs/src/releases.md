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

Assets are downloaded and stored alongside the JSON metadata:

```
json/repos/<repo>/
├── releases.json
└── release_assets/
    └── v1.0.0/
        ├── app-linux-x86_64.tar.gz
        ├── app-linux-x86_64.tar.gz.sha256
        ├── app-darwin-arm64.tar.gz
        ├── app-darwin-arm64.tar.gz.sha256
        ├── checksums.txt
        └── checksums.txt.sha256
```

### How downloads are made safe

- **Streamed, not buffered.** Each asset is written to disk as it arrives
  (to `.<file>.part` in the same directory) and renamed into place when
  complete, so memory use does not depend on the asset size and a crash,
  kill or full disk never leaves a truncated file under the real name.
- **Verified.** The byte count must equal the `size` GitHub reports, and the
  SHA-256 must equal GitHub's `digest` when the release provides one.  A
  download that fails either check is discarded and reported; the remaining
  assets are still attempted.
- **Resumable by re-running.** An asset already on disk is skipped only if its
  size equals GitHub's `size` and its content matches the API digest (or, if
  GitHub has none, the checksum in its `.sha256` sidecar).  A truncated or
  altered file is downloaded again.
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

By default, `--s3-bucket` only syncs JSON metadata.  Add `--s3-include-assets` to also upload the binary assets.

### Storage Estimates

| Repository type | Typical releases JSON | Typical assets |
|----------------|----------------------|----------------|
| Small library | < 100 KB | < 10 MB |
| Desktop app | < 1 MB | 50–500 MB per release |
| Large project (many versions) | 1–10 MB | GBs |
