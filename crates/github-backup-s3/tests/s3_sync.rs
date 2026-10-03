// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! End-to-end tests of the S3 client and sync against an in-process fake S3
//! server that verifies SigV4 independently (see `tests/support`).
//!
//! Server acceptance of real providers (AWS, MinIO, B2, R2, ...) is NOT
//! covered here: no such server was available.  What is covered is that every
//! request the client sends is accepted by an independent verifier that
//! reproduces AWS's published S3 signature examples.

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use github_backup_s3::client::{ClientOptions, HeadOutcome, RetryPolicy};
use github_backup_s3::config::S3Config;
use github_backup_s3::digest;
use github_backup_s3::sync::{sync_to_s3, FailedOperation, SyncOptions, SyncReport};
use github_backup_s3::{S3Client, S3Error};
use support::fake_s3::{Action, FakeConfig, FakeS3, ListEncoding, Rule, SigMode};

const KEY: [u8; 32] = [0x42; 32];

fn config_for(fake: &FakeS3, prefix: &str) -> S3Config {
    let c = fake.config();
    S3Config {
        bucket: c.bucket,
        region: c.region,
        prefix: prefix.to_string(),
        endpoint: Some(fake.endpoint()),
        access_key_id: c.access_key,
        secret_access_key: c.secret_key,
        session_token: c.session_token,
    }
}

fn fast_options() -> ClientOptions {
    ClientOptions {
        retry: RetryPolicy {
            max_attempts: 4,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        },
        idle_timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(5),
        part_size: 16 * 1024 * 1024,
    }
}

fn client_for(fake: &FakeS3, prefix: &str) -> (S3Client, S3Config) {
    let cfg = config_for(fake, prefix);
    (S3Client::with_options(cfg.clone(), fast_options()).unwrap(), cfg)
}

async fn run_sync<'a>(
    client: &S3Client,
    cfg: &S3Config,
    root: &'a Path,
    key_root: &'a str,
    f: impl FnOnce(SyncOptions<'a>) -> SyncOptions<'a>,
) -> SyncReport {
    sync_to_s3(client, cfg, &f(SyncOptions::new(root, key_root)))
        .await
        .expect("sync starts")
}

