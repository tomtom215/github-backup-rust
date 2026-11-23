// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Owner-level pages: the index, gists, projects and the people and
//! repository lists.

use std::fmt::Write as _;

use serde_json::Value;

use super::repo::badges;
use super::{parallel, Summary};
use crate::html::{esc, plural, time, Page};
use crate::json::{self, flag, s, st, text_of, u};
use crate::markdown::{self, Links};
use crate::site::{RenderError, Site};

/// The owner-level lists: file, page, title, and whether it lists people.
const LISTS: [(&str, &str, &str, bool); 6] = [
    (
        "starred.json",
        "starred.html",
        "Starred repositories",
        false,
    ),
    (
        "watched.json",
        "watched.html",
        "Watched repositories",
        false,
    ),
    ("followers.json", "followers.html", "Followers", true),
    ("following.json", "following.html", "Following", true),
    (
        "org_members.json",
        "members.html",
        "Organisation members",
        true,
    ),
    ("org_teams.json", "teams.html", "Teams", false),
];

/// Renders the index and every owner-level page.
pub(crate) fn render(site: &Site, repos: &[Summary]) -> Result<(), RenderError> {
    let mut sections: Vec<(String, String)> = Vec::new();
    if let Some(n) = gists(site)? {
        sections.push(("gists.html".into(), format!("Gists ({n})")));
    }
    if let Some(n) = projects(site)? {
        sections.push(("projects/index.html".into(), format!("Projects ({n})")));
    }
    for (file, page_name, title, people) in LISTS {
        let path = site.json.join(file);
        if !path.is_file() {
            continue;
        }
        let items = json::read_list(&path, &site.warnings);
        let mut page = Page::new(page_name, title).crumb("index.html", site.owner.clone());
        let _ = writeln!(page.body, "<h1>{title}</h1>");
        page.body.push_str(&if file == "org_teams.json" {
            teams(&items)
        } else if people {
            people_list(&items)
        } else {
            repo_list(&items)
        });
        site.write(&page)?;
        sections.push((page_name.into(), format!("{title} ({})", items.len())));
    }
    index(site, repos, &sections)
}

fn index(site: &Site, repos: &[Summary], sections: &[(String, String)]) -> Result<(), RenderError> {
    let mut page = Page::new("index.html", "Repositories");
    let b = &mut page.body;
    let _ = writeln!(b, "<h1>{}</h1>", esc(&site.owner));
    if !sections.is_empty() {
        b.push_str("<nav class=\"tabs\" aria-label=\"Owner\"><ul>");
        for (href, label) in sections {
            let _ = write!(b, "<li><a href=\"{}\">{}</a></li>", esc(href), esc(label));
        }
        b.push_str("</ul></nav>\n");
    }
    let _ = writeln!(
        b,
        "<h2>{}</h2>",
        plural(repos.len(), "repository", "repositories")
    );
    if repos.is_empty() {
        b.push_str("<p class=\"empty\">This backup has no repository metadata.</p>\n");
    } else {
        b.push_str("<ul class=\"repos\">\n");
        for r in repos {
            let _ = write!(
                b,
                "<li><h3><a href=\"repos/{}/index.html\">{}</a></h3> {}",
                r.seg,
                esc(&r.name),
                badges(&r.info)
            );
            if let Some(d) = s(&r.info, "/description").filter(|d| !d.is_empty()) {
                let _ = write!(b, "<p>{}</p>", esc(d));
            }
            let mut facts = Vec::new();
            if let Some(lang) = s(&r.info, "/language") {
                facts.push(esc(lang));
            }
            if let Some(stars) = u(&r.info, "/stargazers_count") {
                facts.push(format!("{stars} ★<span class=\"sr\"> stars</span>"));
            }
            facts.push(format!(
                "<a href=\"repos/{}/issues/index.html\">{} open of {}</a>",
                r.seg,
                r.open_issues,
                plural(r.issues, "issue", "issues")
            ));
            facts.push(format!(
                "<a href=\"repos/{}/pulls/index.html\">{} open of {}</a>",
                r.seg,
                r.open_pulls,
                plural(r.pulls, "pull request", "pull requests")
            ));
            if r.releases > 0 {
                facts.push(format!(
                    "<a href=\"repos/{}/releases.html\">{}</a>",
                    r.seg,
                    plural(r.releases, "release", "releases")
                ));
            }
            if let Some(t) = s(&r.info, "/pushed_at") {
                facts.push(format!("last push {}", time(t)));
            }
            let _ = writeln!(b, "<p class=\"meta\">{}</p></li>", facts.join(" · "));
        }
        b.push_str("</ul>\n");
    }
    site.write(&page)
}

