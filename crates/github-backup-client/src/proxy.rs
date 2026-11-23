// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Proxy support shared by everything in the backup that talks HTTP(S): the
//! GitHub API client, the OAuth device flow, the webhook notifier,
//! `--doctor`, the mirror destinations' APIs and the S3 client.
//!
//! [`ProxySettings`] reads the conventional environment variables
//! (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY`, each in lower and
//! upper case; the lower-case spelling wins, as in curl), [`ProxyConnector`]
//! is a connector for hyper-util's legacy client that obeys them, and
//! [`ProxyClient`] is that client, for any request body type:
//!
//! * an `https://` target goes through the proxy with an HTTP `CONNECT` tunnel
//!   and a TLS handshake inside it;
//! * an `http://` target is sent to the proxy in absolute form, with the
//!   proxy credentials in `Proxy-Authorization`;
//! * a target matching `NO_PROXY` (or with no proxy configured for its scheme)
//!   is contacted directly.
//!
//! Only HTTP proxies are supported (`http://[user:pass@]host[:port]`, or a bare
//! `host:port`); SOCKS and TLS-to-the-proxy are not.  The proxy credentials
//! never appear in `Debug` output, log lines or error messages.

use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::{Body, Incoming};
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper::{Request, Response, Uri};
use hyper_util::client::legacy::connect::{Connected, Connection, HttpConnector};
use hyper_util::client::legacy::{Builder, Client};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tower_service::Service;
use tracing::{info, warn};
use url::Url;

use crate::error::ClientError;

// ── Settings ──────────────────────────────────────────────────────────────────

/// One proxy: where it listens and the optional `Proxy-Authorization` value.
///
/// Its `Debug` output redacts the credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyConfig {
    /// Proxy host name or IP.
    pub host: String,
    /// Proxy port.
    pub port: u16,
    /// Ready-to-send `Proxy-Authorization: Basic <base64>` value, if the proxy
    /// URL carried credentials.
    pub auth_header: Option<String>,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field(
                "auth_header",
                &self.auth_header.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// The proxy configuration of the process environment.
#[derive(Clone, Debug, Default)]
pub struct ProxySettings {
    https: Option<ProxyConfig>,
    http: Option<ProxyConfig>,
    no_proxy: Vec<NoProxyRule>,
}

impl ProxySettings {
    /// Reads `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY`.
    #[must_use]
    pub fn from_env() -> Self {
        let settings = Self::from_lookup(|name| std::env::var(name).ok());
        if settings.is_configured() {
            info!("HTTP proxy configured from the environment");
        }
        settings
    }

    /// Like [`from_env`](Self::from_env) with an explicit variable lookup
    /// (names are passed upper-case; the lower-case spelling is tried first).
    #[must_use]
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let get = |name: &str| {
            lookup(&name.to_ascii_lowercase())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| lookup(name).filter(|v| !v.trim().is_empty()))
        };
        let parse = |name: &str| {
            let raw = get(name)?;
            let parsed = parse_proxy_url(&raw);
            if parsed.is_none() {
                warn!("{name} is set but is not a usable http://host:port proxy URL; ignoring it");
            }
            parsed
        };
        let all = parse("ALL_PROXY");
        Self {
            https: parse("HTTPS_PROXY").or_else(|| all.clone()),
            http: parse("HTTP_PROXY").or(all),
            no_proxy: get("NO_PROXY")
                .map(|v| v.split(',').filter_map(NoProxyRule::parse).collect())
                .unwrap_or_default(),
        }
    }

    /// `true` if any proxy is configured at all.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.https.is_some() || self.http.is_some()
    }

    /// The proxy to use for `uri`, or `None` for a direct connection.
    #[must_use]
    pub fn proxy_for(&self, uri: &Uri) -> Option<&ProxyConfig> {
        let host = uri.host()?;
        let secure = uri.scheme_str() == Some("https");
        let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
        if self.no_proxy.iter().any(|rule| rule.matches(host, port)) {
            return None;
        }
        if secure {
            self.https.as_ref()
        } else {
            self.http.as_ref()
        }
    }
}

