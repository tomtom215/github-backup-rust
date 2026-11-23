// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The render entry point: where the site goes, how it replaces an earlier
//! render, and writing its files.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::html::{seg, unseg, Page};
use crate::json::Warnings;

/// The file that marks a directory as a rendered site.  An existing site
/// directory is replaced only when it holds this file (or is empty), so a
/// mistyped `--render-html` path can never delete someone's data.
pub(crate) const MARKER: &str = ".github-backup-site";

/// The style sheet shared by every page.
const STYLE: &str = include_str!("style.css");

/// What to render, and where.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// The owner's backup directory, `<output>/<owner>`, holding `json/`
    /// (required) and `git/` (optional).
    pub owner_dir: PathBuf,
    /// The owner's name, shown in titles.
    pub owner: String,
    /// The directory the site is written to.  It is replaced as a whole, so it
    /// must not exist, be empty, or hold an earlier render.
    pub site_dir: PathBuf,
}

/// What a render produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderReport {
    /// The site directory (absolute).
    pub site_dir: PathBuf,
    /// Repositories rendered.
    pub repositories: usize,
    /// HTML pages written.
    pub pages: usize,
    /// Bytes written (pages and style sheet).
    pub bytes: u64,
    /// Backup files that exist but could not be read or parsed, sorted.  The
    /// pages that needed them show what else there is.
    pub warnings: Vec<String>,
}

/// Why no site could be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// `<owner_dir>/json` does not exist: there is no backup to render.
    #[error("no backup found: {} does not exist", .0.display())]
    NoBackup(PathBuf),
    /// The site directory exists and is not an earlier render.
    #[error(
        "refusing to replace {}: it is not empty and was not made by --render-html \
         (no {MARKER} file); choose another directory",
        .0.display()
    )]
    NotASite(PathBuf),
    /// The site directory is inside the backup's `json/` directory, where
    /// it would be taken for backup data (manifest, S3 sync).
    #[error("refusing to render into {}: it is inside the backup's json directory", .0.display())]
    InsideBackup(PathBuf),
    /// A file or directory could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The path concerned.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// Attaches the path to an I/O error.
pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> RenderError + '_ {
    move |source| RenderError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Shared state of one render.
pub(crate) struct Site {
    /// The owner's name.
    pub owner: String,
    /// `<owner_dir>/json`.
    pub json: PathBuf,
    /// `<owner_dir>/git`.
    pub git: PathBuf,
    /// The owner directory as seen from the site root: a relative URL prefix
    /// (`../`) or, when there is none, a `file:` URL; ends with `/`.
    to_owner: String,
    /// Whether `to_owner` is relative (and so needs the page's root prefix).
    to_owner_relative: bool,
    /// Where the pages are written (the staging directory).
    out: PathBuf,
    /// Problems with backup files.
    pub warnings: Warnings,
    pages: AtomicUsize,
    bytes: AtomicU64,
}

impl Site {
    /// A link from a page (whose root prefix is `root`) to `rel`, a
    /// `/`-separated, already-encoded path under the owner directory.
    pub(crate) fn owner_href(&self, root: &str, rel: &str) -> String {
        if self.to_owner_relative {
            format!("{root}{}{rel}", self.to_owner)
        } else {
            format!("{}{rel}", self.to_owner)
        }
    }