/// A link to an `http(s)` URL from the backup, or just the text.
fn ext_link(url: Option<&str>, text: &str) -> String {
    match url.filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
        Some(u) => format!("<a href=\"{}\">{}</a>", esc(u), esc(text)),
        None => esc(text),
    }
}

fn repo_list(items: &[Value]) -> String {
    let mut sorted: Vec<&Value> = items.iter().collect();
    sorted.sort_by_key(|r| st(r, "/full_name").to_lowercase());
    let mut out = String::from("<ul class=\"repos\">\n");
    for r in sorted {
        let _ = write!(
            out,
            "<li><h3>{}</h3> {}",
            ext_link(s(r, "/html_url"), st(r, "/full_name")),
            badges(r)
        );
        if let Some(d) = s(r, "/description").filter(|d| !d.is_empty()) {
            let _ = write!(out, "<p>{}</p>", esc(d));
        }
        let mut facts = Vec::new();
        if let Some(lang) = s(r, "/language") {
            facts.push(esc(lang));
        }
        if let Some(stars) = u(r, "/stargazers_count") {
            facts.push(format!("{stars} ★<span class=\"sr\"> stars</span>"));
        }
        let _ = writeln!(out, "<p class=\"meta\">{}</p></li>", facts.join(" · "));
    }
    out.push_str("</ul>\n");
    out
}

fn people_list(items: &[Value]) -> String {
    let mut sorted: Vec<&Value> = items.iter().collect();
    sorted.sort_by_key(|p| st(p, "/login").to_lowercase());
    let mut out = String::from("<ul class=\"people\">\n");
    for p in sorted {
        let _ = writeln!(
            out,
            "<li>{}</li>",
            ext_link(s(p, "/html_url"), st(p, "/login"))
        );
    }
    out.push_str("</ul>\n");
    out
}

fn teams(items: &[Value]) -> String {
    let mut sorted: Vec<&Value> = items.iter().collect();
    sorted.sort_by_key(|t| st(t, "/name").to_lowercase());
    let mut out = String::from("<ul class=\"repos\">\n");
    for t in sorted {
        let _ = write!(
            out,
            "<li><h3>{}</h3>",
            ext_link(s(t, "/html_url"), st(t, "/name"))
        );
        if let Some(d) = s(t, "/description").filter(|d| !d.is_empty()) {
            let _ = write!(out, "<p>{}</p>", esc(d));
        }
        let _ = writeln!(out, "<p class=\"meta\">{}</p></li>", esc(st(t, "/privacy")));
    }
    out.push_str("</ul>\n");
    out
}

