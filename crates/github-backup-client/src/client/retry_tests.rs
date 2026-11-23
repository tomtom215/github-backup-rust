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
    /// The request head (and whatever of the body came with it) of every hit.
    heads: Arc<Mutex<Vec<String>>>,
}

impl Server {
    async fn start(script: impl Fn(&str, usize) -> Act + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let heads = Arc::new(Mutex::new(Vec::<String>::new()));
        let script = Arc::new(script);
        let log = Arc::clone(&hits);
        let head_log = Arc::clone(&heads);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let script = Arc::clone(&script);
                let log = Arc::clone(&log);
                let head_log = Arc::clone(&head_log);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    head_log.lock().expect("heads").push(head.clone());
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
        Self { base, hits, heads }
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

    fn heads(&self) -> Vec<String> {
        self.heads.lock().expect("heads").clone()
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
    // Page 2 always fails.
    let server = Server::start(|target, _| match target {
        "/list" => Act::Reply {
            status: 200,
            headers: vec![("Link", "</list2>; rel=\"next\"".to_string())],
            body: "[1,2]".to_string(),
        },
        _ => reply(502, "{}"),
    })
    .await;

    let err = server
        .client()
        .get_all_pages::<Value>(&server.url("/list"))
        .await
        .expect_err("a partial list must not be returned");
    assert!(
        matches!(err, ClientError::ApiError { status: 502, .. }),
        "got {err:?}"
    );
    assert_eq!(server.hit_count(), 5, "page 2 was retried before giving up");
}

#[tokio::test]
async fn a_valid_hour_long_wait_after_secondary_limits_is_not_fatal() {
    // Three secondary limits (60 + 120 + 240 s), then a primary limit that
    // resets in an hour: every single wait is acceptable, so the request must
    // be waited through rather than failed on the accumulated total.
    let server = Server::start(|_, n| match n {
        0..=2 => reply(
            403,
            r#"{"message":"You have exceeded a secondary rate limit"}"#,
        ),
        3 => reply_with(429, &[("Retry-After", "3600")], "{}"),
        _ => ok(json!({"ok": true})),
    })
    .await;
    let mut client = server.client();
    client.timing.unit = Duration::from_micros(50);
    let value = get(&client, &server.url("/x")).await.expect("waited out");
    assert_eq!(value["ok"], true);
    assert_eq!(server.hit_count(), 5);
}

#[tokio::test]
async fn a_rate_limit_pauses_every_clone_of_the_client() {
    // GitHub's guidance: after a rate limit, stop sending, not just retry the
    // one request.  A clone asking for something else must wait too.
    let server = Server::start(|target, n| {
        if target == "/limited" && n == 0 {
            reply_with(403, &[("Retry-After", "400")], r#"{"message":"slow"}"#)
        } else {
            ok(json!({}))
        }
    })
    .await;
    let client = server.client();
    let other = client.clone();
    let started = Instant::now();
    let limited = {
        let url = server.url("/limited");
        tokio::spawn(async move { get(&client, &url).await })
    };
    // Let the first request hit the limit.
    while server.hit_count() == 0 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    get(&other, &server.url("/free")).await.expect("free");
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "the clone did not wait for the shared back-off: {:?}",
        started.elapsed()
    );
    limited.await.expect("join").expect("retried");
}

#[tokio::test]
async fn token_scopes_wait_out_a_rate_limit() {
    let server = Server::start(|_, n| {
        if n == 0 {
            reply(429, "{}")
        } else {
            reply_with(200, &[("X-OAuth-Scopes", "repo, gist")], "{}")
        }
    })
    .await;
    let scopes = server.client().get_token_scopes().await.expect("retried");
    assert_eq!(scopes, ["repo", "gist"]);
    assert_eq!(server.hit_count(), 2);
}

#[tokio::test]
async fn an_oversized_body_is_reported_as_too_large() {
    let big = "x".repeat(super::MAX_RESPONSE_BYTES + 1);
    let server = Server::start(move |_, _| reply(200, &big)).await;
    let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
    assert!(
        matches!(err, ClientError::BodyTooLarge { .. }),
        "got {err:?}"
    );
    assert_eq!(server.hit_count(), 1, "a cap is not a transient failure");
}

// ── Request headers ───────────────────────────────────────────────────────

#[tokio::test]
async fn requests_send_the_recommended_accept_and_a_pinned_api_version() {
    let server = Server::start(|_, _| ok(json!({}))).await;
    get(&server.client(), &server.url("/x")).await.expect("ok");
    let head = server.heads()[0].to_ascii_lowercase();
    assert!(
        head.contains("\r\naccept: application/vnd.github+json\r\n"),
        "{head}"
    );
    assert!(
        head.contains("\r\nx-github-api-version: 2022-11-28\r\n"),
        "{head}"
    );
}

#[tokio::test]
async fn a_request_can_override_the_api_version() {
    let server = Server::start(|_, _| ok(json!([]))).await;
    let client = server.client();
    let override_version = [(super::API_VERSION_HEADER, "2026-03-10")];
    client
        .get_json_with_headers::<Value>(&server.url("/x"), &override_version)
        .await
        .expect("ok");
    client
        .get_all_pages_with_headers::<Value>(&server.url("/list"), &override_version)
        .await
        .expect("ok");
    for head in server.heads() {
        let head = head.to_ascii_lowercase();
        assert_eq!(
            head.matches("x-github-api-version").count(),
            1,
            "replaced, not added: {head}"
        );
        assert!(head.contains("x-github-api-version: 2026-03-10"), "{head}");
    }
}

// ── Redirects ─────────────────────────────────────────────────────────────

fn redirect(status: u16, location: &str) -> Act {
    reply_with(status, &[("Location", location)], "")
}

#[tokio::test]
async fn a_301_for_a_renamed_repository_is_followed_with_the_credential() {
    let server = Server::start(|target, _| match target {
        "/repos/old/name" => redirect(301, "/repositories/42"),
        _ => ok(json!({"id": 42})),
    })
    .await;
    let value = get(&server.client(), &server.url("/repos/old/name"))
        .await
        .expect("followed");
    assert_eq!(value["id"], 42);
    let heads = server.heads();
    assert_eq!(heads.len(), 2);
    assert!(
        heads[1].starts_with("GET /repositories/42 "),
        "{}",
        heads[1]
    );
    assert!(
        heads[1].contains("ghp_dummy"),
        "same origin keeps the token"
    );
}

#[tokio::test]
async fn a_307_repeats_a_post_with_its_method_and_body() {
    let server = Server::start(|target, _| match target {
        "/repos/old/name/issues" => redirect(307, "/repositories/42/issues"),
        _ => ok(json!({"number": 1})),
    })
    .await;
    let value = server
        .client()
        .post_json::<Value, _>(&server.url("/repos/old/name/issues"), &json!({"t": 1}))
        .await
        .expect("followed");
    assert_eq!(value["number"], 1);
    let second = server.heads()[1].to_ascii_lowercase();
    assert!(
        second.starts_with("post /repositories/42/issues "),
        "{second}"
    );
    assert!(second.contains("content-length: 7\r\n"), "{second}");
}

#[tokio::test]
async fn a_post_is_not_turned_into_a_get_by_a_301() {
    let server = Server::start(|_, _| redirect(301, "/elsewhere")).await;
    let err = server
        .client()
        .post_json::<Value, _>(&server.url("/issues"), &json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::ApiError { status: 301, .. }),
        "got {err:?}"
    );
    assert_eq!(server.hit_count(), 1);
}

#[tokio::test]
async fn an_api_redirect_to_another_origin_is_not_followed() {
    let other = Server::start(|_, _| ok(json!({}))).await;
    let target = other.url("/steal");
    let server = Server::start(move |_, _| redirect(302, &target)).await;
    let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
    assert!(
        matches!(err, ClientError::ApiError { status: 302, .. }),
        "got {err:?}"
    );
    assert_eq!(other.hit_count(), 0, "the token must not leave the origin");
}

#[tokio::test]
async fn a_redirect_loop_ends_after_a_few_hops() {
    let server = Server::start(|_, _| redirect(302, "/x")).await;
    let err = get(&server.client(), &server.url("/x")).await.unwrap_err();
    assert!(err.to_string().contains("too many redirects"), "{err}");
    assert_eq!(server.hit_count(), 4);
}

// ── GraphQL ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn graphql_posts_the_query_and_returns_its_data() {
    let server = Server::start(|_, _| ok(json!({"data": {"viewer": {"login": "octocat"}}}))).await;
    let data = server
        .client()
        .graphql("query($n: Int!) { viewer { login } }", json!({"n": 1}))
        .await
        .expect("data");
    assert_eq!(data["viewer"]["login"], "octocat");
    let head = server.heads()[0].to_ascii_lowercase();
    assert!(head.starts_with("post /graphql "), "{head}");
    assert!(head.contains("authorization: bearer ghp_dummy"), "{head}");
}

