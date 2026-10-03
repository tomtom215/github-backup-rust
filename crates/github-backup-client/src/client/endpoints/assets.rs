// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Release asset download.
//!
//! GitHub answers an asset request with a redirect to a pre-signed URL on its
//! storage host.  That second request must not carry the API credential: it
//! would disclose the token to a third party, and signed storage URLs reject a
//! request that has both a signature and an `Authorization` header.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Method;
use tracing::debug;
use url::Url;

use crate::api_client::AssetSink;
use crate::error::ClientError;

use super::super::{collect_body, GitHubClient, DEFAULT_TIMEOUT_SECS};

/// Maximum number of redirects followed for one asset.
const MAX_REDIRECTS: u8 = 3;

/// What to do with a redirect from `from` to `to`.
#[derive(Debug, PartialEq, Eq)]
enum Hop {
    /// Follow it, still sending the credential (same origin).
    SameOrigin,
    /// Follow it without the credential (different host, port or scheme).
    DropCredential,
    /// Do not follow it (HTTPS to a non-HTTPS URL).
    Refuse,
}

/// Decides how a redirect from `from` to `to` is followed.
///
/// The credential survives only a redirect within the same origin (scheme,
/// host and effective port).  A redirect from `https` to anything else is
/// refused outright.
fn classify_redirect(from: &Url, to: &Url) -> Hop {
    if from.scheme() == "https" && to.scheme() != "https" {
        return Hop::Refuse;
    }
    let same_origin = from.scheme() == to.scheme()
        && from.host_str().map(str::to_ascii_lowercase) == to.host_str().map(str::to_ascii_lowercase)
        && from.port_or_known_default() == to.port_or_known_default();
    if same_origin {
        Hop::SameOrigin
    } else {
        Hop::DropCredential
    }
}

impl GitHubClient {
    /// Streams a release asset into `sink` and returns the number of bytes
    /// delivered.
    ///
    /// Uses the `application/octet-stream` accept header required by GitHub
    /// and follows up to 3 redirects.  The `Authorization` header is sent to
    /// `asset_url` and to redirects within the same origin only; a redirect
    /// to another host, port or scheme is followed without it, and a redirect
    /// from HTTPS to HTTP is refused.  The body is handed to the sink chunk by
    /// chunk (each chunk must arrive within the request timeout), so memory
    /// use does not depend on the asset size.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors, and
    /// [`ClientError::Io`] when the sink fails.
    pub async fn download_release_asset(
        &self,
        asset_url: &str,
        sink: &mut dyn AssetSink,
    ) -> Result<u64, ClientError> {
        let mut url = asset_url.to_string();
        let mut send_auth = true;
        let mut remaining_redirects = MAX_REDIRECTS;

        loop {
            let req = self
                .build_request_with_auth(Method::GET, &url, send_auth)?
                .header("Accept", "application/octet-stream")
                .body(Full::new(Bytes::new()))
                .map_err(ClientError::Http)?;

            let response = tokio::time::timeout(
                std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                self.http.request(req),
            )
            .await
            .map_err(|_| ClientError::Timeout { url: url.clone() })??;

            let status = response.status();

            if status.is_redirection() {
                if remaining_redirects == 0 {
                    return Err(ClientError::ApiError {
                        status: status.as_u16(),
                        body: "too many redirects".to_string(),
                    });
                }
                remaining_redirects -= 1;
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| ClientError::ApiError {
                        status: status.as_u16(),
                        body: "redirect with no Location header".to_string(),
                    })?;
                let current = Url::parse(&url)?;
                let next = current.join(location)?;
                match classify_redirect(&current, &next) {
                    Hop::SameOrigin => {}
                    Hop::DropCredential => {
                        debug!(
                            from = current.host_str().unwrap_or(""),
                            to = next.host_str().unwrap_or(""),
                            "asset redirect leaves the origin: not sending the credential"
                        );
                        send_auth = false;
                    }
                    Hop::Refuse => {
                        return Err(ClientError::ApiError {
                            status: status.as_u16(),
                            body: format!(
                                "refusing to follow a redirect from https://{} to a non-HTTPS URL",
                                current.host_str().unwrap_or("")
                            ),
                        });
                    }
                }
                url = next.into();
                continue;
            }

            if !status.is_success() {
                let body = collect_body(response.into_body()).await?;
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            }

            let mut body = response.into_body();
            let mut written = 0u64;
            loop {
                let frame = tokio::time::timeout(
                    std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                    body.frame(),
                )
                .await
                .map_err(|_| ClientError::Timeout { url: url.clone() })?;
                let Some(frame) = frame else { break };
                if let Some(chunk) = frame?.data_ref() {
                    sink.write_chunk(chunk)?;
                    written += chunk.len() as u64;
                }
            }
            return Ok(written);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::config::Credential;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("url")
    }