/// `true` for an id that can be used as a file name as it is (gist ids are
/// hexadecimal).
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Markdown with no local links and nothing relative (gists and projects
/// belong to no repository).
fn md(text: &str) -> String {
    markdown::render(
        text,
        &Links {
            base: None,
            local: &|_| None,
            issue: &|_| None,
        },
    )
}

/// The gist list and one page per gist; `None` when no gists were backed up.
fn gists(site: &Site) -> Result<Option<usize>, RenderError> {
    let dir = site.json.join("gists");
    let mut all: Vec<(Value, bool)> = Vec::new();
    for (file, starred) in [("index.json", false), ("starred_index.json", true)] {
        all.extend(
            json::read_list(&dir.join(file), &site.warnings)
                .into_iter()
                .filter(|g| s(g, "/id").is_some_and(plain_id))
                .map(|g| (g, starred)),
        );
    }
    if all.is_empty() && !dir.is_dir() {
        return Ok(None);
    }
    all.sort_by(|(a, sa), (b, sb)| {
        (sa, std::cmp::Reverse(st(a, "/created_at")), st(a, "/id")).cmp(&(
            sb,
            std::cmp::Reverse(st(b, "/created_at")),
            st(b, "/id"),
        ))
    });
    all.dedup_by(|(a, _), (b, _)| s(a, "/id") == s(b, "/id"));

    let name_of = |g: &Value| -> String {
        s(g, "/description")
            .filter(|d| !d.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                g.get("files")
                    .and_then(Value::as_object)
                    .and_then(|f| f.keys().next().cloned())
            })
            .unwrap_or_else(|| st(g, "/id").to_string())
    };

    let mut page = Page::new("gists.html", "Gists").crumb("index.html", site.owner.clone());
    let b = &mut page.body;
    b.push_str("<h1>Gists</h1>\n");
    if all.is_empty() {
        b.push_str("<p class=\"empty\">No gists in this backup.</p>\n");
    }
    let mut heading = None;
    for (g, starred) in &all {
        if heading != Some(*starred) {
            if heading.is_some() {
                b.push_str("</ul>\n");
            }
            let _ = writeln!(
                b,
                "<h2>{}</h2>\n<ul class=\"items\">",
                if *starred {
                    "Starred gists"
                } else {
                    "Own gists"
                }
            );
            heading = Some(*starred);
        }
        let id = st(g, "/id");
        let files = g
            .get("files")
            .and_then(Value::as_object)
            .map_or(0, |f| f.len());
        let _ =
            writeln!(
            b,
            "<li><a href=\"gists/{id}.html\">{}</a> {}<div class=\"meta\">{} by {} · {}</div></li>",
            esc(&name_of(g)),
            if flag(g, "/public") { "" } else { "<span class=\"badge\">secret</span>" },
            plural(files, "file", "files"),
            esc(s(g, "/owner/login").unwrap_or("ghost")),
            s(g, "/created_at").map(time).unwrap_or_default(),
        );
    }
    if heading.is_some() {
        b.push_str("</ul>\n");
    }
    site.write(&page)?;

    parallel(&all, |(listed, _)| {
        let id = st(listed, "/id");
        // The full gist (with file contents) when it was saved.
        let full = json::read(&dir.join(format!("{id}.json")), &site.warnings)
            .or_else(|| json::read(&dir.join(format!("{id}.starred.json")), &site.warnings))
            .filter(Value::is_object);
        let g = full.as_ref().unwrap_or(listed);
        let mut page = Page::new(format!("gists/{id}.html"), name_of(g))
            .crumb("index.html", site.owner.clone())
            .crumb("gists.html", "Gists");
        let root = page.root.clone();
        let b = &mut page.body;
        let _ = writeln!(b, "<h1>{}</h1>", esc(&name_of(g)));
        let _ = writeln!(
            b,
            "<p class=\"meta\">{} · created {} · {}</p>",
            if flag(g, "/public") {
                "public"
            } else {
                "secret"
            },
            s(g, "/created_at").map(time).unwrap_or_default(),
            ext_link(s(g, "/html_url"), "on GitHub")
        );
        let clone = site.git.join("gists").join(format!("{id}.git"));
        if clone.is_dir() {
            let href = site.owner_href(&root, &format!("git/gists/{id}.git/"));
            let _ = writeln!(
                b,
                "<p>Local clone: <a href=\"{}\">{}</a></p>",
                esc(&href),
                esc(&clone.display().to_string())
            );
        }
        let mut files: Vec<(&String, &Value)> = g
            .get("files")
            .and_then(Value::as_object)
            .map(|f| f.iter().collect())
            .unwrap_or_default();
        files.sort_by_key(|(name, _)| name.as_str());
        for (name, f) in files {
            let _ = write!(
                b,
                "<section class=\"file\"><h2><code>{}</code></h2>",
                esc(name)
            );
            match s(f, "/content") {
                Some(content) => {
                    let _ = write!(b, "<pre><code>{}</code></pre>", esc(content));
                }
                None => {
                    let _ = write!(
                        b,
                        "<p class=\"meta\">{} · the content is in the git clone</p>",
                        esc(st(f, "/language"))
                    );
                }
            }
            b.push_str("</section>\n");
        }
        let comments = json::read_list(&dir.join(format!("{id}.comments.json")), &site.warnings);
        if !comments.is_empty() {
            b.push_str("<h2>Comments</h2>\n");
            for c in &comments {
                let _ = writeln!(
                    b,
                    "<article class=\"comment\"><header><strong>{}</strong> commented {}</header>\
                     <div class=\"md\">{}</div></article>",
                    esc(json::login(c)),
                    s(c, "/created_at").map(time).unwrap_or_default(),
                    md(st(c, "/body"))
                );
            }
        }
        site.write(&page)
    })?;
    Ok(Some(all.len()))
}

