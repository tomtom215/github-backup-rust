// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Timestamps in the one form every file this tool writes uses:
//! RFC 3339, UTC, whole seconds, `Z` suffix (`2024-01-15T08:00:00Z`).
//!
//! Built on [`jiff`]; nothing else in the workspace parses or formats times.

use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp};

/// Formats `t` as `YYYY-MM-DDTHH:MM:SSZ`, truncating sub-second precision.
#[must_use]
pub fn format_utc(t: Timestamp) -> String {
    // `Timestamp`'s `Display` is RFC 3339 in UTC; a precision of 0 truncates
    // the fractional seconds.
    format!("{t:.0}")
}

/// The current time as `YYYY-MM-DDTHH:MM:SSZ`.
#[must_use]
pub fn now_utc() -> String {
    format_utc(Timestamp::now())
}

/// Formats a Unix time in seconds as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Values outside the range [`Timestamp`] supports (far beyond year 9999)
/// clamp to the Unix epoch.
#[must_use]
pub fn unix_secs_to_utc(secs: u64) -> String {
    let t = i64::try_from(secs)
        .ok()
        .and_then(|s| Timestamp::from_second(s).ok())
        .unwrap_or(Timestamp::UNIX_EPOCH);
    format_utc(t)
}

/// Parses an RFC 3339 timestamp with any UTC offset.
///
/// Stricter than [`Timestamp`]'s own parser, which also accepts a space in
/// place of the `T` and times without seconds: only
/// `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` is accepted.
#[must_use]
pub fn parse_rfc3339(s: &str) -> Option<Timestamp> {
    let b = s.as_bytes();
    let shape_ok = b.len() >= 20
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[7] == b'-'
        && matches!(b[10], b'T' | b't')
        && b[13] == b':'
        && b[16] == b':'
        && b[17..19].iter().all(u8::is_ascii_digit);
    if !shape_ok {
        return None;
    }
    s.parse().ok()
}

/// `true` if `s` is a complete RFC 3339 timestamp (see [`parse_rfc3339`]).
#[must_use]
pub fn is_rfc3339(s: &str) -> bool {
    parse_rfc3339(s).is_some()
}

/// Normalises a `--since` value to `YYYY-MM-DDTHH:MM:SSZ` (UTC).
///
/// Accepts a bare date (`2024-01-01`, taken as midnight UTC) or a full RFC 3339
/// timestamp with any offset.
///
/// # Errors
///
/// Returns what is wrong with `value` when it is neither.
pub fn normalise_since(value: &str) -> Result<String, String> {
    let value = value.trim();
    if let Some(t) = parse_rfc3339(value) {
        return Ok(format_utc(t));
    }
    let date_shape = value.len() == 10
        && value.bytes().enumerate().all(|(i, c)| match i {
            4 | 7 => c == b'-',
            _ => c.is_ascii_digit(),
        });
    date_shape
        .then(|| value.parse::<Date>().ok())
        .flatten()
        .and_then(|d| d.to_zoned_datetime(jiff::tz::TimeZone::UTC).ok())
        .map(|z| format_utc(z.timestamp()))
        .ok_or_else(|| {
            "expected a date like 2024-01-01 or a timestamp like 2024-01-01T00:00:00Z".to_string()
        })
}

/// How long ago `t` was, measured on this machine's clock (negative if `t` is
/// in the future).
#[must_use]
pub fn age(t: Timestamp) -> SignedDuration {
    Timestamp::now().duration_since(t)
}

/// `t` minus `d`, saturating at the smallest representable instant.
#[must_use]
pub fn minus(t: Timestamp, d: SignedDuration) -> Timestamp {
    t.checked_sub(d).unwrap_or(Timestamp::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_since_accepts_dates_and_timestamps() {
        assert_eq!(
            normalise_since("2024-01-01").unwrap(),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            normalise_since("2024-01-01T12:00:00+02:00").unwrap(),
            "2024-01-01T10:00:00Z",
            "offsets are converted to UTC"
        );
        assert_eq!(
            normalise_since(" 2024-01-01T00:00:00Z ").unwrap(),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            normalise_since("2024-01-01T00:00:00.999Z").unwrap(),
            "2024-01-01T00:00:00Z",
            "fractional seconds are truncated"
        );
        for bad in [
            "",
            "yesterday",
            "2024-13-01",
            "2024-02-30",
            "2024-01-01 00:00:00",
            "2024-01-01 00:00:00Z",
            "2024-1-01",
            "+002024-01-01",
            "2024-01-01T00:00Z",
        ] {
            assert!(normalise_since(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn rfc3339_accepts_the_forms_github_uses() {
        assert!(is_rfc3339("2024-01-01T00:00:00Z"));
        assert!(is_rfc3339("2024-01-01T12:00:00+05:30"));
        assert!(is_rfc3339("2024-01-01T12:00:00-08:00"));
        assert!(is_rfc3339("2024-01-01t12:00:00z"));
    }

    #[test]
    fn rfc3339_rejects_malformed_and_out_of_range_values() {
        for bad in [
            "",
            "2024-01-01",
            "2024-01-01 00:00:00Z",
            "XXXX-01-01T00:00:00Z",
            "20240101T000000Z",
            "2024-99-99T99:99:99Z",
            "2024-13-01T00:00:00Z",
            "2024-01-32T00:00:00Z",
            "2024-01-01T25:00:00Z",
            "2024-02-30T00:00:00Z",
            "2024-01-01T00:00:00",
        ] {
            assert!(!is_rfc3339(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn unix_seconds_format_as_utc() {
        assert_eq!(unix_secs_to_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_secs_to_utc(1_705_305_600), "2024-01-15T08:00:00Z");
        assert_eq!(unix_secs_to_utc(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(unix_secs_to_utc(u64::MAX), "1970-01-01T00:00:00Z");
        let s = unix_secs_to_utc(1_700_000_000);
        assert_eq!(s.len(), 20);
        assert!(is_rfc3339(&s));
    }

    #[test]
    fn now_is_whole_seconds_utc() {
        let s = now_utc();
        assert_eq!(s.len(), 20, "{s}");
        assert!(s.ends_with('Z'), "{s}");
        assert!(is_rfc3339(&s));
    }

    #[test]
    fn minus_and_age_behave() {
        let t: Timestamp = "2026-03-01T12:00:00Z".parse().unwrap();
        assert_eq!(
            format_utc(minus(t, SignedDuration::from_mins(15))),
            "2026-03-01T11:45:00Z"
        );
        assert!(age(t) > SignedDuration::ZERO);
        assert_eq!(
            minus(Timestamp::MIN, SignedDuration::from_hours(1)),
            Timestamp::MIN
        );
    }
}