/// Parses `http://[user[:pass]@]host[:port]` (or `host[:port]`); the port
/// defaults to 3128.
fn parse_proxy_url(raw: &str) -> Option<ProxyConfig> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let url = Url::parse(&with_scheme).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    let host = url
        .host_str()?
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    let auth_header = (!url.username().is_empty()).then(|| {
        let userinfo = format!(
            "{}:{}",
            percent_decode(url.username()),
            percent_decode(url.password().unwrap_or(""))
        );
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(userinfo)
        )
    });
    Some(ProxyConfig {
        host,
        port: url.port().unwrap_or(3128),
        auth_header,
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One entry of `NO_PROXY`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum NoProxyRule {
    /// `*`: nothing goes through the proxy.
    All,
    /// A host name (matches itself and its subdomains), optionally with a port.
    Domain { name: String, port: Option<u16> },
    /// An IP address, optionally with a port.
    Ip { addr: IpAddr, port: Option<u16> },
    /// A CIDR block such as `10.0.0.0/8`.
    Cidr { net: IpAddr, prefix: u8 },
}

impl NoProxyRule {
    fn parse(entry: &str) -> Option<Self> {
        let entry = entry.trim().to_ascii_lowercase();
        if entry.is_empty() {
            return None;
        }
        if entry == "*" {
            return Some(Self::All);
        }
        if let Some((net, prefix)) = entry.split_once('/') {
            let net: IpAddr = net.parse().ok()?;
            let prefix: u8 = prefix.parse().ok()?;
            let max = if net.is_ipv4() { 32 } else { 128 };
            return (prefix <= max).then_some(Self::Cidr { net, prefix });
        }
        if let Ok(addr) = entry
            .trim_matches(|c| c == '[' || c == ']')
            .parse::<IpAddr>()
        {
            return Some(Self::Ip { addr, port: None });
        }
        // `host:port`, `[v6]:port` or a bare host.
        let (host, port) = match entry.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') || h.ends_with(']') => (h, p.parse::<u16>().ok()),
            _ => (entry.as_str(), None),
        };
        let host = host.trim_matches(|c| c == '[' || c == ']');
        if let Ok(addr) = host.parse::<IpAddr>() {
            return Some(Self::Ip { addr, port });
        }
        let name = host.trim_start_matches("*.").trim_start_matches('.');
        (!name.is_empty()).then(|| Self::Domain {
            name: name.to_string(),
            port,
        })
    }

    fn matches(&self, host: &str, port: u16) -> bool {
        let host = host
            .trim_matches(|c| c == '[' || c == ']')
            .to_ascii_lowercase();
        let parsed: Option<IpAddr> = host.parse().ok();
        match self {
            Self::All => true,
            Self::Domain { name, port: p } => {
                p.is_none_or(|p| p == port)
                    && (host == *name || host.ends_with(&format!(".{name}")))
            }
            Self::Ip { addr, port: p } => p.is_none_or(|p| p == port) && parsed == Some(*addr),
            Self::Cidr { net, prefix } => parsed.is_some_and(|ip| in_cidr(ip, *net, *prefix)),
        }
    }
}

fn in_cidr(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    let (a, b): (Vec<u8>, Vec<u8>) = match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(b)) => (a.octets().to_vec(), b.octets().to_vec()),
        (IpAddr::V6(a), IpAddr::V6(b)) => (a.octets().to_vec(), b.octets().to_vec()),
        _ => return false,
    };
    let mut bits = usize::from(prefix);
    for (x, y) in a.iter().zip(&b) {
        let take = bits.min(8);
        if take == 0 {
            break;
        }
        let mask = 0xffu8 << (8 - take);
        if x & mask != y & mask {
            return false;
        }
        bits -= take;
    }
    true
}

// ── Connections ───────────────────────────────────────────────────────────────

