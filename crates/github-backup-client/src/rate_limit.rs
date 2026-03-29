// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub API rate limit information parsed from response headers.

use hyper::HeaderMap;

/// Rate-limit state extracted from `X-RateLimit-*` response headers.
///
/// GitHub sends these headers on every API response. When
/// `remaining == 0` the client must wait until `reset_timestamp` (a Unix
/// epoch second) before making further requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitInfo {
    /// Maximum requests allowed in the current window.
    pub limit: u64,
    /// Requests remaining in the current window.
    pub remaining: u64,
    /// Unix timestamp (seconds) at which the window resets.
    pub reset_timestamp: u64,
    /// Requests used in the current window.
    pub used: u64,
}

/// Extra seconds added on top of the `X-RateLimit-Reset` delta to absorb
/// clock skew between our host and GitHub's servers.
const RESET_BUFFER_SECS: u64 = 2;

impl RateLimitInfo {
    /// Attempts to parse rate-limit headers from `headers`.
    ///
    /// Returns `None` if any of the required headers are absent or malformed.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let limit = parse_u64(headers, "x-ratelimit-limit")?;
        let remaining = parse_u64(headers, "x-ratelimit-remaining")?;
        let reset_timestamp = parse_u64(headers, "x-ratelimit-reset")?;
        let used = parse_u64(headers, "x-ratelimit-used").unwrap_or(limit - remaining);

        Some(Self {
            limit,
            remaining,
            reset_timestamp,
            used,
        })
    }

    /// Returns `true` when no requests remain in the current window.
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.remaining == 0
    }

    /// Computes the number of seconds to wait until the window resets.
    ///
    /// Uses `now_secs` as the current Unix time so callers can inject a
    /// deterministic clock during tests.  Adds `RESET_BUFFER_SECS` to absorb
    /// clock skew between the client and GitHub's servers — without the buffer,
    /// a request made exactly at the reset instant often still gets a 429.
    #[must_use]
    pub fn seconds_until_reset(&self, now_secs: u64) -> u64 {
        self.reset_timestamp
            .saturating_sub(now_secs)
            .saturating_add(RESET_BUFFER_SECS)
    }

    /// Parses a `Retry-After` response header value (number of seconds).
    ///
    /// GitHub uses this header for secondary rate limits (abuse detection).
    /// Returns `None` if the header is absent or not a valid integer.
    #[must_use]
    pub fn retry_after(headers: &HeaderMap) -> Option<u64> {
        parse_u64(headers, "retry-after")
    }

    /// Parses the `X-OAuth-Scopes` header and returns the list of granted
    /// token scopes.
    ///
    /// Returns an empty `Vec` if the header is absent (e.g. fine-grained PATs
    /// which do not use this header model).
    #[must_use]
    pub fn oauth_scopes(headers: &HeaderMap) -> Vec<String> {
        headers
            .get("x-oauth-scopes")
            .and_then(|v| v.to_str().ok())
            .map(|s| {
                s.split(',')
                    .map(|sc| sc.trim().to_string())
                    .filter(|sc| !sc.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn parse_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    let value = headers.get(name)?.to_str().ok()?;
    value.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::{HeaderName, HeaderValue};

    fn make_headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn rate_limit_info_from_headers_parses_all_fields() {
        let headers = make_headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "4999"),
            ("x-ratelimit-reset", "1714521600"),
            ("x-ratelimit-used", "1"),
        ]);

        let info = RateLimitInfo::from_headers(&headers).expect("parse rate limit");
        assert_eq!(info.limit, 5000);
        assert_eq!(info.remaining, 4999);
        assert_eq!(info.reset_timestamp, 1_714_521_600);
        assert_eq!(info.used, 1);
    }

    #[test]
    fn rate_limit_info_from_headers_returns_none_when_header_missing() {
        let headers = make_headers(&[("x-ratelimit-limit", "5000")]);
        assert!(RateLimitInfo::from_headers(&headers).is_none());
    }

    #[test]
    fn rate_limit_info_is_exhausted_true_when_remaining_zero() {
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 0,
            reset_timestamp: 9_999_999_999,
            used: 5000,
        };
        assert!(info.is_exhausted());
    }

    #[test]
    fn rate_limit_info_seconds_until_reset_includes_buffer() {
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 0,
            reset_timestamp: 1000,
            used: 5000,
        };
        // 1000 - 800 = 200, plus RESET_BUFFER_SECS = 202
        assert_eq!(
            info.seconds_until_reset(800),
            200 + RESET_BUFFER_SECS,
            "wait time must include the clock-skew buffer"
        );
    }

    #[test]
    fn rate_limit_info_seconds_until_reset_saturates_at_buffer() {
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 0,
            reset_timestamp: 100,
            used: 5000,
        };
        // now > reset → saturates at 0 + buffer
        assert_eq!(
            info.seconds_until_reset(200),
            RESET_BUFFER_SECS,
            "past-reset times must still apply the buffer"
        );
    }

    #[test]
    fn retry_after_parses_integer_header() {
        let headers = make_headers(&[("retry-after", "60")]);
        assert_eq!(RateLimitInfo::retry_after(&headers), Some(60));
    }

    #[test]
    fn retry_after_returns_none_when_absent() {
        let headers = make_headers(&[]);
        assert_eq!(RateLimitInfo::retry_after(&headers), None);
    }

    #[test]
    fn oauth_scopes_parses_comma_separated_values() {
        let headers = make_headers(&[("x-oauth-scopes", "repo, gist, read:org")]);
        let scopes = RateLimitInfo::oauth_scopes(&headers);
        assert_eq!(scopes, vec!["repo", "gist", "read:org"]);
    }

    #[test]
    fn oauth_scopes_returns_empty_when_absent() {
        let headers = make_headers(&[]);
        assert!(RateLimitInfo::oauth_scopes(&headers).is_empty());
    }
}
