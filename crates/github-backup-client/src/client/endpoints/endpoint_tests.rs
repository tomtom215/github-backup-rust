// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Request-target tests for the endpoint methods, against a local plain-HTTP
//! server that answers by exact request target and records every target.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use github_backup_types::config::Credential;

use crate::GitHubClient;

struct FakeApi {
    base: String,
    targets: Arc<Mutex<Vec<String>>>,
}

impl FakeApi {
    /// Serves `routes` (target → JSON body); anything else answers 404.
    async fn start(routes: Vec<(String, Value)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let routes: Arc<HashMap<String, Value>> = Arc::new(routes.into_iter().collect());
        let targets = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&targets);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let routes = Arc::clone(&routes);
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let target = head
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_string();
                    log.lock().expect("log").push(target.clone());
                    let (status, body) = match routes.get(&target) {
                        Some(body) => (200, body.to_string()),
                        None => (404, r#"{"message":"Not Found"}"#.to_string()),
                    };
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { base, targets }
    }

    fn client(&self) -> GitHubClient {
        GitHubClient::for_tests_at(Credential::Token("ghp_test".into()), &self.base)
    }

    fn targets(&self) -> Vec<String> {
        self.targets.lock().expect("log").clone()
    }
}

fn route(target: &str, body: Value) -> (String, Value) {
    (target.to_string(), body)
}

/// `release#2` used to request the protection of `release`.
#[tokio::test]
async fn branch_names_are_percent_encoded_but_keep_their_slashes() {
    let api = FakeApi::start(vec![route(
        "/repos/o/r/branches/feature/release%232/protection",
        json!({"url": "x"}),
    )])
    .await;
    let protection = api
        .client()
        .get_branch_protection("o", "r", "feature/release#2")
        .await
        .expect("encoded target");
    assert_eq!(protection, json!({"url": "x"}));
}

#[tokio::test]
async fn package_names_are_percent_encoded_and_org_packages_use_the_org_route() {
    let api = FakeApi::start(vec![
        route(
            "/orgs/acme/packages?package_type=container&per_page=100",
            json!([]),
        ),
        route(
            "/orgs/acme/packages/container/team%2Fimage/versions?per_page=100",
            json!([{"id": 1, "name": "v1", "created_at": "", "updated_at": ""}]),
        ),
    ])
    .await;
    let client = api.client();
    client
        .list_packages("acme", true, "container")
        .await
        .expect("org route");
    let versions = client
        .list_package_versions("acme", true, "container", "team/image")
        .await
        .expect("encoded name");
    assert_eq!(versions.len(), 1);
}

#[tokio::test]
async fn the_token_owner_s_packages_come_from_the_user_route() {
    let api = FakeApi::start(vec![
        route("/user", json!({"login": "me"})),
        route("/user/packages?package_type=npm&per_page=100", json!([])),
    ])
    .await;
    api.client()
        .list_packages("me", false, "npm")
        .await
        .expect("own packages include private ones");
    assert!(api
        .targets()
        .contains(&"/user/packages?package_type=npm&per_page=100".to_string()));
}

/// The comparison is paginated page by page until `total_commits` arrived,
/// without relying on a `Link` header.
#[tokio::test]
async fn compare_commits_follows_every_page() {
    let commits = |from: usize, n: usize| -> Value {
        Value::Array(
            (from..from + n)
                .map(|i| json!({"sha": format!("{i:040x}"), "commit": {}}))
                .collect(),
        )
    };
    let api = FakeApi::start(vec![
        route(
            "/repos/o/r/compare/aaa...bbb?per_page=100&page=1",
            json!({"total_commits": 260, "commits": commits(0, 100)}),
        ),
        route(
            "/repos/o/r/compare/aaa...bbb?per_page=100&page=2",
            json!({"total_commits": 260, "commits": commits(100, 100)}),
        ),
        route(
            "/repos/o/r/compare/aaa...bbb?per_page=100&page=3",
            json!({"total_commits": 260, "commits": commits(200, 60)}),
        ),
    ])
    .await;
    let page = api
        .client()
        .compare_commits("o", "r", "aaa", "bbb")
        .await
        .expect("three pages");
    assert_eq!(page.len() + page.unparsed_count(), 260);
    assert_eq!(api.targets().len(), 3);
}

#[tokio::test]
async fn project_items_are_asked_for_with_their_field_values() {
    let api = FakeApi::start(vec![
        route(
            "/orgs/acme/projectsV2?per_page=100",
            json!([{"id": 1, "number": 4}]),
        ),
        route(
            "/orgs/acme/projectsV2/4/items?per_page=100&fields=10%2C11",
            json!([{"id": 5, "fields": []}]),
        ),
        route(
            "/users/me/projectsV2/4/fields?per_page=100",
            json!([{"id": 10}]),
        ),
    ])
    .await;
    let client = api.client();
    assert_eq!(
        client.list_projects("acme", true).await.expect("org").len(),
        1
    );
    assert_eq!(
        client
            .list_project_items("acme", true, 4, &[10, 11])
            .await
            .expect("items")
            .len(),
        1
    );
    assert_eq!(
        client
            .list_project_fields("me", false, 4)
            .await
            .expect("user route")
            .len(),
        1
    );
}

