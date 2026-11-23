// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! HTML building blocks: escaping, URL path segments and the page layout.
//!
//! Everything that comes from the backup (titles, user names, label names,
//! descriptions) is attacker-controlled and goes through [`esc`] before it
//! reaches the page; no template engine, so there is no other way in.

use std::fmt::Write as _;

/// The Content-Security-Policy of every page: nothing runs, nothing is loaded
/// from anywhere but the site itself (the style sheet, local images).
pub(crate) const CSP: &str = "default-src 'none'; img-src 'self' data:; style-src 'self'; \
                              base-uri 'none'; form-action 'none'";

/// Escapes `s` for use in HTML text and in double- or single-quoted attributes.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            // Not valid in HTML text; keep the page well-formed.
            '\0' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

/// Percent-encodes one URL path segment (a file or directory name).
pub(crate) fn seg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'@') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Reverses [`seg`]: the file name a percent-encoded segment stands for.
pub(crate) fn unseg(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `../` repeated once per directory level of `rel_path` (a path inside the
/// site, `/`-separated): the prefix that leads from that page to the site root.
pub(crate) fn root_of(rel_path: &str) -> String {
    "../".repeat(rel_path.matches('/').count())
}

/// Formats a GitHub timestamp (`2024-01-02T03:04:05Z`) as a `<time>` element.
/// Anything that does not look like one is shown as given.
pub(crate) fn time(ts: &str) -> String {
    let shown = match (ts.get(..10), ts.get(11..16)) {
        (Some(date), Some(hm)) if ts.as_bytes().get(10) == Some(&b'T') => {
            format!("{date} {hm} UTC")
        }
        _ => ts.to_string(),
    };
    format!("<time datetime=\"{}\">{}</time>", esc(ts), esc(&shown))
}

/// Formats a byte count for people (`1.5 MiB`).
pub(crate) fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

/// `1 issue` / `2 issues`.
pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One page of the site, before the layout is put around it.
pub(crate) struct Page {
    /// Path of the page inside the site, `/`-separated (`repos/x/index.html`).
    pub rel_path: String,
    /// `../`… leading from this page to the site root.
    pub root: String,
    /// The `<title>` and the last breadcrumb.
    pub title: String,
    /// What the page belongs to (a repository name), added to the `<title>`
    /// so that browser tabs and history can be told apart.
    pub context: Option<String>,
    /// Breadcrumbs before the title: (href relative to the site root, label).
    pub crumbs: Vec<(String, String)>,
    /// The content of `<main>`.
    pub body: String,
}

impl Page {
    /// A page at `rel_path` with `title`.
    pub(crate) fn new(rel_path: impl Into<String>, title: impl Into<String>) -> Self {
        let rel_path = rel_path.into();
        Self {
            root: root_of(&rel_path),
            rel_path,
            title: title.into(),
            context: None,
            crumbs: Vec::new(),
            body: String::new(),
        }
    }

    /// Adds a breadcrumb; `href` is relative to the site root.
    pub(crate) fn crumb(mut self, href: impl Into<String>, label: impl Into<String>) -> Self {
        self.crumbs.push((href.into(), label.into()));
        self
    }

    /// The complete HTML document.
    pub(crate) fn render(&self, owner: &str) -> String {
        let root = &self.root;
        let mut out = String::with_capacity(self.body.len() + 2048);
        let _ = write!(
            out,
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\">\n\
             <meta name=\"referrer\" content=\"no-referrer\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <meta name=\"color-scheme\" content=\"light dark\">\n\
             <title>{}{} · {} backup</title>\n\
             <link rel=\"stylesheet\" href=\"{root}style.css\">\n</head>\n<body>\n\
             <a class=\"skip\" href=\"#main\">Skip to content</a>\n\
             <header class=\"top\"><nav aria-label=\"Breadcrumb\"><ol class=\"crumbs\">",
            esc(&self.title),
            self.context
                .as_deref()
                .map(|c| format!(" · {}", esc(c)))
                .unwrap_or_default(),
            esc(owner),
        );
        for (href, label) in &self.crumbs {
            let _ = write!(
                out,
                "<li><a href=\"{root}{}\">{}</a></li>",
                esc(href),
                esc(label)
            );
        }
        let _ = write!(
            out,
            "<li aria-current=\"page\">{}</li></ol></nav></header>\n\
             <main id=\"main\">\n{}\n</main>\n\
             <footer>Offline copy of the GitHub backup of {}, rendered by \
             github-backup.</footer>\n</body>\n</html>\n",
            esc(&self.title),
            self.body,
            esc(owner),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_markup_character() {
        assert_eq!(
            esc("<a href=\"x\" onclick='y'>&</a>"),
            "&lt;a href=&quot;x&quot; onclick=&#39;y&#39;&gt;&amp;&lt;/a&gt;"
        );
    }

    #[test]
    fn segments_are_percent_encoded() {
        assert_eq!(seg("my-repo.git"), "my-repo.git");
        assert_eq!(seg("repo@123"), "repo@123");
        assert_eq!(seg("a b/c?#\"<"), "a%20b%2Fc%3F%23%22%3C");
        assert_eq!(seg("ü"), "%C3%BC");
        for name in ["a b/c?#\"<%", "ü@x", "%41"] {
            assert_eq!(unseg(&seg(name)), name);
        }
    }

    #[test]
    fn root_prefix_counts_directories() {
        assert_eq!(root_of("index.html"), "");
        assert_eq!(root_of("repos/x/issues/1.html"), "../../../");
    }

    #[test]
    fn timestamps_and_sizes_are_readable() {
        assert_eq!(
            time("2024-01-02T03:04:05Z"),
            "<time datetime=\"2024-01-02T03:04:05Z\">2024-01-02 03:04 UTC</time>"
        );
        assert_eq!(time("<b>"), "<time datetime=\"&lt;b&gt;\">&lt;b&gt;</time>");
        assert_eq!(bytes(10), "10 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MiB");
    }
}
