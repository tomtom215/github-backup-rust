// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Directory-to-S3 synchronisation.
//!
//! After a local backup run completes, this module uploads the backup
//! artefacts to S3 and reports exactly what happened.
//!
//! # Object keys
//!
//! Every object is stored under `<prefix>/<key_root>/<relative path>` where
//! `key_root` is `<owner>/json` for the JSON tree, so two owners can share a
//! bucket and a prefix without touching each other's objects.  Encrypted
//! objects get a `.enc` suffix.
//!
//! # Skipping unchanged files
//!
//! Each object carries the digest of the local file in `x-amz-meta-sha256`
//! (a keyed HMAC when encrypting).  A file is skipped only when that digest
//! and the object size both match — a same-size edit is uploaded again.
//!
//! # Deleting stale objects
//!
//! With [`SyncOptions::delete_stale`] objects under `<prefix>/<key_root>/`
//! that no longer correspond to a local file are deleted — but never when
//! the deletion cannot be trusted: the caller says the backup run had
//! failures, part of the local tree could not be read, the tree holds no
//! uploadable file, or the run was aborted.  The listing is scoped to
//! `<prefix>/<key_root>/` (a trailing slash), so sibling prefixes such as
//! `github-backup-old/` are never touched.
//!
//! # Failures
//!
//! Nothing is swallowed: every failure lands in [`SyncReport::failures`] and
//! [`SyncReport::is_success`] is `false` whenever anything failed.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::task::JoinSet;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

use crate::client::{HeadOutcome, S3Client};
use crate::config::S3Config;
use crate::digest::{self, digests_equal, METADATA_NAME};
use crate::encrypt;
use crate::error::S3Error;

/// Maximum number of concurrent S3 upload tasks.
const S3_UPLOAD_CONCURRENCY: usize = 8;
/// Maximum number of concurrent `DeleteObject` requests.
const S3_DELETE_CONCURRENCY: usize = 8;
/// Log a progress line every time this many percent of files complete.
const PROGRESS_INTERVAL_PCT: usize = 10;

// ── Public API ──────────────────────────────────────────────────────────────

/// Statistics from a sync run.
#[derive(Debug, Default, Clone)]
pub struct SyncStats {
    /// Number of files uploaded to S3.
    pub uploaded: usize,
    /// Number of files skipped (the object already holds the same content).
    pub skipped: usize,
    /// Number of files that failed to upload.
    pub errored: usize,
    /// Number of stale S3 objects deleted (only when `delete_stale = true`).
    pub deleted: usize,
}

impl std::fmt::Display for SyncStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "uploaded={} skipped={} errored={} deleted={}",
            self.uploaded, self.skipped, self.errored, self.deleted
        )
    }
}

/// What a failed step was doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedOperation {
    /// Uploading a file.
    Upload,
    /// Deleting a stale object.
    Delete,
    /// Listing the bucket for stale-object detection.
    List,
    /// Reading the local backup tree.
    ReadLocal,
}

impl std::fmt::Display for FailedOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Upload => "upload",
            Self::Delete => "delete",
            Self::List => "list",
            Self::ReadLocal => "read local file",
        })
    }
}

/// One failed step of a sync run.
#[derive(Debug, Clone)]
pub struct SyncFailure {
    /// What was being done.
    pub operation: FailedOperation,
    /// The object key (or the local path for [`FailedOperation::ReadLocal`]).
    pub key: String,
    /// Why it failed (status, code and message when the server sent them).
    pub error: String,
    /// One line about what to check, when there is one.
    pub hint: Option<String>,
}

impl std::fmt::Display for SyncFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: {}", self.operation, self.key, self.error)
    }
}

/// Everything a sync run did — and everything that went wrong.
#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    /// Counters (uploaded, skipped, errored, deleted).
    pub stats: SyncStats,
    /// Every failed step, sorted by key.
    pub failures: Vec<SyncFailure>,
    /// Set when a configuration-class error (bad credentials, missing
    /// bucket, wrong region, unreachable endpoint) stopped the run early.
    pub aborted: Option<String>,
    /// Files that were never attempted because the run was aborted.
    pub not_attempted: usize,
    /// `true` for a dry run: nothing was written to the bucket.
    pub dry_run: bool,
    /// Dry run: keys that would have been uploaded.
    pub would_upload: Vec<String>,
    /// Dry run: keys that would have been deleted.
    pub would_delete: Vec<String>,
    /// Why stale-object deletion was refused, when it was requested but not
    /// carried out.
    pub deletion_skipped: Option<String>,
    /// Number of local files found.
    pub local_files: usize,
}

