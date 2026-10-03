// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Golden tests: GitHub payloads in, the same JSON out.
//!
//! The fixtures under `tests/fixtures/<endpoint>/` are
//!
//! * `example.json` - for each of the 32 endpoints the client calls, the
//!   example response from GitHub's OpenAPI description (`@octokit/openapi`
//!   23.0.2), and
//! * the other files - the same payloads with the `null` / absent properties
//!   the description allows but its examples never show (deleted accounts,
//!   draft advisories, empty repositories, ...), and two lists that contain
//!   elements no model can parse.
//!
//! What is asserted:
//!
//! 1. every payload parses into its typed model (no element lands in
//!    `Page::unparsed` unless the fixture is a deliberate garbage list);
//! 2. what the backup would write - `Page<T>` / `Raw<T>` serialised - equals the
//!    fixture JSON exactly, including key order, so nothing GitHub sent is lost
//!    and nothing is invented;
//! 3. an element the models cannot parse is isolated: it neither fails the list
//!    nor disappears from the output.
//!
//! The proptest suite only proves the structs are consistent with themselves;
//! this suite is what ties them to real payloads.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde_json::Value;

use github_backup_types::{
    Branch, BranchProtection, Collaborator, DeployKey, Environment, Gist, Hook, Issue,
    IssueComment, IssueEvent, Label, Milestone, Package, PackageVersion, Page, PullRequest,
    PullRequestComment, PullRequestCommit, PullRequestReview, Raw, Release, Repository,
    SecurityAdvisory, Team, User, Workflow, WorkflowRun,
};