fn write(root: &Path, rel: &str, body: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

// ── S3C-01: every request verifies against the independent verifier ─────────

#[tokio::test]
async fn s3c01_every_operation_is_accepted_by_a_strict_server() {
    let fake = FakeS3::start().await;
    let (client, _) = client_for(&fake, "pfx");
    client
        .put_object("pfx/a.json", &b"{}"[..], "application/json", &[("sha256", "abcd")])
        .await
        .expect("PUT");
    match client.head_object("pfx/a.json").await.expect("HEAD") {
        HeadOutcome::Found(info) => {
            assert_eq!(info.size, Some(2));
            assert_eq!(info.metadata.get("sha256").map(String::as_str), Some("abcd"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(client.head_object("pfx/none").await.unwrap(), HeadOutcome::Missing);
    assert_eq!(client.list_objects("pfx/").await.expect("LIST"), vec!["pfx/a.json"]);
    client.delete_object("pfx/a.json").await.expect("DELETE");
    assert!(fake.keys().is_empty());
    for r in fake.records() {
        assert!(r.signature_ok, "{} {} rejected: {:?}", r.method, r.target, r.rejection);
        assert!(!r.signed_headers.contains(&"content-type".to_string()) || r.method == "PUT");
    }
}

#[tokio::test]
async fn s3c01_multipart_round_trip_and_abort_are_accepted() {
    let fake = FakeS3::start_with(FakeConfig {
        min_part_size: 4,
        ..FakeConfig::default()
    })
    .await;
    let cfg = config_for(&fake, "");
    let client = S3Client::with_options(
        cfg,
        ClientOptions {
            part_size: 10,
            ..fast_options()
        },
    )
    .unwrap();
    let data: Vec<u8> = (0..35u8).collect();
    client
        .multipart_upload("big.bin", &data, "application/octet-stream", &[("sha256", "ff")])
        .await
        .expect("multipart");
    assert_eq!(fake.body_of("big.bin").unwrap(), data);
    assert_eq!(fake.object("big.bin").unwrap().metadata["sha256"], "ff");
    assert_eq!(fake.open_uploads(), 0);
    assert_eq!(fake.requests("POST", "uploads").len(), 1);
    assert_eq!(fake.requests("PUT", "partNumber").len(), 4);
    assert!(fake.records().iter().all(|r| r.signature_ok));

    // Abort is signed correctly too.
    let upload = client
        .create_multipart_upload("x.bin", "application/octet-stream", &[])
        .await
        .unwrap();
    assert_eq!(fake.open_uploads(), 1);
    client.abort_multipart_upload(&upload).await.expect("abort");
    assert_eq!(fake.open_uploads(), 0);
}

#[tokio::test]
async fn s3c15_complete_with_200_error_body_fails_and_aborts() {
    let fake = FakeS3::start_with(FakeConfig {
        min_part_size: 4,
        ..FakeConfig::default()
    })
    .await;
    // Retries of Complete also hit the rule; keep it on for all attempts.
    fake.add_rule(Rule::new(Action::CompleteWithErrorBody { code: "InternalError" }));
    let client = S3Client::with_options(
        config_for(&fake, ""),
        ClientOptions {
            part_size: 10,
            ..fast_options()
        },
    )
    .unwrap();
    let err = client
        .multipart_upload("big.bin", &[7u8; 25], "application/octet-stream", &[])
        .await
        .expect_err("a 200 carrying <Error> is a failure");
    assert_eq!(err.api_code(), Some("InternalError"), "{err}");
    assert!(fake.object("big.bin").is_none());
    assert_eq!(fake.open_uploads(), 0, "the incomplete upload must be aborted");
}

#[tokio::test]
async fn s3c15_failed_part_aborts_the_upload() {
    let fake = FakeS3::start_with(FakeConfig {
        min_part_size: 4,
        ..FakeConfig::default()
    })
    .await;
    fake.add_rule(
        Rule::new(Action::Error {
            status: 403,
            code: "AccessDenied",
            message: "Access Denied",
        })
        .method("PUT")
        .query_contains("partNumber=2"),
    );
    let client = S3Client::with_options(
        config_for(&fake, ""),
        ClientOptions {
            part_size: 10,
            ..fast_options()
        },
    )
    .unwrap();
    client
        .multipart_upload("big.bin", &[1u8; 25], "application/octet-stream", &[])
        .await
        .expect_err("part 2 is denied");
    assert_eq!(fake.open_uploads(), 0);
}

// ── S3C-02/18: failures surface with status, code and a hint ───────────────

#[tokio::test]
async fn s3c02_wrong_secret_is_reported_with_code_and_hint() {
    let fake = FakeS3::start().await;
    let mut cfg = config_for(&fake, "pfx");
    cfg.secret_access_key = "wrong-secret-wrong-secret-wrong".to_string();
    let client = S3Client::with_options(cfg.clone(), fast_options()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.json", b"{}");
    write(dir.path(), "b.json", b"{}");
    let report = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert!(!report.is_success());
    assert!(report.stats.errored >= 1);
    let text = report.failures[0].to_string();
    assert!(text.contains("403"), "{text}");
    assert!(report.aborted.as_deref().unwrap_or("").contains("SignatureDoesNotMatch") || text.contains("SignatureDoesNotMatch"), "{report:?}");
    assert!(report.failures.iter().any(|f| f.error.contains("hint:")), "{report:?}");
    assert!(fake.keys().is_empty());
}

#[tokio::test]
async fn s3c02_missing_bucket_aborts_after_one_error() {
    let fake = FakeS3::start().await;
    let mut cfg = config_for(&fake, "pfx");
    cfg.bucket = "nobucket".to_string();
    let client = S3Client::with_options(cfg.clone(), fast_options()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for i in 0..30 {
        write(dir.path(), &format!("f{i:02}.json"), b"{}");
    }
    let report = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert!(!report.is_success());
    assert!(report.aborted.as_deref().unwrap().contains("NoSuchBucket"), "{report:?}");
    // HEAD of a missing key in a missing bucket is 404 (reads as "new"), so
    // the PUT is what reveals the bucket problem; far fewer than 30 attempts.
    assert!(report.stats.errored + report.not_attempted >= 1);
    assert!(fake.requests("PUT", "").len() < 30, "must stop early");
}

#[tokio::test]
async fn s3c18_clock_skew_and_wrong_region_have_actionable_messages() {
    let fake = FakeS3::start().await;
    fake.configure(|c| c.clock_offset_secs = 3600);
    let (client, _) = client_for(&fake, "");
    let err = client
        .put_object("a", &b"x"[..], "text/plain", &[])
        .await
        .unwrap_err();
    assert_eq!(err.api_code(), Some("RequestTimeTooSkewed"));
    assert!(err.to_string().contains("clock"), "{err}");

    fake.configure(|c| {
        c.clock_offset_secs = 0;
        c.region = "eu-west-1".to_string();
    });
    let err = client
        .put_object("a", &b"x"[..], "text/plain", &[])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("--s3-region eu-west-1"), "{err}");
}

// ── S3C-04: content-based skipping ─────────────────────────────────────────

#[tokio::test]
async fn s3c04_same_size_edit_is_reuploaded_and_unchanged_is_skipped() {
    let fake = FakeS3::start().await;
    let (client, cfg) = client_for(&fake, "pfx");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "data.json", br#"{"n":1}"#);

    let r1 = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert!(r1.is_success(), "{r1:?}");
    assert_eq!(r1.stats.uploaded, 1);
    let key = "pfx/o/json/data.json";
    assert_eq!(fake.body_of(key).unwrap(), br#"{"n":1}"#);
    assert_eq!(
        fake.object(key).unwrap().metadata["sha256"],
        digest::digest_bytes(br#"{"n":1}"#, None)
    );

    let r2 = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert_eq!((r2.stats.uploaded, r2.stats.skipped), (0, 1), "{r2:?}");

    write(dir.path(), "data.json", br#"{"n":2}"#);
    let r3 = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert_eq!(r3.stats.uploaded, 1, "same-size edit must be uploaded: {r3:?}");
    assert_eq!(fake.body_of(key).unwrap(), br#"{"n":2}"#);
}

#[tokio::test]
async fn s3c04_object_without_digest_is_reuploaded() {
    let fake = FakeS3::start().await;
    let (client, cfg) = client_for(&fake, "pfx");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "data.json", b"{}");
    fake.put_object("pfx/o/json/data.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert_eq!(r.stats.uploaded, 1, "{r:?}");
    assert!(fake.object("pfx/o/json/data.json").unwrap().metadata.contains_key("sha256"));
}

#[tokio::test]
async fn s3c04_encrypted_uploads_use_a_keyed_digest_and_rotate_with_the_key() {
    let fake = FakeS3::start().await;
    let (client, cfg) = client_for(&fake, "pfx");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "secret.json", b"{\"secret\":true}");

    let r1 = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.encrypt_key(Some(&KEY))).await;
    assert_eq!(r1.stats.uploaded, 1, "{r1:?}");
    let key = "pfx/o/json/secret.json.enc";
    let obj = fake.object(key).unwrap();
    let stored = &obj.metadata["sha256"];
    assert_ne!(stored, &digest::digest_bytes(b"{\"secret\":true}", None), "no plaintext hash leaks");
    assert_eq!(stored, &digest::digest_bytes(b"{\"secret\":true}", Some(&KEY)));
    assert_eq!(
        github_backup_s3::encrypt::decrypt(&KEY, &obj.body).unwrap(),
        b"{\"secret\":true}"
    );

    let r2 = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.encrypt_key(Some(&KEY))).await;
    assert_eq!(r2.stats.skipped, 1, "unchanged encrypted file is skipped: {r2:?}");

    let new_key = [0x43u8; 32];
    let r3 = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.encrypt_key(Some(&new_key))).await;
    assert_eq!(r3.stats.uploaded, 1, "a new key re-uploads: {r3:?}");
    let body = fake.body_of(key).unwrap();
    assert!(github_backup_s3::encrypt::decrypt(&KEY, &body).is_err());
    assert!(github_backup_s3::encrypt::decrypt(&new_key, &body).is_ok());
}

// ── S3C-05: two owners in one bucket ───────────────────────────────────────

#[tokio::test]
async fn s3c05_keys_include_owner_and_json_and_owners_do_not_collide() {
    let fake = FakeS3::start().await;
    let (client, cfg) = client_for(&fake, "shared");
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "repos/hello/issues.json", b"alice");
    write(b.path(), "repos/hello/issues.json", b"bob!");
    write(a.path(), "alice_only.json", b"x");
    assert!(run_sync(&client, &cfg, a.path(), "alice/json", |o| o).await.is_success());
    assert!(run_sync(&client, &cfg, b.path(), "bob/json", |o| o.delete_stale(true)).await.is_success());
    assert_eq!(fake.body_of("shared/alice/json/repos/hello/issues.json").unwrap(), b"alice");
    assert_eq!(fake.body_of("shared/bob/json/repos/hello/issues.json").unwrap(), b"bob!");
    assert!(fake.object("shared/alice/json/alice_only.json").is_some(), "bob's delete-stale must not touch alice");
}

// ── S3C-06/07: delete-stale scope and guards ───────────────────────────────

#[tokio::test]
async fn s3c06_delete_stale_stays_inside_the_owner_tree() {
    let fake = FakeS3::start().await;
    for k in [
        "github-backup-old/x.json",
        "github-backup2/y.json",
        "github-backup/octocat/json-old/z.json",
        "github-backup/other/json/z.json",
        "unrelated/z.bin",
        "github-backup/octocat/json/stale.json",
    ] {
        fake.put_object(k, b"z");
    }
    let (client, cfg) = client_for(&fake, "github-backup");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "data.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "octocat/json", |o| o.delete_stale(true)).await;
    assert!(r.is_success(), "{r:?}");
    assert_eq!(r.stats.deleted, 1);
    let keys = fake.keys();
    assert!(!keys.contains(&"github-backup/octocat/json/stale.json".to_string()));
    assert_eq!(keys.len(), 6, "{keys:?}");
    let list = &fake.requests("GET", "list-type")[0];
    assert!(list.query.contains("prefix=github-backup%2Foctocat%2Fjson%2F"), "{}", list.query);
}

#[tokio::test]
async fn s3c06_empty_prefix_delete_stale_cannot_reach_other_data() {
    let fake = FakeS3::start().await;
    fake.put_object("photos/img.jpg", b"p");
    fake.put_object("octocat/json/stale.json", b"s");
    let (client, cfg) = client_for(&fake, "");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "data.json", b"{}");
    run_sync(&client, &cfg, dir.path(), "octocat/json", |o| o.delete_stale(true)).await;
    assert!(fake.object("photos/img.jpg").is_some());
    assert!(fake.object("octocat/json/stale.json").is_none());
    // And without an owner in the key root, deletion is refused outright.
    fake.put_object("photos/b.jpg", b"p");
    let r = run_sync(&client, &cfg, dir.path(), "", |o| o.delete_stale(true)).await;
    assert!(r.deletion_skipped.is_some());
    assert!(fake.object("photos/b.jpg").is_some());
}

#[tokio::test]
async fn s3c07_delete_stale_refuses_on_empty_tree_failed_run_and_keeps_assets() {
    let fake = FakeS3::start().await;
    fake.put_object("p/o/json/old.json", b"old");
    let (client, cfg) = client_for(&fake, "p");

    // Empty local tree.
    let empty = tempfile::tempdir().unwrap();
    let r = run_sync(&client, &cfg, empty.path(), "o/json", |o| o.delete_stale(true)).await;
    assert!(r.deletion_skipped.as_deref().unwrap().contains("no uploadable files"), "{r:?}");
    assert!(fake.object("p/o/json/old.json").is_some());

    // Backup run had failures.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "data.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true).allow_delete(false)).await;
    assert!(r.deletion_skipped.is_some(), "{r:?}");
    assert!(fake.object("p/o/json/old.json").is_some());
    assert_eq!(r.stats.deleted, 0);

    // Dropping --s3-include-assets must not delete previously uploaded assets.
    write(dir.path(), "repos/r/release_assets/v1/app.zip", b"zipdata");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.include_binary_assets(true)).await;
    assert!(r.is_success(), "{r:?}");
    assert!(fake.object("p/o/json/repos/r/release_assets/v1/app.zip").is_some());
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true)).await;
    assert!(r.is_success(), "{r:?}");
    assert!(fake.object("p/o/json/repos/r/release_assets/v1/app.zip").is_some(), "assets survive");
    assert!(fake.object("p/o/json/old.json").is_none(), "genuinely stale object goes");
}

#[cfg(unix)]
#[tokio::test]
async fn s3c07_unreadable_directory_blocks_deletion() {
    use std::os::unix::fs::PermissionsExt;
    let fake = FakeS3::start().await;
    fake.put_object("p/o/json/locked/hidden.json", b"h");
    let (client, cfg) = client_for(&fake, "p");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "ok.json", b"{}");
    write(dir.path(), "locked/hidden.json", b"h");
    let locked = dir.path().join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let enforced = std::fs::read_dir(&locked).is_err();
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true)).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if enforced {
        assert!(!r.is_success());
        assert!(r.failures.iter().any(|f| f.operation == FailedOperation::ReadLocal));
        assert!(r.deletion_skipped.is_some());
        assert!(fake.object("p/o/json/locked/hidden.json").is_some());
    }
}