    /// Writes one page.
    pub(crate) fn write(&self, page: &Page) -> Result<(), RenderError> {
        let html = page.render(&self.owner);
        self.write_file(&page.rel_path, html.as_bytes())?;
        self.pages.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Writes `data` to `rel_path` (`/`-separated) in the site.
    fn write_file(&self, rel_path: &str, data: &[u8]) -> Result<(), RenderError> {
        let path = rel_path
            .split('/')
            .fold(self.out.clone(), |p, part| p.join(unseg(part)));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        // A plain write: the whole site is staged and renamed into place at
        // the end, and it can always be rendered again from the backup.
        std::fs::write(&path, data).map_err(io(&path))?;
        self.bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
        Ok(())
    }
}

/// Renders the backup in `opts.owner_dir` into `opts.site_dir`.
///
/// The site is built in a hidden sibling directory and renamed into place,
/// replacing an earlier render; a directory that is neither empty nor an
/// earlier render is never replaced.
///
/// # Errors
///
/// [`RenderError::NoBackup`] when there is no `json/` directory,
/// [`RenderError::NotASite`] when the site directory is in use for something
/// else, [`RenderError::InsideBackup`] when it is inside `json/`, and [`RenderError::Io`] when the site cannot be written.  Unreadable
/// backup files are not errors; see [`RenderReport::warnings`].
pub fn render_site(opts: &RenderOptions) -> Result<RenderReport, RenderError> {
    let json = opts.owner_dir.join("json");
    if !json.is_dir() {
        return Err(RenderError::NoBackup(json));
    }
    let site_dir = std::path::absolute(&opts.site_dir).map_err(io(&opts.site_dir))?;
    let (Some(parent), Some(name)) = (site_dir.parent(), site_dir.file_name()) else {
        return Err(RenderError::NotASite(site_dir));
    };
    let name = name.to_string_lossy().into_owned();
    if site_dir.exists() && !replaceable(&site_dir)? {
        return Err(RenderError::NotASite(site_dir));
    }
    std::fs::create_dir_all(parent).map_err(io(parent))?;
    let parent = parent.canonicalize().map_err(io(parent))?;
    let owner_dir = opts.owner_dir.canonicalize().map_err(io(&opts.owner_dir))?;
    let json_dir = json.canonicalize().map_err(io(&json))?;
    if parent.join(&name).starts_with(&json_dir) {
        return Err(RenderError::InsideBackup(parent.join(&name)));
    }
    let (to_owner, to_owner_relative) = relative_dir(&parent.join(&name), &owner_dir);

    let staging = parent.join(format!(".{name}.rendering"));
    remove_if_exists(&staging)?;
    std::fs::create_dir(&staging).map_err(io(&staging))?;

    let site = Site {
        owner: opts.owner.clone(),
        json,
        git: owner_dir.join("git"),
        to_owner,
        to_owner_relative,
        out: staging.clone(),
        warnings: Warnings::default(),
        pages: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
    };
    let rendered = crate::pages::render_all(&site).and_then(|repositories| {
        site.write_file("style.css", STYLE.as_bytes())?;
        site.write_file(
            MARKER,
            b"Made by github-backup --render-html; replaced on every render.\n",
        )?;
        Ok(repositories)
    });
    let repositories = match rendered {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    let site_dir = parent.join(&name);
    if site_dir.exists() {
        let old = parent.join(format!(".{name}.old"));
        remove_if_exists(&old)?;
        std::fs::rename(&site_dir, &old).map_err(io(&site_dir))?;
        std::fs::rename(&staging, &site_dir).map_err(io(&staging))?;
        std::fs::remove_dir_all(&old).map_err(io(&old))?;
    } else {
        std::fs::rename(&staging, &site_dir).map_err(io(&staging))?;
    }

    Ok(RenderReport {
        site_dir,
        repositories,
        pages: site.pages.load(Ordering::Relaxed),
        bytes: site.bytes.load(Ordering::Relaxed),
        warnings: site.warnings.into_sorted(),
    })
}

/// `true` if `dir` may be replaced: an empty directory or an earlier render.
fn replaceable(dir: &Path) -> Result<bool, RenderError> {
    if !dir.is_dir() {
        return Ok(false);
    }
    if dir.join(MARKER).is_file() {
        return Ok(true);
    }
    Ok(std::fs::read_dir(dir).map_err(io(dir))?.next().is_none())
}

fn remove_if_exists(dir: &Path) -> Result<(), RenderError> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(dir)(e)),
        _ => Ok(()),
    }
}

/// The URL of directory `to` as seen from directory `from` (both absolute
/// and canonical), ending with `/`: relative when they share a root, else a
/// `file:` URL.  The bool says whether it is relative.
fn relative_dir(from: &Path, to: &Path) -> (String, bool) {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    if from.first() != to.first() {
        let url = url::Url::from_directory_path(to.iter().collect::<PathBuf>())
            .map(String::from)
            .unwrap_or_default();
        return (url, false);
    }
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = "../".repeat(from.len() - common);
    for part in &to[common..] {
        out.push_str(&seg(&part.as_os_str().to_string_lossy()));
        out.push('/');
    }
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_directories() {
        let rel = |a: &str, b: &str| relative_dir(Path::new(a), Path::new(b));
        assert_eq!(rel("/b/o/site", "/b/o"), ("../".to_string(), true));
        assert_eq!(
            rel("/srv/www/s", "/b/o"),
            ("../../../b/o/".to_string(), true)
        );
        assert_eq!(rel("/b/o", "/b/o"), (String::new(), true));
        assert_eq!(rel("/b/o/s", "/b/o w"), ("../../o%20w/".to_string(), true));
    }
}
