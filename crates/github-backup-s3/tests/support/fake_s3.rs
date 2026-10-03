// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! In-process fake of an S3-compatible server (path-style addressing).
//!
//! The fake verifies AWS Signature Version 4 **independently** (see
//! [`super::sigv4_check`]): it rebuilds the canonical request from what it
//! actually receives and rejects the request when the signature does not
//! match, when a header named in `SignedHeaders` was not sent, when an
//! `x-amz-*` header was sent but not signed, when the payload hash does not
//! match the body, or when the request-target is not canonically
//! percent-encoded.  It implements just enough of the S3 API for the backup
//! client: `PutObject`, `HeadObject`, `DeleteObject`, `ListObjectsV2` and the
//! multipart-upload calls, plus fault injection and a request log.
//!
//! Every response body and header is generated here; nothing is copied from
//! the client under test.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use super::sigv4_check::{
    check_encoding, hex, sha256_hex, uri_decode, uri_encode, verify, Received, Rejection,
    VerifyConfig,
};

/// Whether signature problems are enforced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigMode {
    /// Reject every request whose signature does not verify (like S3).
    Strict,
    /// Verify and record the verdict, but serve the request regardless.
    /// Used to observe behaviour that sits *behind* the signature check.
    Shadow,
}

/// How `ListObjectsV2` encodes keys when `encoding-type=url` is requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListEncoding {
    /// Spaces as `%20` (RFC 3986 style).
    Percent20,
    /// Spaces as `+` and a literal `+` as `%2B` (form / Go `QueryEscape` style).
    QueryPlus,
}

/// Fake server settings.
#[derive(Clone, Debug)]
pub struct FakeConfig {
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    /// When `Some`, the server requires this `x-amz-security-token`.
    pub session_token: Option<String>,
    pub sig_mode: SigMode,
    /// Enforce canonical percent-encoding of the request-target.
    pub strict_encoding: bool,
    /// Maximum keys per `ListObjectsV2` page.
    pub list_page_size: usize,
    /// Smallest allowed part (except the last) in a multipart upload.
    pub min_part_size: usize,
    /// Behave like a server that ignores `encoding-type=url`.
    pub list_ignores_encoding_type: bool,
    pub list_encoding: ListEncoding,
    /// Simulate credentials lacking `s3:ListBucket`: HEAD of a missing key
    /// answers 403 instead of 404.
    pub no_list_permission: bool,
    /// Simulate credentials lacking `s3:GetObject`: HEAD always answers 403.
    pub deny_head: bool,
    /// Shift the server clock (seconds) to provoke `RequestTimeTooSkewed`.
    pub clock_offset_secs: i64,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            bucket: "testbucket".to_string(),
            access_key: "AKIDTESTKEY".to_string(),
            secret_key: "SECRETTESTKEY0123456789abcdefghijklmnop".to_string(),
            region: "us-east-1".to_string(),
            session_token: None,
            sig_mode: SigMode::Strict,
            strict_encoding: true,
            list_page_size: 1000,
            min_part_size: 5 * 1024 * 1024,
            list_ignores_encoding_type: false,
            list_encoding: ListEncoding::Percent20,
            no_list_permission: false,
            deny_head: false,
            clock_offset_secs: 0,
        }
    }
}

/// An object held by the fake.
#[derive(Clone, Debug)]
pub struct StoredObject {
    pub body: Vec<u8>,
    pub content_type: String,
    /// `x-amz-meta-*` headers without the prefix.
    pub metadata: BTreeMap<String, String>,
    pub etag: String,
}

/// What a matching fault rule does.
#[derive(Clone, Debug)]
pub enum Action {
    /// Reply with an S3 error document.
    Error {
        status: u16,
        code: &'static str,
        message: &'static str,
    },
    /// Close the connection without answering.
    Drop,
    /// Sleep before processing the request.
    Delay(Duration),
    /// `CompleteMultipartUpload` answers `200 OK` whose body is an `<Error>`.
    CompleteWithErrorBody { code: &'static str },
}

/// A fault-injection rule.  All set predicates must match.
#[derive(Clone, Debug)]
pub struct Rule {
    pub method: Option<&'static str>,
    pub key_contains: Option<String>,
    pub query_contains: Option<&'static str>,
    /// Let this many matching requests through before the rule applies.
    pub skip: usize,
    /// Apply to this many requests (`None` = forever).
    pub times: Option<usize>,
    pub action: Action,
    seen: usize,
    applied: usize,
}

impl Rule {
    pub fn new(action: Action) -> Self {
        Self {
            method: None,
            key_contains: None,
            query_contains: None,
            skip: 0,
            times: None,
            action,
            seen: 0,
            applied: 0,
        }
    }

