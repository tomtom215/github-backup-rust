// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub App authentication against a local plain-HTTP fake of the API:
//! installation discovery, token minting and renewal, single-flight under
//! concurrency, and what is sent with which credential.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

use github_backup_types::config::{AppCredential, Credential};

use super::GitHubClient;
use crate::app_auth::tests::test_key;
use crate::error::ClientError;

/// One request the fake saw: method, target and `Authorization` value.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    authorization: String,
}

/// A fake API.  `POST .../access_tokens` mints `ghs_minted_<n>` valid for
/// `lifetime_secs` (after `mint_delay`); `routes` answer GETs by target.
struct FakeApp {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    mints: Arc<AtomicU64>,
}

impl FakeApp {
    async fn start(
        routes: Vec<(&'static str, u16, Value)>,
        lifetime_secs: u64,
        mint_delay: Duration,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mints = Arc::new(AtomicU64::new(0));
        let (log, counter) = (Arc::clone(&seen), Arc::clone(&mints));
        let routes = Arc::new(routes);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let (log, counter, routes) =
                    (Arc::clone(&log), Arc::clone(&counter), Arc::clone(&routes));
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let mut words = head.split_whitespace();
                    let method = words.next().unwrap_or_default().to_string();
                    let target = words.next().unwrap_or_default().to_string();
                    let authorization = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .starts_with("authorization:")
                                .then(|| l["authorization:".len()..].trim().to_string())
                        })
                        .unwrap_or_default();
                    log.lock().unwrap().push(Seen {
                        method: method.clone(),
                        target: target.clone(),
                        authorization,
                    });
                    let (status, body) = if method == "POST" && target.ends_with("/access_tokens") {
                        tokio::time::sleep(mint_delay).await;
                        let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                        let expires = jiff::Timestamp::now()
                            + jiff::SignedDuration::from_secs(
                                i64::try_from(lifetime_secs).unwrap(),
                            );
                        (
                            201,
                            json!({"token": format!("ghs_minted_{n}"), "expires_at": expires.to_string(),
                                   "permissions": {"contents": "read"}, "repository_selection": "all"}),
                        )
                    } else {
                        routes
                            .iter()
                            .find(|(t, _, _)| *t == target)
                            .map_or((404, json!({"message": "Not Found"})), |(_, s, b)| {
                                (*s, b.clone())
                            })
                    };
                    let body = body.to_string();
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { base, seen, mints }
    }

    fn client(&self, installation_id: Option<u64>) -> GitHubClient {
        let (pem, _, _) = test_key();
        GitHubClient::for_tests_at(
            Credential::App(AppCredential {
                app_id: "Iv23liTest".into(),
                private_key_pem: Zeroizing::new(pem),
                installation_id,
            }),
            &self.base,
        )
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn mints(&self) -> u64 {
        self.mints.load(Ordering::SeqCst)
    }
}

fn installation(id: u64, login: &str, kind: &str) -> Value {
    json!({
        "id": id, "account": {"login": login, "type": kind}, "target_type": kind,
        "repository_selection": "selected",
        "permissions": {"contents": "read", "issues": "read", "metadata": "read"},
    })
}

/// A JWT, not a token: three dot-separated parts after `Bearer `.
fn is_jwt(authorization: &str) -> bool {
    authorization
        .strip_prefix("Bearer ")
        .is_some_and(|t| t.split('.').count() == 3 && !t.starts_with("ghs_"))
}

#[tokio::test]
async fn the_installation_is_found_on_the_organisation_with_the_apps_jwt() {
    let api = FakeApp::start(
        vec![(
            "/orgs/acme/installation",
            200,
            installation(42, "acme", "Organization"),
        )],
        3600,
        Duration::ZERO,
    )
    .await;
    let found = api
        .client(None)
        .app_installation(Some("acme"))
        .await
        .unwrap();
    assert_eq!(found.id, 42);
    assert_eq!(found.account, "acme");
    assert!(found.organization);
    assert_eq!(found.repository_selection, "selected");
    assert_eq!(found.permissions["issues"], "read");
    let seen = api.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(is_jwt(&seen[0].authorization), "{seen:?}");
}

#[tokio::test]
async fn a_user_installation_is_looked_up_after_the_organisation_one() {
    let api = FakeApp::start(
        vec![(
            "/users/alice/installation",
            200,
            installation(7, "alice", "User"),
        )],
        3600,
        Duration::ZERO,
    )
    .await;
    let found = api
        .client(None)
        .app_installation(Some("alice"))
        .await
        .unwrap();
    assert_eq!((found.id, found.organization), (7, false));
    let targets: Vec<String> = api.seen().into_iter().map(|s| s.target).collect();
    assert_eq!(
        targets,
        ["/orgs/alice/installation", "/users/alice/installation"]
    );
}

#[tokio::test]
async fn an_app_not_installed_on_the_owner_is_a_clear_error() {
    let api = FakeApp::start(vec![], 3600, Duration::ZERO).await;
    let e = api
        .client(None)
        .app_installation(Some("nobody"))
        .await
        .unwrap_err();
    assert!(matches!(e, ClientError::App(_)), "{e}");
    assert!(
        e.to_string()
            .contains("not installed on the account nobody"),
        "{e}"
    );
    // Without OWNER and without an ID there is nothing to look up.
    let e = api.client(None).app_installation(None).await.unwrap_err();
    assert!(matches!(e, ClientError::App(_)), "{e}");
}