impl SyncReport {
    /// `true` when nothing failed and the run was not aborted.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.failures.is_empty() && self.aborted.is_none()
    }
}

impl std::fmt::Display for SyncReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.dry_run {
            write!(
                f,
                "dry run: would upload {} would skip {} would delete {} errored={}",
                self.would_upload.len(),
                self.stats.skipped,
                self.would_delete.len(),
                self.stats.errored
            )
        } else {
            write!(f, "{}", self.stats)
        }
    }
}

/// What to synchronise and how.  Build with [`SyncOptions::new`] and the
/// chainable setters.
#[derive(Debug, Clone)]
pub struct SyncOptions<'a> {
    /// Local directory whose files are uploaded (for example
    /// `<output>/<owner>/json`).
    pub backup_root: &'a Path,
    /// Key path under the prefix, for example `octocat/json`.  It must name
    /// the owner so that owners cannot collide; stale-object deletion is
    /// refused when it is empty.
    pub key_root: &'a str,
    /// Also upload release assets (large binaries).
    pub include_binary_assets: bool,
    /// Encrypt every file with this AES-256 key before uploading.
    pub encrypt_key: Option<&'a [u8; 32]>,
    /// Delete objects under `<prefix>/<key_root>/` that no longer match a
    /// local file.
    pub delete_stale: bool,
    /// Master switch for deletion: pass `false` when the backup run had
    /// failures, so an incomplete local copy never removes a good remote one.
    pub allow_delete: bool,
    /// Report what would happen without writing to the bucket.
    pub dry_run: bool,
}

impl<'a> SyncOptions<'a> {
    /// Options for uploading `backup_root` under `key_root`, with release
    /// assets excluded, no encryption, no deletion and no dry run.
    #[must_use]
    pub fn new(backup_root: &'a Path, key_root: &'a str) -> Self {
        Self {
            backup_root,
            key_root,
            include_binary_assets: false,
            encrypt_key: None,
            delete_stale: false,
            allow_delete: true,
            dry_run: false,
        }
    }

    /// Upload release assets too.
    #[must_use]
    pub fn include_binary_assets(mut self, yes: bool) -> Self {
        self.include_binary_assets = yes;
        self
    }

    /// Encrypt with `key` (or not, for `None`).
    #[must_use]
    pub fn encrypt_key(mut self, key: Option<&'a [u8; 32]>) -> Self {
        self.encrypt_key = key;
        self
    }

    /// Delete stale remote objects.
    #[must_use]
    pub fn delete_stale(mut self, yes: bool) -> Self {
        self.delete_stale = yes;
        self
    }

    /// Permit (or forbid) deletion; see [`SyncOptions::allow_delete`].
    #[must_use]
    pub fn allow_delete(mut self, yes: bool) -> Self {
        self.allow_delete = yes;
        self
    }

    /// Dry run: no uploads, no deletions.
    #[must_use]
    pub fn dry_run(mut self, yes: bool) -> Self {
        self.dry_run = yes;
        self
    }
}

