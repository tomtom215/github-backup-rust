// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Percent-encoding and XML helpers shared by request signing and response
//! parsing.
//!
//! The single source of truth for how an object key is written on the wire:
//! the request line **and** the SigV4 canonical URI are both produced by
//! [`encode_path`], so what is sent and what is signed cannot drift apart.

use std::fmt::Write as _;

/// Returns `true` for the characters SigV4 never percent-encodes
/// (`A-Z a-z 0-9 - _ . ~`).
const fn is_unreserved(byte: u8) -> bool {
    matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~')
}

fn push_encoded(out: &mut String, byte: u8) {
    // Writing to a `String` cannot fail.
    let _ = write!(out, "%{byte:02X}");
}

/// SigV4 `UriEncode` of a URL path: every byte except unreserved characters
/// and `/` becomes `%XX` (upper-case hex, over the UTF-8 bytes).
///
/// S3 encodes each path segment exactly once; the same string is used as the
/// canonical URI and as the path of the request line.
#[must_use]
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + path.len() / 4);
    for &byte in path.as_bytes() {
        if is_unreserved(byte) || byte == b'/' {
            out.push(byte as char);
        } else {
            push_encoded(&mut out, byte);
        }
    }
    out
}

/// SigV4 `UriEncode` of a query-string name or value (`/` is encoded too).
#[must_use]
pub fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for &byte in text.as_bytes() {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            push_encoded(&mut out, byte);
        }
    }
    out
}

/// Query parameters encoded with [`encode_component`] and sorted by encoded
/// name, then encoded value — the order SigV4 canonicalisation requires.
fn sorted_encoded_pairs(params: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = params
        .iter()
        .map(|(name, value)| (encode_component(name), encode_component(value)))
        .collect();
    pairs.sort();
    pairs
}