    #[test]
    fn same_origin_keeps_the_credential() {
        let from = url("https://api.github.com/repos/o/r/releases/assets/1");
        assert_eq!(
            classify_redirect(&from, &url("https://api.github.com/other")),
            Hop::SameOrigin
        );
        // Default port spelled out, host case differs: still the same origin.
        assert_eq!(
            classify_redirect(&from, &url("https://API.GitHub.com:443/other")),
            Hop::SameOrigin
        );
    }

    #[test]
    fn other_host_port_or_scheme_drops_the_credential() {
        let from = url("https://api.github.com/a");
        for to in [
            "https://objects.githubusercontent.com/blob?sig=1",
            "https://api.github.com:8443/a",
            "https://api.github.com.evil.example/a",
        ] {
            assert_eq!(classify_redirect(&from, &url(to)), Hop::DropCredential, "{to}");
        }
        // http -> https is an upgrade to another origin: followed, credential dropped.
        assert_eq!(
            classify_redirect(&url("http://ghe.test/a"), &url("https://ghe.test/a")),
            Hop::DropCredential
        );
    }

    #[test]
    fn https_to_http_is_refused() {
        let from = url("https://api.github.com/a");
        assert_eq!(
            classify_redirect(&from, &url("http://api.github.com/a")),
            Hop::Refuse
        );
        assert_eq!(
            classify_redirect(&from, &url("http://elsewhere.test/a")),
            Hop::Refuse
        );
    }

    #[derive(Default)]
    struct VecSink(Vec<u8>);

    impl AssetSink for VecSink {
        fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()> {
            self.0.extend_from_slice(chunk);
            Ok(())
        }
    }

    struct FailingSink;

