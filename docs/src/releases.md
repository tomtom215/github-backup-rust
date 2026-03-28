# Releases & Assets

## Backup Release Metadata

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --releases
```

This saves a JSON array for each repository containing:
- Tag name and target commit
- Release title and body (markdown)
- Author and timestamps
- List of assets (filename, size, download count, URL)
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
└── releases/
    └── v1.0.0/
        ├── app-linux-x86_64.tar.gz
        ├── app-darwin-arm64.tar.gz
        └── checksums.txt
```

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