/// The project list and one page per project; `None` without projects.
fn projects(site: &Site) -> Result<Option<usize>, RenderError> {
    let dir = site.json.join("projects");
    let path = dir.join("projects.json");
    if !path.is_file() {
        return Ok(None);
    }
    let mut projects: Vec<Value> = json::read_list(&path, &site.warnings)
        .into_iter()
        .filter(|p| u(p, "/number").is_some())
        .collect();
    projects.sort_by_key(|p| std::cmp::Reverse(u(p, "/number")));
    projects.dedup_by_key(|p| u(p, "/number"));

    let mut page =
        Page::new("projects/index.html", "Projects").crumb("index.html", site.owner.clone());
    let root = page.root.clone();
    let b = &mut page.body;
    b.push_str("<h1>Projects</h1>\n");
    if projects.is_empty() {
        b.push_str("<p class=\"empty\">No projects in this backup.</p>\n");
    } else {
        b.push_str("<ul class=\"items\">\n");
        for p in &projects {
            let n = u(p, "/number").unwrap_or_default();
            let _ = writeln!(
                b,
                "<li>{} <a href=\"{root}projects/{n}.html\">{}</a> <span class=\"num\">#{n}</span>\
                 <div class=\"meta\">{}</div></li>",
                if flag(p, "/closed") {
                    "<span class=\"state state-closed\">Closed</span>"
                } else {
                    "<span class=\"state state-open\">Open</span>"
                },
                esc(st(p, "/title")),
                esc(st(p, "/short_description")),
            );
        }
        b.push_str("</ul>\n");
    }
    site.write(&page)?;

    parallel(&projects, |p| {
        let n = u(p, "/number").unwrap_or_default();
        let title = st(p, "/title");
        let mut page = Page::new(format!("projects/{n}.html"), title)
            .crumb("index.html", site.owner.clone())
            .crumb("projects/index.html", "Projects");
        let b = &mut page.body;
        let _ = writeln!(b, "<h1>{} <span class=\"num\">#{n}</span></h1>", esc(title));
        if let Some(d) = s(p, "/short_description").filter(|d| !d.is_empty()) {
            let _ = writeln!(b, "<p class=\"lead\">{}</p>", esc(d));
        }
        if let Some(readme) = s(p, "/readme").filter(|r| !r.trim().is_empty()) {
            let _ = writeln!(b, "<div class=\"md\">{}</div>", md(readme));
        }
        let items = json::read_list(&dir.join(n.to_string()).join("items.json"), &site.warnings);
        let _ = writeln!(b, "<h2>{}</h2>", plural(items.len(), "item", "items"));
        // Every field name that has a value on some item, in first-seen order.
        let mut columns: Vec<String> = Vec::new();
        for item in &items {
            for f in item
                .get("fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let name = st(f, "/name");
                if !name.is_empty() && name != "Title" && !columns.iter().any(|c| c == name) {
                    columns.push(name.to_string());
                }
            }
        }
        if !items.is_empty() {
            b.push_str("<div class=\"scroll\"><table>\n<thead><tr><th scope=\"col\">Item</th><th scope=\"col\">Type</th>");
            for c in &columns {
                let _ = write!(b, "<th scope=\"col\">{}</th>", esc(c));
            }
            b.push_str("</tr></thead>\n<tbody>\n");
            for item in &items {
                let content = item.get("content").unwrap_or(&Value::Null);
                let title = s(content, "/title")
                    .or_else(|| {
                        item.get("fields")
                            .and_then(Value::as_array)
                            .and_then(|f| f.iter().find(|f| st(f, "/name") == "Title"))
                            .and_then(|f| f.get("value"))
                            .and_then(|v| {
                                s(v, "/raw")
                                    .or_else(|| s(v, "/text"))
                                    .or_else(|| v.as_str())
                            })
                    })
                    .unwrap_or("(untitled)");
                let _ = write!(
                    b,
                    "<tr><th scope=\"row\">{}</th><td>{}</td>",
                    ext_link(s(content, "/html_url"), title),
                    esc(st(item, "/content_type"))
                );
                for c in &columns {
                    let value = item
                        .get("fields")
                        .and_then(Value::as_array)
                        .and_then(|f| f.iter().find(|f| st(f, "/name") == c))
                        .and_then(|f| f.get("value"))
                        .map(text_of)
                        .unwrap_or_default();
                    let _ = write!(b, "<td>{}</td>", esc(&value));
                }
                b.push_str("</tr>\n");
            }
            b.push_str("</tbody></table></div>\n");
        }
        site.write(&page)
    })?;
    Ok(Some(projects.len()))
}
