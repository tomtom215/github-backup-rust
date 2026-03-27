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
    /// deterministic clock during tests.
    #[must_use]
    pub fn seconds_until_reset(&self, now_secs: u64) -> u64 {
        self.reset_timestamp.saturating_sub(now_secs)
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
    fn rate_limit_info_seconds_until_reset_returns_correct_delta() {
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 0,
            reset_timestamp: 1000,
            used: 5000,
        };
        assert_eq!(info.seconds_until_reset(800), 200);
    }

    #[test]
    fn rate_limit_info_seconds_until_reset_saturates_at_zero() {
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 0,
            reset_timestamp: 100,
            used: 5000,
        };
        assert_eq!(info.seconds_until_reset(200), 0);
    }
}
