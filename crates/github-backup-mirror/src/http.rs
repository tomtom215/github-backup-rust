// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The HTTPS client shared by the Gitea and GitLab API clients: HTTPS only,
//! the proxy environment honoured (the GitHub client's
//! [`ProxyClient`]), a per-request
//! timeout, a cap on response bodies, and retries of transient failures.
//!
//! The API token travels in a request header, so the client refuses plain
//! `http://`: a self-hosted instance must be reachable over HTTPS (a private
//! certificate authority is fine, see the mirroring documentation).

use std::time::Duration;

use bytes::Bytes;
use github_backup_client::{ClientError, ProxyClient};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{Method, Request};
use tracing::warn;

use crate::error::MirrorError;

const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));

/// Time limit for one request, response body included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Largest response body that is read (API answers are a few KiB).
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Attempts per request, the first included.
const MAX_ATTEMPTS: u32 = 3;

/// Back-off before the first retry; doubles each time.
const BASE_DELAY: Duration = Duration::from_millis(500);

/// A response: status and (capped) body.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) body: Bytes,
}

impl Response {
    /// This response as an [`MirrorError::Api`].
    pub(crate) fn into_error(self) -> MirrorError {
        MirrorError::Api {
            status: self.status,
            body: String::from_utf8_lossy(&self.body).into_owned(),
        }
    }
}

/// The HTTP client of one API client.
#[derive(Clone)]
pub(crate) struct ApiHttp {
    http: ProxyClient,
    retry_delay: Duration,
}

impl ApiHttp {
    /// An HTTPS-only client trusting the system certificate store, through
    /// the proxy the environment names (`HTTPS_PROXY`, `NO_PROXY`, ...).
    pub(crate) fn new() -> Result<Self, MirrorError> {
        Ok(Self {
            http: ProxyClient::from_env(false).map_err(|e| match e {
                ClientError::Tls(why) => MirrorError::Tls(why),
                other => MirrorError::Tls(other.to_string()),
            })?,
            retry_delay: BASE_DELAY,
        })
    }

