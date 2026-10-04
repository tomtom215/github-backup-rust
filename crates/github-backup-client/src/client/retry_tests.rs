// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Rate-limit and transient-failure handling, against a scripted local
//! plain-HTTP server.  The test client counts one back-off "second" as one
//! millisecond (see `Timing`), so a 60 s secondary-limit wait takes 60 ms.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use github_backup_types::config::Credential;

use super::GitHubClient;
use crate::error::ClientError;

/// What the server does with one request.
enum Act {
    Reply {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: String,
    },
    /// Close the connection without answering.
    Drop,
    /// Send the headers and part of the body, then go silent.
    StallBody,
    /// Read the request and never answer.
    Hang,
}

fn reply(status: u16, body: &str) -> Act {
    Act::Reply {
        status,
        headers: vec![],
        body: body.to_string(),
    }
}

fn reply_with(status: u16, headers: &[(&'static str, &str)], body: &str) -> Act {
    Act::Reply {
        status,
        headers: headers.iter().map(|(k, v)| (*k, v.to_string())).collect(),
        body: body.to_string(),
    }
}

fn ok(body: Value) -> Act {
    reply(200, &body.to_string())
}

/// Scripted server: `script(target, n)` decides the answer to the `n`-th
/// (0-based) request for `target`.
struct Server {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Server {
    async fn start(script: impl Fn(&str, usize) -> Act + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let script = Arc::new(script);
        let log = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let script = Arc::clone(&script);
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let seen = {
                        let mut log = log.lock().expect("log");
                        let seen = log.iter().filter(|t| **t == target).count();
                        log.push(target.clone());
                        seen
                    };
                    match script(&target, seen) {
                        Act::Reply {
                            status,
                            headers,
                            body,
                        } => {
                            let extra: String = headers
                                .iter()
                                .map(|(k, v)| format!("{k}: {v}\r\n"))
                                .collect();
                            let response = format!(
                                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}\
                                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        }
                        Act::Drop => drop(stream),
                        Act::StallBody => {
                            let _ = stream
                                .write_all(b"HTTP/1.1 200 X\r\nContent-Length: 100\r\n\r\n[1,")
                                .await;
                            tokio::time::sleep(Duration::from_secs(30)).await;
                        }
                        Act::Hang => tokio::time::sleep(Duration::from_secs(30)).await,
                    }
                });
            }
        });
        Self { base, hits }
    }

    fn client(&self) -> GitHubClient {
        GitHubClient::for_tests_at(Credential::Token("ghp_dummy".to_string()), &self.base)
    }

    fn url(&self, target: &str) -> String {
        format!("{}{target}", self.base)
    }

    fn hit_count(&self) -> usize {
        self.hits.lock().expect("log").len()
    }
}

async fn get(client: &GitHubClient, url: &str) -> Result<Value, ClientError> {
    client
        .get_json_with_link::<Value>(url)
        .await
        .map(|(value, _)| value)
}

// ── Rate limits ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_403_with_retry_after_is_waited_out_and_retried() {
    let server = Server::start(|_, n| {
        if n == 0 {
            reply_with(403, &[("Retry-After", "2")], r#"{"message":"slow down"}"#)
        } else {
            ok(json!({"ok": true}))
        }
    })
    .await;
    let started = Instant::now();
    let value = get(&server.client(), &server.url("/x"))
        .await
        .expect("retried");
    assert_eq!(value["ok"], true);
    assert_eq!(server.hit_count(), 2);
    assert!(
        started.elapsed() >= Duration::from_millis(2),
        "Retry-After must be honoured"
    );
}

#[tokio::test]
async fn a_403_with_an_exhausted_primary_limit_is_waited_out_until_reset() {
    let reset = super::unix_now() + 1;
    let server = Server::start(move |_, n| {
        if n == 0 {
            reply_with(
                403,
                &[
                    ("x-ratelimit-limit", "5000"),
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset.to_string()),
                ],
                r#"{"message":"API rate limit exceeded"}"#,
            )
        } else {
            ok(json!([]))
        }
    })
    .await;
    get(&server.client(), &server.url("/x"))
        .await
        .expect("retried");
    assert_eq!(server.hit_count(), 2);
}

