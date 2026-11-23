// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Release asset download.
//!
//! GitHub answers an asset request with a redirect to a pre-signed URL on its
//! storage host.  That second request must not carry the API credential (see
//! [`super::super::redirect`]).

use http_body_util::BodyExt;
use hyper::body::Incoming;

use crate::api_client::AssetSink;
use crate::error::ClientError;

use super::super::{Attempts, Call, GitHubClient};

impl GitHubClient {
    /// Streams a release asset into `sink` and returns the number of bytes
    /// delivered.
    ///
    /// Uses the `application/octet-stream` accept header required by GitHub
    /// and follows up to 3 redirects.  The `Authorization` header is sent to
    /// `asset_url` and to redirects within the same origin only; a redirect
    /// to another host, port or scheme is followed without it, and a redirect
    /// from HTTPS to HTTP is refused.  Rate limits, 5xx responses and
    /// transport failures before the body starts are retried like any other
    /// GET.  The body is handed to the sink chunk by chunk (each chunk must
    /// arrive within the request timeout), so memory use does not depend on
    /// the asset size.
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
        let call = Call {
            leave_origin: true,
            ..Call::get(asset_url, &[("accept", "application/octet-stream")])
        };
        let response = self.send(&call, &mut Attempts::default()).await?;
        self.stream_body(response.into_body(), asset_url, sink)
            .await
    }

    /// Hands `body` to `sink` chunk by chunk (each chunk must arrive within
    /// the request timeout) and returns the number of bytes delivered.
    pub(super) async fn stream_body(
        &self,
        mut body: Incoming,
        url: &str,
        sink: &mut dyn AssetSink,
    ) -> Result<u64, ClientError> {
        let mut written = 0u64;
        loop {
            let frame = tokio::time::timeout(self.timing.request_timeout, body.frame())
                .await
                .map_err(|_| ClientError::Timeout {
                    url: url.to_string(),
                })?;
            let Some(frame) = frame else { break };
            if let Some(chunk) = frame?.data_ref() {
                sink.write_chunk(chunk)?;
                written += chunk.len() as u64;
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::config::Credential;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
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
        assert!(
            !has_auth(&storage[0]),
            "the storage hop must not: {}",
            storage[0]
        );
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
        let (port, _) = serve(vec![
            redirect("/a"),
            redirect("/b"),
            redirect("/c"),
            redirect("/d"),
        ])
        .await;
        let client = GitHubClient::for_tests(Credential::Anonymous);

        let err = client
            .download_release_asset(
                &format!("http://127.0.0.1:{port}/start"),
                &mut VecSink::default(),
            )
            .await
            .expect_err("loop");

        assert!(err.to_string().contains("too many redirects"), "{err}");
    }

    #[tokio::test]
    async fn an_error_status_is_reported_and_nothing_is_written() {
        let (port, _) = serve(vec![
            "HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope"
                .to_string(),
        ])
        .await;
        let client = GitHubClient::for_tests(Credential::Anonymous);
        let mut sink = VecSink::default();

        let err = client
            .download_release_asset(&format!("http://127.0.0.1:{port}/a"), &mut sink)
            .await
            .expect_err("404");

        assert!(
            matches!(err, ClientError::ApiError { status: 404, .. }),
            "{err}"
        );
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
        assert!(
            sink.chunks > 1,
            "must arrive in several chunks, not one buffer"
        );
        assert!(sink.largest < body.len());
    }

    #[tokio::test]
    async fn an_error_body_that_stalls_times_out_instead_of_hanging() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 8192];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 100\r\n\r\npartial")
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });
        let mut client = GitHubClient::for_tests(Credential::Anonymous);
        client.timing.request_timeout = std::time::Duration::from_millis(200);

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.download_release_asset(
                &format!("http://127.0.0.1:{port}/a"),
                &mut VecSink::default(),
            ),
        )
        .await
        .expect("a stalled error body must not hang the download");

        assert!(
            matches!(outcome, Err(ClientError::Timeout { .. })),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_rate_limited_asset_request_is_waited_out_and_retried() {
        let (port, seen) = serve(vec![
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
                .to_string(),
            ok("asset"),
        ])
        .await;
        let client = GitHubClient::for_tests(Credential::Anonymous);
        let mut sink = VecSink::default();

        client
            .download_release_asset(&format!("http://127.0.0.1:{port}/a"), &mut sink)
            .await
            .expect("retried after the rate limit");

        assert_eq!(sink.0, b"asset");
        assert_eq!(seen.lock().expect("seen").len(), 2);
    }

    #[tokio::test]
    async fn a_transient_server_error_on_an_asset_is_retried() {
        let (port, seen) = serve(vec![
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_string(),
            ok("asset"),
        ])
        .await;
        let client = GitHubClient::for_tests(Credential::Anonymous);
        let mut sink = VecSink::default();

        client
            .download_release_asset(&format!("http://127.0.0.1:{port}/a"), &mut sink)
            .await
            .expect("retried after the 503");

        assert_eq!(sink.0, b"asset");
        assert_eq!(seen.lock().expect("seen").len(), 2);
    }

    #[tokio::test]
    async fn the_storage_hop_gets_neither_the_token_nor_the_api_version() {
        let (storage_port, storage_seen) = serve(vec![ok("x")]).await;
        let (api_port, api_seen) = serve(vec![redirect(&format!(
            "http://localhost:{storage_port}/blob"
        ))])
        .await;
        let client = GitHubClient::for_tests(Credential::Token("ghp_SECRET".into()));

        client
            .download_release_asset(
                &format!("http://127.0.0.1:{api_port}/a"),
                &mut VecSink::default(),
            )
            .await
            .expect("download");

        let api = api_seen.lock().expect("api")[0].to_ascii_lowercase();
        let storage = storage_seen.lock().expect("storage")[0].to_ascii_lowercase();
        assert!(
            api.contains("\r\naccept: application/octet-stream\r\n"),
            "{api}"
        );
        assert!(api.contains("x-github-api-version"), "{api}");
        assert!(!storage.contains("x-github-api-version"), "{storage}");
    }
}