    /// A client that also speaks plain HTTP, for the in-process fake servers
    /// of the tests, with no back-off between retries and no proxy.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::for_tests_via(github_backup_client::ProxySettings::default())
    }

    /// Like [`for_tests`](Self::for_tests) with explicit proxy settings.
    #[cfg(test)]
    pub(crate) fn for_tests_via(proxy: github_backup_client::ProxySettings) -> Self {
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        Self {
            http: ProxyClient::new(proxy, tls, true),
            retry_delay: Duration::ZERO,
        }
    }

    /// Sends a request with the given authentication header and optional
    /// JSON body.  Transport failures, timeouts and `5xx` answers are retried
    /// up to [`MAX_ATTEMPTS`] times in total; the last outcome is returned
    /// whatever its status.
    pub(crate) async fn send(
        &self,
        method: Method,
        url: &str,
        auth: (&str, &str),
        json: Option<Vec<u8>>,
    ) -> Result<Response, MirrorError> {
        let mut attempt = 1;
        loop {
            let result = self
                .send_once(method.clone(), url, auth, json.clone())
                .await;
            let transient = match &result {
                Ok(response) => response.status >= 500,
                Err(
                    MirrorError::Transport(_) | MirrorError::Http(_) | MirrorError::Timeout { .. },
                ) => true,
                Err(_) => false,
            };
            if !transient || attempt >= MAX_ATTEMPTS {
                return result;
            }
            let delay = self.retry_delay * 2u32.pow(attempt - 1);
            warn!(%method, url, attempt, max_attempts = MAX_ATTEMPTS, "mirror API request failed; retrying");
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn send_once(
        &self,
        method: Method,
        url: &str,
        (auth_name, auth_value): (&str, &str),
        json: Option<Vec<u8>>,
    ) -> Result<Response, MirrorError> {
        let mut builder = Request::builder()
            .method(method)
            .uri(url)
            .header(auth_name, auth_value)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json");
        if json.is_some() {
            builder = builder.header("Content-Type", "application/json");
        }
        let request = builder.body(Full::new(Bytes::from(json.unwrap_or_default())))?;
        let exchange = async {
            let response = self.http.request(request).await?;
            let status = response.status().as_u16();
            let body = Limited::new(response.into_body(), MAX_BODY_BYTES)
                .collect()
                .await
                .map_err(|e| MirrorError::Body(e.to_string()))?
                .to_bytes();
            Ok(Response { status, body })
        };
        tokio::time::timeout(REQUEST_TIMEOUT, exchange)
            .await
            .map_err(|_| MirrorError::Timeout {
                url: url.to_owned(),
            })?
    }
}

/// Percent-encodes a string for use as one URL path segment (`/` becomes
/// `%2F`, so `group/project` stays a single segment).
pub(crate) fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// An in-process HTTP server answering from a handler, recording every
/// request, for the API client tests.
#[cfg(test)]
pub(crate) mod fake {
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;

    /// One request as the server saw it.
    #[derive(Debug, Clone)]
    pub(crate) struct Seen {
        pub(crate) method: String,
        /// Path and query, as sent (still percent-encoded).
        pub(crate) target: String,
        pub(crate) body: String,
        pub(crate) headers: Vec<(String, String)>,
    }

    type Handler = dyn Fn(&Seen) -> (u16, String) + Send + Sync;

    pub(crate) struct FakeApi {
        pub(crate) base_url: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl FakeApi {
        pub(crate) async fn start(
            handler: impl Fn(&Seen) -> (u16, String) + Send + Sync + 'static,
        ) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
            let handler: Arc<Handler> = Arc::new(handler);
            let log = Arc::clone(&seen);
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let (handler, log) = (Arc::clone(&handler), Arc::clone(&log));
                    tokio::spawn(async move {
                        let service =
                            service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                                let (handler, log) = (Arc::clone(&handler), Arc::clone(&log));
                                async move {
                                    let method = req.method().to_string();
                                    let target = req.uri().to_string();
                                    let headers = req
                                        .headers()
                                        .iter()
                                        .map(|(k, v)| {
                                            (k.to_string(), v.to_str().unwrap_or("").to_owned())
                                        })
                                        .collect();
                                    let body = req.into_body().collect().await.unwrap().to_bytes();
                                    let seen = Seen {
                                        method,
                                        target,
                                        body: String::from_utf8_lossy(&body).into_owned(),
                                        headers,
                                    };
                                    let (status, reply) = handler(&seen);
                                    log.lock().unwrap().push(seen);
                                    Ok::<_, Infallible>(
                                        hyper::Response::builder()
                                            .status(status)
                                            .header("content-type", "application/json")
                                            .body(Full::new(Bytes::from(reply)))
                                            .unwrap(),
                                    )
                                }
                            });
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            });
            Self { base_url, seen }
        }

        pub(crate) fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_segment_encodes_slash_and_keeps_safe_characters() {
        assert_eq!(encode_segment("foo/bar"), "foo%2Fbar");
        assert_eq!(encode_segment("my-repo_v1.0"), "my-repo_v1.0");
        assert_eq!(encode_segment("a b"), "a%20b");
    }

    #[tokio::test]
    async fn server_errors_are_retried_and_client_errors_are_not() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let api = fake::FakeApi::start(move |seen| {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match (seen.target.as_str(), n) {
                ("/flaky", 0) => (502, "{}".into()),
                ("/flaky", _) => (200, "{\"ok\":true}".into()),
                _ => (404, "{}".into()),
            }
        })
        .await;
        let http = ApiHttp::for_tests();
        let ok = http
            .send(
                Method::GET,
                &format!("{}/flaky", api.base_url),
                ("X", "y"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(ok.status, 200);
        let missing = http
            .send(
                Method::GET,
                &format!("{}/missing", api.base_url),
                ("X", "y"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(missing.status, 404);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn the_production_client_refuses_plain_http() {
        let api = fake::FakeApi::start(|_| (200, "{}".into())).await;
        let http = ApiHttp::new().unwrap();
        let err = http
            .send(
                Method::GET,
                &format!("{}/x", api.base_url),
                ("X", "y"),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, MirrorError::Transport(_)), "{err:?}");
        assert!(api.seen().is_empty(), "nothing reached the server");
    }

    // ── Proxy ─────────────────────────────────────────────────────────────

    fn proxy(vars: &[(&str, String)]) -> github_backup_client::ProxySettings {
        let vars: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        github_backup_client::ProxySettings::from_lookup(|n| vars.get(n).cloned())
    }

    /// A proxy that records the first request head of every connection and
    /// refuses every `CONNECT` with `407`.
    async fn refusing_proxy() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let heads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&heads);
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = s
                    .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                    .await;
            }
        });
        (port, heads)
    }

    #[tokio::test]
    async fn https_api_requests_tunnel_through_the_proxy_without_leaking_its_credentials() {
        let (port, heads) = refusing_proxy().await;
        let http = ApiHttp::for_tests_via(proxy(&[(
            "HTTPS_PROXY",
            format!("http://alice:pr0xy-s3cret@127.0.0.1:{port}"),
        )]));
        let err = http
            .send(
                Method::GET,
                "https://gitea.example.invalid/api/v1/version",
                ("Authorization", "token t"),
                None,
            )
            .await
            .unwrap_err();
        let head = heads.lock().unwrap()[0].to_ascii_lowercase();
        assert!(
            head.starts_with("connect gitea.example.invalid:443 http/1.1"),
            "{head}"
        );
        // base64("alice:pr0xy-s3cret")
        assert!(
            head.contains("proxy-authorization: basic ywxpy2u6chiwehktcznjcmv0"),
            "{head}"
        );
        assert!(
            !head.contains("token t"),
            "the API token stays inside the tunnel"
        );
        for text in [
            err.to_string(),
            format!("{err:?}"),
            format!("{:?}", http.http),
        ] {
            assert!(!text.contains("pr0xy-s3cret"), "{text}");
            assert!(!text.contains("YWxpY2U6cHIweHktczNjcmV0"), "{text}");
        }
    }

    #[tokio::test]
    async fn plain_http_api_requests_go_to_the_proxy_in_absolute_form() {
        // The fake API plays the proxy: it records the absolute request target.
        let proxy_api = fake::FakeApi::start(|_| (200, "{\"ok\":true}".into())).await;
        let http = ApiHttp::for_tests_via(proxy(&[(
            "HTTP_PROXY",
            proxy_api.base_url.replace("http://", "http://u:p@"),
        )]));
        let response = http
            .send(
                Method::GET,
                "http://gitlab.example.invalid/api/v4/version",
                ("PRIVATE-TOKEN", "t"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        let seen = proxy_api.seen();
        assert_eq!(
            seen[0].target,
            "http://gitlab.example.invalid/api/v4/version"
        );
        assert!(
            seen[0]
                .headers
                .iter()
                .any(|(k, v)| k == "proxy-authorization" && v == "Basic dTpw"),
            "{:?}",
            seen[0].headers
        );
    }

    #[tokio::test]
    async fn no_proxy_hosts_are_reached_directly() {
        let (port, heads) = refusing_proxy().await;
        let api = fake::FakeApi::start(|_| (200, "{}".into())).await;
        let http = ApiHttp::for_tests_via(proxy(&[
            ("HTTP_PROXY", format!("http://127.0.0.1:{port}")),
            ("NO_PROXY", "localhost,127.0.0.1".to_string()),
        ]));
        let response = http
            .send(
                Method::GET,
                &format!("{}/x", api.base_url),
                ("X", "y"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(api.seen()[0].target, "/x");
        assert!(heads.lock().unwrap().is_empty(), "the proxy was bypassed");
    }
}
