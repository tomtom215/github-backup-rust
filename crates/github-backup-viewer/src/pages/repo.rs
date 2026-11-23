// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository pages: overview, releases, labels and milestones.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use super::Repo;
use crate::html::{bytes, esc, seg, time, Page};
use crate::json::{self, flag, s, st, u};
use crate::site::{RenderError, Site};

/// The repository's sections, for the tab bar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Overview,
    Issues,
    Pulls,
    Discussions,
    Releases,
    Labels,
}

/// The tab bar of a repository page.
pub(crate) fn tabs(repo: &Repo, root: &str, current: Tab) -> String {
    let base = format!("{root}{}", repo.base());
    let mut out = String::from("<nav class=\"tabs\" aria-label=\"Repository\"><ul>");
    let mut tab = |t: Tab, href: &str, label: String| {
        let cur = if t == current {
            " aria-current=\"page\""
        } else {
            ""
        };
        let _ = write!(out, "<li><a href=\"{base}{href}\"{cur}>{label}</a></li>");
    };
    tab(Tab::Overview, "index.html", "Overview".into());
    tab(
        Tab::Issues,
        "issues/index.html",
        format!("Issues <span class=\"count\">{}</span>", repo.issues.len()),
    );
    tab(
        Tab::Pulls,
        "pulls/index.html",
        format!(
            "Pull requests <span class=\"count\">{}</span>",
            repo.pulls.len()
        ),
    );
    if repo.dir.join("discussions.json").is_file() {
        tab(
            Tab::Discussions,
            "discussions/index.html",
            "Discussions".into(),
        );
    }
    tab(Tab::Releases, "releases.html", "Releases".into());
    tab(Tab::Labels, "labels.html", "Labels and milestones".into());
    out.push_str("</ul></nav>\n");
    out
}

/// A page of `repo`, with the breadcrumbs and title heading.
pub(crate) fn page(repo: &Repo, rel: &str, title: &str) -> Page {
    let mut page = Page::new(format!("{}{rel}", repo.base()), title);
    if rel != "index.html" {
        page = page.crumb(format!("{}index.html", repo.base()), repo.name.clone());
        page.context = Some(repo.name.clone());
    }
    page.crumb_first("index.html", "Repositories")
}

impl Page {
    /// Puts a breadcrumb in front of the others.
    fn crumb_first(mut self, href: &str, label: &str) -> Self {
        self.crumbs.insert(0, (href.to_string(), label.to_string()));
        self
    }
}

/// Small coloured badges for a repository's state.
pub(crate) fn badges(info: &Value) -> String {
    let mut out = String::new();
    let visibility = s(info, "/visibility").unwrap_or(if flag(info, "/private") {
        "private"
    } else {
        ""
    });
    if !visibility.is_empty() {
        let _ = write!(out, "<span class=\"badge\">{}</span> ", esc(visibility));
    }
    for (key, label) in [
        ("/archived", "archived"),
        ("/fork", "fork"),
        ("/is_template", "template"),
        ("/disabled", "disabled"),
    ] {
        if flag(info, key) {
            let _ = write!(out, "<span class=\"badge badge-{label}\">{label}</span> ");
        }
    }
    out
}

/// Renders the overview, releases and labels pages; returns the number of
/// releases.
pub(crate) fn render(site: &Site, repo: &Repo) -> Result<usize, RenderError> {
    overview(site, repo)?;
    let releases = releases(site, repo)?;
    labels(site, repo)?;
    Ok(releases)
}