    impl AssetSink for FailingSink {
        fn write_chunk(&mut self, _chunk: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("disk full"))
        }
    }

    /// Serves one canned response per accepted connection and records the
    /// request head it received.
    async fn serve(responses: Vec<String>) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                log.lock()
                    .expect("log")
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        (port, seen)
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(location: &str) -> String {
        format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    fn has_auth(request: &str) -> bool {
        request.to_ascii_lowercase().contains("\r\nauthorization:")
    }

    #[tokio::test]
    async fn credential_is_not_forwarded_when_the_redirect_changes_host() {
        // Storage "host": reached as `localhost`, while the API is `127.0.0.1`.
        let (storage_port, storage_seen) = serve(vec![ok("asset-bytes")]).await;
        let (api_port, api_seen) = serve(vec![redirect(&format!(
            "http://localhost:{storage_port}/blob?sig=abc"
        ))])
        .await;
        let client = GitHubClient::for_tests(Credential::Token("ghp_SECRET".into()));
        let mut sink = VecSink::default();

        let n = client
            .download_release_asset(&format!("http://127.0.0.1:{api_port}/asset/1"), &mut sink)
            .await
            .expect("download");

        assert_eq!(n, 11);
        assert_eq!(sink.0, b"asset-bytes");
        let api = api_seen.lock().expect("api");
        let storage = storage_seen.lock().expect("storage");
        assert!(has_auth(&api[0]), "the API hop carries the credential");
        assert!(api[0].contains("ghp_SECRET"));
        assert!(!has_auth(&storage[0]), "the storage hop must not: {}", storage[0]);
        assert!(!storage[0].contains("ghp_SECRET"));
        assert!(storage[0].starts_with("GET /blob?sig=abc "));
    }

    #[tokio::test]
    async fn credential_is_not_forwarded_when_only_the_port_changes() {
        let (second_port, second_seen) = serve(vec![ok("x")]).await;
        let (first_port, _) = serve(vec![redirect(&format!(
            "http://127.0.0.1:{second_port}/next"
        ))])
        .await;
        let client = GitHubClient::for_tests(Credential::Token("ghp_SECRET".into()));

        client
            .download_release_asset(
                &format!("http://127.0.0.1:{first_port}/a"),
                &mut VecSink::default(),
            )
            .await
            .expect("download");

        assert!(!has_auth(&second_seen.lock().expect("seen")[0]));
    }

    #[tokio::test]
    async fn credential_is_kept_for_a_same_origin_redirect_with_a_relative_location() {
        let (port, seen) = serve(vec![redirect("/final"), ok("done")]).await;
        let client = GitHubClient::for_tests(Credential::Token("ghp_SECRET".into()));
        let mut sink = VecSink::default();

        client
            .download_release_asset(&format!("http://127.0.0.1:{port}/start"), &mut sink)
            .await
            .expect("download");

        let seen = seen.lock().expect("seen");
        assert!(has_auth(&seen[0]) && has_auth(&seen[1]));
        assert!(seen[1].starts_with("GET /final "));
        assert_eq!(sink.0, b"done");
    }

    #[tokio::test]
    async fn too_many_redirects_is_an_error() {
        let (port, _) = serve(vec![redirect("/a"), redirect("/b"), redirect("/c"), redirect("/d")]).await;
        let client = GitHubClient::for_tests(Credential::Anonymous);

        let err = client
            .download_release_asset(&format!("http://127.0.0.1:{port}/start"), &mut VecSink::default())
            .await
            .expect_err("loop");

        assert!(err.to_string().contains("too many redirects"), "{err}");
    }

    #[tokio::test]
    async fn an_error_status_is_reported_and_nothing_is_written() {
        let (port, _) = serve(vec![
            "HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope".to_string(),
        ])
        .await;
        let client = GitHubClient::for_tests(Credential::Anonymous);
        let mut sink = VecSink::default();

        let err = client
            .download_release_asset(&format!("http://127.0.0.1:{port}/a"), &mut sink)
            .await
            .expect_err("404");

        assert!(matches!(err, ClientError::ApiError { status: 404, .. }), "{err}");
        assert!(sink.0.is_empty());
    }

    #[tokio::test]
    async fn a_failing_sink_aborts_the_download_with_an_io_error() {
        let (port, _) = serve(vec![ok("data")]).await;
        let client = GitHubClient::for_tests(Credential::Anonymous);

        let err = client
            .download_release_asset(&format!("http://127.0.0.1:{port}/a"), &mut FailingSink)
            .await
            .expect_err("sink fails");

        assert!(matches!(err, ClientError::Io(_)), "{err}");
    }

    #[tokio::test]
    async fn a_large_body_is_delivered_in_chunks() {
        let body = "z".repeat(3 * 1024 * 1024);
        let (port, _) = serve(vec![ok(&body)]).await;
        let client = GitHubClient::for_tests(Credential::Anonymous);

        #[derive(Default)]
        struct CountingSink {
            total: usize,
            chunks: usize,
            largest: usize,
        }
        impl AssetSink for CountingSink {
            fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()> {
                self.total += chunk.len();
                self.chunks += 1;
                self.largest = self.largest.max(chunk.len());
                Ok(())
            }
        }
        let mut sink = CountingSink::default();

        let n = client
            .download_release_asset(&format!("http://127.0.0.1:{port}/a"), &mut sink)
            .await
            .expect("download");

        assert_eq!(n as usize, body.len());
        assert_eq!(sink.total, body.len());
        assert!(sink.chunks > 1, "must arrive in several chunks, not one buffer");
        assert!(sink.largest < body.len());
    }
}
