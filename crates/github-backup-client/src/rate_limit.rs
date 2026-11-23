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
        let used =
            parse_u64(headers, "x-ratelimit-used").unwrap_or(limit.saturating_sub(remaining));

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

/// How long GitHub asked us to wait after a rate-limit response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RateLimitWait {
    /// Seconds given by `Retry-After` or derived from `X-RateLimit-Reset`.
    Explicit(u64),
    /// A rate limit with no timing hint (secondary limits often have none).
    Unspecified,
}

/// Decides whether a response is a rate-limit response, and how long it asks
/// to wait.
///
/// GitHub signals rate limits with 429 *or* 403: a primary limit with
/// `X-RateLimit-Remaining: 0`, a secondary limit with `Retry-After` or only a
/// message ("You have exceeded a secondary rate limit").  Treating those 403s
/// as a plain "forbidden" would make the backup skip a category as "no admin
/// access" and silently drop data, so they are recognised here; a 403 that is
/// none of these is a real permission error and returns `None`.
pub(crate) fn classify(
    status: u16,
    headers: &HeaderMap,
    body: &[u8],
    now_secs: u64,
) -> Option<RateLimitWait> {
    if status != 429 && status != 403 {
        return None;
    }
    let info = RateLimitInfo::from_headers(headers);
    let exhausted = info.is_some_and(|i| i.is_exhausted());
    if let Some(secs) = explicit_wait(headers, now_secs) {
        return Some(RateLimitWait::Explicit(secs));
    }
    let message = String::from_utf8_lossy(body).to_ascii_lowercase();
    if status == 429 || exhausted || message.contains("rate limit") || message.contains("abuse") {
        return Some(RateLimitWait::Unspecified);
    }
    None
}

/// Like [`classify`] for a GraphQL response with status 200: GitHub reports
/// both the primary and (sometimes) the secondary GraphQL rate limit with a
/// `200` whose `errors` array says so (`"type": "RATE_LIMITED"` or a
/// "rate limit" message).  The wait is `Retry-After`, else the
/// `X-RateLimit-Reset` of an exhausted budget, else unspecified.
pub(crate) fn classify_graphql(
    headers: &HeaderMap,
    body: &[u8],
    now_secs: u64,
) -> Option<RateLimitWait> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let limited = value.get("errors")?.as_array()?.iter().any(|error| {
        error.get("type").and_then(serde_json::Value::as_str) == Some("RATE_LIMITED")
            || error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|m| m.to_ascii_lowercase().contains("rate limit"))
    });
    limited.then(|| {
        explicit_wait(headers, now_secs).map_or(RateLimitWait::Unspecified, RateLimitWait::Explicit)
    })
}