/// A connection made by [`ProxyConnector`]: plain TCP, or TLS (direct or
/// inside a `CONNECT` tunnel).
pub struct ProxyStream {
    inner: Inner,
    via_proxy: bool,
}

enum Inner {
    Plain(TokioIo<TcpStream>),
    Tls(Box<TokioIo<tokio_rustls::client::TlsStream<TcpStream>>>),
}

impl std::fmt::Debug for ProxyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyStream")
            .field("via_proxy", &self.via_proxy)
            .finish_non_exhaustive()
    }
}

impl Read for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.inner {
            Inner::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Inner::Tls(s) => Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

impl Write for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.inner {
            Inner::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Inner::Tls(s) => Pin::new(&mut **s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.inner {
            Inner::Plain(s) => Pin::new(s).poll_flush(cx),
            Inner::Tls(s) => Pin::new(&mut **s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.inner {
            Inner::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Inner::Tls(s) => Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}

impl Connection for ProxyStream {
    fn connected(&self) -> Connected {
        // A plain-HTTP connection to a proxy makes hyper send the request in
        // absolute form; a tunnel carries ordinary origin-form requests.
        Connected::new().proxy(self.via_proxy && matches!(self.inner, Inner::Plain(_)))
    }
}

/// A connector for hyper-util's legacy [`Client`] that applies
/// [`ProxySettings`]: TCP (hyper-util's [`HttpConnector`]) to the proxy or to
/// the target, a `CONNECT` tunnel for an `https://` target behind a proxy,
/// then TLS (rustls) for `https://`.
///
/// Use it through [`ProxyClient`], which also adds the proxy credentials to
/// plain-HTTP requests sent to a proxy.
#[derive(Clone)]
pub struct ProxyConnector {
    settings: Arc<ProxySettings>,
    tls: TlsConnector,
    tcp: HttpConnector,
    allow_http: bool,
}

impl std::fmt::Debug for ProxyConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConnector")
            .field("settings", &self.settings)
            .field("allow_http", &self.allow_http)
            .finish_non_exhaustive()
    }
}

impl ProxyConnector {
    /// A connector applying `settings` and trusting what `tls` trusts.
    /// `allow_http` lets it reach `http://` URLs; without it only `https://`
    /// is accepted.
    #[must_use]
    pub fn new(settings: ProxySettings, tls: rustls::ClientConfig, allow_http: bool) -> Self {
        let mut tcp = HttpConnector::new();
        // `call` checks the scheme; the TCP connector only connects.
        tcp.enforce_http(false);
        tcp.set_nodelay(true);
        Self {
            settings: Arc::new(settings),
            tls: TlsConnector::from(Arc::new(tls)),
            tcp,
            allow_http,
        }
    }

    /// Limits the time to establish a TCP connection (to the proxy or to the
    /// target); `None`, the default, leaves it to the operating system.
    pub fn set_connect_timeout(&mut self, timeout: Option<Duration>) {
        self.tcp.set_connect_timeout(timeout);
    }

    /// Sends TCP keep-alive probes after `interval` of idleness (`None`, the
    /// default, sends none).
    pub fn set_keepalive(&mut self, interval: Option<Duration>) {
        self.tcp.set_keepalive(interval);
    }

    /// The settings this connector applies.
    #[must_use]
    pub fn settings(&self) -> &ProxySettings {
        &self.settings
    }
}

impl Service<Uri> for ProxyConnector {
    type Response = ProxyStream;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<ProxyStream>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let host = uri
                .host()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "URI has no host"))?
                .trim_matches(|c| c == '[' || c == ']')
                .to_string();
            let secure = match uri.scheme_str() {
                Some("https") => true,
                Some("http") if this.allow_http => false,
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unsupported URL scheme {other:?}"),
                    ))
                }
            };
            let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
            let proxy = this.settings.proxy_for(&uri).cloned();

            let peer = match &proxy {
                Some(p) => authority(&p.host, p.port),
                None => authority(&host, port),
            };
            let peer: Uri = format!("http://{peer}")
                .parse()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let mut tcp = this.tcp.clone();
            let tcp = tcp.call(peer).await.map_err(io::Error::other)?.into_inner();
            let tcp = match (&proxy, secure) {
                (Some(p), true) => {
                    connect_tunnel(tcp, &host, port, p.auth_header.as_deref()).await?
                }
                _ => tcp,
            };
            let inner = if secure {
                let name = ServerName::try_from(host.clone())
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
                Inner::Tls(Box::new(TokioIo::new(this.tls.connect(name, tcp).await?)))
            } else {
                Inner::Plain(TokioIo::new(tcp))
            };
            Ok(ProxyStream {
                inner,
                via_proxy: proxy.is_some(),
            })
        })
    }
}

