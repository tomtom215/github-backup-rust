// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Downloads of the files people attach to issues, pull requests, comments
//! and release notes.
//!
//! Attachments are not API resources: their URLs point at GitHub's web host
//! (`https://github.com/user-attachments/...`, `/<owner>/<repo>/files/...`)
//! or at its content hosts (`user-images.githubusercontent.com`), and GitHub
//! answers a web-host URL with a redirect to a signed storage URL.  Since
//! May 2023 an attachment of a private repository needs authentication
//! (<https://github.blog/changelog/2023-05-08-more-secure-private-attachments>).
//!
//! The credential is sent as HTTP Basic authentication
//! (`<token>:x-oauth-basic`), which is what python-github-backup sends for
//! a classic token and reports to work; it goes to `https://github.com`
//! only, and only when this client talks to github.com's API: a GitHub
//! Enterprise Server token is never sent to github.com.  Every other host,
//! and every hop of a redirect that leaves the origin, gets no credential.

use hyper::header::CONTENT_TYPE;
use url::Url;

use crate::api_client::{AssetSink, AttachmentDownload};
use crate::error::ClientError;

use super::super::{Attempts, Call, GitHubClient, OriginHeaders, GITHUB_API_BASE};

impl GitHubClient {
    /// Streams the attachment at `url` into `sink`.
    ///
    /// Follows up to 3 redirects; one that leaves the origin is followed
    /// without the credential, and HTTPS to HTTP is refused.  Rate limits,
    /// 5xx responses and transport failures before the body starts are
    /// retried like any other GET.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS or HTTP errors, and
    /// [`ClientError::Io`] when the sink fails.
    pub async fn download_attachment(
        &self,
        url: &str,
        sink: &mut dyn AssetSink,
    ) -> Result<AttachmentDownload, ClientError> {
        self.fetch_attachment(url, self.attachment_origin(url), sink)
            .await
    }

    /// What the first hop to `url` is sent: the credential for
    /// `https://github.com` when this client is github.com's, nothing
    /// otherwise.
    fn attachment_origin(&self, url: &str) -> OriginHeaders {
        let github_web = Url::parse(url).is_ok_and(|u| {
            u.scheme() == "https"
                && u.host_str() == Some("github.com")
                && u.port().is_none()
                && u.username().is_empty()
                && u.password().is_none()
        });
        if github_web && self.api_base == GITHUB_API_BASE {
            OriginHeaders::WebBasic
        } else {
            OriginHeaders::None
        }
    }

    async fn fetch_attachment(
        &self,
        url: &str,
        origin: OriginHeaders,
        sink: &mut dyn AssetSink,
    ) -> Result<AttachmentDownload, ClientError> {
        let call = Call {
            leave_origin: true,
            origin,
            ..Call::get(url, &[("accept", "*/*")])
        };
        let response = self.send(&call, &mut Attempts::default()).await?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty());
        let bytes = self.stream_body(response.into_body(), url, sink).await?;
        Ok(AttachmentDownload {
            bytes,
            content_type,
        })
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

    fn redirect(location: &str) -> String {
        format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    fn png(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/PNG; charset=binary\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn authorization(request: &str) -> Option<String> {
        request
            .split("\r\n")
            .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
            .map(|line| line["authorization:".len()..].trim().to_string())
    }

    const SECRET: &str = "ghp_SECRET";

    #[tokio::test]
    async fn the_web_host_gets_basic_auth_and_the_storage_host_nothing() {
        let (storage_port, storage_seen) = serve(vec![png("img")]).await;
        let (web_port, web_seen) = serve(vec![redirect(&format!(
            "http://localhost:{storage_port}/blob?X-Amz-Signature=abc"
        ))])
        .await;
        let client = GitHubClient::for_tests(Credential::Token(SECRET.into()));
        let mut sink = VecSink::default();

        let got = client
            .fetch_attachment(
                &format!("http://127.0.0.1:{web_port}/user-attachments/assets/u"),
                OriginHeaders::WebBasic,
                &mut sink,
            )
            .await
            .expect("download");

        assert_eq!(got.bytes, 3);
        assert_eq!(got.content_type.as_deref(), Some("image/png"));
        assert_eq!(sink.0, b"img");
        let web = &web_seen.lock().expect("web")[0];
        let storage = &storage_seen.lock().expect("storage")[0];
        // base64("ghp_SECRET:x-oauth-basic")
        assert_eq!(
            authorization(web).as_deref(),
            Some("Basic Z2hwX1NFQ1JFVDp4LW9hdXRoLWJhc2lj")
        );
        assert!(
            !web.to_ascii_lowercase().contains("x-github-api-version"),
            "{web}"
        );
        assert_eq!(authorization(storage), None, "{storage}");
        assert!(!storage.contains(SECRET) && !storage.contains("Z2hwX1NFQ1JFVD"));
    }

    #[tokio::test]
    async fn a_host_that_is_not_github_gets_no_credential() {
        let (port, seen) = serve(vec![png("x")]).await;
        let client = GitHubClient::for_tests(Credential::Token(SECRET.into()));

        client
            .fetch_attachment(
                &format!("http://127.0.0.1:{port}/a.png"),
                OriginHeaders::None,
                &mut VecSink::default(),
            )
            .await
            .expect("download");

        let seen = &seen.lock().expect("seen")[0];
        assert_eq!(authorization(seen), None, "{seen}");
        assert!(!seen.to_ascii_lowercase().contains("x-github-api-version"));
    }

    #[test]
    fn only_https_github_com_of_a_github_com_client_gets_the_credential() {
        let client = GitHubClient::for_tests_at(Credential::Token(SECRET.into()), GITHUB_API_BASE);
        for url in [
            "https://github.com/user-attachments/assets/0f1e2d3c",
            "https://github.com/o/r/files/1/a.txt",
            "https://GitHub.com:443/user-attachments/files/1/a.txt",
        ] {
            assert_eq!(
                client.attachment_origin(url),
                OriginHeaders::WebBasic,
                "{url}"
            );
        }
        for url in [
            "http://github.com/user-attachments/assets/x",
            "https://github.com:8443/user-attachments/assets/x",
            "https://user-images.githubusercontent.com/1/a.png",
            "https://private-user-images.githubusercontent.com/1/a.png?jwt=x",
            "https://github.com.evil.example/user-attachments/assets/x",
            "https://user:pw@github.com/user-attachments/assets/x",
            "not a url",
        ] {
            assert_eq!(client.attachment_origin(url), OriginHeaders::None, "{url}");
        }

        // A GitHub Enterprise Server token is never sent to github.com.
        let ghes = GitHubClient::for_tests_at(
            Credential::Token(SECRET.into()),
            "https://ghe.example.com/api/v3",
        );
        assert_eq!(
            ghes.attachment_origin("https://github.com/user-attachments/assets/x"),
            OriginHeaders::None
        );
    }

    #[tokio::test]
    async fn a_missing_attachment_is_an_api_error_with_its_status() {
        let (port, _) = serve(vec![
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ])
        .await;
        let client = GitHubClient::for_tests(Credential::Anonymous);

        let err = client
            .fetch_attachment(
                &format!("http://127.0.0.1:{port}/gone"),
                OriginHeaders::WebBasic,
                &mut VecSink::default(),
            )
            .await
            .expect_err("404");

        assert!(
            matches!(err, ClientError::ApiError { status: 404, .. }),
            "{err}"
        );
    }
}
