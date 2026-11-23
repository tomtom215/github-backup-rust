// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Files and images attached to issues, pull requests, discussions,
//! comments, reviews and release notes.
//!
//! GitHub has no API for attachments: they are links in the Markdown `body`
//! of an item, pointing at GitHub's own hosts.  The backup reads the bodies
//! it has already stored, picks out the links to GitHub's attachment hosts
//! and downloads each one once.  The stored JSON is never changed; the index
//! maps every URL to its file.
//!
//! # What counts as an attachment
//!
//! Only `https` URLs on these exact hosts and paths (no other port, no user
//! name), wherever they appear in a body: in Markdown links and images, in
//! HTML `src`/`href` attributes or as bare text, including in code spans:
//!
//! | Host | Path |
//! |---|---|
//! | `github.com` | `/user-attachments/assets/<id>`, `/user-attachments/files/<number>/<name>` |
//! | `github.com` | `/<owner>/<repo>/assets/<number>/<id>`, `/<owner>/<repo>/files/<number>/<name>` |
//! | `user-images.githubusercontent.com` | any |
//! | `private-user-images.githubusercontent.com` | any (signed, short-lived links) |
//!
//! No other host is ever contacted, except where GitHub itself redirects.
//!
//! # Where they are stored
//!
//! `attachments/<sha256>.<ext>` in the repository's metadata directory,
//! named by the SHA-256 of the content (so a file attached twice is stored
//! once), with the extension of the URL's file name, else one for the
//! response's media type, else none; and `attachments/index.json`, which
//! maps every URL to its file, size, media type, digest and the time it was
//! first downloaded.  The index accumulates: an entry is never removed.
//!
//! # Failures
//!
//! * 404 or 410: the attachment is gone.  It is entered in the index as
//!   missing (with the status), one warning per repository is logged, and it
//!   is asked for again after [`RETRY_MISSING_DAYS`] days, or on a `--full`
//!   run.  GitHub is reported to answer 404, not 403, when a fine-grained
//!   or GitHub App token asks for an attachment of a private repository; the
//!   warning says so.
//! * A signed `private-user-images` link answers any client error once its
//!   signature has expired (minutes after the page was rendered); it is
//!   entered as missing in the same way.
//! * 401, 403, or a web page (`text/html`) instead of a file: the token may
//!   not read the attachment.  Under `--all` or the default selection
//!   ([`BackupOptions::tolerate_forbidden`]) one warning per run; when
//!   `--attachments` was asked for explicitly, a failure.  Not entered in the
//!   index, so it is tried again next run.
//! * A file larger than [`MAX_ATTACHMENT_BYTES`], and any other error: a
//!   failure of the repository's attachments step (exit status 3), tried
//!   again next run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use url::Url;

use github_backup_client::{AssetSink, BackupClient, ClientError};
use github_backup_types::config::BackupOptions;

use crate::manifest::lower_hex;
use crate::{
    backup::{json_u64, Outcome},
    error::CoreError,
    storage::{PendingFile, Storage},
};

/// The largest attachment downloaded: 100 MiB.  GitHub accepts at most
/// 100 MB (videos on paid plans; 10 MB for images, 25 MB for other files).
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

/// Days after which an attachment recorded as missing is asked for again.
pub const RETRY_MISSING_DAYS: i64 = 30;

/// The index is written after this many changes, so that an interrupted run
/// keeps what it downloaded, and once more at the end.
const SAVE_EVERY: usize = 25;

/// What the token needs for private repositories' attachments.
const NEEDS: &str = "a classic personal access token with the `repo` scope; GitHub is reported \
                     to refuse fine-grained and GitHub App tokens for private attachments";

/// One URL of the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    /// File name in the `attachments` directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    /// When the URL was first downloaded, or first found missing.
    first_seen: String,
    /// HTTP status of the last attempt, when the attachment was missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    missing: Option<u16>,
    /// When it was last asked for, when it was missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checked_at: Option<String>,
}

type Index = BTreeMap<String, Entry>;