/// Synchronises a local backup directory to S3.
///
/// Walks `options.backup_root` recursively and uploads each regular file
/// under `<prefix>/<key_root>/<relative path>`.  Release assets (anything
/// below a `release_assets` directory) are only uploaded with
/// [`SyncOptions::include_binary_assets`]; objects for assets that are merely
/// excluded this time are never treated as stale.
///
/// When an encryption key is set, every file is encrypted with AES-256-GCM
/// and the key gains a `.enc` suffix (see [`encrypt`]).
///
/// # Errors
///
/// Returns [`S3Error`] only when the sync cannot start (an unusable local
/// root).  Everything that goes wrong afterwards — failed uploads, a failed
/// listing, refused deletions — is recorded in the returned [`SyncReport`].
pub async fn sync_to_s3(
    client: &S3Client,
    config: &S3Config,
    options: &SyncOptions<'_>,
) -> Result<SyncReport, S3Error> {
    let key_root = options.key_root.trim_matches('/').to_string();
    let encrypting = options.encrypt_key.is_some();

    let root = options.backup_root.to_path_buf();
    if !root.is_dir() {
        return Err(S3Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("backup directory {} does not exist", root.display()),
        )));
    }
    let walk = tokio::task::spawn_blocking(move || walk_files(&root))
        .await
        .map_err(|e| S3Error::Io(std::io::Error::other(format!("directory walk failed: {e}"))))?;

    let mut report = SyncReport {
        dry_run: options.dry_run,
        local_files: walk.files.len(),
        ..SyncReport::default()
    };
    for issue in &walk.issues {
        warn!(path = %issue.path.display(), error = %issue.error, "cannot read part of the backup tree");
        report.failures.push(SyncFailure {
            operation: FailedOperation::ReadLocal,
            key: issue.path.display().to_string(),
            error: issue.error.clone(),
            hint: Some(
                "fix the permissions or ownership of the output directory; files below this path \
                 are not uploaded and no stale objects will be deleted"
                    .to_string(),
            ),
        });
    }

    // Plan: which files become which objects.
    let mut jobs: Vec<FileJob> = Vec::new();
    // Everything that legitimately exists remotely; the rest of the listing
    // is stale.
    let mut expected: HashSet<String> = HashSet::new();
    for file in &walk.files {
        let relative = match relative_key_path(options.backup_root, file) {
            Ok(relative) => relative,
            Err(why) => {
                warn!(path = %file.display(), "{why}; skipping the file");
                report.stats.errored += 1;
                report.failures.push(SyncFailure {
                    operation: FailedOperation::ReadLocal,
                    key: file.display().to_string(),
                    error: why,
                    hint: None,
                });
                continue;
            }
        };
        if !options.include_binary_assets && is_binary_asset(file) {
            debug!(path = %file.display(), "skipping release asset (--s3-include-assets not set)");
            // An asset that is only excluded this time must survive
            // `--s3-delete-stale`: never delete data merely because a flag
            // was dropped.
            expected.insert(object_key(config, &key_root, &relative, false));
            expected.insert(object_key(config, &key_root, &relative, true));
            continue;
        }
        let key = object_key(config, &key_root, &relative, encrypting);
        expected.insert(key.clone());
        jobs.push(FileJob {
            path: file.clone(),
            key,
        });
    }

    let total = jobs.len();
    info!(
        count = total,
        concurrency = S3_UPLOAD_CONCURRENCY,
        bucket = %config.bucket,
        dry_run = options.dry_run,
        "syncing backup to S3"
    );

    // Transfer.
    let ctx = Arc::new(Ctx {
        client: client.clone(),
        key: options.encrypt_key.map(|k| Arc::new(Zeroizing::new(*k))),
        dry_run: options.dry_run,
        head_denied_logged: AtomicBool::new(false),
        abort: AtomicBool::new(false),
        successes: AtomicUsize::new(0),
    });
    let mut done = 0usize;
    let mut last_logged_bucket = 0usize;
    let mut tasks: JoinSet<TaskResult> = JoinSet::new();
    let mut pending = jobs.into_iter();
    loop {
        while tasks.len() < S3_UPLOAD_CONCURRENCY && !ctx.abort.load(Ordering::Relaxed) {
            let Some(job) = pending.next() else { break };
            let ctx = Arc::clone(&ctx);
            tasks.spawn(async move {
                let outcome = sync_one(&ctx, &job).await;
                if matches!(
                    outcome,
                    Ok(FileOutcome::Uploaded | FileOutcome::Skipped | FileOutcome::WouldUpload(_))
                ) {
                    ctx.successes.fetch_add(1, Ordering::Relaxed);
                }
                TaskResult { job, outcome }
            });
        }
        let Some(joined) = tasks.join_next().await else {
            break;
        };
        done += 1;
        match joined {
            Ok(result) => record_result(&mut report, &ctx, result),
            Err(join_error) => {
                report.stats.errored += 1;
                report.failures.push(SyncFailure {
                    operation: FailedOperation::Upload,
                    key: "(unknown)".to_string(),
                    error: format!("upload task failed: {join_error}"),
                    hint: None,
                });
            }
        }
        if let Some(pct) = (done * 100).checked_div(total) {
            let bucket = pct / PROGRESS_INTERVAL_PCT;
            if bucket > last_logged_bucket {
                last_logged_bucket = bucket;
                info!(done, total, percent = pct, "S3 sync progress");
            }
        }
    }
    report.not_attempted += pending.count();
    if report.not_attempted > 0 {
        warn!(
            not_attempted = report.not_attempted,
            "S3 sync aborted early; the remaining files were not attempted"
        );
    }

    // Optionally delete objects that no longer exist locally.
    if options.delete_stale {
        let blocker = deletion_blocker(
            options.allow_delete,
            walk.issues.len(),
            total,
            &key_root,
            report.aborted.is_some(),
        )
        .or_else(|| {
            (report.stats.errored > 0).then(|| {
                "some uploads failed in this run; the objects they should have replaced \
                 must not be deleted"
                    .to_string()
            })
        });
        match blocker {
            Some(reason) => {
                warn!(reason = %reason, "stale-object deletion refused");
                report.deletion_skipped = Some(reason);
            }
            None => {
                delete_stale_objects(
                    client,
                    config,
                    &key_root,
                    &expected,
                    options.dry_run,
                    &mut report,
                )
                .await;
            }
        }
    }

    report.failures.sort_by(|a, b| a.key.cmp(&b.key));
    report.would_upload.sort();
    report.would_delete.sort();
    info!(stats = %report.stats, dry_run = report.dry_run, "S3 sync complete");
    Ok(report)
}