    pub fn method(mut self, method: &'static str) -> Self {
        self.method = Some(method);
        self
    }

    pub fn key_contains(mut self, text: &str) -> Self {
        self.key_contains = Some(text.to_string());
        self
    }

    pub fn query_contains(mut self, text: &'static str) -> Self {
        self.query_contains = Some(text);
        self
    }

    pub fn skip(mut self, n: usize) -> Self {
        self.skip = n;
        self
    }

    pub fn times(mut self, n: usize) -> Self {
        self.times = Some(n);
        self
    }

    fn matches(&self, method: &str, key: &str, query: &str) -> bool {
        self.method.is_none_or(|m| m == method)
            && self.key_contains.as_deref().is_none_or(|k| key.contains(k))
            && self.query_contains.is_none_or(|q| query.contains(q))
    }
}

/// One request as the server saw it.
#[derive(Clone, Debug)]
pub struct Record {
    pub method: String,
    /// Raw request-target (`/bucket/key?query`).
    pub target: String,
    pub path: String,
    pub query: String,
    /// Decoded object key (empty for bucket-level requests).
    pub key: String,
    pub headers: Vec<(String, String)>,
    pub body_len: usize,
    /// Final HTTP status (0 when the connection was dropped).
    pub status: u16,
    /// Whether the signature (and header rules) verified.
    pub signature_ok: bool,
    /// Code of the rejection, when verification failed (even in shadow mode).
    pub rejection: Option<String>,
    pub signed_headers: Vec<String>,
}

impl Record {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

struct Upload {
    key: String,
    content_type: String,
    metadata: BTreeMap<String, String>,
    parts: BTreeMap<u32, (Vec<u8>, String)>,
}

#[derive(Default)]
struct State {
    objects: BTreeMap<String, StoredObject>,
    uploads: HashMap<String, Upload>,
    records: Vec<Record>,
    rules: Vec<Rule>,
    next_upload: u64,
}

struct Shared {
    cfg: Mutex<FakeConfig>,
    state: Mutex<State>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("fake state poisoned")
    }
    fn cfg(&self) -> FakeConfig {
        self.cfg.lock().expect("fake config poisoned").clone()
    }
}

/// A running fake S3 server.  Dropping it stops accepting connections.
pub struct FakeS3 {
    addr: SocketAddr,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeS3 {
    /// Starts a server with default settings.
    pub async fn start() -> Self {
        Self::start_with(FakeConfig::default()).await
    }

    /// Starts a server on an ephemeral loopback port.
    pub async fn start_with(cfg: FakeConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake S3 listener");
        let addr = listener.local_addr().expect("listener address");
        let shared = Arc::new(Shared {
            cfg: Mutex::new(cfg),
            state: Mutex::new(State::default()),
        });
        let task = tokio::spawn(accept_loop(listener, Arc::clone(&shared)));
        Self { addr, shared, task }
    }

    /// `http://127.0.0.1:PORT`.
    pub fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The `Host` header value clients must send.
    pub fn host(&self) -> String {
        self.addr.to_string()
    }

    pub fn config(&self) -> FakeConfig {
        self.shared.cfg()
    }

    /// Mutates the settings (takes effect for the next request).
    pub fn configure(&self, f: impl FnOnce(&mut FakeConfig)) {
        f(&mut self.shared.cfg.lock().expect("fake config poisoned"));
    }

    pub fn bucket(&self) -> String {
        self.config().bucket
    }

    // ── object store access (bypasses authentication) ───────────────────

    pub fn put_object(&self, key: &str, body: &[u8]) {
        self.put_object_with(key, body, "application/octet-stream", BTreeMap::new());
    }

    pub fn put_object_with(
        &self,
        key: &str,
        body: &[u8],
        content_type: &str,
        metadata: BTreeMap<String, String>,
    ) {
        self.shared.state().objects.insert(
            key.to_string(),
            StoredObject {
                body: body.to_vec(),
                content_type: content_type.to_string(),
                metadata,
                etag: etag_of(body),
            },
        );
    }

    pub fn object(&self, key: &str) -> Option<StoredObject> {
        self.shared.state().objects.get(key).cloned()
    }

    pub fn body_of(&self, key: &str) -> Option<Vec<u8>> {
        self.object(key).map(|o| o.body)
    }

    pub fn keys(&self) -> Vec<String> {
        self.shared.state().objects.keys().cloned().collect()
    }

    pub fn remove_object(&self, key: &str) {
        self.shared.state().objects.remove(key);
    }

    /// Number of multipart uploads that were started but neither completed
    /// nor aborted.
    pub fn open_uploads(&self) -> usize {
        self.shared.state().uploads.len()
    }

    // ── request log and fault injection ─────────────────────────────────

    pub fn records(&self) -> Vec<Record> {
        self.shared.state().records.clone()
    }

    pub fn clear_records(&self) {
        self.shared.state().records.clear();
    }

    /// Requests with the given method (and, if non-empty, query substring).
    pub fn requests(&self, method: &str, query_contains: &str) -> Vec<Record> {
        self.records()
            .into_iter()
            .filter(|r| r.method == method && r.query.contains(query_contains))
            .collect()
    }

    /// Number of mutating requests (PUT, POST, DELETE) seen so far.
    pub fn mutating_requests(&self) -> usize {
        self.records()
            .iter()
            .filter(|r| matches!(r.method.as_str(), "PUT" | "POST" | "DELETE"))
            .count()
    }

    pub fn add_rule(&self, rule: Rule) {
        self.shared.state().rules.push(rule);
    }

    pub fn clear_rules(&self) {
        self.shared.state().rules.clear();
    }
}

fn etag_of(body: &[u8]) -> String {
    format!("\"{}\"", &sha256_hex(body)[..32])
}

fn now_unix(offset: i64) -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    secs + offset
}

async fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let service = service_fn(move |req| handle(req, Arc::clone(&shared)));
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

/// A fully built reply.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn xml(status: u16, body: String) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_string(), "application/xml".to_string())],
            body: body.into_bytes(),
        }
    }

    fn error(status: u16, code: &str, message: &str) -> Self {
        Self::xml(status, error_xml(code, message, None))
    }

    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn error_xml(code: &str, message: &str, canonical_request: Option<&str>) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{}</Message>",
        xml_escape(message)
    );
    if let Some(creq) = canonical_request {
        out.push_str(&format!(
            "<CanonicalRequest>{}</CanonicalRequest>",
            xml_escape(creq)
        ));
    }
    out.push_str("<RequestId>FAKEREQUEST</RequestId><HostId>fakehost</HostId></Error>");
    out
}