/// Downloads the attachments linked from the bodies stored in `meta_dir`
/// that are not in its index yet (see the module documentation).
///
/// `first_refusal` returns `true` the first time in a run that a refusal of
/// attachments is reported, so that under `--all` there is one warning per
/// run, not one per repository.
///
/// Returns the number of files downloaded in this run.
///
/// # Errors
///
/// Propagates storage errors from writing the index and any fatal error
/// (after saving the index).
pub async fn backup_attachments(
    client: &impl BackupClient,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
    first_refusal: &(dyn Fn() -> bool + Sync),
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.attachments {
        return Ok(outcome);
    }
    let dir = meta_dir.join("attachments");
    let index_path = dir.join("index.json");
    let mut index = load_index(storage, &index_path)?;
    let urls = stored_urls(storage, meta_dir, &mut outcome)?;

    let mut unsaved = 0usize;
    let mut missing = 0usize;
    for url in &urls {
        if !wanted(index.get(url), &dir, storage, opts.full) {
            continue;
        }
        let what = format!("attachment {}", without_query(url));
        match download(client, url, &dir, storage).await {
            Ok(Stored {
                file,
                size,
                content_type,
                sha256,
            }) => {
                let first_seen = index
                    .get(url)
                    .map_or_else(crate::time::now_utc, |e| e.first_seen.clone());
                index.insert(
                    url.clone(),
                    Entry {
                        file: Some(file),
                        size: Some(size),
                        content_type,
                        sha256: Some(sha256),
                        first_seen,
                        missing: None,
                        checked_at: None,
                    },
                );
                outcome.count += 1;
                unsaved += 1;
            }
            Err(Failure::Missing(status)) => {
                let now = crate::time::now_utc();
                let entry = index.entry(url.clone()).or_insert_with(|| Entry {
                    file: None,
                    size: None,
                    content_type: None,
                    sha256: None,
                    first_seen: now.clone(),
                    missing: None,
                    checked_at: None,
                });
                // A file downloaded earlier stays recorded; only its absence
                // on GitHub is noted.
                entry.missing = Some(status);
                entry.checked_at = Some(now);
                missing += 1;
                unsaved += 1;
            }
            Err(Failure::Refused(why)) => {
                if opts.tolerate_forbidden {
                    if first_refusal() {
                        warn!(
                            "the token is not permitted to download some attachments ({why}); \
                             they need {NEEDS}; skipping such attachments in this run"
                        );
                    }
                } else {
                    outcome.fail(
                        what,
                        format!(
                            "the token is not permitted to download it ({why}); it needs {NEEDS}"
                        ),
                    );
                }
            }
            Err(Failure::Error(e)) if e.is_fatal() => {
                if unsaved > 0 {
                    storage.write_json(&index_path, &index)?;
                }
                return Err(e);
            }
            Err(Failure::Error(e)) => {
                outcome.fail(what, e.to_string().replace(url, without_query(url)));
            }
        }
        if unsaved >= SAVE_EVERY {
            storage.write_json(&index_path, &index)?;
            unsaved = 0;
        }
    }
    if unsaved > 0 {
        storage.write_json(&index_path, &index)?;
    }

    if missing > 0 {
        warn!(
            dir = %dir.display(),
            missing,
            "attachments no longer exist on GitHub (404/410) or their signed link has expired; \
             recorded as missing and asked for again after {RETRY_MISSING_DAYS} days or with \
             --full.  For a private repository, GitHub is reported to answer 404 to \
             fine-grained and GitHub App tokens: use a classic token"
        );
    }
    info!(
        dir = %dir.display(),
        linked = urls.len(),
        downloaded = outcome.count,
        "attachments backed up"
    );
    Ok(outcome)
}

/// Whether `url` has to be downloaded: it is not in the index, its file is
/// gone from disk, or it was missing long enough ago (or `full`).
fn wanted(entry: Option<&Entry>, dir: &Path, storage: &impl Storage, full: bool) -> bool {
    let Some(entry) = entry else { return true };
    if entry.missing.is_some() {
        if full {
            return true;
        }
        let checked = entry
            .checked_at
            .as_deref()
            .and_then(crate::time::parse_rfc3339);
        return checked.is_none_or(|t| {
            crate::time::age(t) >= jiff::SignedDuration::from_hours(24 * RETRY_MISSING_DAYS)
        });
    }
    entry
        .file
        .as_deref()
        .is_none_or(|file| !storage.exists(&dir.join(crate::paths::file_name(file))))
}

/// The index at `path`; an unreadable one is reported and replaced, like
/// the merged lists (its files stay on disk and are found again by digest).
fn load_index(storage: &impl Storage, path: &Path) -> Result<Index, CoreError> {
    let Some(bytes) = storage.read(path)? else {
        return Ok(Index::new());
    };
    Ok(serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        warn!(
            path = %path.display(),
            error = %e,
            "the attachment index is unreadable; starting a new one"
        );
        Index::new()
    }))
}