/// `host:port`, with an IPv6 literal bracketed (RFC 3986, section 3.2.2).
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Sends `CONNECT host:port` to the proxy and waits for a `2xx` response.
async fn connect_tunnel(
    mut stream: TcpStream,
    host: &str,
    port: u16,
    auth: Option<&str>,
) -> io::Result<TcpStream> {
    let auth_line = auth
        .map(|a| format!("Proxy-Authorization: {a}\r\n"))
        .unwrap_or_default();
    let authority = authority(host, port);
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{auth_line}\r\n");
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

    // Any 2xx status establishes the tunnel (RFC 9110, section 9.3.6).
    let established = buf.starts_with(b"HTTP/1.")
        && buf.get(8) == Some(&b' ')
        && buf.get(9) == Some(&b'2')
        && buf
            .get(10..12)
            .is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
        && matches!(buf.get(12), Some(b' ' | b'\r'));
    if !established {
        let line = String::from_utf8_lossy(&buf)
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        return Err(io::Error::other(format!("proxy CONNECT failed: {line}")));
    }
    Ok(stream)
}

/// A [`rustls::ClientConfig`] trusting the operating system's certificate
/// store (or `SSL_CERT_FILE` / `SSL_CERT_DIR`).
///
/// # Errors
///
/// [`ClientError::Tls`] if no CA certificate can be loaded.
pub fn native_tls_config() -> Result<rustls::ClientConfig, ClientError> {
    let mut root_store = rustls::RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    if cert_result.certs.is_empty() {
        let msg = cert_result
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no CA certificates found".to_string());
        return Err(ClientError::Tls(msg));
    }
    root_store.add_parsable_certificates(cert_result.certs);
    Ok(rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth())
}

// ── Client ────────────────────────────────────────────────────────────────────

/// An HTTP/1.1 client that honours the proxy environment (see the module
/// docs), for requests whose body is a `B`: by default a whole
/// [`Full<Bytes>`]; the S3 client streams its own body type.
pub struct ProxyClient<B = Full<Bytes>> {
    client: Client<ProxyConnector, B>,
    settings: Arc<ProxySettings>,
}

impl<B> Clone for ProxyClient<B> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            settings: Arc::clone(&self.settings),
        }
    }
}

