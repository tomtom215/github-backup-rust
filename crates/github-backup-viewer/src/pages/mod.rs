// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The pages of the site.
//!
//! | Path | Page |
//! |------|------|
//! | `index.html` | repositories, links to the owner's other data |
//! | `gists.html`, `gists/<id>.html` | gists |
//! | `starred.html`, `watched.html`, `followers.html`, `following.html`, `members.html`, `teams.html` | owner lists |
//! | `projects/index.html`, `projects/<n>.html` | Projects (v2) |
//! | `repos/<repo>/index.html` | repository overview |
//! | `repos/<repo>/issues/index.html`, `closed.html`, `<n>.html` | issues |
//! | `repos/<repo>/pulls/index.html`, `closed.html`, `<n>.html` | pull requests |
//! | `repos/<repo>/discussions/index.html`, `<n>.html` | discussions, when backed up |
//! | `repos/<repo>/releases.html`, `labels.html` | releases; labels and milestones |

mod items;
mod owner;
mod repo;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;
use url::Url;

use crate::html::seg;
use crate::json::{self, s, u};
use crate::markdown::{self, Links};
use crate::site::{io, RenderError, Site};

/// Renders every page; returns the number of repositories.
pub(crate) fn render_all(site: &Site) -> Result<usize, RenderError> {
    let listed = repos_by_name(site);
    let repos_dir = site.json.join("repos");
    let mut names: Vec<String> = match std::fs::read_dir(&repos_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(io(&repos_dir)(e)),
    };
    names.sort();

    let mut summaries = Vec::with_capacity(names.len());
    for name in names {
        let repo = Repo::load(site, &name, listed.get(&name));
        let releases = repo::render(site, &repo)?;
        items::render(site, &repo)?;
        summaries.push(repo.summary(releases));
    }
    let count = summaries.len();
    owner::render(site, &summaries)?;
    Ok(count)
}

/// `repos.json` by repository name: the fallback for a repository whose
/// `info.json` is missing.
fn repos_by_name(site: &Site) -> HashMap<String, Value> {
    json::read_list(&site.json.join("repos.json"), &site.warnings)
        .into_iter()
        .filter_map(|r| Some((s(&r, "/name")?.to_string(), r)))
        .collect()
}

/// What the index shows of a repository.
pub(crate) struct Summary {
    pub name: String,
    pub seg: String,
    pub info: Value,
    pub open_issues: usize,
    pub issues: usize,
    pub open_pulls: usize,
    pub pulls: usize,
    pub releases: usize,
}

/// Whether a number is an issue or a pull request (or a discussion).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Issue,
    Pull,
}

/// One repository's backed-up data.
pub(crate) struct Repo {
    /// The directory name under `json/repos/`.
    pub name: String,
    /// `name`, percent-encoded for URLs.
    pub seg: String,
    /// `json/repos/<name>/`.
    pub dir: PathBuf,
    /// `info.json`, else the `repos.json` entry, else `{}`.
    pub info: Value,
    /// `html_url`, for resolving relative links and recognising own URLs.
    pub html_url: Option<Url>,
    /// Issues (not pull requests), newest first.
    pub issues: Vec<Value>,
    /// Pull requests from `pulls.json`, plus those only `issues.json` has,
    /// newest first.
    pub pulls: Vec<Value>,
    /// Issue and pull request numbers.
    pub kinds: HashMap<u64, Kind>,
    /// Attachment URL to a path under the owner directory (encoded).
    pub attachments: HashMap<String, String>,
}

impl Repo {
    fn load(site: &Site, name: &str, listed: Option<&Value>) -> Self {
        let dir = site.json.join("repos").join(name);
        let info = json::read(&dir.join("info.json"), &site.warnings)
            .filter(Value::is_object)
            .or_else(|| listed.cloned())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let html_url = s(&info, "/html_url").and_then(|h| Url::parse(h).ok());

        let mut issues = Vec::new();
        let mut issue_pulls = BTreeMap::new();
        for issue in json::read_list(&dir.join("issues.json"), &site.warnings) {
            let Some(n) = u(&issue, "/number") else {
                continue;
            };
            if issue.get("pull_request").is_some() {
                issue_pulls.insert(n, issue);
            } else {
                issues.push(issue);
            }
        }
        let mut pulls: Vec<Value> = json::read_list(&dir.join("pulls.json"), &site.warnings)
            .into_iter()
            .filter(|p| u(p, "/number").is_some())
            .collect();
        for p in &pulls {
            issue_pulls.remove(&u(p, "/number").unwrap_or_default());
        }
        pulls.extend(issue_pulls.into_values());
        let by_number_desc = |a: &Value, b: &Value| u(b, "/number").cmp(&u(a, "/number"));
        issues.sort_by(by_number_desc);
        pulls.sort_by(by_number_desc);
        issues.dedup_by_key(|i| u(i, "/number"));
        pulls.dedup_by_key(|p| u(p, "/number"));

        let mut kinds = HashMap::new();
        for i in &issues {
            kinds.insert(u(i, "/number").unwrap_or_default(), Kind::Issue);
        }
        for p in &pulls {
            kinds.insert(u(p, "/number").unwrap_or_default(), Kind::Pull);
        }

        Self {
            name: name.to_string(),
            seg: seg(name),
            attachments: attachments(site, &dir),
            dir,
            info,
            html_url,
            issues,
            pulls,
            kinds,
        }
    }

