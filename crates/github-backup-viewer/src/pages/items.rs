// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issues, pull requests and discussions: the list pages and one page each.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use serde_json::Value;

use super::repo::{page, tabs, Tab};
use super::{parallel, Kind, Repo};
use crate::html::{esc, plural, time};
use crate::json::{self, flag, login, s, st, u};
use crate::site::{RenderError, Site};

/// Renders the issue, pull request and discussion pages of `repo`.
pub(crate) fn render(site: &Site, repo: &Repo) -> Result<(), RenderError> {
    for kind in [Kind::Issue, Kind::Pull] {
        let items = match kind {
            Kind::Issue => &repo.issues,
            Kind::Pull => &repo.pulls,
        };
        list(site, repo, kind, items, true)?;
        list(site, repo, kind, items, false)?;
        parallel(items, |item| item_page(site, repo, kind, item))?;
    }
    discussions(site, repo)
}

/// `"issues"` / `"pulls"`: the directory of the kind's pages.
fn dir(kind: Kind) -> &'static str {
    match kind {
        Kind::Issue => "issues",
        Kind::Pull => "pulls",
    }
}

/// The state badge of an issue or pull request.
fn state_badge(item: &Value) -> String {
    let state = if s(item, "/merged_at").is_some() || s(item, "/pull_request/merged_at").is_some() {
        "merged"
    } else if flag(item, "/draft") && s(item, "/state") == Some("open") {
        "draft"
    } else {
        match s(item, "/state") {
            Some("open") => "open",
            Some("closed") => "closed",
            _ => "unknown",
        }
    };
    let label = match state {
        "merged" => "Merged",
        "draft" => "Draft",
        "open" => "Open",
        "closed" => "Closed",
        _ => "Unknown",
    };
    format!("<span class=\"state state-{state}\">{label}</span>")
}

/// An item's labels as badges.
fn labels(item: &Value) -> String {
    let mut out = String::new();
    // REST: a list; GraphQL (discussions): a connection with `nodes`.
    let list = item.get("labels").map(|l| l.get("nodes").unwrap_or(l));
    for l in list.and_then(Value::as_array).into_iter().flatten() {
        let name = l.as_str().or_else(|| s(l, "/name")).unwrap_or_default();
        if !name.is_empty() {
            let _ = write!(out, " <span class=\"label\">{}</span>", esc(name));
        }
    }
    out
}

/// One list page: open (`index.html`) or closed (`closed.html`) items.
fn list(
    site: &Site,
    repo: &Repo,
    kind: Kind,
    items: &[Value],
    open: bool,
) -> Result<(), RenderError> {
    let (noun, title, tab) = match kind {
        Kind::Issue => ("issues", "Issues", Tab::Issues),
        Kind::Pull => ("pull requests", "Pull requests", Tab::Pulls),
    };
    let file = if open { "index.html" } else { "closed.html" };
    let shown: Vec<&Value> = items
        .iter()
        .filter(|i| (s(i, "/state") == Some("open")) == open)
        .collect();
    let open_count = items
        .iter()
        .filter(|i| s(i, "/state") == Some("open"))
        .count();
    let mut page = page(repo, &format!("{}/{file}", dir(kind)), title);
    let root = page.root.clone();
    let base = format!("{root}{}{}/", repo.base(), dir(kind));
    let b = &mut page.body;
    let _ = write!(b, "<h1>{title}</h1>\n{}", tabs(repo, &root, tab));
    let cur = |is: bool| if is { " aria-current=\"page\"" } else { "" };
    let _ = writeln!(
        b,
        "<nav class=\"filter\" aria-label=\"State\"><a href=\"{base}index.html\"{}>{open_count} open</a> \
         <a href=\"{base}closed.html\"{}>{} closed</a></nav>",
        cur(open),
        cur(!open),
        items.len() - open_count,
    );
    if shown.is_empty() {
        let _ = writeln!(
            b,
            "<p class=\"empty\">No {} {noun} in this backup.</p>",
            if open { "open" } else { "closed" }
        );
    } else {
        b.push_str("<ul class=\"items\">\n");
        for item in shown {
            let n = u(item, "/number").unwrap_or_default();
            let comments = u(item, "/comments").map_or(String::new(), |c| {
                format!(
                    " · {}",
                    plural(usize::try_from(c).unwrap_or(0), "comment", "comments")
                )
            });
            let _ = writeln!(
                b,
                "<li>{} <a href=\"{base}{n}.html\">{}</a> <span class=\"num\">#{n}</span>{}\
                 <div class=\"meta\">opened {} by {}{comments}</div></li>",
                state_badge(item),
                esc(st(item, "/title")),
                labels(item),
                s(item, "/created_at").map(time).unwrap_or_default(),
                esc(login(item)),
            );
        }
        b.push_str("</ul>\n");
    }
    site.write(&page)
}

