// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `--doctor`'s API and token check, against a local plain-HTTP server.

use std::sync::{Arc, Mutex};

use github_backup_client::GitHubClient;
use github_backup_types::config::Credential;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{check_api, check_api_with, Check, Status};

/// Answers every request with `status` and `body`; returns its base URL and
/// the request heads it saw.
async fn api(status: u16, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(reply.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (base, seen)
}

fn render(checks: &[Check]) -> String {
    checks
        .iter()
        .map(|c| format!("{:?} {} {}", c.status, c.label, c.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn run(base: &str, token: Option<&str>) -> Vec<Check> {
    let cred = token.map_or(Credential::Anonymous, |t| Credential::Token(t.into()));
    let client = GitHubClient::for_tests_at(cred, base);
    check_api_with(&client, base, token.is_some()).await
}

#[tokio::test]
async fn a_valid_token_passes_and_is_sent_to_the_api() {
    let (base, seen) = api(
        200,
        r#"{"resources":{"core":{"limit":5000,"remaining":4990,"reset":1}}}"#,
    )
    .await;
    let checks = run(&base, Some("ghp_dummy")).await;
    assert_eq!(checks.len(), 2, "{}", render(&checks));
    assert!(
        checks.iter().all(|c| c.status == Status::Pass),
        "{}",
        render(&checks)
    );
    assert!(render(&checks).contains("4990"));
    let head = seen.lock().unwrap()[0].to_ascii_lowercase();
    assert!(head.starts_with("get /rate_limit "), "{head}");
    assert!(head.contains("authorization: "), "{head}");
}

#[tokio::test]
async fn a_revoked_token_is_a_failure_not_a_pass() {
    let (base, _) = api(401, r#"{"message":"Bad credentials"}"#).await;
    let checks = run(&base, Some("ghp_revoked")).await;
    let token = checks
        .iter()
        .find(|c| c.label == "token")
        .expect("token line");
    assert_eq!(token.status, Status::Fail, "{}", render(&checks));
    assert!(token.detail.contains("401") && token.detail.contains("Bad credentials"));
    assert!(token.hint.as_deref().unwrap().contains("revoked"));
    // The server did answer: connectivity itself is fine.
    assert_eq!(checks[0].status, Status::Pass);
}

#[tokio::test]
async fn an_unreachable_api_fails_connectivity() {
    // A port nothing listens on.
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", free.local_addr().unwrap());
    drop(free);
    let checks = run(&base, Some("ghp_dummy")).await;
    assert_eq!(checks.len(), 1, "{}", render(&checks));
    assert_eq!(checks[0].label, "API connectivity");
    assert_eq!(checks[0].status, Status::Fail);
}

#[tokio::test]
async fn a_server_without_a_rate_limit_endpoint_gives_a_warning() {
    let (base, _) = api(404, r#"{"message":"Not Found"}"#).await;
    let checks = run(&base, Some("ghp_dummy")).await;
    let token = checks
        .iter()
        .find(|c| c.label == "token")
        .expect("token line");
    assert_eq!(token.status, Status::Warn);
}

#[tokio::test]
async fn anonymous_runs_check_connectivity_only() {
    let (base, _) = api(200, "{}").await;
    let checks = run(&base, None).await;
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].status, Status::Pass);
}

#[tokio::test]
async fn an_insecure_api_url_is_rejected_before_any_request() {
    let args = <crate::cli::Args as clap::Parser>::parse_from([
        "github-backup",
        "--doctor",
        "--api-url",
        "http://ghe.example.com/api/v3",
        "--token",
        "ghp_dummy",
    ]);
    let checks = check_api(&args).await;
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].label, "API url");
    assert_eq!(checks[0].status, Status::Fail);
    assert!(
        checks[0].detail.contains("https://"),
        "{}",
        checks[0].detail
    );
}