// ── Harness ──────────────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn read_text(endpoint: &str, name: &str) -> String {
    let path = fixtures_dir().join(endpoint).join(format!("{name}.json"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn read_json(endpoint: &str, name: &str) -> Value {
    let text = read_text(endpoint, name);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{endpoint}/{name}: {e}"))
}

/// Decodes a list fixture the way the HTTP client does: array of `Value`s,
/// element by element.
fn load<T: DeserializeOwned>(endpoint: &str, name: &str) -> Page<T> {
    let Value::Array(values) = read_json(endpoint, name) else {
        panic!("{endpoint}/{name} is not a JSON array");
    };
    Page::from_values(values)
}

/// What decoding one fixture produced.
struct Decoded {
    typed: usize,
    unparsed: usize,
    /// The JSON the backup would write for it.
    written: Value,
}

fn decode_list<T: DeserializeOwned>(json: &Value) -> Decoded {
    let Value::Array(values) = json else {
        panic!("expected a JSON array");
    };
    let page: Page<T> = Page::from_values(values.clone());
    Decoded {
        typed: page.len(),
        unparsed: page.unparsed_count(),
        written: serde_json::to_value(&page).expect("serialise page"),
    }
}

fn decode_single<T: DeserializeOwned>(json: &Value) -> Decoded {
    let raw: Raw<T> = Raw::from_value(json.clone()).expect("typed parse");
    Decoded {
        typed: 1,
        unparsed: 0,
        written: serde_json::to_value(&raw).expect("serialise raw"),
    }
}

/// `{"names": ["a", "b"]}`: the topics response has no model, the client reads
/// the `names` array directly.
fn decode_names(json: &Value) -> Decoded {
    let names: Vec<String> =
        serde_json::from_value(json["names"].clone()).expect("names is an array of strings");
    Decoded {
        typed: names.len(),
        unparsed: 0,
        written: json.clone(),
    }
}

#[derive(Clone, Copy)]
enum Shape {
    /// A JSON array of objects.
    List,
    /// An object wrapping the list under this key (`{"total_count": n, key: [...]}`).
    Wrapped(&'static str),
    /// One object.
    Single,
    /// The topics response.
    Names,
}

struct Endpoint {
    dir: &'static str,
    shape: Shape,
    decode: fn(&Value) -> Decoded,
}

fn endpoints() -> Vec<Endpoint> {
    fn list(dir: &'static str, decode: fn(&Value) -> Decoded) -> Endpoint {
        Endpoint {
            dir,
            shape: Shape::List,
            decode,
        }
    }
    fn wrapped(dir: &'static str, key: &'static str, decode: fn(&Value) -> Decoded) -> Endpoint {
        Endpoint {
            dir,
            shape: Shape::Wrapped(key),
            decode,
        }
    }
    vec![
        list("user_repos", decode_list::<Repository>),
        list("org_repos", decode_list::<Repository>),
        list("followers", decode_list::<User>),
        list("following", decode_list::<User>),
        list("starred", decode_list::<Repository>),
        list("watched", decode_list::<Repository>),
        list("gists", decode_list::<Gist>),
        list("starred_gists", decode_list::<Gist>),
        list("issues", decode_list::<Issue>),
        list("issue_comments", decode_list::<IssueComment>),
        list("issue_events", decode_list::<IssueEvent>),
        list("pulls", decode_list::<PullRequest>),
        list("pull_comments", decode_list::<PullRequestComment>),
        list("pull_commits", decode_list::<PullRequestCommit>),
        list("pull_reviews", decode_list::<PullRequestReview>),
        list("labels", decode_list::<Label>),
        list("milestones", decode_list::<Milestone>),
        list("releases", decode_list::<Release>),
        list("hooks", decode_list::<Hook>),
        list("security_advisories", decode_list::<SecurityAdvisory>),
        Endpoint {
            dir: "topics",
            shape: Shape::Names,
            decode: decode_names,
        },
        list("branches", decode_list::<Branch>),
        Endpoint {
            dir: "branch_protection",
            shape: Shape::Single,
            decode: decode_single::<BranchProtection>,
        },
        list("deploy_keys", decode_list::<DeployKey>),
        list("collaborators", decode_list::<Collaborator>),
        list("org_members", decode_list::<User>),
        list("org_teams", decode_list::<Team>),
        wrapped("workflows", "workflows", decode_list::<Workflow>),
        wrapped("workflow_runs", "workflow_runs", decode_list::<WorkflowRun>),
        wrapped("environments", "environments", decode_list::<Environment>),
        list("user_packages", decode_list::<Package>),
        list("package_versions", decode_list::<PackageVersion>),
    ]
}

/// Fixtures that deliberately hold elements no model parses, with how many.
fn expected_unparsed(endpoint: &str, name: &str) -> usize {
    match (endpoint, name) {
        ("issues", "mixed_with_garbage") => 3,
        ("pulls", "mixed_with_garbage") => 2,
        _ => 0,
    }
}

fn fixture_names(endpoint: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures_dir().join(endpoint))
        .unwrap_or_else(|e| panic!("read fixtures/{endpoint}: {e}"))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names
}

/// The part of a fixture that holds the elements.
fn payload(shape: Shape, json: &Value) -> &Value {
    match shape {
        Shape::Wrapped(key) => &json[key],
        _ => json,
    }
}

fn element_count(shape: Shape, payload: &Value) -> usize {
    match shape {
        Shape::Single => 1,
        Shape::Names => payload["names"].as_array().map_or(0, Vec::len),
        Shape::List | Shape::Wrapped(_) => payload.as_array().map_or(0, Vec::len),
    }
}

// ── 1 + 2: every fixture parses and is written back exactly ──────────────────

#[test]
fn every_fixture_parses_and_is_written_back_exactly() {
    let mut checked = 0;
    for endpoint in endpoints() {
        for name in fixture_names(endpoint.dir) {
            let label = format!("{}/{name}", endpoint.dir);
            let text = read_text(endpoint.dir, &name);
            let json: Value = serde_json::from_str(&text).expect(&label);
            let relevant = payload(endpoint.shape, &json);

            let decoded = (endpoint.decode)(relevant);

            assert_eq!(
                decoded.unparsed,
                expected_unparsed(endpoint.dir, &name),
                "{label}: unexpected number of elements the typed model could not parse"
            );
            assert_eq!(
                decoded.typed + decoded.unparsed,
                element_count(endpoint.shape, relevant),
                "{label}: an element went missing"
            );

            if decoded.unparsed == 0 {
                // Exact: same JSON, same key order, same bytes as the fixture
                // file (the fixtures use serde_json's pretty format).
                assert_eq!(&decoded.written, relevant, "{label}: JSON differs");
                assert_eq!(
                    serde_json::to_string(&decoded.written).expect("compact"),
                    serde_json::to_string(relevant).expect("compact"),
                    "{label}: key order differs"
                );
                if !matches!(endpoint.shape, Shape::Wrapped(_)) {
                    let pretty = serde_json::to_string_pretty(&decoded.written).expect("pretty");
                    assert_eq!(
                        format!("{pretty}\n"),
                        text,
                        "{label}: written bytes differ from the fixture file"
                    );
                }
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 54,
        "expected at least 54 fixtures, saw {checked}"
    );
}

#[test]
fn there_is_one_base_example_per_endpoint_and_no_orphan_fixture_directory() {
    let table: BTreeSet<&str> = endpoints().iter().map(|e| e.dir).collect();
    assert_eq!(table.len(), 32, "32 endpoints are called by the client");

    let on_disk: BTreeSet<String> = fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .map(|entry| entry.expect("dir entry"))
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let on_disk: BTreeSet<&str> = on_disk.iter().map(String::as_str).collect();
    assert_eq!(
        on_disk, table,
        "fixture directories and endpoint table differ"
    );

    for dir in &table {
        assert!(
            fixture_names(dir).contains(&"example".to_string()),
            "{dir}: missing example.json"
        );
    }

    let variants: usize = table.iter().map(|d| fixture_names(d).len() - 1).sum();
    assert!(
        variants >= 13,
        "expected at least 13 variants, saw {variants}"
    );
}

// ── 1 + 2 in detail: what the null / absent variants mean for the models ─────

#[test]
fn issue_with_a_deleted_author() {
    let page = load::<Issue>("issues", "user_null");
    assert_eq!(page.unparsed_count(), 0);
    assert!(page[0].user.is_none());
}

#[test]
fn issue_labels_may_have_a_null_colour_and_be_bare() {
    let page = load::<Issue>("issues", "label_color_null");

    assert_eq!(page.unparsed_count(), 0);
    let issue = &page[0];
    assert!(issue.labels[0].color.is_none());
    assert_eq!(issue.labels[1].name, "bare-label");
    assert!(
        issue.assignees.is_empty(),
        "absent assignees default to empty"
    );
}

#[test]
fn issue_that_is_a_pull_request_with_a_null_stub() {
    let page = load::<Issue>("issues", "pull_request_stub_null");

    assert_eq!(page.unparsed_count(), 0);
    assert!(page[0].is_pull_request());
    assert!(page[0]
        .pull_request
        .as_ref()
        .is_some_and(|s| s.url.is_none()));
}

#[test]
fn comments_reviews_and_review_comments_from_deleted_accounts() {
    let comments = load::<IssueComment>("issue_comments", "user_null");
    let reviews = load::<PullRequestReview>("pull_reviews", "user_null_commit_id_null");
    let review_comments = load::<PullRequestComment>("pull_comments", "user_null");

    assert_eq!(
        comments.unparsed_count() + reviews.unparsed_count() + review_comments.unparsed_count(),
        0
    );
    assert!(comments[0].user.is_none());
    assert!(reviews[0].user.is_none() && reviews[0].commit_id.is_none());
    assert!(review_comments[0].user.is_none());
}

#[test]
fn pull_request_from_a_deleted_fork_and_user() {
    let page = load::<PullRequest>("pulls", "head_repo_and_user_null");

    assert_eq!(page.unparsed_count(), 0);
    assert!(page[0].user.is_none());
    assert!(page[0].head.repo.is_none());
    assert!(page[0].base.repo.is_some());
}

#[test]
fn merged_pull_request() {
    let page = load::<PullRequest>("pulls", "merged");

    assert_eq!(page.unparsed_count(), 0);
    assert_eq!(page[0].state, "closed");
    assert!(page[0].merged_at.is_some());
}

#[test]
fn commits_without_linked_accounts_or_git_identities() {
    let page = load::<PullRequestCommit>("pull_commits", "unlinked_accounts");

    assert_eq!(page.unparsed_count(), 0);
    let commit = &page[0];
    assert!(commit.author.is_none(), "author: null");
    assert!(commit.committer.is_none(), "committer: {{}}");
    assert!(commit.commit.committer.is_none(), "commit.committer: null");
    assert!(commit.commit.author.is_some());
}

#[test]
fn draft_release_has_no_publication_date() {
    let page = load::<Release>("releases", "draft_without_published_at");

    assert_eq!(page.unparsed_count(), 0);
    assert!(page[0].draft);
    assert!(page[0].published_at.is_none());
}

#[test]
fn release_assets_in_open_state_and_with_a_digest() {
    let open = load::<Release>("releases", "asset_open_state");
    let digest = load::<Release>("releases", "asset_with_digest");

    assert_eq!(open.unparsed_count() + digest.unparsed_count(), 0);
    assert_eq!(open[0].assets[0].state, "open");
    assert!(open[0].assets[0].digest.is_none());
    assert_eq!(
        digest[0].assets[0].digest.as_deref(),
        Some(format!("sha256:{}", "ab".repeat(32)).as_str())
    );
}

#[test]
fn empty_repository_has_null_timestamps() {
    let page = load::<Repository>("user_repos", "empty_repo");

    assert_eq!(page.unparsed_count(), 0);
    let repo = &page[0];
    assert!(repo.pushed_at.is_none() && repo.created_at.is_none() && repo.updated_at.is_none());
}

#[test]
fn the_documented_security_advisory_parses_with_its_vulnerabilities() {
    // This payload failed with "missing field `severity`" before the model
    // was rewritten to the OpenAPI description.
    let page = load::<SecurityAdvisory>("security_advisories", "example");

    assert_eq!(page.unparsed_count(), 0);
    let vulnerabilities = page[0].vulnerabilities.as_deref().expect("vulnerabilities");
    assert_eq!(vulnerabilities.len(), 2);
    assert!(vulnerabilities[0].patched_versions.is_some());
    assert_eq!(
        vulnerabilities[0]
            .package
            .as_ref()
            .and_then(|p| p.name.as_deref()),
        Some("a-package")
    );
}

#[test]
fn draft_advisory_has_null_severity_timestamps_and_vulnerabilities() {
    let page = load::<SecurityAdvisory>("security_advisories", "draft");

    assert_eq!(page.unparsed_count(), 0);
    let advisory = &page[0];
    assert!(advisory.severity.is_none() && advisory.vulnerabilities.is_none());
    assert!(advisory.created_at.is_none() && advisory.updated_at.is_none());
}

#[test]
fn package_without_an_owner() {
    let page = load::<Package>("user_packages", "owner_null");

    assert_eq!(page.unparsed_count(), 0);
    assert!(page[0].owner.is_none());
}

#[test]
fn workflow_run_with_a_null_status() {
    let json = read_json("workflow_runs", "status_null");
    let page: Page<WorkflowRun> = Page::from_values(
        json["workflow_runs"]
            .as_array()
            .expect("array")
            .iter()
            .cloned(),
    );

    assert_eq!(page.unparsed_count(), 0);
    assert!(page[0].status.is_none());
}

#[test]
fn branch_protection_without_strict_parses() {
    // The documented example (and the minimal variant) have no `strict`; this
    // failed with "missing field `strict`" before it became optional.
    for name in ["example", "minimal_no_strict"] {
        let raw: Raw<BranchProtection> =
            Raw::from_value(read_json("branch_protection", name)).expect(name);

        let checks = raw.required_status_checks.as_ref().expect("status checks");
        assert_eq!(checks.strict, None, "{name}");
    }
}

#[test]
fn anonymous_gist_with_a_bare_file() {
    let page = load::<Gist>("gists", "anonymous");

    assert_eq!(page.unparsed_count(), 0);
    let gist = &page[0];
    assert!(gist.owner.is_none());
    let file = &gist.files["snippet.txt"];
    assert_eq!(file.filename, "snippet.txt");
    assert_eq!(file.size, 0);
}

#[test]
fn collaborator_permissions_without_triage_and_maintain() {
    let page = load::<Collaborator>("collaborators", "permissions_without_triage");

    assert_eq!(page.unparsed_count(), 0);
    let permissions = page[0].permissions.as_ref().expect("permissions");
    assert!(permissions.pull && permissions.push);
    assert!(!permissions.triage && !permissions.maintain);
}

// ── 2: nothing the models do not name is lost, nothing is invented ───────────

#[test]
fn properties_the_models_do_not_name_survive_in_the_written_json() {
    let issues = load::<Issue>("issues", "example");
    let written = serde_json::to_value(&issues).expect("serialise");

    // The typed projection drops these...
    let typed_only = serde_json::to_value(issues[0].typed()).expect("typed");
    for key in [
        "node_id",
        "reactions",
        "author_association",
        "locked",
        "state_reason",
    ] {
        assert!(typed_only.get(key).is_none(), "{key} is not modelled");
    }
    // ...the written file keeps what GitHub sent.
    for key in ["node_id", "author_association", "locked", "state_reason"] {
        assert!(written[0].get(key).is_some(), "{key} lost from issues.json");
    }
    assert!(
        written[0]["user"].get("gravatar_id").is_some(),
        "nested user fields kept"
    );
}

#[test]
fn nothing_is_invented_for_pull_requests() {
    let pulls = load::<PullRequest>("pulls", "example");
    let written = serde_json::to_value(&pulls).expect("serialise");
    let object = written[0].as_object().expect("object");

    // The typed struct once carried these and wrote them as `null`; they exist
    // only on the single-PR endpoint, which the backup never calls.
    for key in [
        "merged",
        "commits",
        "changed_files",
        "additions",
        "deletions",
    ] {
        assert!(!object.contains_key(key), "{key} must not appear");
    }
    for key in [
        "_links",
        "requested_reviewers",
        "auto_merge",
        "diff_url",
        "merge_commit_sha",
    ] {
        assert!(object.contains_key(key), "{key} lost from pulls.json");
    }
}

#[test]
fn webhook_last_response_is_kept_although_the_model_has_no_field_for_it() {
    let page = load::<Hook>("hooks", "with_last_response");
    let written = serde_json::to_value(&page).expect("serialise");

    assert_eq!(written[0]["last_response"]["code"], 422);
    assert_eq!(written[0]["last_response"]["status"], "misconfigured");
}

#[test]
fn environment_reviewer_properties_survive() {
    let json = read_json("environments", "example");
    let page: Page<Environment> = Page::from_values(
        json["environments"]
            .as_array()
            .expect("array")
            .iter()
            .cloned(),
    );
    let written = serde_json::to_value(&page).expect("serialise");

    assert_eq!(page.unparsed_count(), 0);
    assert_eq!(
        written, json["environments"],
        "environments written unchanged"
    );
}

// ── 3: an element no model can parse is isolated, not fatal, not dropped ─────

#[test]
fn garbage_among_issues_is_isolated_and_still_written() {
    let input = read_json("issues", "mixed_with_garbage");
    let input = input.as_array().expect("array");
    let page = load::<Issue>("issues", "mixed_with_garbage");

    assert_eq!(page.len(), 2, "the two real issues stay typed");
    assert_eq!(page[0].number, 1347);
    assert_eq!(page[1].number, 1348);
    assert_eq!(page.unparsed_count(), 3);
    assert_eq!(
        page.unparsed(),
        &[input[1].clone(), input[2].clone(), input[3].clone()]
    );

    let written = serde_json::to_value(&page).expect("serialise");
    assert_eq!(
        written,
        Value::Array(vec![
            input[0].clone(),
            input[4].clone(),
            input[1].clone(),
            input[2].clone(),
            input[3].clone()
        ]),
        "typed elements first, then the unparsed ones, all verbatim"
    );
}

#[test]
fn garbage_among_pull_requests_is_isolated_and_still_written() {
    let input = read_json("pulls", "mixed_with_garbage");
    let input = input.as_array().expect("array");
    let page = load::<PullRequest>("pulls", "mixed_with_garbage");

    assert_eq!(page.len(), 2);
    assert_eq!(page.unparsed(), &[input[1].clone(), input[3].clone()]);
    assert_eq!(
        serde_json::to_value(&page)
            .expect("serialise")
            .as_array()
            .map(Vec::len),
        Some(4)
    );
}

#[test]
fn decoding_a_fixture_from_text_equals_decoding_it_from_values() {
    // `Page` can be read straight from text (as `restore`-style readers would);
    // that must agree with the element-by-element path the client uses.
    let text = read_text("issues", "mixed_with_garbage");

    let from_text: Page<Issue> = serde_json::from_str(&text).expect("tolerant parse");
    let from_values = load::<Issue>("issues", "mixed_with_garbage");

    assert_eq!(from_text, from_values);
}

// ── Determinism ──────────────────────────────────────────────────────────────

#[test]
fn serialising_every_list_fixture_twice_gives_identical_bytes() {
    for endpoint in endpoints() {
        if !matches!(endpoint.shape, Shape::List) {
            continue;
        }
        for name in fixture_names(endpoint.dir) {
            let json = read_json(endpoint.dir, &name);

            let first = serde_json::to_string_pretty(&(endpoint.decode)(&json).written);
            let second = serde_json::to_string_pretty(&(endpoint.decode)(&json).written);

            assert_eq!(
                first.expect("first"),
                second.expect("second"),
                "{}/{name}",
                endpoint.dir
            );
        }
    }
}

#[test]
fn hook_config_and_gist_file_order_follow_the_api_not_a_hash() {
    let hooks = load::<Hook>("hooks", "example");
    let keys_in_fixture: Vec<String> = read_json("hooks", "example")[0]["config"]
        .as_object()
        .expect("config")
        .keys()
        .cloned()
        .collect();
    let keys_written: Vec<String> = serde_json::to_value(&hooks).expect("serialise")[0]["config"]
        .as_object()
        .expect("config")
        .keys()
        .cloned()
        .collect();

    assert_eq!(keys_written, keys_in_fixture);
}