// ── Internals ───────────────────────────────────────────────────────────────

/// A file to upload and the object it maps to.
struct FileJob {
    path: PathBuf,
    key: String,
}

/// Why an object is (re)uploaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UploadReason {
    /// No such object yet.
    New,
    /// The object's digest differs from the local file's.
    Changed,
    /// The object carries no digest (uploaded by another tool or version).
    NoDigest,
    /// The object has a different size than the upload would have.
    SizeDiffers,
    /// `HeadObject` was denied, so nothing is known about the object.
    HeadDenied,
}

impl std::fmt::Display for UploadReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::New => "new object",
            Self::Changed => "content changed",
            Self::NoDigest => "object has no content digest",
            Self::SizeDiffers => "object size differs",
            Self::HeadDenied => "HEAD denied, state unknown",
        })
    }
}

#[derive(Debug)]
enum FileOutcome {
    Uploaded,
    Skipped,
    WouldUpload(UploadReason),
    /// The run was aborted before this file's turn.
    NotAttempted,
}

struct TaskResult {
    job: FileJob,
    outcome: Result<FileOutcome, S3Error>,
}

/// State shared by the upload tasks.
struct Ctx {
    client: S3Client,
    /// The encryption key, one wiped-on-drop copy shared by all tasks.
    key: Option<Arc<Zeroizing<[u8; 32]>>>,
    dry_run: bool,
    head_denied_logged: AtomicBool,
    /// Set once a configuration-class error ended the run.
    abort: AtomicBool,
    successes: AtomicUsize,
}

/// Folds one finished file into the report.
fn record_result(report: &mut SyncReport, ctx: &Ctx, result: TaskResult) {
    let TaskResult { job, outcome } = result;
    match outcome {
        Ok(FileOutcome::Uploaded) => report.stats.uploaded += 1,
        Ok(FileOutcome::Skipped) => report.stats.skipped += 1,
        Ok(FileOutcome::NotAttempted) => report.not_attempted += 1,
        Ok(FileOutcome::WouldUpload(reason)) => {
            info!(key = %job.key, reason = %reason, "dry run: would upload");
            report.would_upload.push(job.key);
        }
        Err(error) => {
            let hint = error.hint();
            warn!(
                path = %job.path.display(),
                key = %job.key,
                error = %error,
                "failed to upload file to S3"
            );
            report.stats.errored += 1;
            // A configuration-class failure before anything succeeded means
            // every other file would fail the same way: stop instead of
            // producing thousands of identical errors.
            if error.is_fatal()
                && ctx.successes.load(Ordering::Relaxed) == 0
                && report.aborted.is_none()
            {
                report.aborted = Some(error.to_string());
                ctx.abort.store(true, Ordering::Relaxed);
            }
            report.failures.push(SyncFailure {
                operation: FailedOperation::Upload,
                key: job.key,
                error: error.to_string(),
                hint,
            });
        }
    }
}

/// Runs a blocking closure on the blocking pool.
async fn blocking<T, F>(f: F) -> Result<T, S3Error>
where
    F: FnOnce() -> Result<T, S3Error> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        S3Error::Io(std::io::Error::other(format!(
            "background task failed: {e}"
        )))
    })?
}