#[tokio::test]
async fn graphql_errors_fail_the_query_even_next_to_partial_data() {
    let server = Server::start(|_, _| {
        ok(json!({
            "data": {"repository": null},
            "errors": [{"type": "NOT_FOUND", "message": "Could not resolve to a Repository"}]
        }))
    })
    .await;
    let err = server
        .client()
        .graphql(
            "query { repository(owner: \"o\", name: \"r\") { id } }",
            json!({}),
        )
        .await
        .unwrap_err();
    match err {
        ClientError::GraphQl { kinds, message } => {
            assert_eq!(kinds, ["NOT_FOUND"]);
            assert!(message.contains("Could not resolve"), "{message}");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn a_graphql_rate_limit_reported_with_200_is_waited_out() {
    let server = Server::start(|_, n| {
        if n == 0 {
            reply_with(
                200,
                &[("Retry-After", "2")],
                r#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#,
            )
        } else {
            ok(json!({"data": {"ok": true}}))
        }
    })
    .await;
    let data = server
        .client()
        .graphql("query { ok }", json!({}))
        .await
        .expect("retried");
    assert_eq!(data["ok"], true);
    assert_eq!(server.hit_count(), 2);
}

#[tokio::test]
async fn a_graphql_query_is_retried_after_a_server_error() {
    let server = Server::start(|_, n| {
        if n == 0 {
            reply(502, "{}")
        } else {
            ok(json!({"data": {"ok": true}}))
        }
    })
    .await;
    server
        .client()
        .graphql("query { ok }", json!({}))
        .await
        .expect("retried");
    assert_eq!(server.hit_count(), 2);
}

// ── Deprecation notices ───────────────────────────────────────────────────

#[derive(Clone, Default)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("capture").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_deprecation_notice_is_logged_once_per_endpoint() {
    let server = Server::start(|_, _| {
        reply_with(
            200,
            &[
                ("Deprecation", "@1767225600"),
                ("Sunset", "Wed, 01 Jul 2026 00:00:00 GMT"),
            ],
            "{}",
        )
    })
    .await;
    let capture = LogCapture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let client = server.client();
    for repo in ["a", "b", "c"] {
        get(&client, &server.url(&format!("/repos/o/{repo}/topics")))
            .await
            .expect("ok");
    }
    get(&client, &server.url("/repos/o/a/hooks"))
        .await
        .expect("ok");
    let log = String::from_utf8(capture.0.lock().expect("capture").clone()).expect("utf-8");
    assert_eq!(
        log.matches("deprecated").count(),
        2,
        "once for topics, once for hooks:\n{log}"
    );
    assert!(log.contains("WARN"), "{log}");
    assert!(
        log.contains("Wed, 01 Jul 2026"),
        "the sunset date is named: {log}"
    );
}
