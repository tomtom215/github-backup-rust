// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! End-to-end tests: render a backup written to a temporary directory and
//! check the whole site (hostile content, links, determinism, robustness).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::html::unseg;
use crate::site::MARKER;
use crate::{render_site, RenderError, RenderOptions};

const XSS_TITLE: &str = "<script>alert('title')</script>";
const XSS_USER: &str = "\"><img src=x onerror=alert(1)>";
const XSS_BODY: &str = "Hi <script>alert(1)</script> <img src=x onerror=alert(2)>\n\n\
    [click](javascript:alert(3)) [d](data:text/html,<script>alert(4)</script>)\n\n\
    <a href=\"javascript:alert(5)\" onclick=\"alert(6)\">raw</a>\n\n\
    ![pixel](https://tracker.example/p.gif) ![shot](https://github.com/user-attachments/assets/abc)\n\n\
    See #1 and #2 and https://github.com/octocat/evil/issues/1#issuecomment-11 and #999.\n\n\
    <details><summary>x</summary><svg onload=alert(7)></svg></details>";

fn write(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn user(login: &str) -> Value {
    json!({"login": login, "id": 1, "html_url": format!("https://github.com/{login}")})
}

/// A backup of `octocat` with one repository full of hostile content, one
/// plain repository, and the owner-level files.  Returns the owner directory.
fn fixture(root: &Path) -> PathBuf {
    let owner = root.join("octocat");
    let json = owner.join("json");
    let repo = json.join("repos").join("evil");
    let html = "https://github.com/octocat/evil";
    write(
        &json.join("repos.json"),
        &json!([
            {"name": "evil", "full_name": "octocat/evil"},
            {"name": "plain", "full_name": "octocat/plain", "description": "from repos.json"}
        ]),
    );
    write(
        &repo.join("info.json"),
        &json!({
            "name": "evil", "full_name": "octocat/evil", "html_url": html,
            "description": XSS_TITLE, "private": false, "visibility": "public",
            "fork": true, "archived": true, "stargazers_count": 42, "forks_count": 3,
            "language": "<b>Rust</b>", "license": {"name": "MIT License"},
            "homepage": "javascript:alert(8)", "default_branch": "main",
            "created_at": "2020-01-01T00:00:00Z", "pushed_at": "2024-05-06T07:08:09Z",
            "topics": ["b-topic", "<a-topic>"], "parent": {"full_name": "up/evil"}
        }),
    );
    let label = json!({"name": XSS_USER, "color": "ff0000", "description": XSS_TITLE});
    write(
        &repo.join("issues.json"),
        &json!([
            {"number": 1, "title": XSS_TITLE, "state": "open", "user": {"login": XSS_USER},
             "body": XSS_BODY, "labels": [label], "comments": 2,
             "html_url": format!("{html}/issues/1"),
             "created_at": "2024-01-01T00:00:00Z", "assignees": [user("a<b>")],
             "milestone": {"title": "<v1>"}},
            {"number": 2, "title": "a PR", "state": "closed", "user": user("bob"),
             "pull_request": {"url": "x", "merged_at": "2024-02-02T00:00:00Z"},
             "created_at": "2024-02-01T00:00:00Z"},
            {"number": 3, "title": "closed issue", "state": "closed", "user": null,
             "created_at": "2024-03-01T00:00:00Z"},
            {"number": 4, "title": "PR only in issues.json", "state": "open",
             "pull_request": {"url": "y"}, "user": user("carol"),
             "created_at": "2024-03-02T00:00:00Z"}
        ]),
    );
    write(
        &repo.join("issue_comments/1.json"),
        &json!([
            {"id": 11, "user": {"login": XSS_USER}, "body": XSS_BODY,
             "created_at": "2024-01-02T00:00:00Z", "author_association": "<OWNER>",
             "html_url": format!("{html}/issues/1#issuecomment-11")},
            {"id": 12, "user": user("bob"), "body": "second", "created_at": "2024-01-03T00:00:00Z"}
        ]),
    );
    write(
        &repo.join("issue_events/1.json"),
        &json!([
            {"id": 21, "event": "labeled", "actor": {"login": XSS_USER}, "label": label,
             "created_at": "2024-01-02T12:00:00Z"},
            {"id": 22, "event": "renamed", "actor": user("bob"),
             "rename": {"from": "<old>", "to": XSS_TITLE}, "created_at": "2024-01-02T13:00:00Z"},
            {"id": 23, "event": "some_<new>_event", "actor": user("bob"),
             "created_at": "2024-01-02T14:00:00Z"}
        ]),
    );
    write(
        &repo.join("pulls.json"),
        &json!([
            {"number": 2, "title": "a PR", "state": "closed", "user": user("bob"),
             "body": "Fixes #1", "merged_at": "2024-02-02T00:00:00Z",
             "html_url": format!("{html}/pull/2"),
             "head": {"label": "bob:<feature>"}, "base": {"label": "octocat:main"},
             "created_at": "2024-02-01T00:00:00Z", "labels": []}
        ]),
    );
    write(
        &repo.join("pull_reviews/2.json"),
        &json!([
            {"id": 31, "user": user("carol"), "state": "APPROVED", "body": XSS_BODY,
             "submitted_at": "2024-02-01T10:00:00Z"},
            {"id": 32, "user": user("dave"), "state": "COMMENTED", "body": "",
             "submitted_at": "2024-02-01T11:00:00Z"},
            {"id": 33, "user": user("erin"), "state": "PENDING", "body": "draft"}
        ]),
    );
    write(
        &repo.join("pull_comments/2.json"),
        &json!([
            {"id": 41, "user": user("carol"), "path": "src/<main>.rs", "line": 10,
             "diff_hunk": "@@ -1,3 +1,3 @@\n-<script>old</script>\n+new", "body": "nit",
             "created_at": "2024-02-01T10:00:00Z"},
            {"id": 42, "user": user("bob"), "path": "src/<main>.rs", "line": 10,
             "in_reply_to_id": 41, "body": "fixed", "created_at": "2024-02-01T10:30:00Z"},
            {"id": 43, "user": user("carol"), "path": "README.md", "original_line": 3,
             "line": null, "body": "outdated", "created_at": "2024-02-01T10:40:00Z"}
        ]),
    );
    write(
        &repo.join("pull_commits/2.json"),
        &json!([
            {"sha": "0123456789abcdef", "commit": {"message": "Add <thing>\n\nbody",
             "author": {"name": "Bob", "date": "2024-02-01T09:00:00Z"}}, "author": null}
        ]),
    );
    write(
        &repo.join("releases.json"),
        &json!([
            {"id": 1, "tag_name": "v1.0", "name": XSS_TITLE, "body": XSS_BODY,
             "created_at": "2024-01-01T00:00:00Z", "prerelease": true, "author": user("bob"),
             "assets": [
                {"name": "tool.tar.gz", "size": 2048,
                 "browser_download_url": "https://github.com/x/tool.tar.gz"},
                {"name": "remote only.zip", "size": 10,
                 "browser_download_url": "https://github.com/x/remote.zip"},
                {"name": "evil", "browser_download_url": "javascript:alert(9)"}
             ]},
            {"id": 2, "tag_name": "release/v2", "created_at": "2024-06-01T00:00:00Z",
             "assets": [{"name": "a b.bin", "size": 1}]}
        ]),
    );
    std::fs::create_dir_all(repo.join("release_assets/v1.0")).unwrap();
    std::fs::write(repo.join("release_assets/v1.0/tool.tar.gz"), b"tgz").unwrap();
    std::fs::create_dir_all(repo.join("release_assets/release/v2")).unwrap();
    std::fs::write(repo.join("release_assets/release/v2/a b.bin"), b"b").unwrap();
    write(
        &repo.join("labels.json"),
        &json!([label, {"name": "bug", "color": "00ff00"}]),
    );
    write(
        &repo.join("milestones.json"),
        &json!([{"number": 1, "title": XSS_TITLE, "state": "open", "description": XSS_BODY,
                  "open_issues": 1, "closed_issues": 2, "due_on": "2025-01-01T00:00:00Z"}]),
    );
    write(
        &repo.join("discussions.json"),
        &json!([{"number": 7, "title": XSS_TITLE, "body": XSS_BODY, "author": {"login": XSS_USER},
                 "category": {"name": "<Q&A>"}, "created_at": "2024-01-01T00:00:00Z"}]),
    );
    write(
        &repo.join("discussion_comments/7.json"),
        &json!([{"id": 1, "author": user("bob"), "body": "answer",
                 "replies": {"nodes": [{"author": user("carol"), "body": XSS_BODY}]}}]),
    );
    write(
        &repo.join("attachments/index.json"),
        &json!({
            "https://github.com/user-attachments/assets/abc": "abc.png",
            "https://example.com/escape": "../../../../etc/passwd"
        }),
    );
    std::fs::write(
        repo.join("attachments/abc.png"),
        // A 1x1 PNG.
        [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52,
            0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49,
            0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0, 5, 0, 1, 0xFF,
            0x89, 0x99, 0x3D, 0x1D, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ],
    )
    .unwrap();
    // A plain repository with nothing but its directory.
    std::fs::create_dir_all(json.join("repos").join("plain")).unwrap();

    write(
        &json.join("gists/index.json"),
        &json!([
            {"id": "aa11", "description": XSS_TITLE, "public": false, "owner": {"login": XSS_USER},
             "files": {"x.sh": {"filename": "x.sh"}}, "html_url": "https://gist.github.com/aa11",
             "created_at": "2024-01-01T00:00:00Z"},
            {"id": "../../etc", "description": "unsafe id"}
        ]),
    );
    write(
        &json.join("gists/aa11.json"),
        &json!({"id": "aa11", "description": XSS_TITLE, "public": false,
                "files": {"x.sh": {"filename": "x.sh", "content": "echo '<script>'"}}}),
    );
    write(
        &json.join("gists/aa11.comments.json"),
        &json!([{"id": 1, "user": user("bob"), "body": XSS_BODY}]),
    );
    write(
        &json.join("starred.json"),
        &json!([{"full_name": XSS_TITLE, "html_url": "javascript:alert(10)", "description": XSS_USER}]),
    );
    write(
        &json.join("followers.json"),
        &json!([user(XSS_USER), user("zed")]),
    );
    write(
        &json.join("projects/projects.json"),
        &json!([{"id": 1, "number": 5, "title": XSS_TITLE, "short_description": XSS_USER}]),
    );
    write(
        &json.join("projects/5/items.json"),
        &json!([{"id": 1, "content_type": "Issue",
                 "content": {"title": XSS_TITLE, "html_url": format!("{html}/issues/1")},
                 "fields": [{"name": "Status", "value": {"name": XSS_USER}},
                            {"name": "<Estimate>", "value": 3}]}]),
    );
    std::fs::create_dir_all(owner.join("git/repos/evil.git")).unwrap();
    std::fs::create_dir_all(owner.join("git/wikis/evil.wiki.git")).unwrap();
    std::fs::create_dir_all(owner.join("git/gists/aa11.git")).unwrap();
    owner
}

fn render(owner: &Path, site: &Path) -> crate::RenderReport {
    render_site(&RenderOptions {
        owner_dir: owner.to_path_buf(),
        owner: "octocat".into(),
        site_dir: site.to_path_buf(),
    })
    .unwrap()
}

/// Every file of the site, by path relative to it.
fn files(site: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(site, site, &mut out);
    out
}

/// The real tags of `html` (escaped text never contains a raw `<`): each
/// tag's lower-case name and its attributes, values unescaped.
pub(crate) fn tags(html: &str) -> Vec<(String, Vec<(String, String)>)> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = html[i..].find('<') {
        i += off + 1;
        if matches!(b.get(i), Some(b'/' | b'!')) {
            i += html[i..].find('>').unwrap();
            continue;
        }
        let name_len = html[i..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap();
        let name = html[i..i + name_len].to_ascii_lowercase();
        i += name_len;
        let mut attrs = Vec::new();
        loop {
            while b[i].is_ascii_whitespace() || b[i] == b'/' {
                i += 1;
            }
            if b[i] == b'>' {
                break;
            }
            let n_len = html[i..]
                .find(|c: char| c.is_whitespace() || c == '=' || c == '>')
                .unwrap();
            let attr = html[i..i + n_len].to_ascii_lowercase();
            i += n_len;
            let mut value = String::new();
            if b[i] == b'=' {
                i += 1;
                let q = b[i];
                assert!(
                    q == b'"' || q == b'\'',
                    "unquoted attribute {attr} in <{name}>"
                );
                let len = html[i + 1..].find(char::from(q)).unwrap();
                value = unescape(&html[i + 1..i + 1 + len]);
                i += len + 2;
            }
            attrs.push((attr, value));
        }
        out.push((name, attrs));
    }
    out
}

/// Fails unless `html` can neither run code nor load anything remote: no
/// script-capable element, no event handler, no dangerous URL.
pub(crate) fn assert_inert(html: &str) {
    for (name, attrs) in tags(html) {
        assert!(
            !matches!(
                name.as_str(),
                "script"
                    | "svg"
                    | "math"
                    | "iframe"
                    | "frame"
                    | "object"
                    | "embed"
                    | "base"
                    | "form"
                    | "style"
                    | "details"
                    | "video"
                    | "audio"
                    | "source"
                    | "template"
            ),
            "<{name}> in {html}"
        );
        if name == "meta" {
            continue;
        }
        for (attr, value) in attrs {
            assert!(!attr.starts_with("on"), "{attr} on <{name}> in {html}");
            if matches!(
                attr.as_str(),
                "href" | "src" | "action" | "formaction" | "srcset"
            ) {
                let v = value.trim().to_ascii_lowercase();
                for scheme in ["javascript:", "data:", "vbscript:", "file:"] {
                    assert!(!v.starts_with(scheme), "{attr}={value} on <{name}>");
                }
                if attr == "src" {
                    assert!(
                        !v.contains("://") && !v.starts_with("//"),
                        "remote {attr}={value}"
                    );
                }
            }
        }
    }
}

/// The values of every `attr` attribute in `html`.
fn attrs(html: &str, attr: &str) -> Vec<String> {
    tags(html)
        .into_iter()
        .flat_map(|(_, attrs)| attrs)
        .filter(|(a, _)| a == attr)
        .map(|(_, v)| v)
        .collect()
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[test]
fn hostile_content_cannot_run_or_load_anything() {
    let dir = tempfile::tempdir().unwrap();
    let owner = fixture(dir.path());
    let site = owner.join("site");
    let report = render(&owner, &site);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let pages = files(&site);
    let mut html_pages = 0;
    for (path, bytes) in &pages {
        if !path.ends_with(".html") {
            continue;
        }
        html_pages += 1;
        let html = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            html.contains(
                "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none';"
            ),
            "{path}: no CSP"
        );
        assert!(
            html.starts_with("<!DOCTYPE html>\n<html lang=\"en\">"),
            "{path}"
        );
        assert_inert(&html);
    }
    assert_eq!(html_pages, report.pages);
    // Hostile strings arrive as text.
    let issue = String::from_utf8(pages["repos/evil/issues/1.html"].clone()).unwrap();
    assert!(
        issue.contains("&lt;script&gt;alert(&#39;title&#39;)&lt;/script&gt;"),
        "{issue}"
    );
    assert!(
        issue.contains("&quot;&gt;&lt;img src=x onerror=alert(1)&gt;"),
        "{issue}"
    );
    // The image with a local copy is shown, the tracker is only a link.
    assert!(
        issue.contains("<img src=\"../../../../json/repos/evil/attachments/abc.png\" alt=\"shot\""),
        "{issue}"
    );
    assert!(
        issue.contains("<a href=\"https://tracker.example/p.gif\">Image: pixel</a>"),
        "{issue}"
    );
    // References become local links; unknown ones stay text.
    assert!(
        issue.contains("<a href=\"../../../repos/evil/pulls/2.html\">#2</a>"),
        "{issue}"
    );
    assert!(
        issue.contains("href=\"../../../repos/evil/issues/1.html#issuecomment-11\""),
        "{issue}"
    );
    assert!(issue.contains(" #999."), "{issue}");
}

/// Every relative `href`/`src` of every page points at a file that exists
/// (pages of the site, or backup files and clones), and every in-site
/// fragment at an element with that id.
#[test]
fn every_relative_link_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let owner = fixture(dir.path());
    let site = owner.join("site");
    render(&owner, &site);
    let pages = files(&site);
    let mut checked = 0;
    for (path, bytes) in &pages {
        if !path.ends_with(".html") {
            continue;
        }
        let html = String::from_utf8(bytes.clone()).unwrap();
        let page_dir = site.join(path).parent().unwrap().to_path_buf();
        for v in attrs(&html, "href").into_iter().chain(attrs(&html, "src")) {
            if v.starts_with("https://") || v.starts_with("http://") || v.starts_with("mailto:") {
                continue;
            }
            assert!(!v.contains(':'), "{path}: unexpected absolute link {v}");
            let (target, fragment) = v.split_once('#').unwrap_or((&v, ""));
            let resolved = if target.is_empty() {
                site.join(path)
            } else {
                target
                    .trim_end_matches('/')
                    .split('/')
                    .fold(page_dir.clone(), |p, part| match part {
                        ".." => p.parent().unwrap().to_path_buf(),
                        part => p.join(unseg(part)),
                    })
            };
            assert!(
                resolved.exists(),
                "{path}: {v} -> {} does not exist",
                resolved.display()
            );
            if !fragment.is_empty() && resolved.extension().is_some_and(|e| e == "html") {
                let target_html = std::fs::read_to_string(&resolved).unwrap();
                assert!(
                    target_html.contains(&format!("id=\"{fragment}\"")),
                    "{path}: {v}: no element with id {fragment}"
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 100, "only {checked} links checked");
    for expected in [
        "index.html",
        "style.css",
        "gists.html",
        "gists/aa11.html",
        "starred.html",
        "followers.html",
        "projects/index.html",
        "projects/5.html",
        "repos/evil/index.html",
        "repos/evil/issues/index.html",
        "repos/evil/issues/closed.html",
        "repos/evil/issues/1.html",
        "repos/evil/issues/3.html",
        "repos/evil/pulls/2.html",
        "repos/evil/pulls/4.html",
        "repos/evil/releases.html",
        "repos/evil/labels.html",
        "repos/evil/discussions/index.html",
        "repos/evil/discussions/7.html",
        "repos/plain/index.html",
        "repos/plain/issues/index.html",
    ] {
        assert!(pages.contains_key(expected), "missing {expected}");
    }
    // The gist with an id that is not a plain name gets no page.
    assert!(
        !pages.keys().any(|p| p.contains("etc")),
        "{:?}",
        pages.keys()
    );
}

#[test]
fn pages_show_the_backed_up_data() {
    let dir = tempfile::tempdir().unwrap();
    let owner = fixture(dir.path());
    let site = owner.join("site");
    render(&owner, &site);
    let read = |p: &str| std::fs::read_to_string(site.join(p)).unwrap();

    let pr = read("repos/evil/pulls/2.html");
    assert!(pr.contains("state-merged"), "{pr}");
    assert!(pr.contains("approved these changes"), "{pr}");
    assert!(!pr.contains("erin"), "a pending review is not shown: {pr}");
    assert!(pr.contains("<code>src/&lt;main&gt;.rs</code>"), "{pr}");
    assert!(pr.contains("-&lt;script&gt;old&lt;/script&gt;"), "{pr}");
    let nit = pr.find(">nit<").unwrap();
    let fixed = pr.find(">fixed<").unwrap();
    assert!(nit < fixed, "the reply follows its comment");
    assert!(pr.contains("Line 3 (outdated)"), "{pr}");
    assert!(
        pr.contains("<code>0123456</code>") && pr.contains("Add &lt;thing&gt;"),
        "{pr}"
    );
    assert!(
        pr.contains("Fixes <a href=\"../../../repos/evil/issues/1.html\">#1</a>"),
        "{pr}"
    );

    let issue = read("repos/evil/issues/1.html");
    let labeled = issue.find("added the label").unwrap();
    let renamed = issue.find("changed the title").unwrap();
    let second = issue.find(">second<").unwrap();
    assert!(
        labeled < renamed && renamed < second,
        "timeline in time order"
    );
    assert!(issue.contains("some &lt;new&gt; event"), "{issue}");

    let releases = read("repos/evil/releases.html");
    assert!(releases.contains("href=\"../../../json/repos/evil/release_assets/v1.0/tool.tar.gz\">tool.tar.gz</a> (local copy)"), "{releases}");
    assert!(
        releases.contains("href=\"../../../json/repos/evil/release_assets/release/v2/a%20b.bin\""),
        "{releases}"
    );
    assert!(
        releases.contains(
            "<a href=\"https://github.com/x/remote.zip\">remote only.zip</a> (on GitHub)"
        ),
        "{releases}"
    );
    // Newest first.
    assert!(releases.find("release/v2").unwrap() < releases.find("v1.0").unwrap());

    let overview = read("repos/evil/index.html");
    assert!(overview.contains("git clone"), "{overview}");
    assert!(
        overview.contains("href=\"../../../git/repos/evil.git/\""),
        "{overview}"
    );
    assert!(
        overview.contains("href=\"../../../git/wikis/evil.wiki.git/\""),
        "{overview}"
    );
    assert!(!overview.contains("href=\"javascript"), "{overview}");
    assert!(overview.find("&lt;a-topic&gt;").unwrap() < overview.find("b-topic").unwrap());

    let plain = read("repos/plain/index.html");
    assert!(
        plain.contains("from repos.json"),
        "falls back to repos.json: {plain}"
    );

    let index = read("index.html");
    assert!(index.find(">evil<").unwrap() < index.find(">plain<").unwrap());
    assert!(index.contains("1 open of 2 issues"), "{index}");

    let gist = read("gists/aa11.html");
    assert!(gist.contains("echo &#39;&lt;script&gt;&#39;"), "{gist}");
    let project = read("projects/5.html");
    assert!(
        project.contains("<th scope=\"col\">&lt;Estimate&gt;</th>")
            && project.contains("<td>3</td>"),
        "{project}"
    );
    let discussion = read("repos/evil/discussions/7.html");
    assert!(discussion.contains("replied"), "{discussion}");
}

#[test]
fn renders_are_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let owner = fixture(dir.path());
    let first = owner.join("site");
    render(&owner, &first);
    let before = files(&first);
    render(&owner, &first);
    assert_eq!(
        before,
        files(&first),
        "re-rendering in place changes nothing"
    );
    let second = owner.join("site2");
    render(&owner, &second);
    assert_eq!(before, files(&second));
}

#[test]
fn missing_and_broken_files_are_never_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let owner = dir.path().join("octocat");
    // An empty backup.
    std::fs::create_dir_all(owner.join("json")).unwrap();
    let report = render(&owner, &owner.join("site"));
    assert_eq!((report.repositories, report.pages), (0, 1));
    assert!(report.warnings.is_empty());

    // Broken files: the pages still render, and the problems are reported.
    let repo = owner.join("json/repos/r");
    std::fs::create_dir_all(repo.join("issue_comments")).unwrap();
    std::fs::write(repo.join("info.json"), b"{broken").unwrap();
    std::fs::write(
        repo.join("issues.json"),
        br#"[{"number": 1, "title": "t", "state": "open"}, null, 3, {"no": "number"}]"#,
    )
    .unwrap();
    std::fs::write(repo.join("issue_comments/1.json"), b"not json").unwrap();
    std::fs::write(repo.join("pulls.json"), b"{}").unwrap();
    std::fs::write(owner.join("json/repos.json"), b"\xff").unwrap();
    let report = render(&owner, &owner.join("site"));
    assert_eq!(report.repositories, 1);
    assert_eq!(report.warnings.len(), 4, "{:?}", report.warnings);
    let issue = std::fs::read_to_string(owner.join("site/repos/r/issues/1.html")).unwrap();
    assert!(
        issue.contains("<h1>t <span class=\"num\">#1</span></h1>"),
        "{issue}"
    );
}

#[test]
fn only_an_empty_directory_or_an_earlier_render_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let owner = dir.path().join("octocat");
    std::fs::create_dir_all(owner.join("json")).unwrap();
    let opts = |site: PathBuf| RenderOptions {
        owner_dir: owner.clone(),
        owner: "octocat".into(),
        site_dir: site,
    };

    let precious = dir.path().join("precious");
    std::fs::create_dir_all(&precious).unwrap();
    std::fs::write(precious.join("thesis.txt"), b"keep me").unwrap();
    assert!(matches!(
        render_site(&opts(precious.clone())),
        Err(RenderError::NotASite(_))
    ));
    assert_eq!(
        std::fs::read(precious.join("thesis.txt")).unwrap(),
        b"keep me"
    );

    assert!(matches!(
        render_site(&opts(owner.join("json/site"))),
        Err(RenderError::InsideBackup(_))
    ));
    assert!(matches!(
        render_site(&opts(dir.path().to_path_buf())),
        Err(RenderError::NotASite(_))
    ));
    assert!(matches!(
        render_site(&RenderOptions {
            owner_dir: dir.path().join("nobody"),
            ..opts(dir.path().join("s"))
        }),
        Err(RenderError::NoBackup(_))
    ));

    // An earlier render is replaced as a whole: stale pages go.
    let site = dir.path().join("elsewhere/site");
    render_site(&opts(site.clone())).unwrap();
    assert!(site.join(MARKER).is_file());
    std::fs::write(site.join("stale.html"), b"old").unwrap();
    let report = render_site(&opts(site.clone())).unwrap();
    assert!(!site.join("stale.html").exists());
    assert!(site.join("index.html").is_file());
    let leftovers: Vec<_> = std::fs::read_dir(site.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["site"], "no staging directory is left behind");
    assert_eq!(report.site_dir, site.canonicalize().unwrap());
}