#[tokio::test]
async fn s3c07_failed_listing_is_a_failure_not_a_silent_skip() {
    let fake = FakeS3::start().await;
    fake.add_rule(Rule::new(Action::Error { status: 403, code: "AccessDenied", message: "Access Denied" }).method("GET"));
    let (client, cfg) = client_for(&fake, "p");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true)).await;
    assert!(!r.is_success());
    assert!(r.failures.iter().any(|f| f.operation == FailedOperation::List), "{r:?}");
}

#[tokio::test]
async fn s3c06_pagination_deletes_everything_stale() {
    let fake = FakeS3::start_with(FakeConfig { list_page_size: 3, ..FakeConfig::default() }).await;
    for i in 0..10 {
        fake.put_object(&format!("p/o/json/stale{i}.json"), b"s");
    }
    let (client, cfg) = client_for(&fake, "p");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "keep.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true)).await;
    assert_eq!(r.stats.deleted, 10, "{r:?}");
    assert!(fake.requests("GET", "list-type").len() >= 4);
}

// ── S3C-08: dry run ────────────────────────────────────────────────────────

#[tokio::test]
async fn s3c08_dry_run_performs_no_writes() {
    let fake = FakeS3::start().await;
    fake.put_object("p/o/json/stale.json", b"s");
    let (client, cfg) = client_for(&fake, "p");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "new.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true).dry_run(true)).await;
    assert!(r.is_success(), "{r:?}");
    assert!(r.dry_run);
    assert_eq!(r.would_upload, vec!["p/o/json/new.json"]);
    assert_eq!(r.would_delete, vec!["p/o/json/stale.json"]);
    assert_eq!(r.stats.uploaded + r.stats.deleted, 0);
    assert_eq!(fake.mutating_requests(), 0, "{:?}", fake.records());
    assert_eq!(fake.keys(), vec!["p/o/json/stale.json"]);
}