#[tokio::test]
async fn a_configured_installation_id_is_used_directly() {
    let api = FakeApp::start(
        vec![(
            "/app/installations/9",
            200,
            installation(9, "acme", "Organization"),
        )],
        3600,
        Duration::ZERO,
    )
    .await;
    let found = api
        .client(Some(9))
        .app_installation(Some("acme"))
        .await
        .unwrap();
    assert_eq!(found.id, 9);
    assert_eq!(api.seen().len(), 1);
}

/// API requests carry the installation token, minted once with the JWT and
/// then reused while it has more than five minutes left.
#[tokio::test]
async fn requests_use_one_minted_installation_token_while_it_is_fresh() {
    let api = FakeApp::start(
        vec![(
            "/rate_limit",
            200,
            json!({"resources": {"core": {"remaining": 14_999}}}),
        )],
        3600,
        Duration::ZERO,
    )
    .await;
    let client = api.client(Some(5));
    assert_eq!(client.verify_token().await.unwrap(), Some(14_999));
    assert_eq!(client.clone().verify_token().await.unwrap(), Some(14_999));
    assert_eq!(api.mints(), 1);
    let seen = api.seen();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].target, "/app/installations/5/access_tokens");
    assert!(is_jwt(&seen[0].authorization), "{seen:?}");
    for request in &seen[1..] {
        assert_eq!(request.authorization, "Bearer ghs_minted_1", "{seen:?}");
    }
    // The fixed-token accessor never exposes the installation token.
    assert_eq!(client.token(), None);
}

/// A token with less than five minutes left is replaced before it is used.
#[tokio::test]
async fn a_token_close_to_expiry_is_renewed_before_the_next_request() {
    let api = FakeApp::start(
        vec![(
            "/rate_limit",
            200,
            json!({"resources": {"core": {"remaining": 1}}}),
        )],
        4 * 60,
        Duration::ZERO,
    )
    .await;
    let client = api.client(Some(5));
    client.verify_token().await.unwrap();
    client.verify_token().await.unwrap();
    assert_eq!(api.mints(), 2);
    let used: Vec<String> = api
        .seen()
        .into_iter()
        .filter(|s| s.target == "/rate_limit")
        .map(|s| s.authorization)
        .collect();
    assert_eq!(used, ["Bearer ghs_minted_1", "Bearer ghs_minted_2"]);
}

/// Many workers needing a token at once share one mint.
#[tokio::test]
async fn concurrent_requests_mint_a_single_token() {
    let api = FakeApp::start(
        vec![(
            "/rate_limit",
            200,
            json!({"resources": {"core": {"remaining": 1}}}),
        )],
        3600,
        Duration::from_millis(200),
    )
    .await;
    let client = api.client(Some(5));
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let c = client.clone();
            tokio::spawn(async move { c.verify_token().await })
        })
        .collect();
    for w in workers {
        w.await.unwrap().unwrap();
    }
    assert_eq!(api.mints(), 1);
}

/// git asks the same source, so it shares the API's token and its renewal.
#[tokio::test]
async fn the_git_token_follows_the_renewed_installation_token() {
    let api = FakeApp::start(vec![], 4 * 60, Duration::ZERO).await;
    let git = api.client(Some(5)).git_token().expect("a git credential");
    assert_eq!(git.current().await.unwrap(), "ghs_minted_1");
    assert_eq!(git.current().await.unwrap(), "ghs_minted_2");
    assert!(!format!("{git:?}").contains("ghs_"));
}

#[tokio::test]
async fn a_refused_jwt_is_the_apis_401() {
    let api = FakeApp::start(
        vec![(
            "/app",
            401,
            json!({"message": "A JSON web token could not be decoded"}),
        )],
        3600,
        Duration::ZERO,
    )
    .await;
    let e = api.client(None).app_name().await.unwrap_err();
    assert!(
        matches!(e, ClientError::ApiError { status: 401, .. }),
        "{e}"
    );
}

/// The installation's repositories, private ones included, are the listing;
/// repositories of other accounts are left out.
#[tokio::test]
async fn an_app_lists_the_repositories_of_its_installation() {
    let repo = |id: u64, owner: &str, private: bool| {
        json!({"id": id, "name": format!("r{id}"), "full_name": format!("{owner}/r{id}"),
               "owner": {"id": 1, "login": owner, "type": "Organization", "avatar_url": "", "html_url": ""},
               "private": private, "fork": false, "description": null, "node_id": format!("R_{id}"),
               "html_url": "", "clone_url": "", "ssh_url": "", "temp_clone_token": "secret"})
    };
    let api = FakeApp::start(
        vec![(
            "/installation/repositories?per_page=100",
            200,
            json!({"total_count": 3, "repository_selection": "all",
                   "repositories": [repo(1, "acme", true), repo(2, "ACME", false), repo(3, "other", false)]}),
        )],
        3600,
        Duration::ZERO,
    )
    .await;
    let client = api.client(Some(5));
    let repos = client.list_org_repos("acme").await.unwrap();
    let ids: Vec<u64> = repos
        .iter()
        .map(|r| r.json()["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [1, 2]);
    assert!(repos
        .iter()
        .all(|r| r.json().get("temp_clone_token").is_none()));
    // A user installation lists the same way (no `GET /user`).
    client.list_user_repos("acme").await.unwrap();
    assert!(api.seen().iter().all(|s| s.target != "/user"));
}

#[test]
fn an_unusable_key_fails_client_construction() {
    let e = GitHubClient::new(Credential::App(AppCredential {
        app_id: "1".into(),
        private_key_pem: Zeroizing::new("garbage".into()),
        installation_id: None,
    }))
    .unwrap_err();
    assert!(matches!(e, ClientError::App(_)), "{e}");
}