/// Builds the canonical query string: names and values encoded with
/// [`encode_component`], sorted by encoded name then encoded value, every
/// parameter in `name=value` form (an empty value yields `name=`).
///
/// SigV4 requires the `name=` form for value-less sub-resources such as
/// `?uploads`; signing the bare `uploads` produces a signature no compliant
/// server accepts.
#[must_use]
pub fn canonical_query(params: &[(&str, &str)]) -> String {
    sorted_encoded_pairs(params)
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// The query string as it is written on the request line: the same sorted,
/// encoded parameters as [`canonical_query`], except that a value-less
/// sub-resource is written bare (`?uploads`), which is how every S3 SDK sends
/// it.  A server canonicalises the bare form to `uploads=`.
#[must_use]
pub fn request_query(params: &[(&str, &str)]) -> String {
    sorted_encoded_pairs(params)
        .iter()
        .map(|(name, value)| {
            if value.is_empty() {
                name.clone()
            } else {
                format!("{name}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-decodes `input`.
///
/// With `plus_is_space` a literal `+` decodes to a space, which is how a
/// `encoding-type=url` listing renders spaces on some servers (MinIO, Go's
/// `QueryEscape`); servers that write `%20` are unaffected because a literal
/// `+` is always sent as `%2B`.  A malformed escape (a `%` not followed by two
/// hex digits) is kept verbatim.  Invalid UTF-8 is replaced lossily.
#[must_use]
pub fn percent_decode(input: &str, plus_is_space: bool) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hi = bytes.get(i + 1).copied().and_then(hex_value);
                let lo = bytes.get(i + 2).copied().and_then(hex_value);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push(hi * 16 + lo);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decodes the five predefined XML entities and numeric character references.
///
/// Unknown or malformed entities are left as they are.
#[must_use]
pub fn xml_unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        // An entity is `&` + up to 10 characters + `;`.
        let end = after
            .char_indices()
            .take(12)
            .find(|&(_, c)| c == ';')
            .map(|(i, _)| i);
        let decoded = end.and_then(|end| decode_entity(&after[1..end]).map(|c| (c, end)));
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Escapes text for inclusion in XML element content or attribute values.
#[must_use]
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Text of the first `<tag>…</tag>` element, entity-decoded.
#[must_use]
pub fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml_unescape(&xml[start..end]))
}

/// Text of every `<tag>…</tag>` element, entity-decoded, in document order.
#[must_use]
pub fn xml_tags(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut result = Vec::new();
    let mut from = 0;
    while let Some(open_at) = xml[from..].find(&open) {
        let content_start = from + open_at + open.len();
        let Some(close_at) = xml[content_start..].find(&close) else {
            break;
        };
        result.push(xml_unescape(&xml[content_start..content_start + close_at]));
        from = content_start + close_at + close.len();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_path_keeps_unreserved_and_slash() {
        assert_eq!(encode_path("/a/b-c_d.e~f/G9"), "/a/b-c_d.e~f/G9");
    }

    #[test]
    fn encode_path_escapes_everything_else_as_utf8_bytes() {
        assert_eq!(encode_path("/with space"), "/with%20space");
        assert_eq!(encode_path("/a+b"), "/a%2Bb");
        assert_eq!(encode_path("/100%"), "/100%25");
        assert_eq!(encode_path("/what?"), "/what%3F");
        assert_eq!(encode_path("/file#1"), "/file%231");
        assert_eq!(encode_path("/a&b=c"), "/a%26b%3Dc");
        assert_eq!(encode_path("/(1)*!'$,;:@"), "/%281%29%2A%21%27%24%2C%3B%3A%40");
        assert_eq!(encode_path("/r\u{e9}sum\u{e9}"), "/r%C3%A9sum%C3%A9");
        assert_eq!(encode_path("/\u{1f600}"), "/%F0%9F%98%80");
    }

    #[test]
    fn encode_component_also_escapes_slash() {
        assert_eq!(encode_component("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn canonical_query_uses_name_equals_form_and_sorts() {
        assert_eq!(canonical_query(&[("uploads", "")]), "uploads=");
        assert_eq!(
            canonical_query(&[("prefix", "a b/"), ("list-type", "2"), ("max-keys", "1000")]),
            "list-type=2&max-keys=1000&prefix=a%20b%2F"
        );
        assert_eq!(
            canonical_query(&[("uploadId", "x"), ("partNumber", "3")]),
            "partNumber=3&uploadId=x"
        );
        assert_eq!(canonical_query(&[]), "");
    }

    #[test]
    fn request_query_writes_value_less_parameters_bare() {
        assert_eq!(request_query(&[("uploads", "")]), "uploads");
        assert_eq!(
            request_query(&[("uploadId", "a b"), ("partNumber", "3")]),
            "partNumber=3&uploadId=a%20b"
        );
        assert_eq!(request_query(&[("b", ""), ("a", "1")]), "a=1&b");
        assert_eq!(request_query(&[]), "");
    }

    #[test]
    fn request_query_and_canonical_query_agree_up_to_the_equals_form() {
        let params = [("z", ""), ("a", "x y"), ("m", "1/2"), ("uploads", "")];
        let canonical = canonical_query(&params);
        let request = request_query(&params);
        // A server turns every bare name in the request line into `name=`.
        let normalised: Vec<String> = request
            .split('&')
            .map(|part| {
                if part.contains('=') {
                    part.to_string()
                } else {
                    format!("{part}=")
                }
            })
            .collect();
        assert_eq!(normalised.join("&"), canonical);
    }

    #[test]
    fn canonical_query_sorts_by_encoded_name() {
        // 'Z' (0x5A) sorts before 'a' (0x61) in byte order.
        assert_eq!(canonical_query(&[("a", "1"), ("Z", "2")]), "Z=2&a=1");
    }

    #[test]
    fn percent_decode_roundtrips_encode_path() {
        for key in ["a b", "a+b", "100%", "what?", "f#1", "a&b", "r\u{e9}sum\u{e9}", "\u{1f600}"] {
            assert_eq!(percent_decode(&encode_path(key), false), key);
            assert_eq!(percent_decode(&encode_path(key), true), key);
        }
    }

    #[test]
    fn percent_decode_plus_handling() {
        assert_eq!(percent_decode("a+b", true), "a b");
        assert_eq!(percent_decode("a+b", false), "a+b");
        assert_eq!(percent_decode("a%2Bb", true), "a+b");
    }

    #[test]
    fn percent_decode_keeps_malformed_escapes() {
        assert_eq!(percent_decode("100%", false), "100%");
        assert_eq!(percent_decode("%zz", false), "%zz");
        assert_eq!(percent_decode("%4", false), "%4");
    }

    #[test]
    fn xml_unescape_decodes_predefined_and_numeric_entities() {
        assert_eq!(xml_unescape("a&amp;b"), "a&b");
        assert_eq!(xml_unescape("&lt;x&gt;&quot;y&apos;"), "<x>\"y'");
        assert_eq!(xml_unescape("&#65;&#x42;&#X43;"), "ABC");
        assert_eq!(xml_unescape("&amp;lt;"), "&lt;", "decoded exactly once");
    }

    #[test]
    fn xml_unescape_leaves_unknown_entities_alone() {
        assert_eq!(xml_unescape("a & b"), "a & b");
        assert_eq!(xml_unescape("&nosuch;"), "&nosuch;");
        assert_eq!(xml_unescape("&#xZZ;"), "&#xZZ;");
        assert_eq!(xml_unescape("trailing &"), "trailing &");
    }

    #[test]
    fn xml_escape_roundtrips() {
        let s = "a&b<c>d\"e'f";
        assert_eq!(xml_unescape(&xml_escape(s)), s);
    }

    #[test]
    fn xml_tag_extracts_and_decodes() {
        let xml = "<r><Code>NoSuchBucket</Code><Message>a &amp; b</Message></r>";
        assert_eq!(xml_tag(xml, "Code").as_deref(), Some("NoSuchBucket"));
        assert_eq!(xml_tag(xml, "Message").as_deref(), Some("a & b"));
        assert_eq!(xml_tag(xml, "Missing"), None);
    }

    #[test]
    fn xml_tags_finds_all_in_order() {
        let xml = "<r><Key>a/b.json</Key><Key>c&amp;d.json</Key></r>";
        assert_eq!(xml_tags(xml, "Key"), vec!["a/b.json", "c&d.json"]);
        assert!(xml_tags("<r><Name>b</Name></r>", "Key").is_empty());
    }

    #[test]
    fn xml_tags_ignores_unterminated_element() {
        assert_eq!(xml_tags("<Key>a</Key><Key>b", "Key"), vec!["a"]);
    }
}