// ── S3C-12/19: keys on the wire and in listings ────────────────────────────

const AWKWARD: &[&str] = &[
    "with space.json",
    "a+b.json",
    "100%.json",
    "what?.json",
    "file#1.json",
    "a&b.json",
    "r\u{e9}sum\u{e9}.json",
    "app (1).json",
    "q=1;x,y.json",
    "emoji-\u{1f600}.json",
    "a<b>.json",
];

#[tokio::test]
async fn s3c12_awkward_key_names_round_trip_exactly() {
    let fake = FakeS3::start().await;
    let (client, cfg) = client_for(&fake, "pfx");
    let dir = tempfile::tempdir().unwrap();
    for name in AWKWARD {
        write(dir.path(), name, name.as_bytes());
    }
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert!(r.is_success(), "{r:?}");
    for name in AWKWARD {
        let key = format!("pfx/o/json/{name}");
        assert_eq!(fake.body_of(&key).as_deref(), Some(name.as_bytes()), "{key}: {:?}", fake.keys());
    }
    // Second run: all skipped, and delete-stale finds nothing stale.
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true)).await;
    assert_eq!((r.stats.skipped, r.stats.deleted, r.stats.uploaded), (AWKWARD.len(), 0, 0), "{r:?}");
}

#[tokio::test]
async fn s3c19_listing_decodes_keys_for_every_server_behaviour() {
    for (ignore, encoding) in [
        (false, ListEncoding::Percent20),
        (false, ListEncoding::QueryPlus),
        (true, ListEncoding::Percent20),
    ] {
        let fake = FakeS3::start_with(FakeConfig {
            list_ignores_encoding_type: ignore,
            list_encoding: encoding,
            ..FakeConfig::default()
        })
        .await;
        for name in AWKWARD {
            fake.put_object(&format!("pfx/{name}"), b"1");
        }
        let (client, _) = client_for(&fake, "pfx");
        let mut keys = client.list_objects("pfx/").await.unwrap();
        keys.sort();
        let mut want: Vec<String> = AWKWARD.iter().map(|n| format!("pfx/{n}")).collect();
        want.sort();
        assert_eq!(keys, want, "ignore={ignore} encoding={encoding:?}");
    }
}