/// Decides whether `job` needs uploading and, unless it is a dry run, does it.
async fn sync_one(ctx: &Ctx, job: &FileJob) -> Result<FileOutcome, S3Error> {
    if ctx.abort.load(Ordering::Relaxed) {
        // Another task hit a configuration-class error while this one was
        // being scheduled.
        return Ok(FileOutcome::NotAttempted);
    }
    let key = ctx.key.clone();

    // Digest of the local file, read from disk in a streaming fashion.
    let local_digest = {
        let path = job.path.clone();
        let key = key.clone();
        blocking(move || {
            digest::digest_file(&path, key.as_deref().map(|k| &**k)).map_err(S3Error::from)
        })
        .await?
    };
    let local_size = tokio::fs::metadata(&job.path).await?.len();
    let expected_remote_size = if key.is_some() {
        encrypt::encrypted_len(local_size)
    } else {
        local_size
    };

    let reason = match ctx.client.head_object(&job.key).await? {
        HeadOutcome::Missing => UploadReason::New,
        HeadOutcome::Forbidden => {
            if !ctx.head_denied_logged.swap(true, Ordering::Relaxed) {
                warn!(
                    "HeadObject was denied (403): unchanged files cannot be detected, so every \
                     file is uploaded on every run; grant s3:GetObject and s3:ListBucket to \
                     avoid that"
                );
            }
            UploadReason::HeadDenied
        }
        HeadOutcome::Found(info) => {
            let remote_digest = info.metadata.get(METADATA_NAME);
            let digest_matches = remote_digest.is_some_and(|d| digests_equal(d, &local_digest));
            if !digest_matches {
                if remote_digest.is_some() {
                    UploadReason::Changed
                } else {
                    UploadReason::NoDigest
                }
            } else if info.size != Some(expected_remote_size) {
                UploadReason::SizeDiffers
            } else {
                debug!(key = %job.key, "object already holds this content, skipping");
                return Ok(FileOutcome::Skipped);
            }
        }
    };
    if reason == UploadReason::SizeDiffers || reason == UploadReason::Changed {
        debug!(key = %job.key, reason = %reason, "object differs from the local file");
    }

    if ctx.dry_run {
        return Ok(FileOutcome::WouldUpload(reason));
    }
    upload_file(ctx, job).await?;
    Ok(FileOutcome::Uploaded)
}

/// Uploads `job.path` (encrypting when a key is set), tagging the object with
/// the digest of what is uploaded.
async fn upload_file(ctx: &Ctx, job: &FileJob) -> Result<(), S3Error> {
    let key = ctx.key.clone();
    let path = job.path.clone();
    let (body, digest, content_type) = blocking(move || {
        let plaintext = std::fs::read(&path)?;
        // The digest describes exactly the bytes that are uploaded, even if
        // the file changed since it was first hashed.
        let digest = digest::digest_bytes(&plaintext, key.as_deref().map(|k| &**k));
        match key.as_deref() {
            Some(k) => Ok((
                encrypt::encrypt(k, &plaintext)?,
                digest,
                "application/octet-stream",
            )),
            None => Ok((plaintext, digest, guess_content_type(&path))),
        }
    })
    .await?;

    debug!(
        path = %job.path.display(),
        key = %job.key,
        bytes = body.len(),
        encrypted = ctx.key.is_some(),
        "uploading to S3"
    );
    let metadata = [(METADATA_NAME, digest.as_str())];
    if body.len() > ctx.client.options().part_size {
        ctx.client
            .multipart_upload(&job.key, &body, content_type, &metadata)
            .await
    } else {
        ctx.client
            .put_object(&job.key, body, content_type, &metadata)
            .await
    }
}

/// Why stale-object deletion must not run, if it must not.
///
/// Deleting is the one destructive step, so it only runs when the evidence
/// it rests on — the local tree — is trustworthy.
fn deletion_blocker(
    allow_delete: bool,
    unreadable_paths: usize,
    uploadable_files: usize,
    key_root: &str,
    aborted: bool,
) -> Option<String> {
    if !allow_delete {
        return Some(
            "the backup run reported failures, so nothing is deleted from S3 (an incomplete local \
             copy must not remove the last good remote copy)"
                .to_string(),
        );
    }
    if key_root.is_empty() {
        return Some(
            "no owner is part of the object keys, so the listing would span the whole prefix"
                .to_string(),
        );
    }
    if unreadable_paths > 0 {
        return Some(format!(
            "{unreadable_paths} part(s) of the local backup tree could not be read; a partial \
             view of the local files must not be used to decide what is stale"
        ));
    }
    if uploadable_files == 0 {
        return Some(
            "the local backup holds no uploadable files; refusing to treat every object in the \
             bucket as stale (is the output directory empty or unmounted?)"
                .to_string(),
        );
    }
    if aborted {
        return Some("the sync was aborted by an earlier error".to_string());
    }
    None
}