/// Every attachment URL in the bodies stored in `meta_dir`: issues, pull
/// requests, discussions, releases and commit comments, the comment, review
/// comment and review files of every stored issue and pull request, and the
/// comments and replies of every stored discussion.  A file that
/// cannot be read is recorded and the others are still scanned.
fn stored_urls(
    storage: &impl Storage,
    meta_dir: &Path,
    outcome: &mut Outcome,
) -> Result<BTreeSet<String>, CoreError> {
    let mut urls = BTreeSet::new();
    let mut scan = |relative: &str, outcome: &mut Outcome| -> Result<Option<Value>, CoreError> {
        let path = meta_dir.join(relative);
        let Some(bytes) = storage.read(&path)? else {
            return Ok(None);
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => {
                for body in bodies(&value) {
                    extract_urls(body, &mut urls);
                }
                Ok(Some(value))
            }
            Err(e) => {
                outcome.fail(
                    format!("attachments in {relative}"),
                    format!("the stored file is not valid JSON: {e}"),
                );
                Ok(None)
            }
        }
    };
    let numbers = |value: Option<Value>| -> Vec<u64> {
        value
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| json_u64(item, "number"))
            .collect()
    };

    let issues = numbers(scan("issues.json", outcome)?);
    let pulls = numbers(scan("pulls.json", outcome)?);
    let discussions = numbers(scan("discussions.json", outcome)?);
    scan("releases.json", outcome)?;
    scan("commit_comments.json", outcome)?;
    for n in discussions {
        scan(&format!("discussion_comments/{n}.json"), outcome)?;
    }
    let conversations: BTreeSet<u64> = issues.iter().chain(&pulls).copied().collect();
    for n in conversations {
        scan(&format!("issue_comments/{n}.json"), outcome)?;
    }
    for n in pulls {
        scan(&format!("pull_comments/{n}.json"), outcome)?;
        scan(&format!("pull_reviews/{n}.json"), outcome)?;
    }
    Ok(urls)
}

/// The `body` of every object in a stored list (or of a stored object),
/// and of the replies a discussion comment holds in `replies.nodes`.
fn bodies(value: &Value) -> impl Iterator<Item = &str> {
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    items
        .into_iter()
        .flat_map(|item| {
            let replies = item
                .pointer("/replies/nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            std::iter::once(item).chain(replies)
        })
        .filter_map(|item| item.get("body").and_then(Value::as_str))
}

/// Adds every attachment URL in `text` to `urls`.
///
/// A URL ends at white space, a quote, a backslash, an angle, round or
/// square bracket or a backtick; trailing sentence punctuation is dropped,
/// `&amp;` is read as `&` (HTML attributes), and a `#fragment` is left out.
pub(crate) fn extract_urls(text: &str, urls: &mut BTreeSet<String>) {
    for (start, _) in text.match_indices("https://") {
        let rest = &text[start..];
        let end = rest
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '"' | '\''
                            | '\\'
                            | '<'
                            | '>'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '`'
                            | '{'
                            | '}'
                            | '|'
                    )
            })
            .unwrap_or(rest.len());
        let candidate = rest[..end]
            .split('#')
            .next()
            .unwrap_or_default()
            .trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_', '~'])
            .replace("&amp;", "&");
        if is_attachment_url(&candidate) {
            urls.insert(candidate);
        }
    }
}

/// Whether `candidate` is on the attachment allow-list (see the module
/// documentation).
pub(crate) fn is_attachment_url(candidate: &str) -> bool {
    let Ok(url) = Url::parse(candidate) else {
        return false;
    };
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    let Some(segments) = url.path_segments() else {
        return false;
    };
    let segments: Vec<&str> = segments.collect();
    let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let named = |s: &str| !s.is_empty();
    match url.host_str() {
        Some("github.com") => match segments.as_slice() {
            ["user-attachments", "assets", id] => named(id),
            ["user-attachments", "files", n, name] => number(n) && named(name),
            [owner, repo, "assets" | "files", n, name] => {
                named(owner) && named(repo) && number(n) && named(name)
            }
            _ => false,
        },
        Some("user-images.githubusercontent.com" | "private-user-images.githubusercontent.com") => {
            segments.iter().any(|s| !s.is_empty())
        }
        _ => false,
    }
}