// ── S3C-13: retries, timeouts, HEAD fallback ───────────────────────────────

#[tokio::test]
async fn s3c13_transient_errors_are_retried() {
    let fake = FakeS3::start().await;
    fake.add_rule(
        Rule::new(Action::Error { status: 503, code: "SlowDown", message: "Please reduce your request rate." })
            .method("PUT")
            .times(2),
    );
    let (client, _) = client_for(&fake, "");
    client.put_object("a", &b"x"[..], "text/plain", &[]).await.expect("succeeds on the 3rd attempt");
    assert_eq!(fake.requests("PUT", "").len(), 3);
    assert_eq!(fake.body_of("a").unwrap(), b"x");
}

#[tokio::test]
async fn s3c13_dropped_connections_are_retried() {
    let fake = FakeS3::start().await;
    fake.add_rule(Rule::new(Action::Drop).method("PUT").times(1));
    let (client, _) = client_for(&fake, "");
    client.put_object("a", &b"x"[..], "text/plain", &[]).await.expect("retried");
    assert!(fake.object("a").is_some());
}

#[tokio::test]
async fn s3c13_permanent_errors_are_not_retried() {
    let fake = FakeS3::start().await;
    fake.add_rule(Rule::new(Action::Error { status: 403, code: "AccessDenied", message: "Access Denied" }));
    let (client, _) = client_for(&fake, "");
    let err = client.put_object("a", &b"x"[..], "text/plain", &[]).await.unwrap_err();
    assert_eq!(fake.requests("PUT", "").len(), 1);
    assert_eq!(err.status(), Some(403));
}