/// Lists `<prefix>/<key_root>/` and deletes (or, in a dry run, reports) the
/// objects that are not in `expected`.
async fn delete_stale_objects(
    client: &S3Client,
    config: &S3Config,
    key_root: &str,
    expected: &HashSet<String>,
    dry_run: bool,
    report: &mut SyncReport,
) {
    // The trailing slash keeps the listing inside this owner's tree:
    // `github-backup/octocat/json/` never matches `github-backup/octocat/json-old/`.
    let scope = config.full_key(&format!("{key_root}/"));
    let remote = match client.list_objects(&scope).await {
        Ok(keys) => keys,
        Err(error) => {
            warn!(error = %error, "failed to list S3 objects for stale-object detection");
            report.failures.push(SyncFailure {
                operation: FailedOperation::List,
                key: scope,
                hint: error.hint(),
                error: error.to_string(),
            });
            return;
        }
    };
    let stale: Vec<String> = remote
        .into_iter()
        .filter(|key| key.starts_with(&scope) && !expected.contains(key))
        .collect();
    if stale.is_empty() {
        debug!("no stale S3 objects to delete");
        return;
    }
    if dry_run {
        for key in &stale {
            info!(key = %key, "dry run: would delete stale object");
        }
        report.would_delete = stale;
        return;
    }

    info!(count = stale.len(), "deleting stale S3 objects");
    let mut tasks: JoinSet<(String, Result<(), S3Error>)> = JoinSet::new();
    let mut pending = stale.into_iter();
    loop {
        while tasks.len() < S3_DELETE_CONCURRENCY {
            let Some(key) = pending.next() else { break };
            let client = client.clone();
            tasks.spawn(async move {
                let result = client.delete_object(&key).await;
                (key, result)
            });
        }
        match tasks.join_next().await {
            None => break,
            Some(Ok((key, Ok(())))) => {
                report.stats.deleted += 1;
                debug!(key = %key, "deleted stale S3 object");
            }
            Some(Ok((key, Err(error)))) => {
                warn!(key = %key, error = %error, "failed to delete stale S3 object");
                report.failures.push(SyncFailure {
                    operation: FailedOperation::Delete,
                    key,
                    hint: error.hint(),
                    error: error.to_string(),
                });
            }
            Some(Err(join_error)) => {
                report.failures.push(SyncFailure {
                    operation: FailedOperation::Delete,
                    key: "(unknown)".to_string(),
                    error: format!("delete task failed: {join_error}"),
                    hint: None,
                });
            }
        }
    }
}

/// The object key for `relative` (a `/`-separated path below the backup
/// root): `<prefix>/<key_root>/<relative>[.enc]`.
fn object_key(config: &S3Config, key_root: &str, relative: &str, encrypted: bool) -> String {
    let suffix = if encrypted { ".enc" } else { "" };
    if key_root.is_empty() {
        config.full_key(&format!("{relative}{suffix}"))
    } else {
        config.full_key(&format!("{key_root}/{relative}{suffix}"))
    }
}

/// `file` relative to `root`, joined with `/` (never a platform separator).
fn relative_key_path(root: &Path, file: &Path) -> Result<String, String> {
    let relative = file
        .strip_prefix(root)
        .map_err(|_| "the file is outside the backup directory".to_string())?;
    let mut parts: Vec<&str> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_str().ok_or_else(|| {
                "the file name is not valid UTF-8 and cannot be used as an object key".to_string()
            })?),
            _ => return Err("unexpected path component".to_string()),
        }
    }
    if parts.is_empty() {
        return Err("empty relative path".to_string());
    }
    Ok(parts.join("/"))
}

/// Something under the backup root that could not be read.
#[derive(Debug, Clone)]
struct WalkIssue {
    path: PathBuf,
    error: String,
}

/// The result of walking the backup tree.
#[derive(Debug, Default)]
struct Walk {
    /// Regular files, sorted.
    files: Vec<PathBuf>,
    /// Directories or entries that could not be read.  While this is
    /// non-empty `files` is an incomplete picture of the tree.
    issues: Vec<WalkIssue>,
}

