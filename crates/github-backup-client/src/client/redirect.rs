// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Which redirects the client follows, and with which credential.
//!
//! GitHub answers a request for a renamed or transferred repository with
//! `301` (and `307` for methods other than `GET`), and a release asset request
//! with a redirect to a pre-signed URL on its storage host.  The credential
//! must survive only a redirect within the same origin: anywhere else it would
//! be disclosed to a third party (and signed storage URLs reject a request
//! that has both a signature and an `Authorization` header).

use url::Url;

/// Maximum number of redirects followed for one request.
pub(crate) const MAX_REDIRECTS: u8 = 3;

/// What to do with a redirect from `from` to `to`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Hop {
    /// Follow it, still sending the credential (same origin).
    SameOrigin,
    /// Another host, port or scheme: never send the credential there.
    OtherOrigin,
    /// Do not follow it (HTTPS to a non-HTTPS URL).
    Refuse,
}

/// Decides how a redirect (or a next-page link) from `from` to `to` may be
/// followed.
///
/// The origin is the scheme, host (compared case-insensitively) and effective
/// port.  A move from `https` to anything else is refused outright.
pub(crate) fn classify(from: &Url, to: &Url) -> Hop {
    if from.scheme() == "https" && to.scheme() != "https" {
        return Hop::Refuse;
    }
    let same_origin = from.scheme() == to.scheme()
        && from.host_str().map(str::to_ascii_lowercase)
            == to.host_str().map(str::to_ascii_lowercase)
        && from.port_or_known_default() == to.port_or_known_default();
    if same_origin {
        Hop::SameOrigin
    } else {
        Hop::OtherOrigin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("url")
    }

    #[test]
    fn same_origin_keeps_the_credential() {
        let from = url("https://api.github.com/repos/o/r/releases/assets/1");
        assert_eq!(
            classify(&from, &url("https://api.github.com/other")),
            Hop::SameOrigin
        );
        // Default port spelled out, host case differs: still the same origin.
        assert_eq!(
            classify(&from, &url("https://API.GitHub.com:443/other")),
            Hop::SameOrigin
        );
    }

    #[test]
    fn other_host_port_or_scheme_is_another_origin() {
        let from = url("https://api.github.com/a");
        for to in [
            "https://objects.githubusercontent.com/blob?sig=1",
            "https://api.github.com:8443/a",
            "https://api.github.com.evil.example/a",
        ] {
            assert_eq!(classify(&from, &url(to)), Hop::OtherOrigin, "{to}");
        }
        // http -> https is an upgrade to another origin.
        assert_eq!(
            classify(&url("http://ghe.test/a"), &url("https://ghe.test/a")),
            Hop::OtherOrigin
        );
    }

    #[test]
    fn https_to_http_is_refused() {
        let from = url("https://api.github.com/a");
        assert_eq!(
            classify(&from, &url("http://api.github.com/a")),
            Hop::Refuse
        );
        assert_eq!(
            classify(&from, &url("http://elsewhere.test/a")),
            Hop::Refuse
        );
    }
}