/// One entry of an item's timeline.
enum Entry<'a> {
    Comment(&'a Value),
    Event(&'a Value),
    Review(&'a Value),
}

impl Entry<'_> {
    fn key(&self) -> (String, u8, u64) {
        let (v, t, order) = match self {
            Entry::Comment(v) => (v, "/created_at", 1),
            Entry::Event(v) => (v, "/created_at", 0),
            Entry::Review(v) => (v, "/submitted_at", 2),
        };
        (st(v, t).to_string(), order, u(v, "/id").unwrap_or_default())
    }
}

/// A comment (or review, or the item's own text) as an `<article>`.
fn comment(site: &Site, repo: &Repo, root: &str, c: &Value, verb: &str, anchor: &str) -> String {
    let mut out = String::new();
    // REST objects are snake_case, the GraphQL nodes of discussions camelCase.
    let when = s(c, "/created_at")
        .or_else(|| s(c, "/submitted_at"))
        .or_else(|| s(c, "/createdAt"))
        .map(time)
        .unwrap_or_default();
    let id_attr = if anchor.is_empty() {
        String::new()
    } else {
        format!(" id=\"{}\"", esc(anchor))
    };
    let _ = write!(
        out,
        "<article class=\"comment\"{id_attr}>\n<header><strong>{}</strong> {verb} ",
        esc(login(c))
    );
    if anchor.is_empty() {
        out.push_str(&when);
    } else {
        let _ = write!(out, "<a href=\"#{}\">{when}</a>", esc(anchor));
    }
    if flag(c, "/isAnswer") {
        out.push_str(" <span class=\"state state-open\">Answer</span>");
    }
    let assoc = s(c, "/author_association").or_else(|| s(c, "/authorAssociation"));
    if let Some(assoc) = assoc.filter(|a| *a != "NONE") {
        let _ = write!(
            out,
            " <span class=\"badge\">{}</span>",
            esc(&assoc.to_lowercase())
        );
    }
    out.push_str("</header>\n");
    match s(c, "/body").filter(|b| !b.trim().is_empty()) {
        Some(body) => {
            let _ = writeln!(
                out,
                "<div class=\"md\">{}</div>",
                repo.md(site, root, s(c, "/html_url").or_else(|| s(c, "/url")), body)
            );
        }
        None => out.push_str("<p class=\"empty\">No description provided.</p>\n"),
    }
    out.push_str("</article>\n");
    out
}

/// A short commit id as `<code>`.
fn short_sha(sha: &str) -> String {
    format!("<code>{}</code>", esc(sha.get(..7).unwrap_or(sha)))
}

/// What an issue event says, as HTML (`added the label bug`).
fn event_text(e: &Value) -> String {
    let event = st(e, "/event");
    let name = |p: &str| esc(st(e, p));
    let commit = s(e, "/commit_id")
        .map(|c| format!(" in {}", short_sha(c)))
        .unwrap_or_default();
    match event {
        "labeled" => format!(
            "added the label <span class=\"label\">{}</span>",
            name("/label/name")
        ),
        "unlabeled" => format!(
            "removed the label <span class=\"label\">{}</span>",
            name("/label/name")
        ),
        "assigned" => format!("assigned {}", name("/assignee/login")),
        "unassigned" => format!("unassigned {}", name("/assignee/login")),
        "milestoned" => format!("added this to the milestone {}", name("/milestone/title")),
        "demilestoned" => format!(
            "removed this from the milestone {}",
            name("/milestone/title")
        ),
        "renamed" => format!(
            "changed the title from <del>{}</del> to <ins>{}</ins>",
            name("/rename/from"),
            name("/rename/to")
        ),
        "closed" => format!("closed this{commit}"),
        "reopened" => "reopened this".to_string(),
        "merged" => format!("merged this{commit}"),
        "referenced" => format!("referenced this{commit}"),
        "review_requested" | "review_request_removed" => {
            let who = s(e, "/requested_reviewer/login")
                .or_else(|| s(e, "/requested_team/name"))
                .unwrap_or_default();
            let verb = if event == "review_requested" {
                "requested a review from"
            } else {
                "removed the review request for"
            };
            format!("{verb} {}", esc(who))
        }
        "head_ref_deleted" => "deleted the head branch".to_string(),
        "head_ref_restored" => "restored the head branch".to_string(),
        "head_ref_force_pushed" => "force-pushed the head branch".to_string(),
        "convert_to_draft" => "marked this as a draft".to_string(),
        "ready_for_review" => "marked this as ready for review".to_string(),
        "locked" => "locked the conversation".to_string(),
        "unlocked" => "unlocked the conversation".to_string(),
        other => esc(&other.replace('_', " ")),
    }
}

/// One issue or pull request page.
fn item_page(site: &Site, repo: &Repo, kind: Kind, item: &Value) -> Result<(), RenderError> {
    let n = u(item, "/number").unwrap_or_default();
    let title = st(item, "/title");
    let mut page = page(
        repo,
        &format!("{}/{n}.html", dir(kind)),
        &format!("{title} #{n}"),
    );
    page = page.crumb(
        format!("{}{}/index.html", repo.base(), dir(kind)),
        match kind {
            Kind::Issue => "Issues",
            Kind::Pull => "Pull requests",
        },
    );
    let root = page.root.clone();
    let file = format!("{n}.json");
    let comments = json::read_list(&repo.dir.join("issue_comments").join(&file), &site.warnings);
    let events = json::read_list(&repo.dir.join("issue_events").join(&file), &site.warnings);
    let (reviews, review_comments, commits) = if kind == Kind::Pull {
        (
            json::read_list(&repo.dir.join("pull_reviews").join(&file), &site.warnings),
            json::read_list(&repo.dir.join("pull_comments").join(&file), &site.warnings),
            json::read_list(&repo.dir.join("pull_commits").join(&file), &site.warnings),
        )
    } else {
        Default::default()
    };

    let b = &mut page.body;
    let _ = write!(
        b,
        "<h1>{} <span class=\"num\">#{n}</span></h1>\n{}",
        esc(title),
        tabs(
            repo,
            &root,
            if kind == Kind::Issue {
                Tab::Issues
            } else {
                Tab::Pulls
            }
        )
    );
    let _ = write!(
        b,
        "<p class=\"meta\">{} <strong>{}</strong> opened this {}",
        state_badge(item),
        esc(login(item)),
        s(item, "/created_at").map(time).unwrap_or_default()
    );
    if let Some(closed) = s(item, "/merged_at").or_else(|| s(item, "/closed_at")) {
        let verb = if s(item, "/merged_at").is_some() {
            "merged"
        } else {
            "closed"
        };
        let _ = write!(b, " · {verb} {}", time(closed));
    }
    let _ = writeln!(
        b,
        " · {}</p>",
        plural(comments.len(), "comment", "comments")
    );
    if kind == Kind::Pull {
        if let (Some(head), Some(base)) = (s(item, "/head/label"), s(item, "/base/label")) {
            let _ = writeln!(
                b,
                "<p class=\"meta\">Wants to merge <code>{}</code> into <code>{}</code></p>",
                esc(head),
                esc(base)
            );
        }
    }

    b.push_str("<dl class=\"facts\">\n");
    let l = labels(item);
    if !l.is_empty() {
        let _ = writeln!(b, "<div><dt>Labels</dt><dd>{l}</dd></div>");
    }
    let assignees: Vec<String> = item
        .get("assignees")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| s(a, "/login").map(esc))
        .collect();
    if !assignees.is_empty() {
        let _ = writeln!(
            b,
            "<div><dt>Assignees</dt><dd>{}</dd></div>",
            assignees.join(", ")
        );
    }
    if let Some(m) = s(item, "/milestone/title") {
        let _ = writeln!(b, "<div><dt>Milestone</dt><dd>{}</dd></div>", esc(m));
    }
    if let Some(url) = s(item, "/html_url") {
        let _ = writeln!(
            b,
            "<div><dt>On GitHub</dt><dd><a href=\"{0}\">{0}</a></dd></div>",
            esc(url)
        );
    }
    b.push_str("</dl>\n");

    b.push_str(&comment(site, repo, &root, item, "wrote", ""));

    let mut timeline: Vec<Entry> = comments
        .iter()
        .map(Entry::Comment)
        .chain(events.iter().map(Entry::Event))
        .chain(
            reviews
                .iter()
                // A pending review has no time and is the reviewer's draft.
                .filter(|r| s(r, "/submitted_at").is_some())
                .map(Entry::Review),
        )
        .collect();
    timeline.sort_by_cached_key(Entry::key);
    if !timeline.is_empty() {
        b.push_str("<h2>Conversation</h2>\n<ol class=\"timeline\">\n");
        for entry in &timeline {
            b.push_str("<li>");
            match entry {
                Entry::Comment(c) => {
                    let anchor = u(c, "/id")
                        .map(|id| format!("issuecomment-{id}"))
                        .unwrap_or_default();
                    b.push_str(&comment(site, repo, &root, c, "commented", &anchor));
                }
                Entry::Review(r) => {
                    let verb = match s(r, "/state") {
                        Some("APPROVED") => "approved these changes",
                        Some("CHANGES_REQUESTED") => "requested changes",
                        Some("DISMISSED") => "reviewed (dismissed)",
                        _ => "reviewed",
                    };
                    let anchor = u(r, "/id")
                        .map(|id| format!("review-{id}"))
                        .unwrap_or_default();
                    if s(r, "/body").is_some_and(|t| !t.trim().is_empty()) {
                        b.push_str(&comment(site, repo, &root, r, verb, &anchor));
                    } else {
                        let _ = write!(
                            b,
                            "<p class=\"event\" id=\"{}\"><strong>{}</strong> {verb} {}</p>",
                            esc(&anchor),
                            esc(login(r)),
                            s(r, "/submitted_at").map(time).unwrap_or_default()
                        );
                    }
                }
                Entry::Event(e) => {
                    let _ = write!(
                        b,
                        "<p class=\"event\"><strong>{}</strong> {} {}</p>",
                        esc(login(e)),
                        event_text(e),
                        s(e, "/created_at").map(time).unwrap_or_default()
                    );
                }
            }
            b.push_str("</li>\n");
        }
        b.push_str("</ol>\n");
    }

    if !review_comments.is_empty() {
        b.push_str(&review_threads(site, repo, &root, &review_comments));
    }
    if !commits.is_empty() {
        b.push_str(&commit_table(&commits));
    }
    site.write(&page)
}