/// Recursively walks `dir`, collecting regular files.
///
/// Symbolic links and special files are skipped (a link cycle must not loop
/// forever, and the backup never creates links).  Anything that cannot be read
/// is recorded in [`Walk::issues`] instead of being skipped silently.
fn walk_files(dir: &Path) -> Walk {
    let mut walk = Walk::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match std::fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) => {
                walk.issues.push(WalkIssue {
                    path: current,
                    error: error.to_string(),
                });
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    walk.issues.push(WalkIssue {
                        path: current.clone(),
                        error: error.to_string(),
                    });
                    continue;
                }
            };
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => walk.files.push(path),
                Ok(_) => debug!(path = %path.display(), "skipping symbolic link or special file"),
                Err(error) => walk.issues.push(WalkIssue {
                    path,
                    error: error.to_string(),
                }),
            }
        }
    }
    walk.files.sort();
    walk
}

/// Returns `true` if the file is a release asset: anything below a
/// `release_assets` directory (these can be very large binaries).
fn is_binary_asset(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "release_assets")
}

/// Guesses the `Content-Type` for a file based on its extension.
fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => "application/json",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn config(prefix: &str) -> S3Config {
        S3Config {
            bucket: "b".to_string(),
            region: "us-east-1".to_string(),
            prefix: prefix.to_string(),
            endpoint: None,
            access_key_id: "k".to_string(),
            secret_access_key: "s".to_string(),
            session_token: None,
        }
    }

    #[test]
    fn walk_files_finds_nested_files() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub/deep")).unwrap();
        fs::write(dir.path().join("a.json"), b"{}").unwrap();
        fs::write(dir.path().join("sub/b.json"), b"{}").unwrap();
        fs::write(dir.path().join("sub/deep/c.json"), b"{}").unwrap();

        let walk = walk_files(dir.path());
        assert_eq!(walk.files.len(), 3);
        assert!(walk.issues.is_empty());
    }

    #[test]
    fn walk_files_is_sorted_and_deterministic() {
        let dir = tempdir().unwrap();
        for name in ["z.json", "a.json", "m.json"] {
            fs::write(dir.path().join(name), b"{}").unwrap();
        }
        let names: Vec<String> = walk_files(dir.path())
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.json", "m.json", "z.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn walk_files_does_not_follow_symlinks() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        fs::write(dir.path().join("real/f.json"), b"{}").unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("real/cycle")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real/f.json"), dir.path().join("flink"))
            .unwrap();
        let walk = walk_files(dir.path());
        assert_eq!(walk.files, vec![dir.path().join("real/f.json")]);
        assert!(walk.issues.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn walk_files_reports_unreadable_directories() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("hidden.json"), b"{}").unwrap();
        fs::write(dir.path().join("ok.json"), b"{}").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        // Root bypasses permission checks; only assert when the lock holds.
        let locked_is_enforced = fs::read_dir(&locked).is_err();
        let walk = walk_files(dir.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if locked_is_enforced {
            assert_eq!(walk.files, vec![dir.path().join("ok.json")]);
            assert_eq!(walk.issues.len(), 1);
            assert_eq!(walk.issues[0].path, locked);
        }
    }

    #[test]
    fn walk_files_on_a_missing_directory_is_an_issue_not_an_empty_tree() {
        let dir = tempdir().unwrap();
        let walk = walk_files(&dir.path().join("nope"));
        assert!(walk.files.is_empty());
        assert_eq!(walk.issues.len(), 1);
    }

    #[test]
    fn is_binary_asset_returns_true_for_release_assets() {
        let path = PathBuf::from("/backup/owner/json/repos/my-repo/release_assets/v1.0/app.zip");
        assert!(is_binary_asset(&path));
    }

    #[test]
    fn is_binary_asset_returns_false_for_json() {
        let path = PathBuf::from("/backup/owner/json/repos/my-repo/info.json");
        assert!(!is_binary_asset(&path));
    }

    #[test]
    fn json_named_release_assets_are_still_assets() {
        let path = PathBuf::from("/b/o/json/repos/r/release_assets/v1/schema.json");
        assert!(is_binary_asset(&path));
        let sidecar = PathBuf::from("/b/o/json/repos/r/release_assets/v1/app.zip.sha256");
        assert!(is_binary_asset(&sidecar));
    }

    #[test]
    fn guess_content_type_json() {
        assert_eq!(
            guess_content_type(Path::new("data.json")),
            "application/json"
        );
    }

    #[test]
    fn guess_content_type_binary() {
        assert_eq!(
            guess_content_type(Path::new("archive.tar.gz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn sync_stats_display() {
        let s = SyncStats {
            uploaded: 5,
            skipped: 3,
            errored: 1,
            deleted: 2,
        };
        assert_eq!(s.to_string(), "uploaded=5 skipped=3 errored=1 deleted=2");
    }

    #[test]
    fn object_keys_are_prefix_owner_json_relative() {
        let cfg = config("github-backup");
        assert_eq!(
            object_key(&cfg, "octocat/json", "repos/hello/issues.json", false),
            "github-backup/octocat/json/repos/hello/issues.json"
        );
        assert_eq!(
            object_key(&cfg, "octocat/json", "repos/hello/issues.json", true),
            "github-backup/octocat/json/repos/hello/issues.json.enc"
        );
        let root_cfg = config("");
        assert_eq!(
            object_key(&root_cfg, "octocat/json", "backup_state.json", false),
            "octocat/json/backup_state.json"
        );
    }

    #[test]
    fn two_owners_never_share_a_key() {
        let cfg = config("shared/");
        let alice = object_key(&cfg, "alice/json", "backup_state.json", false);
        let bob = object_key(&cfg, "bob/json", "backup_state.json", false);
        assert_ne!(alice, bob);
    }

    #[test]
    fn relative_key_path_uses_forward_slashes() {
        let root = Path::new("/out/octocat/json");
        let file = root.join("repos").join("hello").join("a b.json");
        assert_eq!(
            relative_key_path(root, &file).unwrap(),
            "repos/hello/a b.json"
        );
        assert!(relative_key_path(root, Path::new("/elsewhere/x")).is_err());
        assert!(relative_key_path(root, root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn relative_key_path_rejects_non_utf8_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let root = Path::new("/out");
        let file = root.join(OsStr::from_bytes(b"bad\xff.json"));
        assert!(relative_key_path(root, &file).is_err());
    }

    // ── Deletion guards ─────────────────────────────────────────────────

    #[test]
    fn deletion_is_allowed_when_every_guard_passes() {
        assert_eq!(deletion_blocker(true, 0, 3, "octocat/json", false), None);
    }

    #[test]
    fn deletion_is_refused_when_the_run_had_failures() {
        let reason = deletion_blocker(false, 0, 3, "octocat/json", false).unwrap();
        assert!(reason.contains("failures"), "{reason}");
    }

    #[test]
    fn deletion_is_refused_without_an_owner_in_the_key() {
        let reason = deletion_blocker(true, 0, 3, "", false).unwrap();
        assert!(reason.contains("owner"), "{reason}");
    }

    #[test]
    fn deletion_is_refused_when_the_local_walk_was_incomplete() {
        let reason = deletion_blocker(true, 2, 3, "octocat/json", false).unwrap();
        assert!(reason.contains("could not be read"), "{reason}");
    }

    #[test]
    fn deletion_is_refused_for_an_empty_local_tree() {
        let reason = deletion_blocker(true, 0, 0, "octocat/json", false).unwrap();
        assert!(reason.contains("no uploadable files"), "{reason}");
    }

    #[test]
    fn deletion_is_refused_after_an_abort() {
        let reason = deletion_blocker(true, 0, 3, "octocat/json", true).unwrap();
        assert!(reason.contains("aborted"), "{reason}");
    }

    #[test]
    fn report_is_a_success_only_without_failures_or_abort() {
        let mut report = SyncReport::default();
        assert!(report.is_success());
        report.deletion_skipped = Some("x".to_string());
        assert!(report.is_success(), "a refused deletion is not a failure");
        report.failures.push(SyncFailure {
            operation: FailedOperation::Upload,
            key: "k".to_string(),
            error: "e".to_string(),
            hint: None,
        });
        assert!(!report.is_success());
        let aborted = SyncReport {
            aborted: Some("boom".to_string()),
            ..SyncReport::default()
        };
        assert!(!aborted.is_success());
    }

    #[test]
    fn report_display_summarises_dry_runs_separately() {
        let report = SyncReport {
            dry_run: true,
            would_upload: vec!["a".into(), "b".into()],
            would_delete: vec!["c".into()],
            ..SyncReport::default()
        };
        let text = report.to_string();
        assert!(text.contains("would upload 2"), "{text}");
        assert!(text.contains("would delete 1"), "{text}");
    }

    #[test]
    fn failure_display_names_operation_key_and_error() {
        let failure = SyncFailure {
            operation: FailedOperation::Delete,
            key: "pfx/o/json/x".to_string(),
            error: "S3 DeleteObject failed with HTTP 403 AccessDenied".to_string(),
            hint: None,
        };
        let text = failure.to_string();
        assert!(text.starts_with("delete pfx/o/json/x:"), "{text}");
    }
}