fn overview(site: &Site, repo: &Repo) -> Result<(), RenderError> {
    let info = &repo.info;
    let full_name = s(info, "/full_name")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{}", site.owner, repo.name));
    let mut page = page(repo, "index.html", &repo.name);
    let root = page.root.clone();
    let b = &mut page.body;
    let _ = write!(
        b,
        "<h1>{}</h1>\n{}",
        esc(&full_name),
        tabs(repo, &root, Tab::Overview)
    );
    let _ = writeln!(b, "<p>{}</p>", badges(info));
    if let Some(d) = s(info, "/description").filter(|d| !d.is_empty()) {
        let _ = writeln!(b, "<p class=\"lead\">{}</p>", esc(d));
    }
    if let Some(parent) = s(info, "/parent/full_name") {
        let _ = writeln!(b, "<p>Fork of {}</p>", esc(parent));
    }

    b.push_str("<dl class=\"facts\">\n");
    let mut fact = |k: &str, v: String| {
        if !v.is_empty() {
            let _ = writeln!(b, "<div><dt>{k}</dt><dd>{v}</dd></div>");
        }
    };
    for (key, label) in [
        ("/stargazers_count", "Stars"),
        ("/forks_count", "Forks"),
        ("/subscribers_count", "Watchers"),
        ("/open_issues_count", "Open issues and pull requests"),
    ] {
        fact(
            label,
            u(info, key).map(|n| n.to_string()).unwrap_or_default(),
        );
    }
    fact("Language", esc(st(info, "/language")));
    fact("Licence", esc(st(info, "/license/name")));
    fact("Default branch", esc(st(info, "/default_branch")));
    if let Some(home) =
        s(info, "/homepage").filter(|h| h.starts_with("https://") || h.starts_with("http://"))
    {
        fact("Homepage", format!("<a href=\"{0}\">{0}</a>", esc(home)));
    }
    for (key, label) in [
        ("/created_at", "Created"),
        ("/pushed_at", "Last push"),
        ("/updated_at", "Updated"),
    ] {
        fact(label, s(info, key).map(time).unwrap_or_default());
    }
    if let Some(url) = s(info, "/html_url") {
        fact("On GitHub", format!("<a href=\"{0}\">{0}</a>", esc(url)));
    }
    b.push_str("</dl>\n");

    let mut topics: Vec<String> = json::read(&repo.dir.join("topics.json"), &site.warnings)
        .or_else(|| info.get("topics").cloned())
        .and_then(|t| t.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect();
    topics.sort();
    if !topics.is_empty() {
        b.push_str("<h2>Topics</h2>\n<ul class=\"topics\">");
        for t in &topics {
            let _ = write!(b, "<li>{}</li>", esc(t));
        }
        b.push_str("</ul>\n");
    }

    b.push_str("<h2>Local copies</h2>\n");
    let clones = [
        (
            "Repository",
            site.git.join("repos"),
            format!("{}.git", repo.name),
            "repos",
        ),
        (
            "Repository (working tree)",
            site.git.join("repos"),
            repo.name.clone(),
            "repos",
        ),
        (
            "Wiki",
            site.git.join("wikis"),
            format!("{}.wiki.git", repo.name),
            "wikis",
        ),
    ];
    let mut any = false;
    for (label, dir, name, sub) in clones {
        let path = dir.join(&name);
        if !path.is_dir() {
            continue;
        }
        any = true;
        let href = site.owner_href(&root, &format!("git/{sub}/{}/", seg(&name)));
        let checkout = name.trim_end_matches(".git");
        let _ = writeln!(
            b,
            "<h3>{label}</h3>\n<p><a href=\"{}\">{}</a></p>\n\
             <p>Check it out with:</p>\n<pre><code>git clone {} {}</code></pre>",
            esc(&href),
            esc(&path.display().to_string()),
            esc(&shell_quote(&path)),
            esc(&shell_quote(Path::new(checkout))),
        );
    }
    if !any {
        b.push_str("<p>This backup has no git clone of the repository.</p>\n");
    }

    let raw = raw_files(&repo.dir);
    if !raw.is_empty() {
        b.push_str(
            "<h2>Backed-up data</h2>\n<p>The JSON files as GitHub returned them:</p>\n<ul class=\"files\">",
        );
        for name in raw {
            let href = site.owner_href(&root, &format!("json/repos/{}/{}", repo.seg, seg(&name)));
            let _ = write!(b, "<li><a href=\"{}\">{}</a></li>", esc(&href), esc(&name));
        }
        b.push_str("</ul>\n");
    }
    site.write(&page)
}

/// The `.json` files directly in `dir`, sorted.
fn raw_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json") && !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// `path` quoted for a POSIX shell when it needs it.
fn shell_quote(path: &Path) -> String {
    let p = path.display().to_string();
    if p.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '@' | ':'))
    {
        p
    } else {
        format!("'{}'", p.replace('\'', "'\\''"))
    }
}

/// The local file of a release asset, as the backup names it (the same rules
/// as `github-backup-core`: a tag keeps its `/` segments, each made safe; an
/// asset name is reduced to its last component).
fn asset_rel_path(tag: &str, name: &str) -> Vec<String> {
    let safe = |p: &str| {
        let p = p.replace('\0', "_");
        if p.is_empty() || p == "." || p == ".." {
            "_".to_string()
        } else {
            p
        }
    };
    let mut parts: Vec<String> = tag.split(['/', '\\']).map(safe).collect();
    parts.push(safe(name.rsplit(['/', '\\']).next().unwrap_or("")));
    parts
}