/// Inline review comments, grouped by file, then by thread (a comment and its
/// replies), each thread under the diff it was made on.
fn review_threads(site: &Site, repo: &Repo, root: &str, comments: &[Value]) -> String {
    let ids: std::collections::HashSet<u64> = comments.iter().filter_map(|c| u(c, "/id")).collect();
    let mut replies: HashMap<u64, Vec<&Value>> = HashMap::new();
    let mut files: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for c in comments {
        match u(c, "/in_reply_to_id").filter(|p| ids.contains(p)) {
            Some(parent) => replies.entry(parent).or_default().push(c),
            None => files.entry(st(c, "/path")).or_default().push(c),
        }
    }
    let line = |c: &Value| u(c, "/line").or_else(|| u(c, "/original_line"));
    let mut out = String::from("<h2>Review comments</h2>\n");
    for (path, mut roots) in files {
        roots.sort_by_key(|c| (line(c), u(c, "/id")));
        let _ = writeln!(
            out,
            "<section class=\"file\"><h3><code>{}</code></h3>",
            esc(path)
        );
        for root_comment in roots {
            out.push_str("<div class=\"thread\">\n");
            if let Some(l) = line(root_comment) {
                let outdated = u(root_comment, "/line").is_none();
                let _ = writeln!(
                    out,
                    "<p class=\"meta\">Line {l}{}</p>",
                    if outdated { " (outdated)" } else { "" }
                );
            }
            if let Some(hunk) = s(root_comment, "/diff_hunk") {
                let _ = writeln!(out, "<pre class=\"diff\"><code>{}</code></pre>", esc(hunk));
            }
            let mut thread = vec![root_comment];
            // Replies of replies, in id order.
            let mut i = 0;
            while i < thread.len() {
                if let Some(r) = u(thread[i], "/id").and_then(|id| replies.get(&id)) {
                    thread.extend(r.iter().copied());
                }
                i += 1;
            }
            thread[1..].sort_by_key(|c| u(c, "/id"));
            for c in thread {
                let anchor = u(c, "/id")
                    .map(|id| format!("discussion_r{id}"))
                    .unwrap_or_default();
                out.push_str(&comment(site, repo, root, c, "commented", &anchor));
            }
            out.push_str("</div>\n");
        }
        out.push_str("</section>\n");
    }
    out
}