impl<B> std::fmt::Debug for ProxyClient<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyClient")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl<B> ProxyClient<B>
where
    B: Body + Send + Unpin + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    /// A client following the process environment, trusting the system CA
    /// bundle.  `allow_http` lets it reach `http://` URLs; without it only
    /// `https://` is accepted (what the GitHub API needs).
    ///
    /// # Errors
    ///
    /// [`ClientError::Tls`] if no CA certificates can be loaded.
    pub fn from_env(allow_http: bool) -> Result<Self, ClientError> {
        Ok(Self::new(
            ProxySettings::from_env(),
            native_tls_config()?,
            allow_http,
        ))
    }

    /// A client with explicit settings and TLS configuration.
    #[must_use]
    pub fn new(settings: ProxySettings, tls: rustls::ClientConfig, allow_http: bool) -> Self {
        Self::with_connector(
            &Client::builder(TokioExecutor::new()),
            ProxyConnector::new(settings, tls, allow_http),
        )
    }

    /// A client made by `builder` (connection-pool settings) on `connector`
    /// (connection settings).
    #[must_use]
    pub fn with_connector(builder: &Builder, connector: ProxyConnector) -> Self {
        Self {
            settings: Arc::clone(&connector.settings),
            client: builder.build(connector),
        }
    }

    /// A client for tests against local plain-HTTP servers: no proxy, an
    /// empty CA store, `http://` allowed.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn for_tests() -> Self {
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        Self::new(ProxySettings::default(), tls, true)
    }

    /// The settings this client applies.
    #[must_use]
    pub fn settings(&self) -> &ProxySettings {
        &self.settings
    }

    /// Sends a request, through the proxy when the settings call for it.
    ///
    /// # Errors
    ///
    /// The transport error of the underlying client.
    pub async fn request(
        &self,
        mut req: Request<B>,
    ) -> Result<Response<Incoming>, hyper_util::client::legacy::Error> {
        // A plain-HTTP request is handed to the proxy itself, so the proxy
        // credentials travel in the request.
        if req.uri().scheme_str() == Some("http") {
            if let Some(auth) = self
                .settings
                .proxy_for(req.uri())
                .and_then(|p| p.auth_header.as_deref())
                .and_then(|a| hyper::header::HeaderValue::from_str(a).ok())
            {
                req.headers_mut()
                    .insert(hyper::header::PROXY_AUTHORIZATION, auth);
            }
        }
        self.client.request(req).await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn settings(vars: &[(&str, &str)]) -> ProxySettings {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        ProxySettings::from_lookup(|name| vars.get(name).cloned())
    }

    fn proxy_for<'a>(s: &'a ProxySettings, url: &str) -> Option<&'a str> {
        s.proxy_for(&url.parse().unwrap()).map(|p| p.host.as_str())
    }

    #[test]
    fn variables_are_read_in_either_case_and_lower_case_wins() {
        let s = settings(&[("HTTPS_PROXY", "http://upper:1")]);
        assert_eq!(proxy_for(&s, "https://api.github.com"), Some("upper"));
        let s = settings(&[("https_proxy", "http://lower:1")]);
        assert_eq!(proxy_for(&s, "https://api.github.com"), Some("lower"));
        let s = settings(&[
            ("HTTPS_PROXY", "http://upper:1"),
            ("https_proxy", "http://lower:1"),
        ]);
        assert_eq!(proxy_for(&s, "https://api.github.com"), Some("lower"));
        let s = settings(&[("http_proxy", "http://p:1"), ("NO_PROXY", "")]);
        assert_eq!(proxy_for(&s, "http://hook.example"), Some("p"));
        assert_eq!(proxy_for(&s, "https://api.github.com"), None);
    }

    #[test]
    fn all_proxy_is_the_fallback_for_both_schemes() {
        let s = settings(&[("ALL_PROXY", "all:8080"), ("HTTPS_PROXY", "http://sec:1")]);
        assert_eq!(proxy_for(&s, "https://x.example"), Some("sec"));
        assert_eq!(proxy_for(&s, "http://x.example"), Some("all"));
        assert_eq!(s.http.as_ref().unwrap().port, 8080);
    }

    #[test]
    fn nothing_configured_or_unusable_means_direct() {
        assert!(!settings(&[]).is_configured());
        assert!(!settings(&[("HTTPS_PROXY", "")]).is_configured());
        assert!(!settings(&[("HTTPS_PROXY", "socks5://h:1")]).is_configured());
    }

    #[test]
    fn credentials_become_a_basic_proxy_authorization_and_port_defaults() {
        let s = settings(&[("HTTPS_PROXY", "http://alice:s3cr3t@proxy.example.com")]);
        let p = s.https.unwrap();
        assert_eq!(p.port, 3128);
        // base64("alice:s3cr3t")
        assert_eq!(p.auth_header.as_deref(), Some("Basic YWxpY2U6czNjcjN0"));
        // Percent-encoded characters are decoded: base64("bob:p@ss")
        let p = parse_proxy_url("http://bob:p%40ss@h:1").unwrap();
        assert_eq!(p.auth_header.as_deref(), Some("Basic Ym9iOnBAc3M="));
        assert!(parse_proxy_url("http://proxy.example.com:3128")
            .unwrap()
            .auth_header
            .is_none());
    }

    #[test]
    fn no_proxy_matches_hosts_suffixes_ports_ips_and_cidrs() {
        let s = settings(&[
            ("HTTPS_PROXY", "http://p:1"),
            (
                "no_proxy",
                "localhost, .corp.example,git.internal:8443,10.0.0.0/8,192.168.1.5,[::1]",
            ),
        ]);
        let via = |u: &str| proxy_for(&s, u).is_some();
        assert!(!via("https://localhost/x"));
        assert!(
            !via("https://corp.example/x"),
            "a bare domain entry also covers the domain"
        );
        assert!(!via("https://ghe.corp.example/api/v3"));
        assert!(
            via("https://notcorp.example/x"),
            "suffix match is on label boundaries"
        );
        assert!(!via("https://git.internal:8443/x"));
        assert!(via("https://git.internal/x"), "port-specific entry");
        assert!(!via("https://10.20.30.40/x"));
        assert!(via("https://11.0.0.1/x"));
        assert!(!via("https://192.168.1.5/x"));
        assert!(via("https://192.168.1.6/x"));
        assert!(!via("https://[::1]/x"));
        assert!(via("https://api.github.com/x"));
    }

    #[test]
    fn no_proxy_star_disables_the_proxy() {
        let s = settings(&[("HTTPS_PROXY", "http://p:1"), ("NO_PROXY", "*")]);
        assert_eq!(proxy_for(&s, "https://api.github.com"), None);
    }

    #[test]
    fn cidr_prefixes_that_are_not_byte_aligned_work() {
        let net: IpAddr = "172.16.0.0".parse().unwrap();
        assert!(in_cidr("172.31.255.255".parse().unwrap(), net, 12));
        assert!(!in_cidr("172.32.0.0".parse().unwrap(), net, 12));
        assert!(in_cidr(
            "8.8.8.8".parse().unwrap(),
            "0.0.0.0".parse().unwrap(),
            0
        ));
        assert!(
            !in_cidr("::1".parse().unwrap(), net, 12),
            "families do not mix"
        );
    }

    // ── Through a fake proxy ──────────────────────────────────────────────

    use tokio::net::TcpListener;

    /// Accepts connections, records each request head, answers
    /// `reply_to_connect` for a CONNECT and a fixed 200 otherwise.
    async fn fake_server(
        reply_to_connect: &'static str,
    ) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let heads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = Arc::clone(&heads);
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let connect = head.starts_with("CONNECT");
                    log.lock().unwrap().push(head);
                    let out = if connect {
                        reply_to_connect.to_string()
                    } else {
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into()
                    };
                    let _ = s.write_all(out.as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        (port, heads)
    }

    fn client(vars: &[(&str, String)]) -> ProxyClient {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        ProxyClient::new(
            ProxySettings::from_lookup(|n| vars.get(n).cloned()),
            tls,
            true,
        )
    }

    fn get(url: &str) -> Request<Full<Bytes>> {
        Request::get(url).body(Full::new(Bytes::new())).unwrap()
    }

    #[tokio::test]
    async fn http_requests_go_to_the_proxy_in_absolute_form_with_credentials() {
        let (proxy_port, heads) = fake_server("").await;
        let c = client(&[("HTTP_PROXY", format!("http://u:p@127.0.0.1:{proxy_port}"))]);
        let resp = c
            .request(get("http://hook.invalid:9/path?q=1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let head = heads.lock().unwrap()[0].to_ascii_lowercase();
        assert!(
            head.starts_with("get http://hook.invalid:9/path?q=1 http/1.1"),
            "{head}"
        );
        assert!(head.contains("proxy-authorization: basic dtpw"), "{head}");
    }

    #[tokio::test]
    async fn no_proxy_hosts_are_contacted_directly() {
        let (proxy_port, proxy_heads) = fake_server("").await;
        let (origin_port, origin_heads) = fake_server("").await;
        let c = client(&[
            ("HTTP_PROXY", format!("http://127.0.0.1:{proxy_port}")),
            ("NO_PROXY", "127.0.0.1".to_string()),
        ]);
        let resp = c
            .request(get(&format!("http://127.0.0.1:{origin_port}/direct")))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert!(
            proxy_heads.lock().unwrap().is_empty(),
            "the proxy must be bypassed"
        );
        assert!(origin_heads.lock().unwrap()[0].starts_with("GET /direct "));
    }

    #[tokio::test]
    async fn https_requests_open_a_connect_tunnel_with_credentials() {
        let (proxy_port, heads) = fake_server("HTTP/1.1 403 Forbidden\r\n\r\n").await;
        let c = client(&[("https_proxy", format!("http://u:p@127.0.0.1:{proxy_port}"))]);
        let err = c
            .request(get("https://api.example.invalid/x"))
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("proxy CONNECT failed"),
            "{err:?}"
        );
        let head = heads.lock().unwrap()[0].to_ascii_lowercase();
        assert!(
            head.starts_with("connect api.example.invalid:443 http/1.1"),
            "{head}"
        );
        assert!(head.contains("proxy-authorization: basic dtpw"), "{head}");
    }

    #[tokio::test]
    async fn https_only_clients_refuse_plain_http() {
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let c = ProxyClient::new(
            ProxySettings::from_lookup(|n| {
                (n == "HTTP_PROXY").then(|| "http://127.0.0.1:1".into())
            }),
            tls,
            false,
        );
        assert!(c.request(get("http://example.invalid/x")).await.is_err());
    }

    #[test]
    fn debug_output_never_shows_the_proxy_credentials() {
        let s = settings(&[("HTTPS_PROXY", "http://alice:s3cr3t@proxy.example.com:1")]);
        for text in [format!("{s:?}"), format!("{:?}", s.https.as_ref().unwrap())] {
            assert!(!text.contains("YWxpY2U6czNjcjN0"), "{text}");
            assert!(!text.contains("s3cr3t"), "{text}");
            assert!(text.contains("proxy.example.com"), "{text}");
            assert!(text.contains("[redacted]"), "{text}");
        }
    }

    #[tokio::test]
    async fn connect_brackets_an_ipv6_target() {
        let (proxy_port, heads) = fake_server("HTTP/1.1 403 Forbidden\r\n\r\n").await;
        let c = client(&[("HTTPS_PROXY", format!("http://127.0.0.1:{proxy_port}"))]);
        let _ = c.request(get("https://[2001:db8::1]:8443/x")).await;
        let head = heads.lock().unwrap()[0].clone();
        assert!(
            head.starts_with("CONNECT [2001:db8::1]:8443 HTTP/1.1\r\nHost: [2001:db8::1]:8443\r\n"),
            "{head}"
        );
    }

    #[tokio::test]
    async fn any_2xx_answer_to_connect_opens_the_tunnel() {
        // RFC 9110 section 9.3.6: any 2xx means the tunnel is established.  The fake
        // proxy then closes the connection, so the TLS handshake fails - but
        // not with "proxy CONNECT failed".
        let (proxy_port, _) = fake_server("HTTP/1.1 204 No Content\r\n\r\n").await;
        let c = client(&[("HTTPS_PROXY", format!("http://127.0.0.1:{proxy_port}"))]);
        let err = c
            .request(get("https://api.example.invalid/x"))
            .await
            .unwrap_err();
        assert!(
            !format!("{err:?}").contains("proxy CONNECT failed"),
            "{err:?}"
        );
    }
}