/// A site outside the owner directory links to the backup files with a
/// longer relative path.
#[test]
fn a_site_elsewhere_links_back_to_the_backup() {
    let dir = tempfile::tempdir().unwrap();
    let owner = fixture(dir.path());
    let site = dir.path().join("www/backup");
    render(&owner, &site);
    let overview = std::fs::read_to_string(site.join("repos/evil/index.html")).unwrap();
    assert!(
        overview.contains("href=\"../../../../octocat/git/repos/evil.git/\""),
        "{overview}"
    );
}

/// The synthetic backup used to measure rendering speed: `issues` issues and
/// as many pull requests, each with comments and events.
pub(crate) fn synthetic(root: &Path, issues: u64) -> PathBuf {
    let owner = root.join("octocat");
    let repo = owner.join("json/repos/big");
    let html = "https://github.com/octocat/big";
    write(
        &repo.join("info.json"),
        &json!({"name": "big", "full_name": "octocat/big", "html_url": html}),
    );
    let body = "Steps:\n\n1. run `cargo test`\n2. see #1 and https://example.com/x\n\n\
                | a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n";
    let mut list = Vec::new();
    let mut pulls = Vec::new();
    for n in 1..=issues * 2 {
        let item = json!({
            "number": n, "title": format!("Item {n} <with> markup"),
            "state": if n % 3 == 0 { "closed" } else { "open" },
            "user": user("someone"), "body": body, "comments": 3,
            "labels": [{"name": "bug"}], "html_url": format!("{html}/issues/{n}"),
            "created_at": "2024-01-01T00:00:00Z"
        });
        if n % 2 == 0 {
            pulls.push(item);
        } else {
            list.push(item);
        }
        let comments: Vec<Value> = (0..3)
            .map(|c| {
                json!({"id": n * 10 + c, "user": user("other"), "body": body,
                            "created_at": format!("2024-01-0{}T00:00:00Z", c + 2)})
            })
            .collect();
        write(
            &repo.join(format!("issue_comments/{n}.json")),
            &Value::Array(comments),
        );
        write(
            &repo.join(format!("issue_events/{n}.json")),
            &json!([{"id": n, "event": "labeled", "actor": user("x"), "label": {"name": "bug"},
                     "created_at": "2024-01-01T01:00:00Z"}]),
        );
    }
    write(&repo.join("issues.json"), &Value::Array(list));
    write(&repo.join("pulls.json"), &Value::Array(pulls));
    owner
}

/// Renders 10 000 issues and 10 000 pull requests and prints the time and
/// size.  Run with `cargo test --release -p github-backup-viewer -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn ten_thousand_issues() {
    let dir = tempfile::tempdir().unwrap();
    let owner = synthetic(dir.path(), 10_000);
    let start = std::time::Instant::now();
    let report = render(&owner, &owner.join("site"));
    let elapsed = start.elapsed();
    println!(
        "rendered {} pages, {} bytes, in {:.2?} ({} warnings)",
        report.pages,
        report.bytes,
        elapsed,
        report.warnings.len()
    );
    assert_eq!(report.pages, 20_000 + 4 + 4);
}