/// The commits of a pull request as a table.
fn commit_table(commits: &[Value]) -> String {
    let mut out = String::from(
        "<h2>Commits</h2>\n<div class=\"scroll\"><table class=\"commits\">\n<thead><tr>\
         <th scope=\"col\">Commit</th><th scope=\"col\">Message</th><th scope=\"col\">Author</th>\
         <th scope=\"col\">Date</th></tr></thead>\n<tbody>\n",
    );
    for c in commits {
        let message = st(c, "/commit/message").lines().next().unwrap_or_default();
        let author = s(c, "/author/login")
            .or_else(|| s(c, "/commit/author/name"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            short_sha(st(c, "/sha")),
            esc(message),
            esc(author),
            s(c, "/commit/author/date").map(time).unwrap_or_default()
        );
    }
    out.push_str("</tbody></table></div>\n");
    out
}

/// Discussions, when the backup has `discussions.json` (and, per discussion,
/// `discussion_comments/<n>.json`): the same layout as issues.  The files hold
/// GraphQL nodes (camelCase); a comment carries its replies in
/// `replies.nodes`.
fn discussions(site: &Site, repo: &Repo) -> Result<(), RenderError> {
    let path = repo.dir.join("discussions.json");
    if !path.is_file() {
        return Ok(());
    }
    let mut items: Vec<Value> = json::read_list(&path, &site.warnings)
        .into_iter()
        .filter(|d| u(d, "/number").is_some())
        .collect();
    items.sort_by_key(|d| std::cmp::Reverse(u(d, "/number")));
    items.dedup_by_key(|d| u(d, "/number"));

    let mut list = page(repo, "discussions/index.html", "Discussions");
    let root = list.root.clone();
    let b = &mut list.body;
    let _ = write!(
        b,
        "<h1>Discussions</h1>\n{}",
        tabs(repo, &root, Tab::Discussions)
    );
    if items.is_empty() {
        b.push_str("<p class=\"empty\">No discussions in this backup.</p>\n");
    } else {
        b.push_str("<ul class=\"items\">\n");
        for d in &items {
            let n = u(d, "/number").unwrap_or_default();
            let category = s(d, "/category/name")
                .map(|c| format!(" <span class=\"label\">{}</span>", esc(c)))
                .unwrap_or_default();
            let _ = writeln!(
                b,
                "<li>{} <a href=\"{root}{}discussions/{n}.html\">{}</a> <span class=\"num\">#{n}</span>{category}{}\
                 <div class=\"meta\">started {} by {}</div></li>",
                discussion_state(d),
                repo.base(),
                esc(st(d, "/title")),
                labels(d),
                s(d, "/createdAt").map(time).unwrap_or_default(),
                esc(login(d)),
            );
        }
        b.push_str("</ul>\n");
    }
    site.write(&list)?;

    parallel(&items, |d| {
        let n = u(d, "/number").unwrap_or_default();
        let mut page = page(
            repo,
            &format!("discussions/{n}.html"),
            &format!("{} #{n}", st(d, "/title")),
        )
        .crumb(
            format!("{}discussions/index.html", repo.base()),
            "Discussions",
        );
        let root = page.root.clone();
        let comments = json::read_list(
            &repo
                .dir
                .join("discussion_comments")
                .join(format!("{n}.json")),
            &site.warnings,
        );
        let b = &mut page.body;
        let _ = write!(
            b,
            "<h1>{} <span class=\"num\">#{n}</span></h1>\n{}",
            esc(st(d, "/title")),
            tabs(repo, &root, Tab::Discussions)
        );
        let _ = write!(b, "<p class=\"meta\">{}", discussion_state(d));
        if let Some(c) = s(d, "/category/name") {
            let _ = write!(b, " Category: {}", esc(c));
        }
        let l = labels(d);
        if !l.is_empty() {
            let _ = write!(b, " ·{l}");
        }
        if let Some(url) = s(d, "/url") {
            let _ = write!(b, " · <a href=\"{0}\">{0}</a>", esc(url));
        }
        b.push_str("</p>\n");
        b.push_str(&comment(site, repo, &root, d, "started this", ""));
        if !comments.is_empty() {
            b.push_str("<h2>Comments</h2>\n<ol class=\"timeline\">\n");
            for c in &comments {
                b.push_str("<li>");
                b.push_str(&comment(site, repo, &root, c, "commented", ""));
                let replies = c
                    .get("replies")
                    .map(|r| r.get("nodes").unwrap_or(r))
                    .and_then(Value::as_array);
                if let Some(replies) = replies.filter(|r| !r.is_empty()) {
                    b.push_str("<ol class=\"replies\">");
                    for r in replies.iter().filter(|r| r.is_object()) {
                        b.push_str("<li>");
                        b.push_str(&comment(site, repo, &root, r, "replied", ""));
                        b.push_str("</li>");
                    }
                    b.push_str("</ol>");
                }
                b.push_str("</li>\n");
            }
            b.push_str("</ol>\n");
        }
        site.write(&page)
    })
}

/// Open, closed or answered, for a discussion (a GraphQL node).
fn discussion_state(d: &Value) -> &'static str {
    if flag(d, "/isAnswered") {
        "<span class=\"state state-merged\">Answered</span>"
    } else if flag(d, "/closed") {
        "<span class=\"state state-closed\">Closed</span>"
    } else {
        "<span class=\"state state-open\">Open</span>"
    }
}