#[tokio::test]
async fn s3c13_retries_are_bounded() {
    let fake = FakeS3::start().await;
    fake.add_rule(Rule::new(Action::Error { status: 500, code: "InternalError", message: "oops" }));
    let (client, _) = client_for(&fake, "");
    let err = client.put_object("a", &b"x"[..], "text/plain", &[]).await.unwrap_err();
    assert_eq!(fake.requests("PUT", "").len(), 4);
    assert!(err.to_string().contains("InternalError"), "{err}");
}

#[tokio::test]
async fn s3c13_a_stalled_server_times_out_by_idleness() {
    let fake = FakeS3::start().await;
    fake.add_rule(Rule::new(Action::Delay(Duration::from_secs(3))).method("PUT"));
    let client = S3Client::with_options(
        config_for(&fake, ""),
        ClientOptions {
            retry: RetryPolicy::none(),
            idle_timeout: Duration::from_millis(300),
            ..fast_options()
        },
    )
    .unwrap();
    let started = std::time::Instant::now();
    let err = client.put_object("a", &b"x"[..], "text/plain", &[]).await.unwrap_err();
    assert!(matches!(err, S3Error::Timeout { .. }), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn s3c11_head_403_falls_back_to_uploading() {
    let fake = FakeS3::start_with(FakeConfig { deny_head: true, ..FakeConfig::default() }).await;
    let (client, cfg) = client_for(&fake, "p");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert!(r.is_success(), "{r:?}");
    assert_eq!(r.stats.uploaded, 1);
    // Without ListBucket a missing key also reads as 403.
    let fake = FakeS3::start_with(FakeConfig { no_list_permission: true, ..FakeConfig::default() }).await;
    let (client, cfg) = client_for(&fake, "p");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o).await;
    assert_eq!(r.stats.uploaded, 1, "{r:?}");
}

// ── S3C-16: session tokens, credentials ────────────────────────────────────

#[tokio::test]
async fn s3c16_session_token_is_signed_and_required() {
    let fake = FakeS3::start_with(FakeConfig { session_token: Some("TOKEN//abc==".into()), ..FakeConfig::default() }).await;
    let (client, cfg) = client_for(&fake, "p");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.json", b"{}");
    let r = run_sync(&client, &cfg, dir.path(), "o/json", |o| o.delete_stale(true)).await;
    assert!(r.is_success(), "{r:?}");
    assert!(fake.records().iter().all(|r| r.signed_headers.contains(&"x-amz-security-token".to_string())));

    // Without the token the server refuses.
    let mut no_token = cfg.clone();
    no_token.session_token = None;
    let client = S3Client::with_options(no_token, fast_options()).unwrap();
    assert!(client.put_object("p/x", &b"x"[..], "text/plain", &[]).await.is_err());
}

#[test]
fn s3c16_missing_credentials_fail_before_any_request() {
    let cfg = S3Config {
        bucket: "b".into(),
        region: "us-east-1".into(),
        prefix: String::new(),
        endpoint: Some("http://127.0.0.1:9".into()),
        access_key_id: String::new(),
        secret_access_key: String::new(),
        session_token: None,
    };
    let err = S3Client::new(cfg).unwrap_err();
    assert!(matches!(err, S3Error::InvalidConfig(_)), "{err}");
}

// ── Shadow mode sanity: the verifier is what rejects bad requests ──────────

#[tokio::test]
async fn shadow_mode_serves_but_records_signature_problems() {
    let fake = FakeS3::start_with(FakeConfig { sig_mode: SigMode::Shadow, ..FakeConfig::default() }).await;
    let mut cfg = config_for(&fake, "");
    cfg.secret_access_key = "another-secret-another-secret-12".into();
    let client = S3Client::with_options(cfg, fast_options()).unwrap();
    client.put_object("a", &b"x"[..], "text/plain", &[]).await.expect("shadow serves");
    assert!(!fake.records()[0].signature_ok);
    let _ = BTreeMap::<String, String>::new();
}