fn releases(site: &Site, repo: &Repo) -> Result<usize, RenderError> {
    let mut releases = json::read_list(&repo.dir.join("releases.json"), &site.warnings);
    releases.sort_by(|a, b| {
        let key = |r: &Value| (s(r, "/created_at").map(str::to_string), u(r, "/id"));
        key(b).cmp(&key(a))
    });
    let mut page = page(repo, "releases.html", "Releases");
    let root = page.root.clone();
    let b = &mut page.body;
    let _ = write!(b, "<h1>Releases</h1>\n{}", tabs(repo, &root, Tab::Releases));
    if releases.is_empty() {
        b.push_str("<p class=\"empty\">No releases in this backup.</p>\n");
    }
    for r in &releases {
        let tag = st(r, "/tag_name");
        let name = s(r, "/name").filter(|n| !n.is_empty()).unwrap_or(tag);
        let _ = write!(b, "<section class=\"release\">\n<h2>{}", esc(name));
        if flag(r, "/draft") {
            b.push_str(" <span class=\"badge\">draft</span>");
        }
        if flag(r, "/prerelease") {
            b.push_str(" <span class=\"badge\">pre-release</span>");
        }
        let _ = write!(
            b,
            "</h2>\n<p class=\"meta\">Tag <code>{}</code> · {}",
            esc(tag),
            esc(json::login(r))
        );
        if let Some(t) = s(r, "/published_at").or_else(|| s(r, "/created_at")) {
            let _ = write!(b, " · {}", time(t));
        }
        b.push_str("</p>\n");
        if let Some(body) = s(r, "/body").filter(|t| !t.trim().is_empty()) {
            let _ = writeln!(
                b,
                "<div class=\"md\">{}</div>",
                repo.md(site, &root, s(r, "/html_url"), body)
            );
        }
        let assets = r
            .get("assets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !assets.is_empty() {
            b.push_str("<h3>Assets</h3>\n<ul class=\"assets\">");
            for a in &assets {
                let name = st(a, "/name");
                let parts = asset_rel_path(tag, name);
                let local = parts
                    .iter()
                    .fold(repo.dir.join("release_assets"), |p, x| p.join(x));
                let size = u(a, "/size").map(bytes).unwrap_or_default();
                let link = if local.is_file() {
                    let rel: Vec<String> = parts.iter().map(|p| seg(p)).collect();
                    let href = site.owner_href(
                        &root,
                        &format!("json/repos/{}/release_assets/{}", repo.seg, rel.join("/")),
                    );
                    format!("<a href=\"{}\">{}</a> (local copy)", esc(&href), esc(name))
                } else if let Some(url) = s(a, "/browser_download_url")
                    .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
                {
                    format!("<a href=\"{}\">{}</a> (on GitHub)", esc(url), esc(name))
                } else {
                    esc(name)
                };
                let _ = write!(b, "<li>{link} <span class=\"meta\">{size}</span></li>");
            }
            b.push_str("</ul>\n");
        }
        b.push_str("</section>\n");
    }
    site.write(&page)?;
    Ok(releases.len())
}

fn labels(site: &Site, repo: &Repo) -> Result<(), RenderError> {
    let mut labels = json::read_list(&repo.dir.join("labels.json"), &site.warnings);
    labels.sort_by(|a, b| {
        st(a, "/name")
            .to_lowercase()
            .cmp(&st(b, "/name").to_lowercase())
    });
    let mut milestones = json::read_list(&repo.dir.join("milestones.json"), &site.warnings);
    milestones.sort_by_key(|m| std::cmp::Reverse(u(m, "/number")));

    let mut page = page(repo, "labels.html", "Labels and milestones");
    let root = page.root.clone();
    let b = &mut page.body;
    let _ = write!(
        b,
        "<h1>Labels and milestones</h1>\n{}<h2>Labels</h2>\n",
        tabs(repo, &root, Tab::Labels)
    );
    if labels.is_empty() {
        b.push_str("<p class=\"empty\">No labels in this backup.</p>\n");
    } else {
        b.push_str(
            "<div class=\"scroll\"><table>\n<thead><tr><th scope=\"col\">Label</th>\
             <th scope=\"col\">Description</th><th scope=\"col\">Colour</th></tr></thead>\n<tbody>\n",
        );
        for l in &labels {
            let _ = writeln!(
                b,
                "<tr><th scope=\"row\"><span class=\"label\">{}</span></th><td>{}</td>\
                 <td><code>#{}</code></td></tr>",
                esc(st(l, "/name")),
                esc(st(l, "/description")),
                esc(st(l, "/color")),
            );
        }
        b.push_str("</tbody></table></div>\n");
    }
    b.push_str("<h2>Milestones</h2>\n");
    if milestones.is_empty() {
        b.push_str("<p class=\"empty\">No milestones in this backup.</p>\n");
    }
    for m in &milestones {
        let open = u(m, "/open_issues").unwrap_or(0);
        let closed = u(m, "/closed_issues").unwrap_or(0);
        let _ = write!(
            b,
            "<section class=\"milestone\"><h3>{title} <span class=\"state state-{state}\">{state}</span></h3>\n\
             <p class=\"meta\">{open} open · {closed} closed",
            title = esc(st(m, "/title")),
            state = esc(st(m, "/state")),
        );
        if let Some(due) = s(m, "/due_on") {
            let _ = write!(b, " · due {}", time(due));
        }
        b.push_str("</p>\n");
        if let Some(d) = s(m, "/description").filter(|d| !d.trim().is_empty()) {
            let _ = writeln!(
                b,
                "<div class=\"md\">{}</div>",
                repo.md(site, &root, s(m, "/html_url"), d)
            );
        }
        b.push_str("</section>\n");
    }
    site.write(&page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_follow_the_backup() {
        assert_eq!(asset_rel_path("v1", "a.tar.gz"), ["v1", "a.tar.gz"]);
        assert_eq!(asset_rel_path("release/v1", "../x"), ["release", "v1", "x"]);
        assert_eq!(asset_rel_path("..", ".."), ["_", "_"]);
    }

    #[test]
    fn shell_quoting() {
        assert_eq!(shell_quote(Path::new("/b/o/git/r.git")), "/b/o/git/r.git");
        assert_eq!(shell_quote(Path::new("/a b/it's")), "'/a b/it'\\''s'");
    }
}