/// The wait a rate-limit response names: `Retry-After`, else the time until
/// `X-RateLimit-Reset` when `X-RateLimit-Remaining` is 0.
fn explicit_wait(headers: &HeaderMap, now_secs: u64) -> Option<u64> {
    RateLimitInfo::retry_after(headers).or_else(|| {
        RateLimitInfo::from_headers(headers)
            .filter(RateLimitInfo::is_exhausted)
            .map(|i| i.seconds_until_reset(now_secs))
    })
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

    // ── Additional edge cases pinned down for mutation testing ────────────

    #[test]
    fn rate_limit_info_is_exhausted_false_when_remaining_positive() {
        // Pin down the negative case for `is_exhausted` so a mutant
        // returning the constant `true` is observable.
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 1,
            reset_timestamp: 9_999_999_999,
            used: 4999,
        };
        assert!(!info.is_exhausted());
    }

    #[test]
    fn rate_limit_info_is_exhausted_false_when_full() {
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 5000,
            reset_timestamp: 9_999_999_999,
            used: 0,
        };
        assert!(!info.is_exhausted());
    }

    #[test]
    fn rate_limit_info_used_defaults_to_limit_minus_remaining() {
        // When the `x-ratelimit-used` header is absent, `from_headers`
        // computes `used = limit - remaining`. Pins down that arithmetic.
        let headers = make_headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "4250"),
            ("x-ratelimit-reset", "1714521600"),
        ]);
        let info = RateLimitInfo::from_headers(&headers).expect("parse");
        assert_eq!(info.used, 750);
    }

    #[test]
    fn rate_limit_info_from_headers_returns_none_for_non_numeric_value() {
        let headers = make_headers(&[
            ("x-ratelimit-limit", "abc"),
            ("x-ratelimit-remaining", "4999"),
            ("x-ratelimit-reset", "1714521600"),
        ]);
        assert!(RateLimitInfo::from_headers(&headers).is_none());
    }

    #[test]
    fn retry_after_returns_none_for_non_numeric() {
        let headers = make_headers(&[("retry-after", "soon")]);
        assert!(RateLimitInfo::retry_after(&headers).is_none());
    }

    #[test]
    fn oauth_scopes_filters_empty_entries() {
        // Trailing comma or double comma should not produce empty scope strings.
        let headers = make_headers(&[("x-oauth-scopes", "repo,, gist,")]);
        let scopes = RateLimitInfo::oauth_scopes(&headers);
        assert_eq!(scopes, vec!["repo", "gist"]);
    }

    #[test]
    fn seconds_until_reset_zero_at_reset_returns_buffer() {
        // Boundary: now == reset → no wait + the buffer.
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 0,
            reset_timestamp: 500,
            used: 5000,
        };
        assert_eq!(info.seconds_until_reset(500), RESET_BUFFER_SECS);
    }

    #[test]
    fn seconds_until_reset_one_second_before() {
        let info = RateLimitInfo {
            limit: 5000,
            remaining: 0,
            reset_timestamp: 1001,
            used: 5000,
        };
        // 1 + RESET_BUFFER_SECS
        assert_eq!(info.seconds_until_reset(1000), 1 + RESET_BUFFER_SECS);
    }

    #[test]
    fn classify_tells_rate_limits_from_permission_errors() {
        let none = make_headers(&[]);
        let exhausted = make_headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1030"),
        ]);
        let left = make_headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "10"),
            ("x-ratelimit-reset", "1030"),
        ]);
        let retry = make_headers(&[("retry-after", "7")]);
        let secondary = br#"{"message":"You have exceeded a secondary rate limit"}"#;

        assert_eq!(
            classify(429, &none, b"", 1000),
            Some(RateLimitWait::Unspecified)
        );
        assert_eq!(
            classify(403, &retry, b"", 1000),
            Some(RateLimitWait::Explicit(7))
        );
        assert_eq!(
            classify(403, &exhausted, b"", 1000),
            Some(RateLimitWait::Explicit(32))
        );
        assert_eq!(
            classify(403, &none, secondary, 0),
            Some(RateLimitWait::Unspecified)
        );
        // Real permission errors, with and without rate-limit headers.
        assert_eq!(classify(403, &none, br#"{"message":"Forbidden"}"#, 0), None);
        assert_eq!(
            classify(403, &left, br#"{"message":"Not accessible"}"#, 0),
            None
        );
        assert_eq!(classify(404, &exhausted, b"", 0), None);
        assert_eq!(classify(500, &retry, b"", 0), None);
    }

    #[test]
    fn more_remaining_than_the_limit_does_not_underflow() {
        // A misbehaving proxy or server; `used` must not wrap or panic.
        let headers = make_headers(&[
            ("x-ratelimit-limit", "10"),
            ("x-ratelimit-remaining", "20"),
            ("x-ratelimit-reset", "1"),
        ]);
        let info = RateLimitInfo::from_headers(&headers).expect("parse");
        assert_eq!(info.used, 0);
    }

    #[test]
    fn classify_graphql_recognises_rate_limit_errors_only() {
        let exhausted = make_headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1030"),
        ]);
        let none = make_headers(&[]);
        let typed = br#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit already exceeded for user ID 1."}]}"#;
        let secondary = br#"{"errors":[{"message":"You have exceeded a secondary rate limit."}]}"#;
        assert_eq!(
            classify_graphql(&exhausted, typed, 1000),
            Some(RateLimitWait::Explicit(32))
        );
        assert_eq!(
            classify_graphql(&none, secondary, 0),
            Some(RateLimitWait::Unspecified)
        );
        assert_eq!(
            classify_graphql(&make_headers(&[("retry-after", "9")]), secondary, 0),
            Some(RateLimitWait::Explicit(9))
        );
        // Other errors, data, or no JSON at all are not rate limits, even
        // when the request happened to use the last point of the budget.
        let not_found =
            br#"{"data":null,"errors":[{"type":"NOT_FOUND","message":"Could not resolve"}]}"#;
        assert_eq!(classify_graphql(&exhausted, not_found, 1000), None);
        assert_eq!(classify_graphql(&exhausted, br#"{"data":{}}"#, 1000), None);
        assert_eq!(classify_graphql(&none, b"not json", 0), None);
    }
}
