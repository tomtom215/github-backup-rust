// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Validates the *independent* verifier used by the fake S3 server against
//! the four worked examples AWS publishes for S3, so that a "signature
//! verified" verdict in the other tests means something.

mod support;

use support::sigv4_check::{canonical_query, canonical_request, canonical_uri, sha256_hex, signature};

const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const DATE: &str = "20130524T000000Z";
const HOST: &str = "examplebucket.s3.amazonaws.com";

fn check(
    method: &str,
    raw_path: &str,
    raw_query: &str,
    headers: &[(&str, String)],
    payload: &str,
    expected: &str,
) {
    let signed: Vec<String> = headers.iter().map(|(n, _)| n.to_string()).collect();
    let lookup = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
    };
    let creq = canonical_request(
        method,
        &canonical_uri(raw_path).unwrap(),
        &canonical_query(raw_query).unwrap(),
        &lookup,
        &signed,
        payload,
    );
    assert_eq!(signature(SECRET, DATE, "us-east-1", "s3", &creq), expected);
}

#[test]
fn aws_published_s3_examples() {
    let empty = sha256_hex(b"");
    check(
        "GET",
        "/test.txt",
        "",
        &[
            ("host", HOST.into()),
            ("range", "bytes=0-9".into()),
            ("x-amz-content-sha256", empty.clone()),
            ("x-amz-date", DATE.into()),
        ],
        &empty,
        "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41",
    );
    let body = sha256_hex(b"Welcome to Amazon S3.");
    check(
        "PUT",
        "/test%24file.text",
        "",
        &[
            ("date", "Fri, 24 May 2013 00:00:00 GMT".into()),
            ("host", HOST.into()),
            ("x-amz-content-sha256", body.clone()),
            ("x-amz-date", DATE.into()),
            ("x-amz-storage-class", "REDUCED_REDUNDANCY".into()),
        ],
        &body,
        "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd",
    );
    let plain = [
        ("host", HOST.to_string()),
        ("x-amz-content-sha256", empty.clone()),
        ("x-amz-date", DATE.to_string()),
    ];
    check(
        "GET",
        "/",
        "lifecycle",
        &plain,
        &empty,
        "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543",
    );
    check(
        "GET",
        "/",
        "max-keys=2&prefix=J",
        &plain,
        &empty,
        "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7",
    );
}