    /// The site path (from the root) of the repository's directory.
    pub(crate) fn base(&self) -> String {
        format!("repos/{}/", self.seg)
    }

    /// The site path of issue or pull request `n`, if the backup has it.
    pub(crate) fn item_path(&self, n: u64) -> Option<String> {
        let dir = match self.kinds.get(&n)? {
            Kind::Issue => "issues",
            Kind::Pull => "pulls",
        };
        Some(format!("{}{dir}/{n}.html", self.base()))
    }

    /// A link to the local copy of `url` from a page with root prefix `root`:
    /// a downloaded attachment, or the page of one of this repository's
    /// issues or pull requests (`.../issues/12#issuecomment-3`).
    fn local_href(&self, site: &Site, root: &str, url: &str) -> Option<String> {
        if let Some(path) = self.attachments.get(url) {
            return Some(site.owner_href(root, path));
        }
        let own = self.html_url.as_ref()?.as_str().trim_end_matches('/');
        let rest = url.strip_prefix(own)?;
        let rest = rest
            .strip_prefix("/issues/")
            .or_else(|| rest.strip_prefix("/pull/"))?;
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let n: u64 = rest[..digits].parse().ok()?;
        let fragment = match &rest[digits..] {
            "" | "/" => "",
            f if f.starts_with("#issuecomment-") => f,
            _ => return None,
        };
        Some(format!("{root}{}{fragment}", self.item_path(n)?))
    }

    /// Renders Markdown written at `written_at` (a URL) for a page with root
    /// prefix `root`.
    pub(crate) fn md(
        &self,
        site: &Site,
        root: &str,
        written_at: Option<&str>,
        text: &str,
    ) -> String {
        let base = written_at
            .and_then(|w| Url::parse(w).ok())
            .or_else(|| self.html_url.clone());
        let local = |url: &str| self.local_href(site, root, url);
        let issue = |n: u64| self.item_path(n).map(|p| format!("{root}{p}"));
        markdown::render(
            text,
            &Links {
                base: base.as_ref(),
                local: &local,
                issue: &issue,
            },
        )
    }

    fn summary(&self, releases: usize) -> Summary {
        let open = |list: &[Value]| {
            list.iter()
                .filter(|i| s(i, "/state") == Some("open"))
                .count()
        };
        Summary {
            name: self.name.clone(),
            seg: self.seg.clone(),
            info: self.info.clone(),
            open_issues: open(&self.issues),
            issues: self.issues.len(),
            open_pulls: open(&self.pulls),
            pulls: self.pulls.len(),
            releases,
        }
    }
}

/// The attachment index of a repository, `attachments/index.json` (written
/// by `--attachments`): a JSON object mapping each attachment URL to its
/// entry, whose `file` is the file in that `attachments/` directory (an entry
/// without one is an attachment that could not be downloaded).  A plain string
/// value is taken as the file too.  Files that do not exist, and paths that
/// could leave the directory, are ignored.
fn attachments(site: &Site, dir: &Path) -> HashMap<String, String> {
    let index = dir.join("attachments").join("index.json");
    let Some(Value::Object(map)) = json::read(&index, &site.warnings) else {
        return HashMap::new();
    };
    let Ok(rel_dir) = dir.strip_prefix(site.json.parent().unwrap_or(&site.json)) else {
        return HashMap::new();
    };
    let prefix: String = rel_dir
        .components()
        .map(|c| format!("{}/", seg(&c.as_os_str().to_string_lossy())))
        .collect();
    map.into_iter()
        .filter_map(|(url, path)| {
            let path = path.as_str().or_else(|| s(&path, "/file"))?;
            let parts: Vec<&str> = path.split('/').collect();
            let safe = parts
                .iter()
                .all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.contains('\\'));
            if !safe || !dir.join("attachments").join(path).is_file() {
                return None;
            }
            let encoded: Vec<String> = parts.iter().map(|p| seg(p)).collect();
            Some((url, format!("{prefix}attachments/{}", encoded.join("/"))))
        })
        .collect()
}

/// Runs `f` on every element of `items` on all CPUs; the first error wins.
pub(crate) fn parallel<T: Sync>(
    items: &[T],
    f: impl Fn(&T) -> Result<(), RenderError> + Sync,
) -> Result<(), RenderError> {
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(items.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = items.get(i) else {
                        return Ok(());
                    };
                    if let Err(e) = f(item) {
                        // Stop the others too.
                        next.store(items.len(), std::sync::atomic::Ordering::Relaxed);
                        return Err(e);
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .try_for_each(|w| w.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
    })
}
