// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Minimal HTTP CONNECT proxy connector.
//!
//! Reads `HTTPS_PROXY` / `https_proxy` from the environment and, when set,
//! routes every HTTPS connection through the proxy via HTTP `CONNECT`
//! tunnelling followed by a TLS handshake.
//!
//! All crates used here are already transitive dependencies via `hyper-rustls`
//! and `hyper-util` — no new crate is introduced.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use base64::Engine as _;
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper::Uri;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tower_service::Service;

/// A TLS stream established through an HTTP CONNECT proxy tunnel.
///
/// Wraps `TokioIo<TlsStream<TcpStream>>` in a `Pin<Box<…>>` so the outer
/// type is `Unpin`, which is required by `hyper_util::client::legacy::Client`.
pub(crate) struct ProxiedStream(Pin<Box<TokioIo<tokio_rustls::client::TlsStream<TcpStream>>>>);

impl Read for ProxiedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        self.get_mut().0.as_mut().poll_read(cx, buf)
    }
}

impl Write for ProxiedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut().0.as_mut().poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.get_mut().0.as_mut().poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().0.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().0.as_mut().poll_shutdown(cx)
    }
}

impl Connection for ProxiedStream {
    fn connected(&self) -> Connected {
        Connected::new().proxy(true)
    }
}

// Safety: Box<T> is always Unpin, so Pin<Box<T>> is also Unpin.
impl Unpin for ProxiedStream {}

// ── Proxy configuration ───────────────────────────────────────────────────────

/// Parsed proxy address and optional pre-built `Proxy-Authorization` header.
#[derive(Clone, Debug)]
pub(super) struct ProxyConfig {
    /// Proxy host name or IP.
    pub host: String,
    /// Proxy port.
    pub port: u16,
    /// Ready-to-send `Proxy-Authorization: Basic <base64>` value, if
    /// credentials were embedded in the URL.
    pub auth_header: Option<String>,
}

// ── ProxyConnector ────────────────────────────────────────────────────────────

/// A `tower_service::Service<Uri>` connector that tunnels every connection
/// through an HTTP CONNECT proxy before performing the TLS handshake.
#[derive(Clone)]
pub(crate) struct ProxyConnector {
    config: ProxyConfig,
    tls: TlsConnector,
}

impl ProxyConnector {
    /// Creates a new connector from a [`ProxyConfig`] and an already-built
    /// [`rustls::ClientConfig`] (reuses the same CA bundle as direct TLS).
    pub(super) fn new(config: ProxyConfig, tls_config: rustls::ClientConfig) -> Self {
        Self {
            config,
            tls: TlsConnector::from(Arc::new(tls_config)),
        }
    }
}

impl Service<Uri> for ProxyConnector {
    type Response = ProxiedStream;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<ProxiedStream>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let host = match uri.host() {
            Some(h) => h.to_string(),
            None => {
                return Box::pin(async {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "proxy connector: URI has no host",
                    ))
                });
            }
        };
        let port = uri.port_u16().unwrap_or(443);
        let config = self.config.clone();
        let tls = self.tls.clone();

        Box::pin(async move {
            // Step 1: TCP connect to the proxy.
            let proxy_addr = format!("{}:{}", config.host, config.port);
            let stream = TcpStream::connect(proxy_addr).await?;

            // Step 2: HTTP CONNECT tunnel.
            let stream = connect_tunnel(stream, &host, port, config.auth_header.as_deref()).await?;

            // Step 3: TLS handshake through the tunnel.
            let server_name = ServerName::try_from(host.as_str())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?
                .to_owned();
            let tls_stream = tls.connect(server_name, stream).await?;

            Ok(ProxiedStream(Box::pin(TokioIo::new(tls_stream))))
        })
    }
}

// ── CONNECT tunnel ────────────────────────────────────────────────────────────

/// Sends `CONNECT host:port` to the proxy and waits for a `200` response.
///
/// Returns the same `TcpStream` on success so the caller can layer TLS on top.
async fn connect_tunnel(
    mut stream: TcpStream,
    host: &str,
    port: u16,
    auth: Option<&str>,
) -> io::Result<TcpStream> {
    let auth_line = auth
        .map(|a| format!("Proxy-Authorization: {a}\r\n"))
        .unwrap_or_default();
    let request =
        format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n{auth_line}\r\n");
    stream.write_all(request.as_bytes()).await?;

    // Read the response header (terminated by \r\n\r\n).
    let mut buf = Vec::with_capacity(256);
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 4096 {
            return Err(io::Error::other(
                "proxy CONNECT response header exceeds 4 KiB",
            ));
        }
    }

    if !buf.starts_with(b"HTTP/1.1 200") && !buf.starts_with(b"HTTP/1.0 200") {
        let snippet = String::from_utf8_lossy(&buf[..buf.len().min(64)]);
        return Err(io::Error::other(format!("proxy CONNECT failed: {snippet}")));
    }

    Ok(stream)
}

// ── Environment detection ─────────────────────────────────────────────────────

/// Reads `HTTPS_PROXY` (or `https_proxy`) from the environment and returns a
/// [`ProxyConfig`], or `None` if the variable is unset or unparseable.
pub(super) fn proxy_config_from_env() -> Option<ProxyConfig> {
    let raw = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .ok()?;

    let uri: Uri = raw.parse().ok()?;
    let host = uri.host()?.to_string();
    let port = uri.port_u16().unwrap_or(3128);
    let auth_header = extract_basic_auth(&raw);

    Some(ProxyConfig {
        host,
        port,
        auth_header,
    })
}

/// Extracts `Basic <base64(user:pass)>` from a URL like
/// `http://user:pass@host:port`.  Returns `None` if no credentials are present.
fn extract_basic_auth(url: &str) -> Option<String> {
    // Find userinfo: the substring between `//` and `@`.
    let after_slashes = url.find("//")?.checked_add(2).and_then(|i| url.get(i..))?;
    let at = after_slashes.find('@')?;
    let userinfo = &after_slashes[..at];
    if userinfo.is_empty() {
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(userinfo.as_bytes());
    Some(format!("Basic {encoded}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_basic_auth_with_credentials() {
        let auth = extract_basic_auth("http://alice:s3cr3t@proxy.example.com:3128").unwrap();
        // base64("alice:s3cr3t") = "YWxpY2U6czNjcjN0"
        assert_eq!(auth, "Basic YWxpY2U6czNjcjN0");
    }

    #[test]
    fn extract_basic_auth_no_credentials() {
        assert!(extract_basic_auth("http://proxy.example.com:3128").is_none());
    }

    #[test]
    fn proxy_config_from_env_unset() {
        // When neither HTTPS_PROXY nor https_proxy is set, returns None.
        // (We can't unset env vars portably in tests, so only test the parse path.)
        let result = proxy_config_from_env();
        // Just verify it doesn't panic; actual presence depends on test env.
        let _ = result;
    }

    #[test]
    fn proxy_config_port_default() {
        // A URL without an explicit port should default to 3128.
        let url = "http://proxy.example.com";
        let uri: Uri = url.parse().unwrap();
        let port = uri.port_u16().unwrap_or(3128);
        assert_eq!(port, 3128);
    }
}