/// `url` without its query, for messages: a signed link's query is a
/// short-lived credential.
fn without_query(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// What one download achieved.
struct Stored {
    file: String,
    size: u64,
    content_type: Option<String>,
    sha256: String,
}

/// Why one download did not.
enum Failure {
    /// Gone (404/410), or a signed link that expired: with the status.
    Missing(u16),
    /// The token may not read it: what GitHub answered.
    Refused(String),
    Error(CoreError),
}

/// Downloads `url` into `dir`, named by its digest.
async fn download(
    client: &impl BackupClient,
    url: &str,
    dir: &Path,
    storage: &impl Storage,
) -> Result<Stored, Failure> {
    info!(url = without_query(url), "downloading attachment");
    // The digest names the file, so the download lands at a fixed name
    // first; a run killed in between leaves it to be overwritten next time.
    let staging = dir.join(".incoming");
    let mut sink = CappedSink {
        file: storage.begin_write(&staging).map_err(Failure::Error)?,
        hasher: Sha256::new(),
        size: 0,
    };
    let got = match client.download_attachment(url, &mut sink).await {
        Ok(got) => got,
        Err(e) => return Err(classify(url, e)),
    };
    if got.content_type.as_deref() == Some("text/html") {
        return Err(Failure::Refused(
            "GitHub answered with a web page, probably a sign-in page, instead of the file"
                .to_string(),
        ));
    }
    let sha256 = lower_hex(&sink.hasher.clone().finalize());
    let file = crate::paths::file_name(&match extension(url, got.content_type.as_deref()) {
        Some(ext) => format!("{sha256}.{ext}"),
        None => sha256.clone(),
    });
    let size = sink.size;
    sink.file.finish().map_err(Failure::Error)?;
    let dest = dir.join(&file);
    if storage.exists(&dest) {
        storage.remove_file(&staging).map_err(Failure::Error)?;
    } else {
        storage.rename(&staging, &dest).map_err(Failure::Error)?;
    }
    Ok(Stored {
        file,
        size,
        content_type: got.content_type,
        sha256,
    })
}

/// Reads the error of one download (see the module documentation).
fn classify(url: &str, error: ClientError) -> Failure {
    let signed = Url::parse(url)
        .is_ok_and(|u| u.host_str() == Some("private-user-images.githubusercontent.com"));
    match error {
        ClientError::ApiError { status, .. }
            if matches!(status, 404 | 410) || (signed && (400..500).contains(&status)) =>
        {
            Failure::Missing(status)
        }
        ClientError::ApiError {
            status: status @ (401 | 403),
            ..
        } => Failure::Refused(format!("HTTP {status}")),
        // The body of an error page is a whole web page: the status says
        // enough.
        ClientError::ApiError { status, .. } if status != 0 => {
            Failure::Error(CoreError::Client(ClientError::ApiError {
                status,
                body: format!("GitHub answered HTTP {status}"),
            }))
        }
        other => Failure::Error(other.into()),
    }
}

/// The extension for the stored file: the URL's file name's (letters and
/// digits only, at most 10), else the usual one for `content_type`.
fn extension(url: &str, content_type: Option<&str>) -> Option<String> {
    let parsed = Url::parse(url).ok();
    let last = parsed
        .as_ref()
        .and_then(Url::path_segments)
        .and_then(|mut segments| segments.next_back())
        .unwrap_or_default();
    let from_name = last
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| {
            (1..=10).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase);
    from_name.or_else(|| {
        let ext = match content_type? {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            "image/bmp" => "bmp",
            "image/avif" => "avif",
            "video/mp4" => "mp4",
            "video/quicktime" => "mov",
            "video/webm" => "webm",
            "application/pdf" => "pdf",
            "application/zip" => "zip",
            "application/gzip" | "application/x-gzip" => "gz",
            "application/json" => "json",
            "text/plain" => "txt",
            "text/markdown" => "md",
            "text/csv" => "csv",
            _ => return None,
        };
        Some(ext.to_string())
    })
}

/// Receives a download chunk by chunk: writes it to the pending file, hashes
/// it and stops it at [`MAX_ATTACHMENT_BYTES`].
struct CappedSink {
    file: Box<dyn PendingFile>,
    hasher: Sha256,
    size: u64,
}

impl AssetSink for CappedSink {
    fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        self.size += chunk.len() as u64;
        if self.size > MAX_ATTACHMENT_BYTES {
            return Err(std::io::Error::other(format!(
                "the attachment is larger than {} MiB, the limit for one attachment",
                MAX_ATTACHMENT_BYTES >> 20
            )));
        }
        self.file.write_chunk(chunk)?;
        self.hasher.update(chunk);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const META: &str = "/meta";
    const IMG: &str =
        "https://github.com/user-attachments/assets/0b1f6c2e-aaaa-bbbb-cccc-1234567890ab";
    const LOG: &str = "https://github.com/user-attachments/files/17/build-output.LOG";
    const OLD: &str = "https://user-images.githubusercontent.com/42/abc-def.png";
    const REPO_FILE: &str = "https://github.com/octocat/hello/files/99/report.pdf";

    fn urls(text: &str) -> Vec<String> {
        let mut set = BTreeSet::new();
        extract_urls(text, &mut set);
        set.into_iter().collect()
    }

    fn opts() -> BackupOptions {
        BackupOptions {
            attachments: true,
            ..Default::default()
        }
    }

    fn meta(path: &str) -> PathBuf {
        Path::new(META).join(path)
    }

    fn put(storage: &MemStorage, path: &str, value: &Value) {
        storage.write_json(&meta(path), value).expect("write");
    }

    fn index(storage: &MemStorage) -> Index {
        serde_json::from_slice(&storage.get(&meta("attachments/index.json")).expect("index"))
            .expect("index json")
    }

    async fn run_with(
        client: &MockBackupClient,
        storage: &MemStorage,
        opts: &BackupOptions,
    ) -> (Result<Outcome, CoreError>, usize) {
        let refusals = AtomicUsize::new(0);
        let first = || refusals.fetch_add(1, Ordering::SeqCst) == 0;
        let result = backup_attachments(client, opts, Path::new(META), storage, &first).await;
        (result, refusals.load(Ordering::SeqCst))
    }

    async fn run(client: &MockBackupClient, storage: &MemStorage) -> Outcome {
        run_with(client, storage, &opts()).await.0.expect("run")
    }

    fn downloads(client: &MockBackupClient) -> usize {
        client
            .calls()
            .iter()
            .filter(|c| c.starts_with("attachment:"))
            .count()
    }

    // ── Extraction ───────────────────────────────────────────────────────

    #[test]
    fn markdown_html_and_bare_urls_are_found() {
        let text = format!(
            "![screenshot]({IMG})\n[log]({LOG}), see <img width=\"300\" src=\"{OLD}\"> \
             and <a href='{REPO_FILE}'>the report</a>.\nAlso {IMG}."
        );
        assert_eq!(urls(&text), [REPO_FILE, IMG, LOG, OLD]);
    }

    #[test]
    fn trailing_punctuation_fragments_and_html_entities_are_handled() {
        let signed = "https://private-user-images.githubusercontent.com/1/x.png?jwt=a.b.c&amp;v=2";
        let text = format!("See {IMG}#frag! and <img src=\"{signed}\"> **{LOG}**");
        assert_eq!(
            urls(&text),
            [
                IMG,
                LOG,
                "https://private-user-images.githubusercontent.com/1/x.png?jwt=a.b.c&v=2",
            ]
        );
    }

    #[test]
    fn a_url_in_a_code_span_counts_too() {
        assert_eq!(urls(&format!("`{IMG}`")), [IMG]);
    }

    #[test]
    fn only_the_allow_listed_hosts_and_paths_count() {
        for not in [
            "https://evil.example/user-attachments/assets/x",
            "https://github.com.evil.example/user-attachments/assets/x",
            "http://github.com/user-attachments/assets/x",
            "https://github.com:8443/user-attachments/assets/x",
            "https://user:pw@github.com/user-attachments/assets/x",
            "https://github.com/octocat/hello/issues/1",
            "https://github.com/octocat/hello/files/notanumber/a.txt",
            "https://github.com/user-attachments/files/1",
            "https://github.com/user-attachments/assets/",
            "https://avatars.githubusercontent.com/u/1?v=4",
            "https://objects.githubusercontent.com/github-production-release-asset/1",
            "https://raw.githubusercontent.com/o/r/main/a.png",
            "https://camo.githubusercontent.com/abc",
            "https://user-images.githubusercontent.com/",
            "https://user-images.githubusercontent.com.evil.example/1/a.png",
        ] {
            assert!(!is_attachment_url(not), "{not}");
            assert!(urls(&format!("x {not} y")).is_empty(), "{not}");
        }
        // A URL hidden in the query of another host is not picked out of it.
        assert!(
            urls("https://evil.example/?u=https:/github.com/user-attachments/assets/x").is_empty()
        );
        for yes in [
            IMG,
            LOG,
            OLD,
            REPO_FILE,
            "https://github.com/octocat/hello/assets/12/0b1f6c2e-aaaa",
            "https://GitHub.com/user-attachments/assets/x",
            "https://private-user-images.githubusercontent.com/1/2-3.png?jwt=x",
        ] {
            assert!(is_attachment_url(yes), "{yes}");
        }
    }

    #[test]
    fn extensions_come_from_the_name_or_the_media_type_and_are_safe() {
        assert_eq!(extension(LOG, Some("text/plain")).as_deref(), Some("log"));
        assert_eq!(extension(IMG, Some("image/jpeg")).as_deref(), Some("jpg"));
        assert_eq!(extension(IMG, Some("application/octet-stream")), None);
        assert_eq!(extension(IMG, None), None);
        for hostile in [
            "https://github.com/o/r/files/1/a.%2F..%2Fx",
            "https://github.com/o/r/files/1/a.p%5C..",
            "https://github.com/o/r/files/1/a.verylongextension",
            "https://github.com/o/r/files/1/a.ex%00e",
        ] {
            let ext = extension(hostile, Some("image/png")).expect("ext");
            assert_eq!(ext, "png", "{hostile}");
        }
    }

    // ── Downloads ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn every_body_is_scanned_and_each_url_downloaded_once() {
        let storage = MemStorage::default();
        let issues = json!([{"number": 1, "body": format!("![a]({IMG})")}]);
        put(&storage, "issues.json", &issues);
        put(
            &storage,
            "pulls.json",
            &json!([{"number": 2, "body": null}]),
        );
        put(
            &storage,
            "issue_comments/1.json",
            &json!([{"id": 5, "body": format!("again {IMG}")}]),
        );
        put(
            &storage,
            "issue_comments/2.json",
            &json!([{"id": 6, "body": OLD}]),
        );
        put(
            &storage,
            "pull_comments/2.json",
            &json!([{"id": 7, "body": format!("[x]({LOG})")}]),
        );
        put(
            &storage,
            "pull_reviews/2.json",
            &json!([{"id": 8, "body": REPO_FILE}]),
        );
        let in_release = "https://github.com/user-attachments/assets/release-notes";
        put(
            &storage,
            "releases.json",
            &json!([{"id": 3, "body": in_release}]),
        );
        let in_commit = "https://github.com/user-attachments/assets/commit";
        put(
            &storage,
            "commit_comments.json",
            &json!([{"id": 4, "body": in_commit}]),
        );
        let in_discussion = "https://github.com/user-attachments/assets/discussion";
        let in_reply = "https://github.com/user-attachments/assets/reply";
        put(
            &storage,
            "discussions.json",
            &json!([{"id": "D_1", "number": 9, "body": in_discussion}]),
        );
        put(
            &storage,
            "discussion_comments/9.json",
            &json!([{"id": "DC_1", "body": "", "replies": {"nodes": [{"id": "DC_2", "body": in_reply}]}}]),
        );
        let client = MockBackupClient::new()
            .with_attachment(IMG, b"png-bytes", Some("image/png"))
            .with_attachment(LOG, b"log text", Some("text/plain"))
            .with_attachment(OLD, b"old", Some("image/png"))
            // The same content under two URLs is stored once.
            .with_attachment(REPO_FILE, b"png-bytes", Some("application/pdf"))
            .with_attachment(in_release, b"r", None)
            .with_attachment(in_commit, b"c", Some("image/gif"))
            .with_attachment(in_discussion, b"d", Some("image/png"))
            .with_attachment(in_reply, b"e", Some("image/png"));

        let outcome = run(&client, &storage).await;

        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(outcome.count, 8);
        assert_eq!(downloads(&client), 8, "each URL once");
        let index = index(&storage);
        assert_eq!(index.len(), 8);
        let img = &index[IMG];
        let digest = crate::manifest::sha256_hex(b"png-bytes");
        assert_eq!(img.file, Some(format!("{digest}.png")));
        assert_eq!(img.sha256.as_deref(), Some(digest.as_str()));
        assert_eq!(img.size, Some(9));
        assert_eq!(img.content_type.as_deref(), Some("image/png"));
        assert!(crate::time::is_rfc3339(&img.first_seen));
        assert_eq!(
            storage
                .get(&meta(&format!("attachments/{digest}.png")))
                .as_deref(),
            Some(&b"png-bytes"[..])
        );
        assert_eq!(index[REPO_FILE].file, Some(format!("{digest}.pdf")));
        assert_eq!(
            index[in_release].file,
            Some(crate::manifest::sha256_hex(b"r"))
        );
        assert!(index[LOG]
            .file
            .as_deref()
            .is_some_and(|f| f.ends_with(".log")));
        assert!(storage.get(&meta("attachments/.incoming")).is_none());
        // The stored JSON is not touched.
        assert_eq!(
            storage.get(&meta("issues.json")),
            Some(serde_json::to_vec_pretty(&issues).unwrap())
        );
    }

    #[tokio::test]
    async fn indexed_urls_are_not_downloaded_again_and_old_entries_are_kept() {
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": IMG}]),
        );
        let client = MockBackupClient::new().with_attachment(IMG, b"x", Some("image/png"));
        run(&client, &storage).await;
        // An entry for a URL that no body links to any more.
        let mut kept = index(&storage);
        let edited_out = "https://github.com/user-attachments/assets/edited-out";
        kept.insert(edited_out.into(), kept[IMG].clone());
        storage
            .write_json(&meta("attachments/index.json"), &kept)
            .expect("write");

        let second = run(&client, &storage).await;

        assert_eq!(second.count, 0);
        assert_eq!(downloads(&client), 1, "not asked for again");
        assert!(index(&storage).contains_key(edited_out));
    }

    #[tokio::test]
    async fn an_indexed_file_deleted_from_disk_is_downloaded_again() {
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": IMG}]),
        );
        let client = MockBackupClient::new().with_attachment(IMG, b"x", Some("image/png"));
        run(&client, &storage).await;
        let file = meta(&format!(
            "attachments/{}",
            index(&storage)[IMG].file.clone().expect("file")
        ));
        storage.remove_file(&file).expect("rm");

        assert_eq!(run(&client, &storage).await.count, 1);
        assert!(storage.get(&file).is_some());
    }

    #[tokio::test]
    async fn a_missing_attachment_is_indexed_not_failed_and_not_asked_for_every_run() {
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": format!("{IMG} {OLD}")}]),
        );
        let client = MockBackupClient::new()
            .with_attachment(OLD, b"x", None)
            .with_error(&format!("attachment:{IMG}"), 410);

        let outcome = run(&client, &storage).await;

        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        let entry = &index(&storage)[IMG];
        assert_eq!(entry.missing, Some(410));
        assert!(entry.file.is_none());
        assert!(entry.checked_at.is_some());

        run(&client, &storage).await;
        assert_eq!(downloads(&client), 2, "not asked for again soon");

        // --full asks again.
        let full = BackupOptions {
            full: true,
            ..opts()
        };
        run_with(&client, &storage, &full).await.0.expect("run");
        assert_eq!(downloads(&client), 3);

        // So does a run after the retry period.
        let mut old = index(&storage);
        old.get_mut(IMG).expect("entry").checked_at = Some("2020-01-01T00:00:00Z".into());
        storage
            .write_json(&meta("attachments/index.json"), &old)
            .expect("write");
        run(&client, &storage).await;
        assert_eq!(downloads(&client), 4);
        assert_ne!(
            index(&storage)[IMG].checked_at.as_deref(),
            Some("2020-01-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn an_expired_signed_link_is_missing_not_a_failure() {
        let signed = "https://private-user-images.githubusercontent.com/1/x.png?jwt=e";
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": signed}]),
        );
        let client = MockBackupClient::new().with_error(&format!("attachment:{signed}"), 403);

        let (outcome, refusals) = run_with(&client, &storage, &opts()).await;

        assert!(outcome.expect("run").failed.is_empty());
        assert_eq!(refusals, 0);
        assert_eq!(index(&storage)[signed].missing, Some(403));
    }

    #[tokio::test]
    async fn a_refusal_fails_when_asked_for_and_is_tolerated_under_all() {
        let other = "https://github.com/user-attachments/assets/other";
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": format!("{IMG} {LOG} {other}")}]),
        );
        let client = MockBackupClient::new()
            .with_error(&format!("attachment:{IMG}"), 403)
            .with_error(&format!("attachment:{LOG}"), 401)
            .with_attachment(other, b"<html>sign in</html>", Some("text/html"));

        let explicit = run(&client, &storage).await;
        assert_eq!(explicit.failed.len(), 3, "{:?}", explicit.failed);
        assert!(explicit.failed.iter().all(|(_, m)| m.contains("classic")));
        assert!(
            storage.get(&meta("attachments/index.json")).is_none(),
            "nothing indexed: tried again next run"
        );

        let all = BackupOptions {
            tolerate_forbidden: true,
            ..opts()
        };
        let (tolerated, refusals) = run_with(&client, &storage, &all).await;
        assert!(tolerated.expect("run").failed.is_empty());
        assert_eq!(
            refusals, 3,
            "reported each time; the engine warns once per run"
        );
        assert!(storage.get(&meta("attachments/index.json")).is_none());
    }

    #[tokio::test]
    async fn other_errors_are_failures_and_the_rest_is_still_downloaded() {
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": format!("{IMG} {OLD}")}]),
        );
        storage
            .write_bytes(&meta("pulls.json"), b"{broken")
            .expect("write");
        let client = MockBackupClient::new()
            .with_attachment(OLD, b"x", None)
            .with_error(&format!("attachment:{IMG}"), 500);

        let outcome = run(&client, &storage).await;

        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(
            whats,
            ["attachments in pulls.json", &format!("attachment {IMG}")]
        );
        assert!(
            !outcome.failed[1].1.contains("mock"),
            "not the error page: {}",
            outcome.failed[1].1
        );
        assert_eq!(outcome.count, 1);
        let index = index(&storage);
        assert!(!index.contains_key(IMG), "tried again next run");
        assert!(index[OLD].file.is_some());
    }

    #[tokio::test]
    async fn a_signed_query_is_left_out_of_failure_messages() {
        let signed = "https://private-user-images.githubusercontent.com/1/x.png?jwt=SECRETJWT";
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": signed}]),
        );
        let client = MockBackupClient::new().with_error(&format!("attachment:{signed}"), 502);

        let outcome = run(&client, &storage).await;

        assert_eq!(outcome.failed.len(), 1);
        assert!(!format!("{:?}", outcome.failed).contains("SECRETJWT"));
    }

    #[test]
    fn the_size_cap_stops_a_download() {
        let storage = MemStorage::default();
        let mut sink = CappedSink {
            file: storage
                .begin_write(&meta("attachments/.incoming"))
                .expect("begin"),
            hasher: Sha256::new(),
            size: MAX_ATTACHMENT_BYTES - 2,
        };
        sink.write_chunk(b"ab").expect("exactly at the cap");
        let err = sink.write_chunk(b"c").expect_err("over the cap");
        assert!(err.to_string().contains("100 MiB"), "{err}");
    }

    #[tokio::test]
    async fn switched_off_does_nothing() {
        let storage = MemStorage::default();
        put(
            &storage,
            "issues.json",
            &json!([{"number": 1, "body": IMG}]),
        );
        let client = MockBackupClient::new();
        let (outcome, _) = run_with(&client, &storage, &BackupOptions::default()).await;
        assert_eq!(outcome.expect("run"), Outcome::default());
        assert!(client.calls().is_empty());
    }

    // ── In the engine ────────────────────────────────────────────────────

    fn repository() -> github_backup_types::Repository {
        let owner = github_backup_types::User {
            id: 1,
            login: "octocat".into(),
            user_type: "User".into(),
            avatar_url: String::new(),
            html_url: String::new(),
        };
        github_backup_types::Repository {
            id: 7,
            full_name: "octocat/hello".into(),
            name: "hello".into(),
            owner,
            private: false,
            fork: false,
            archived: false,
            disabled: false,
            description: None,
            clone_url: "https://github.com/octocat/hello.git".into(),
            ssh_url: "git@github.com:octocat/hello.git".into(),
            default_branch: Some("main".into()),
            size: 0,
            has_issues: true,
            has_wiki: false,
            created_at: None,
            pushed_at: None,
            updated_at: None,
            html_url: "https://github.com/octocat/hello".into(),
        }
    }

    /// Runs the engine for one repository whose only issue links `IMG`
    /// (served) and `LOG` (refused).
    async fn engine_run(tolerate_forbidden: bool) -> (crate::stats::BackupStats, MemStorage) {
        use github_backup_types::config::OutputConfig;
        let root = tempfile::tempdir().expect("tempdir");
        let storage = MemStorage::default();
        let client = MockBackupClient::new()
            .with_user_repos(vec![repository()])
            .with_raw(
                "issues",
                vec![json!({"number": 1, "body": format!("![a]({IMG}) [b]({LOG})")})],
            )
            .with_attachment(IMG, b"img", Some("image/png"))
            .with_error(&format!("attachment:{LOG}"), 403);
        let opts = BackupOptions {
            issues: true,
            attachments: true,
            tolerate_forbidden,
            concurrency: 1,
            ..Default::default()
        };
        let engine = crate::engine::BackupEngine::new(
            client,
            storage.clone(),
            crate::git::test_support::SpyGitRunner::default(),
            OutputConfig::new(root.path()),
            opts,
        );
        let stats = engine.run("octocat").await.expect("run");
        let file = OutputConfig::new(root.path())
            .repo_meta_dir("octocat", "hello")
            .join("attachments")
            .join(format!("{}.png", crate::manifest::sha256_hex(b"img")));
        assert_eq!(storage.get(&file).as_deref(), Some(&b"img"[..]));
        (stats, storage)
    }

    #[tokio::test]
    async fn the_engine_downloads_what_the_stored_issues_link() {
        let (stats, _) = engine_run(true).await;
        assert!(!stats.has_failures(), "{:?}", stats.failures());

        let (stats, _) = engine_run(false).await;
        let failures = stats.failures();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].step, "attachments");
        assert!(
            failures[0].message.contains("classic"),
            "{}",
            failures[0].message
        );
    }
}