fn to_response(reply: Reply, is_head: bool) -> Response<Full<Bytes>> {
    let mut builder = Response::builder().status(reply.status);
    let mut has_length = false;
    for (name, value) in &reply.headers {
        if name.eq_ignore_ascii_case("content-length") {
            has_length = true;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder = builder.header("x-amz-request-id", "FAKEREQUEST");
    if is_head {
        if !has_length {
            builder = builder.header("content-length", reply.body.len().to_string());
        }
        return builder
            .body(Full::new(Bytes::new()))
            .expect("valid response");
    }
    builder
        .body(Full::new(Bytes::from(reply.body)))
        .expect("valid response")
}

/// The request after parsing, as handed to the S3 dispatcher.
struct Parsed {
    method: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    bucket: String,
    key: String,
    params: HashMap<String, String>,
}

impl Parsed {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

async fn handle(
    req: Request<Incoming>,
    shared: Arc<Shared>,
) -> Result<Response<Full<Bytes>>, std::io::Error> {
    let (head, body) = req.into_parts();
    let method = head.method.as_str().to_string();
    let path = head.uri.path().to_string();
    let query = head.uri.query().unwrap_or("").to_string();
    let target = match head.uri.query() {
        Some(q) => format!("{path}?{q}"),
        None => path.clone(),
    };
    let headers: Vec<(String, String)> = head
        .headers
        .iter()
        .map(|(n, v)| {
            (
                n.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body: Bytes = body
        .collect()
        .await
        .map_err(std::io::Error::other)?
        .to_bytes();
    let is_head = method == "HEAD";
    let cfg = shared.cfg();

    let decoded_path = uri_decode(&path)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default();
    let trimmed = decoded_path.trim_start_matches('/');
    let (bucket, key) = match trimmed.split_once('/') {
        Some((b, k)) => (b.to_string(), k.to_string()),
        None => (trimmed.to_string(), String::new()),
    };
    let params: HashMap<String, String> = super::sigv4_check::query_pairs(&query)
        .unwrap_or_default()
        .into_iter()
        .map(|(n, v)| {
            (
                String::from_utf8_lossy(&n).into_owned(),
                String::from_utf8_lossy(&v).into_owned(),
            )
        })
        .collect();

    let mut record = Record {
        method: method.clone(),
        target,
        path: path.clone(),
        query: query.clone(),
        key: key.clone(),
        headers: headers.clone(),
        body_len: body.len(),
        status: 0,
        signature_ok: true,
        rejection: None,
        signed_headers: Vec::new(),
    };

    // Pre-authentication faults: dropped connections and delays.
    let early = take_fault(&shared, &method, &key, &query, false);
    if let Some(action) = early {
        match action {
            Action::Drop => {
                shared.state().records.push(record);
                return Err(std::io::Error::other("fake S3: connection dropped by rule"));
            }
            Action::Delay(d) => tokio::time::sleep(d).await,
            _ => {}
        }
    }

    // Authentication.
    let body_hash = sha256_hex(&body);
    let rx = Received {
        method: &method,
        raw_path: &path,
        raw_query: &query,
        headers: &headers,
        body_sha256: &body_hash,
    };
    let mut rejection: Option<Rejection> = None;
    if cfg.strict_encoding {
        if let Err(r) = check_encoding(&rx) {
            rejection = Some(r);
        }
    }
    if rejection.is_none() {
        let vcfg = VerifyConfig {
            access_key: &cfg.access_key,
            secret: &cfg.secret_key,
            region: &cfg.region,
            now_unix: now_unix(cfg.clock_offset_secs),
            session_token: cfg.session_token.as_deref(),
        };
        match verify(&rx, &vcfg) {
            Ok(auth) => record.signed_headers = auth.signed_headers,
            Err(r) => rejection = Some(r),
        }
    }
    if let Some(rej) = rejection {
        record.signature_ok = false;
        record.rejection = Some(rej.code.to_string());
        let enforce = !(cfg.sig_mode == SigMode::Shadow && rej.signature_class);
        if enforce {
            let mut reply = Reply::xml(
                rej.status,
                error_xml(rej.code, &rej.message, rej.canonical_request.as_deref()),
            );
            reply.headers.extend(rej.headers.clone());
            record.status = rej.status;
            shared.state().records.push(record);
            return Ok(to_response(reply, is_head));
        }
    }

    let parsed = Parsed {
        method: method.clone(),
        path,
        query: query.clone(),
        headers,
        bucket,
        key: key.clone(),
        params,
    };

    // Post-authentication faults: service-side errors.
    if let Some(Action::Error {
        status,
        code,
        message,
    }) = take_fault(&shared, &method, &key, &query, true)
    {
        record.status = status;
        shared.state().records.push(record);
        return Ok(to_response(Reply::error(status, code, message), is_head));
    }

    let reply = dispatch(&shared, &cfg, &parsed, &body);
    record.status = reply.status;
    shared.state().records.push(record);
    Ok(to_response(reply, is_head))
}

/// Finds the first applicable rule.  `service_side` selects `Error` and
/// `CompleteWithErrorBody` actions; otherwise `Drop` and `Delay`.
fn take_fault(
    shared: &Shared,
    method: &str,
    key: &str,
    query: &str,
    service_side: bool,
) -> Option<Action> {
    let mut state = shared.state();
    for rule in state.rules.iter_mut() {
        let is_service = matches!(
            rule.action,
            Action::Error { .. } | Action::CompleteWithErrorBody { .. }
        );
        if is_service != service_side || !rule.matches(method, key, query) {
            continue;
        }
        if matches!(rule.action, Action::CompleteWithErrorBody { .. }) {
            // Handled by the multipart dispatcher, not here.
            continue;
        }
        rule.seen += 1;
        if rule.seen <= rule.skip {
            continue;
        }
        if rule.times.is_some_and(|t| rule.applied >= t) {
            continue;
        }
        rule.applied += 1;
        return Some(rule.action.clone());
    }
    None
}

fn complete_error_rule(shared: &Shared, key: &str, query: &str) -> Option<&'static str> {
    let mut state = shared.state();
    for rule in state.rules.iter_mut() {
        if let Action::CompleteWithErrorBody { code } = rule.action {
            if !rule.matches("POST", key, query) {
                continue;
            }
            rule.seen += 1;
            if rule.seen <= rule.skip || rule.times.is_some_and(|t| rule.applied >= t) {
                continue;
            }
            rule.applied += 1;
            return Some(code);
        }
    }
    None
}

fn metadata_of(p: &Parsed) -> BTreeMap<String, String> {
    p.headers
        .iter()
        .filter_map(|(n, v)| {
            n.strip_prefix("x-amz-meta-")
                .map(|rest| (rest.to_string(), v.clone()))
        })
        .collect()
}

fn dispatch(shared: &Shared, cfg: &FakeConfig, p: &Parsed, body: &Bytes) -> Reply {
    if p.bucket != cfg.bucket {
        return Reply::error(404, "NoSuchBucket", "The specified bucket does not exist");
    }
    let has = |name: &str| p.params.contains_key(name);
    match (p.method.as_str(), p.key.is_empty()) {
        ("GET", true) if p.params.get("list-type").map(String::as_str) == Some("2") => {
            list_objects(shared, cfg, p)
        }
        ("GET", true) => Reply::error(501, "NotImplemented", "only ListObjectsV2 is implemented"),
        ("HEAD", false) => head_object(shared, cfg, p),
        ("GET", false) => match shared.state().objects.get(&p.key) {
            Some(o) => Reply {
                status: 200,
                headers: vec![
                    ("etag".to_string(), o.etag.clone()),
                    ("content-type".to_string(), o.content_type.clone()),
                ],
                body: o.body.clone(),
            },
            None => Reply::error(404, "NoSuchKey", "The specified key does not exist."),
        },
        ("PUT", false) if has("partNumber") && has("uploadId") => upload_part(shared, cfg, p, body),
        ("PUT", false) => put_object(shared, p, body),
        ("POST", false) if has("uploads") => create_upload(shared, p),
        ("POST", false) if has("uploadId") => complete_upload(shared, cfg, p, body),
        ("DELETE", false) if has("uploadId") => abort_upload(shared, p),
        ("DELETE", false) => {
            shared.state().objects.remove(&p.key);
            Reply::empty(204)
        }
        _ => Reply::error(
            405,
            "MethodNotAllowed",
            "method not allowed for this resource",
        ),
    }
}

fn head_object(shared: &Shared, cfg: &FakeConfig, p: &Parsed) -> Reply {
    if cfg.deny_head {
        return Reply::empty(403);
    }
    match shared.state().objects.get(&p.key) {
        Some(o) => {
            let mut reply = Reply::empty(200)
                .header("content-length", o.body.len().to_string())
                .header("etag", o.etag.clone())
                .header("content-type", o.content_type.clone())
                .header("last-modified", "Wed, 01 Jan 2025 00:00:00 GMT");
            for (k, v) in &o.metadata {
                reply = reply.header(&format!("x-amz-meta-{k}"), v.clone());
            }
            reply
        }
        None if cfg.no_list_permission => Reply::empty(403),
        None => Reply::empty(404),
    }
}

fn put_object(shared: &Shared, p: &Parsed, body: &Bytes) -> Reply {
    if p.header("transfer-encoding").is_some() {
        return Reply::error(
            501,
            "NotImplemented",
            "A header you provided implies functionality that is not implemented (Transfer-Encoding)",
        );
    }
    if p.header("content-length").is_none() {
        return Reply::error(
            411,
            "MissingContentLength",
            "You must provide the Content-Length HTTP header.",
        );
    }
    if p.key.len() > 1024 {
        return Reply::error(400, "KeyTooLongError", "Your key is too long");
    }
    let content_type = p
        .header("content-type")
        .unwrap_or("binary/octet-stream")
        .to_string();
    let etag = etag_of(body);
    shared.state().objects.insert(
        p.key.clone(),
        StoredObject {
            body: body.to_vec(),
            content_type,
            metadata: metadata_of(p),
            etag: etag.clone(),
        },
    );
    Reply::empty(200).header("etag", etag)
}

fn create_upload(shared: &Shared, p: &Parsed) -> Reply {
    let mut state = shared.state();
    state.next_upload += 1;
    let id = format!("upload-{:04}", state.next_upload);
    state.uploads.insert(
        id.clone(),
        Upload {
            key: p.key.clone(),
            content_type: p
                .header("content-type")
                .unwrap_or("binary/octet-stream")
                .to_string(),
            metadata: metadata_of(p),
            parts: BTreeMap::new(),
        },
    );
    Reply::xml(
        200,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>{}</Bucket><Key>{}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>",
            xml_escape(&p.bucket),
            xml_escape(&p.key)
        ),
    )
}

fn upload_part(shared: &Shared, _cfg: &FakeConfig, p: &Parsed, body: &Bytes) -> Reply {
    let Some(id) = p.params.get("uploadId") else {
        return Reply::error(400, "InvalidArgument", "missing uploadId");
    };
    let Some(number) = p
        .params
        .get("partNumber")
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| (1..=10_000).contains(n))
    else {
        return Reply::error(
            400,
            "InvalidArgument",
            "Part number must be an integer between 1 and 10000",
        );
    };
    let mut state = shared.state();
    let Some(upload) = state.uploads.get_mut(id) else {
        return Reply::error(404, "NoSuchUpload", "The specified upload does not exist.");
    };
    if upload.key != p.key {
        return Reply::error(404, "NoSuchUpload", "upload belongs to another key");
    }
    let etag = etag_of(body);
    upload.parts.insert(number, (body.to_vec(), etag.clone()));
    Reply::empty(200).header("etag", etag)
}

/// Parses `<Part><PartNumber>N</PartNumber><ETag>"x"</ETag></Part>` entries.
fn parse_complete_body(body: &str) -> Vec<(u32, String)> {
    let mut parts = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("<Part>") {
        let after = &rest[start + "<Part>".len()..];
        let Some(end) = after.find("</Part>") else {
            break;
        };
        let block = &after[..end];
        let number = between(block, "<PartNumber>", "</PartNumber>").and_then(|n| n.parse().ok());
        let etag = between(block, "<ETag>", "</ETag>").map(|e| e.trim().to_string());
        if let (Some(n), Some(e)) = (number, etag) {
            parts.push((n, e));
        }
        rest = &after[end + "</Part>".len()..];
    }
    parts
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

fn complete_upload(shared: &Shared, cfg: &FakeConfig, p: &Parsed, body: &Bytes) -> Reply {
    let Some(id) = p.params.get("uploadId") else {
        return Reply::error(400, "InvalidArgument", "missing uploadId");
    };
    if let Some(code) = complete_error_rule(shared, &p.key, &p.query) {
        // AWS may answer 200 and report the failure in the body.
        return Reply::xml(
            200,
            error_xml(
                code,
                "We encountered an internal error. Please try again.",
                None,
            ),
        );
    }
    let requested = parse_complete_body(&String::from_utf8_lossy(body));
    let mut state = shared.state();
    let Some(upload) = state.uploads.get(id) else {
        return Reply::error(404, "NoSuchUpload", "The specified upload does not exist.");
    };
    if upload.key != p.key {
        return Reply::error(404, "NoSuchUpload", "upload belongs to another key");
    }
    if requested.is_empty() {
        return Reply::error(
            400,
            "MalformedXML",
            "The XML you provided was not well-formed",
        );
    }
    let mut assembled = Vec::new();
    let mut last_number = 0;
    for (i, (number, etag)) in requested.iter().enumerate() {
        if *number <= last_number {
            return Reply::error(
                400,
                "InvalidPartOrder",
                "The list of parts was not in ascending order.",
            );
        }
        last_number = *number;
        let Some((data, stored_etag)) = upload.parts.get(number) else {
            return Reply::error(
                400,
                "InvalidPart",
                "One or more of the specified parts could not be found.",
            );
        };
        if stored_etag.trim_matches('"') != etag.trim_matches('"') {
            return Reply::error(400, "InvalidPart", "ETag mismatch for a part");
        }
        if i + 1 < requested.len() && data.len() < cfg.min_part_size {
            return Reply::error(
                400,
                "EntityTooSmall",
                "Your proposed upload is smaller than the minimum allowed object size.",
            );
        }
        assembled.extend_from_slice(data);
    }
    let upload = state.uploads.remove(id).expect("checked above");
    let etag = format!("\"{}-{}\"", &sha256_hex(&assembled)[..32], requested.len());
    state.objects.insert(
        upload.key.clone(),
        StoredObject {
            body: assembled,
            content_type: upload.content_type,
            metadata: upload.metadata,
            etag: etag.clone(),
        },
    );
    Reply::xml(
        200,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Location>http://fake/{}</Location><Bucket>{}</Bucket><Key>{}</Key><ETag>{}</ETag></CompleteMultipartUploadResult>",
            xml_escape(&upload.key),
            xml_escape(&p.bucket),
            xml_escape(&upload.key),
            xml_escape(&etag)
        ),
    )
}

fn abort_upload(shared: &Shared, p: &Parsed) -> Reply {
    let Some(id) = p.params.get("uploadId") else {
        return Reply::error(400, "InvalidArgument", "missing uploadId");
    };
    match shared.state().uploads.remove(id) {
        Some(_) => Reply::empty(204),
        None => Reply::error(404, "NoSuchUpload", "The specified upload does not exist."),
    }
}

fn list_objects(shared: &Shared, cfg: &FakeConfig, p: &Parsed) -> Reply {
    let prefix = p.params.get("prefix").cloned().unwrap_or_default();
    let max_keys = p
        .params
        .get("max-keys")
        .and_then(|m| m.parse::<usize>().ok())
        .unwrap_or(1000)
        .min(cfg.list_page_size)
        .max(1);
    let start_after: String = match p.params.get("continuation-token") {
        Some(token) => {
            let decoded: Option<Vec<u8>> = (0..token.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(token.get(i..i + 2)?, 16).ok())
                .collect();
            match decoded.and_then(|b| String::from_utf8(b).ok()) {
                Some(s) => s,
                None => {
                    return Reply::error(
                        400,
                        "InvalidArgument",
                        "The continuation token provided is incorrect",
                    )
                }
            }
        }
        None => String::new(),
    };
    let wants_url = p.params.get("encoding-type").map(String::as_str) == Some("url");
    let encode_keys = wants_url && !cfg.list_ignores_encoding_type;

    let keys: Vec<String> = shared
        .state()
        .objects
        .keys()
        .filter(|k| {
            k.starts_with(&prefix) && (start_after.is_empty() || k.as_str() > start_after.as_str())
        })
        .cloned()
        .collect();
    let truncated = keys.len() > max_keys;
    let page: Vec<&String> = keys.iter().take(max_keys).collect();

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
    );
    xml.push_str(&format!("<Name>{}</Name>", xml_escape(&cfg.bucket)));
    xml.push_str(&format!(
        "<Prefix>{}</Prefix>",
        encode_or_escape(&prefix, encode_keys, cfg)
    ));
    xml.push_str(&format!(
        "<KeyCount>{}</KeyCount><MaxKeys>{max_keys}</MaxKeys>",
        page.len()
    ));
    if encode_keys {
        xml.push_str("<EncodingType>url</EncodingType>");
    }
    xml.push_str(&format!("<IsTruncated>{truncated}</IsTruncated>"));
    if truncated {
        if let Some(last) = page.last() {
            xml.push_str(&format!(
                "<NextContinuationToken>{}</NextContinuationToken>",
                hex(last.as_bytes())
            ));
        }
    }
    let state = shared.state();
    for key in page {
        let size = state.objects.get(key).map_or(0, |o| o.body.len());
        xml.push_str(&format!(
            "<Contents><Key>{}</Key><LastModified>2025-01-01T00:00:00.000Z</LastModified><Size>{size}</Size><StorageClass>STANDARD</StorageClass></Contents>",
            encode_or_escape(key, encode_keys, cfg)
        ));
    }
    xml.push_str("</ListBucketResult>");
    Reply::xml(200, xml)
}

fn encode_or_escape(s: &str, url: bool, cfg: &FakeConfig) -> String {
    if !url {
        return xml_escape(s);
    }
    match cfg.list_encoding {
        ListEncoding::Percent20 => uri_encode(s.as_bytes(), true),
        ListEncoding::QueryPlus => uri_encode(s.as_bytes(), true).replace("%20", "+"),
    }
}