#[tokio::test]
async fn a_429_is_retried() {
    let server = Server::start(|_, n| {
        if n == 0 {
            reply(429, "{}")
        } else {
            ok(json!({}))
        }
    })
    .await;
    get(&server.client(), &server.url("/x"))
        .await
        .expect("retried");
    assert_eq!(server.hit_count(), 2);
}

#[tokio::test]
async fn a_secondary_limit_without_any_header_waits_at_least_a_minute_and_backs_off() {
    let server = Server::start(|_, n| {
        if n < 2 {
            reply(
                403,
                r#"{"message":"You have exceeded a secondary rate limit. Please wait."}"#,
            )
        } else {
            ok(json!({}))
        }
    })
    .await;
    let started = Instant::now();
    get(&server.client(), &server.url("/x"))
        .await
        .expect("retried");
    assert_eq!(server.hit_count(), 3);
    // 60 s then 120 s, at one millisecond per second.
    assert!(
        started.elapsed() >= Duration::from_millis(180),
        "waited only {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_rate_limit_that_never_clears_ends_as_rate_limit_exceeded_not_as_403() {
    let server =
        Server::start(|_, _| reply_with(403, &[("Retry-After", "1")], r#"{"message":"limit"}"#))
            .await;
    let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
    assert!(
        matches!(err, ClientError::RateLimitExceeded { .. }),
        "got {err:?}"
    );
    assert_eq!(
        server.hit_count(),
        1 + super::MAX_RATE_LIMIT_RETRIES as usize
    );
}

#[tokio::test]
async fn a_reset_further_away_than_the_wait_budget_fails_at_once() {
    let server = Server::start(|_, _| reply_with(429, &[("Retry-After", "86400")], "{}")).await;
    let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
    assert!(
        matches!(
            err,
            ClientError::RateLimitExceeded {
                retry_after_secs: 86400
            }
        ),
        "got {err:?}"
    );
    assert_eq!(
        server.hit_count(),
        1,
        "no point in waiting and asking again"
    );
}

#[tokio::test]
async fn a_permission_403_is_not_mistaken_for_a_rate_limit() {
    let server = Server::start(|_, _| {
        reply_with(
            403,
            &[
                ("x-ratelimit-limit", "5000"),
                ("x-ratelimit-remaining", "4990"),
                ("x-ratelimit-reset", "1"),
            ],
            r#"{"message":"Resource not accessible by personal access token"}"#,
        )
    })
    .await;
    let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
    assert!(
        matches!(err, ClientError::ApiError { status: 403, .. }),
        "got {err:?}"
    );
    assert_eq!(server.hit_count(), 1);
}

#[tokio::test]
async fn topics_wait_out_a_rate_limit_instead_of_reporting_a_403() {
    let server = Server::start(|_, n| {
        if n == 0 {
            reply_with(403, &[("Retry-After", "1")], "{}")
        } else {
            ok(json!({"names": ["rust"]}))
        }
    })
    .await;
    let topics = server
        .client()
        .list_repo_topics("o", "r")
        .await
        .expect("retried");
    assert_eq!(topics, vec!["rust".to_string()]);
}

#[tokio::test]
async fn dropping_the_future_ends_a_long_rate_limit_wait_at_once() {
    let server = Server::start(|_, _| reply_with(429, &[("Retry-After", "3000")], "{}")).await;
    let mut client = server.client();
    client.timing.unit = Duration::from_secs(1); // real seconds
    let url = server.url("/x");
    let started = Instant::now();
    let outcome = tokio::time::timeout(Duration::from_millis(300), get(&client, &url)).await;
    assert!(outcome.is_err(), "still waiting when the engine cancels");
    assert!(started.elapsed() < Duration::from_secs(5));
}

// ── Transient failures ────────────────────────────────────────────────────

#[tokio::test]
async fn transient_server_errors_are_retried_for_gets() {
    for status in [500u16, 502, 503, 504] {
        let server = Server::start(move |_, n| {
            if n < 2 {
                reply(status, "{}")
            } else {
                ok(json!({"v": 1}))
            }
        })
        .await;
        let value = get(&server.client(), &server.url("/x"))
            .await
            .expect("retried");
        assert_eq!(value["v"], 1, "status {status}");
        assert_eq!(server.hit_count(), 3, "status {status}");
    }
}

#[tokio::test]
async fn a_persistent_server_error_fails_after_four_attempts() {
    let server = Server::start(|_, _| reply(503, r#"{"message":"down"}"#)).await;
    let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
    assert!(
        matches!(err, ClientError::ApiError { status: 503, .. }),
        "got {err:?}"
    );
    assert_eq!(server.hit_count(), 4);
}

#[tokio::test]
async fn other_statuses_are_not_retried() {
    for status in [404u16, 422, 501] {
        let server = Server::start(move |_, _| reply(status, "{}")).await;
        let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
        assert!(
            matches!(err, ClientError::ApiError { status: s, .. } if s == status),
            "got {err:?}"
        );
        assert_eq!(server.hit_count(), 1, "status {status}");
    }
}

#[tokio::test]
async fn a_dropped_connection_is_retried_for_gets() {
    let server = Server::start(|_, n| {
        if n < 2 {
            Act::Drop
        } else {
            ok(json!({"v": 2}))
        }
    })
    .await;
    let value = get(&server.client(), &server.url("/x"))
        .await
        .expect("retried");
    assert_eq!(value["v"], 2);
    assert_eq!(server.hit_count(), 3);
}

#[tokio::test]
async fn a_server_that_never_answers_times_out_and_is_retried() {
    let server = Server::start(|_, _| Act::Hang).await;
    let mut client = server.client();
    client.timing.request_timeout = Duration::from_millis(100);
    let err = get(&client, &server.url("/x")).await.unwrap_err();
    assert!(matches!(err, ClientError::Timeout { .. }), "got {err:?}");
    assert_eq!(server.hit_count(), 4);
}

#[tokio::test]
async fn a_body_that_stalls_midway_times_out_instead_of_hanging() {
    let server = Server::start(|_, n| {
        if n == 0 {
            Act::StallBody
        } else {
            ok(json!({"v": 3}))
        }
    })
    .await;
    let mut client = server.client();
    client.timing.request_timeout = Duration::from_millis(150);
    // First attempt stalls, the retry succeeds.
    let value = get(&client, &server.url("/x")).await.expect("retried");
    assert_eq!(value["v"], 3);

    let stuck = Server::start(|_, _| Act::StallBody).await;
    let mut client = stuck.client();
    client.timing.request_timeout = Duration::from_millis(150);
    let err = get(&client, &stuck.url("/x")).await.unwrap_err();
    assert!(matches!(err, ClientError::Timeout { .. }), "got {err:?}");
}

#[tokio::test]
async fn posts_are_not_retried_on_server_errors_or_dropped_connections() {
    for act in [503u16, 0] {
        let server = Server::start(move |_, _| {
            if act == 0 {
                Act::Drop
            } else {
                reply(act, "{}")
            }
        })
        .await;
        let result = server
            .client()
            .post_json::<Value, _>(&server.url("/issues"), &json!({"title": "t"}))
            .await;
        assert!(result.is_err());
        assert_eq!(server.hit_count(), 1, "a repeated POST could create twice");
    }
}

#[tokio::test]
async fn posts_still_wait_out_rate_limits() {
    let server = Server::start(|_, n| {
        if n == 0 {
            reply_with(403, &[("Retry-After", "1")], "{}")
        } else {
            ok(json!({"id": 1}))
        }
    })
    .await;
    let value = server
        .client()
        .post_json::<Value, _>(&server.url("/issues"), &json!({"title": "t"}))
        .await
        .expect("retried");
    assert_eq!(value["id"], 1);
}

#[tokio::test]
async fn a_list_whose_second_page_keeps_failing_fails_instead_of_returning_page_one() {
    // Page 2 lives on a server that always fails.
    let failing = Server::start(|_, _| reply(502, "{}")).await;
    let next = failing.url("/list2");
    let first = Server::start(move |_, _| Act::Reply {
        status: 200,
        headers: vec![("Link", format!("<{next}>; rel=\"next\""))],
        body: "[1,2]".to_string(),
    })
    .await;

    let err = first
        .client()
        .get_all_pages::<Value>(&first.url("/list"))
        .await
        .expect_err("a partial list must not be returned");
    assert!(
        matches!(err, ClientError::ApiError { status: 502, .. }),
        "got {err:?}"
    );
    assert_eq!(
        failing.hit_count(),
        4,
        "page 2 was retried before giving up"
    );
}