#[tokio::test]
async fn edited_comments_are_listed_with_an_encoded_since() {
    let api = FakeApi::start(vec![route(
        "/repos/o/r/issues/comments?since=2024-01-01T00%3A00%3A00%2B00%3A00&per_page=100",
        json!([]),
    )])
    .await;
    api.client()
        .list_repo_issue_comments_since("o", "r", "2024-01-01T00:00:00+00:00")
        .await
        .expect("`+` must not become a space");
}

#[tokio::test]
async fn issues_are_listed_in_full_or_since_a_time() {
    let api = FakeApi::start(vec![
        route(
            "/repos/o/r/issues?state=all&per_page=100",
            json!([{"number": 1}]),
        ),
        route(
            "/repos/o/r/issues?state=all&since=2024-01-01T00%3A00%3A00Z&per_page=100",
            json!([]),
        ),
    ])
    .await;
    let client = api.client();
    let all = client.list_issues("o", "r", None).await.expect("all");
    assert_eq!(all.len() + all.unparsed_count(), 1);
    client
        .list_issues("o", "r", Some("2024-01-01T00:00:00Z"))
        .await
        .expect("since");
}

fn pulls_updated(from: usize, n: usize, updated_at: &str) -> Value {
    Value::Array(
        (from..from + n)
            .map(|i| json!({"number": i, "updated_at": updated_at}))
            .collect(),
    )
}

const PULLS_BY_UPDATE: &str = "/repos/o/r/pulls?state=all&sort=updated&direction=desc&per_page=100";

/// `GET /pulls` has no `since`: the list is read newest first, and paging
/// stops at the first page that reaches back before the watermark.
#[tokio::test]
async fn pulls_since_stop_at_the_first_page_reaching_older_ones() {
    let api = FakeApi::start(vec![
        route(
            &format!("{PULLS_BY_UPDATE}&page=1"),
            pulls_updated(0, 100, "2024-03-01T00:00:00Z"),
        ),
        route(
            &format!("{PULLS_BY_UPDATE}&page=2"),
            pulls_updated(100, 100, "2023-12-31T23:59:59Z"),
        ),
        route(
            &format!("{PULLS_BY_UPDATE}&page=3"),
            pulls_updated(200, 100, "2023-01-01T00:00:00Z"),
        ),
    ])
    .await;
    let pulls = api
        .client()
        .list_pull_requests("o", "r", Some("2024-01-01T00:00:00Z"))
        .await
        .expect("pulls");
    assert_eq!(pulls.len() + pulls.unparsed_count(), 200);
    assert_eq!(api.targets().len(), 2, "page 3 is never asked for");
}

/// A short page is the last one; a page whose last pull request has no
/// readable `updated_at` does not end the list.
#[tokio::test]
async fn pulls_since_read_on_when_in_doubt_and_stop_at_a_short_page() {
    let mut first = pulls_updated(0, 100, "2024-03-01T00:00:00Z");
    first[99] = json!({"number": 99});
    let api = FakeApi::start(vec![
        route(&format!("{PULLS_BY_UPDATE}&page=1"), first),
        route(
            &format!("{PULLS_BY_UPDATE}&page=2"),
            pulls_updated(100, 3, "2024-02-01T00:00:00Z"),
        ),
    ])
    .await;
    let pulls = api
        .client()
        .list_pull_requests("o", "r", Some("2024-01-01T00:00:00Z"))
        .await
        .expect("pulls");
    assert_eq!(pulls.len() + pulls.unparsed_count(), 103);
    assert_eq!(api.targets().len(), 2);
}

#[tokio::test]
async fn pulls_without_since_are_listed_in_full() {
    let api = FakeApi::start(vec![route(
        "/repos/o/r/pulls?state=all&per_page=100",
        pulls_updated(0, 2, "2024-03-01T00:00:00Z"),
    )])
    .await;
    let pulls = api
        .client()
        .list_pull_requests("o", "r", None)
        .await
        .expect("pulls");
    assert_eq!(pulls.len() + pulls.unparsed_count(), 2);
}

#[tokio::test]
async fn the_full_repository_never_carries_a_clone_token() {
    let api = FakeApi::start(vec![route(
        "/repos/o/r",
        json!({
            "id": 1, "name": "r", "full_name": "o/r",
            "owner": {"id": 1, "login": "o", "type": "User", "avatar_url": "", "html_url": ""},
            "private": true, "fork": false, "clone_url": "", "ssh_url": "", "html_url": "",
            "temp_clone_token": "SECRET", "parent": {"full_name": "up/r"}
        }),
    )])
    .await;
    let repo = api.client().get_repository("o", "r").await.expect("repo");
    assert!(repo.json().get("temp_clone_token").is_none());
    assert_eq!(repo.json()["parent"]["full_name"], "up/r");
}
