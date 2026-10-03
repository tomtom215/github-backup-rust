// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! HTTP-level tests for pagination, lossless decoding and the choice between
//! the public and the authenticated listings, against a local plain-HTTP
//! server (no network, no GitHub).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use github_backup_types::config::Credential;

use super::GitHubClient;
use crate::error::ClientError;

/// One canned response.
struct Reply {
    status: u16,
    link_next: Option<String>,
    body: Value,
}

impl Reply {
    fn ok(body: Value) -> Self {
        Self {
            status: 200,
            link_next: None,
            body,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            link_next: None,
            body: json!({"message": "canned"}),
        }
    }

    fn with_next(mut self, target: &str) -> Self {
        self.link_next = Some(target.to_string());
        self
    }
}

/// A local API: replies by exact request target, records every target and
/// whether it carried an `Authorization` header.
struct FakeApi {
    base: String,
    requests: Arc<Mutex<Vec<(String, bool)>>>,
}

impl FakeApi {
    async fn start(routes: Vec<(&str, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let base = format!("http://127.0.0.1:{port}");
        let routes: HashMap<String, Reply> = routes
            .into_iter()
            .map(|(target, reply)| (target.to_string(), reply))
            .collect();
        let routes = Arc::new(routes);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        let next_base = base.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let routes = Arc::clone(&routes);
                let log = Arc::clone(&log);
                let next_base = next_base.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let target = head
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_string();
                    let auth = head.to_ascii_lowercase().contains("\r\nauthorization:");
                    log.lock().expect("log").push((target.clone(), auth));
                    let (status, link, body) = match routes.get(&target) {
                        Some(r) => (r.status, r.link_next.clone(), r.body.to_string()),
                        None => (404, None, r#"{"message":"no route"}"#.to_string()),
                    };
                    let link = link
                        .map(|t| format!("Link: <{next_base}{t}>; rel=\"next\"\r\n"))
                        .unwrap_or_default();
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{link}\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { base, requests }
    }

    fn targets(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("log")
            .iter()
            .map(|(t, _)| t.clone())
            .collect()
    }

    fn client(&self, credential: Credential) -> GitHubClient {
        GitHubClient::for_tests_at(credential, &self.base)
    }
}

fn token() -> Credential {
    Credential::Token("ghp_test".to_string())
}

fn repo(id: u64, name: &str, private: bool) -> Value {
    json!({
        "id": id, "full_name": format!("octocat/{name}"), "name": name,
        "owner": {"id": 1, "login": "octocat", "type": "User", "avatar_url": "", "html_url": ""},
        "private": private, "fork": false, "description": null,
        "clone_url": format!("https://github.com/octocat/{name}.git"),
        "ssh_url": format!("git@github.com:octocat/{name}.git"),
        "html_url": format!("https://github.com/octocat/{name}"),
        "node_id": format!("R_{id}")
    })
}

fn workflow(id: u64) -> Value {
    json!({
        "id": id, "name": format!("wf-{id}"), "path": ".github/workflows/ci.yml",
        "state": "active", "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2020-01-01T00:00:00Z", "url": "u", "html_url": "h",
        "badge_url": "b", "node_id": format!("W_{id}")
    })
}

fn run(id: u64) -> Value {
    json!({
        "id": id, "run_number": id, "head_sha": "abc", "event": "push",
        "status": "completed", "conclusion": "success", "workflow_id": 7,
        "created_at": "2020-01-01T00:00:00Z", "updated_at": "2020-01-01T00:00:00Z",
        "url": "u", "html_url": "h", "check_suite_id": 99
    })
}

fn environment(id: u64) -> Value {
    json!({
        "id": id, "node_id": format!("E_{id}"), "name": format!("env-{id}"),
        "url": "u", "html_url": "h",
        "created_at": "2020-01-01T00:00:00Z", "updated_at": "2020-01-01T00:00:00Z",
        "protection_rules": [{"id": 1, "node_id": "P", "type": "wait_timer", "wait_timer": 5, "prevent_self_review": false}]
    })
}

// ── Pagination of the wrapped lists (TM-15) ──────────────────────────────────

#[tokio::test]
async fn workflows_follow_every_page_and_merge_them() {
    let first = "/repos/o/r/actions/workflows?per_page=100";
    let second = "/repos/o/r/actions/workflows?per_page=100&page=2";
    let api = FakeApi::start(vec![
        (
            first,
            Reply::ok(json!({"total_count": 3, "workflows": [workflow(1), workflow(2)]}))
                .with_next(second),
        ),
        (
            second,
            Reply::ok(json!({"total_count": 3, "workflows": [workflow(3)]})),
        ),
    ])
    .await;

    let page = api
        .client(token())
        .list_workflows("o", "r")
        .await
        .expect("workflows");

    assert_eq!(page.len(), 3, "all pages merged");
    assert_eq!(page[2].id, 3);
    assert_eq!(api.targets(), [first, second]);
    // Nothing the model does not name is lost.
    let written = serde_json::to_value(&page).expect("serialise");
    assert_eq!(written[0]["node_id"], "W_1");
}

#[tokio::test]
async fn workflow_runs_follow_every_page_and_merge_them() {
    let first = "/repos/o/r/actions/workflows/7/runs?per_page=100";
    let second = "/repos/o/r/actions/workflows/7/runs?per_page=100&page=2";
    let api = FakeApi::start(vec![
        (
            first,
            Reply::ok(json!({"total_count": 2, "workflow_runs": [run(1)]})).with_next(second),
        ),
        (
            second,
            Reply::ok(json!({"total_count": 2, "workflow_runs": [run(2)]})),
        ),
    ])
    .await;

    let page = api
        .client(token())
        .list_workflow_runs("o", "r", 7)
        .await
        .expect("runs");

    assert_eq!(page.len(), 2);
    assert_eq!(api.targets(), [first, second]);
    assert_eq!(
        serde_json::to_value(&page).expect("serialise")[1]["check_suite_id"],
        99
    );
}

#[tokio::test]
async fn environments_follow_every_page_and_keep_unmodelled_properties() {
    let first = "/repos/o/r/environments?per_page=100";
    let second = "/repos/o/r/environments?per_page=100&page=2";
    let api = FakeApi::start(vec![
        (
            first,
            Reply::ok(json!({"total_count": 2, "environments": [environment(1)]}))
                .with_next(second),
        ),
        (
            second,
            Reply::ok(json!({"total_count": 2, "environments": [environment(2)]})),
        ),
    ])
    .await;

    let page = api
        .client(token())
        .list_environments("o", "r")
        .await
        .expect("environments");

    assert_eq!(page.len(), 2);
    let written = serde_json::to_value(&page).expect("serialise");
    assert_eq!(
        written[0]["protection_rules"][0]["prevent_self_review"],
        false
    );
}

#[tokio::test]
async fn a_wrapper_without_the_expected_array_is_an_error_naming_the_url() {
    let target = "/repos/o/r/actions/workflows?per_page=100";
    let api = FakeApi::start(vec![(target, Reply::ok(json!({"total_count": 0})))]).await;

    let err = api
        .client(token())
        .list_workflows("o", "r")
        .await
        .expect_err("no `workflows` key");

    let text = err.to_string();
    assert!(
        text.contains("`workflows`") && text.contains(target),
        "{text}"
    );
}

// ── Lossless, per-element decoding through the real HTTP path ────────────────

#[tokio::test]
async fn a_garbage_element_in_a_list_does_not_fail_the_list() {
    let target = "/orgs/acme/repos?type=all&per_page=100";
    let api = FakeApi::start(vec![(
        target,
        Reply::ok(json!([
            repo(1, "good", false),
            null,
            {"id": "x", "name": 5},
            repo(2, "also-good", true)
        ])),
    )])
    .await;

    let page = api
        .client(token())
        .list_org_repos("acme")
        .await
        .expect("listing succeeds");

    assert_eq!(page.len(), 2);
    assert_eq!(page.unparsed_count(), 2);
    let written = serde_json::to_value(&page).expect("serialise");
    assert_eq!(written.as_array().map(Vec::len), Some(4), "nothing dropped");
    assert_eq!(written[0]["node_id"], "R_1", "unmodelled property kept");
}

// ── Private repositories and secret gists for the account's own listing ──────

#[tokio::test]
async fn own_account_lists_private_repositories_through_user_repos() {
    let own = "/user/repos?affiliation=owner&visibility=all&per_page=100";
    let own_2 = "/user/repos?affiliation=owner&visibility=all&per_page=100&page=2";
    let api = FakeApi::start(vec![
        ("/user", Reply::ok(json!({"login": "Octocat"}))),
        (
            own,
            Reply::ok(json!([repo(1, "public", false)])).with_next(own_2),
        ),
        (own_2, Reply::ok(json!([repo(2, "secret", true)]))),
    ])
    .await;

    // The login differs in case: GitHub logins are case-insensitive.
    let page = api
        .client(token())
        .list_user_repos("octocat")
        .await
        .expect("repos");

    assert_eq!(page.len(), 2);
    assert!(
        page.iter().any(|r| r.private),
        "the private repository is listed"
    );
    assert_eq!(api.targets(), ["/user", own, own_2]);
}

#[tokio::test]
async fn other_users_keep_the_public_listing() {
    let public = "/users/someone/repos?type=all&per_page=100";
    let api = FakeApi::start(vec![
        ("/user", Reply::ok(json!({"login": "octocat"}))),
        (public, Reply::ok(json!([repo(1, "public", false)]))),
    ])
    .await;

    let page = api
        .client(token())
        .list_user_repos("someone")
        .await
        .expect("repos");

    assert_eq!(page.len(), 1);
    assert_eq!(api.targets(), ["/user", public]);
}

#[tokio::test]
async fn anonymous_clients_never_call_user_and_use_the_public_listing() {
    let public = "/users/octocat/repos?type=all&per_page=100";
    let api = FakeApi::start(vec![(public, Reply::ok(json!([])))]).await;

    api.client(Credential::Anonymous)
        .list_user_repos("octocat")
        .await
        .expect("repos");

    assert_eq!(api.targets(), [public], "no GET /user without a credential");
}

#[tokio::test]
async fn the_login_is_looked_up_once_per_client_and_shared_by_clones() {
    let own = "/user/repos?affiliation=owner&visibility=all&per_page=100";
    let gists = "/gists?per_page=100";
    let api = FakeApi::start(vec![
        ("/user", Reply::ok(json!({"login": "octocat"}))),
        (own, Reply::ok(json!([]))),
        (gists, Reply::ok(json!([]))),
    ])
    .await;
    let client = api.client(token());

    client.list_user_repos("octocat").await.expect("repos");
    client.clone().list_gists("octocat").await.expect("gists");
    client
        .list_user_repos("octocat")
        .await
        .expect("repos again");

    let user_calls = api.targets().iter().filter(|t| *t == "/user").count();
    assert_eq!(
        user_calls,
        1,
        "one GET /user for the whole client: {:?}",
        api.targets()
    );
}

#[tokio::test]
async fn a_token_that_cannot_read_user_falls_back_to_public_listings() {
    // GitHub App installation tokens get 403 on GET /user.
    let public_repos = "/users/octocat/repos?type=all&per_page=100";
    let public_gists = "/users/octocat/gists?per_page=100";
    let api = FakeApi::start(vec![
        ("/user", Reply::status(403)),
        (public_repos, Reply::ok(json!([repo(1, "public", false)]))),
        (public_gists, Reply::ok(json!([]))),
    ])
    .await;
    let client = api.client(token());

    let repos = client.list_user_repos("octocat").await.expect("falls back");
    client.list_gists("octocat").await.expect("falls back");

    assert_eq!(repos.len(), 1);
    assert_eq!(
        api.targets(),
        ["/user", public_repos, public_gists],
        "the refusal is cached: /user is asked once"
    );
}

#[tokio::test]
async fn a_revoked_token_falls_back_on_401_too() {
    let public = "/users/octocat/repos?type=all&per_page=100";
    let api = FakeApi::start(vec![
        ("/user", Reply::status(401)),
        (public, Reply::ok(json!([]))),
    ])
    .await;

    api.client(token())
        .list_user_repos("octocat")
        .await
        .expect("falls back");

    assert_eq!(api.targets(), ["/user", public]);
}

#[tokio::test]
async fn own_account_lists_secret_gists_through_gists() {
    let own = "/gists?per_page=100";
    let api = FakeApi::start(vec![
        ("/user", Reply::ok(json!({"login": "octocat"}))),
        (
            own,
            Reply::ok(json!([{
                "id": "abc", "description": null, "public": false, "owner": null,
                "files": {}, "git_pull_url": "https://gist.github.com/abc.git",
                "created_at": "a", "updated_at": "b", "html_url": "h"
            }])),
        ),
    ])
    .await;

    let page = api
        .client(token())
        .list_gists("octocat")
        .await
        .expect("gists");

    assert_eq!(page.len(), 1);
    assert!(!page[0].public, "a secret gist is listed");
    assert_eq!(api.targets(), ["/user", own]);
}

#[tokio::test]
async fn other_users_gists_keep_the_public_route() {
    let public = "/users/someone/gists?per_page=100";
    let api = FakeApi::start(vec![
        ("/user", Reply::ok(json!({"login": "octocat"}))),
        (public, Reply::ok(json!([]))),
    ])
    .await;

    api.client(token())
        .list_gists("someone")
        .await
        .expect("gists");

    assert_eq!(api.targets(), ["/user", public]);
}

#[tokio::test]
async fn a_client_error_other_than_401_403_from_user_is_not_swallowed() {
    let api = FakeApi::start(vec![("/user", Reply::status(404))]).await;

    let err = api
        .client(token())
        .list_user_repos("octocat")
        .await
        .expect_err("must propagate");

    assert!(
        matches!(err, ClientError::ApiError { status: 404, .. }),
        "{err}"
    );
}
