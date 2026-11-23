// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub-flavoured Markdown to safe HTML.
//!
//! Issue bodies, comments and release notes are written by anyone who can
//! open an issue, so the renderer trusts none of it:
//!
//! - raw HTML is never passed through: inline HTML becomes visible text
//!   (`<br>` becomes a line break), an HTML block becomes a code block, and an
//!   HTML comment (`<!-- issue template hint -->`, invisible on GitHub too) is
//!   dropped;
//! - a link is kept only for `http`, `https` and `mailto` URLs (and relative
//!   ones, resolved against the page the text was written on); `javascript:`,
//!   `data:`, `vbscript:`, `file:` and anything else become plain text;
//! - an image is shown only from a local copy; otherwise it becomes a link to
//!   the remote image (the pages' Content-Security-Policy would block it
//!   anyway, and a backup should not phone home when it is opened).
//!
//! On top of CommonMark: tables, strikethrough, task lists, footnotes, GFM
//! alerts, and bare `http(s)://` URLs and `#123` references turned into links.

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use url::Url;

/// How links in one text are resolved.
pub(crate) struct Links<'a> {
    /// Where the text was written (an issue's `html_url`): relative links
    /// resolve against it, as they do on GitHub.
    pub base: Option<&'a Url>,
    /// An absolute URL to a page or file of the site, relative to the page
    /// being rendered, when the site has one (an attachment, an issue page).
    pub local: &'a dyn Fn(&str) -> Option<String>,
    /// `#123` to the page of issue or pull request 123, relative to the page.
    pub issue: &'a dyn Fn(u64) -> Option<String>,
}

impl Links<'_> {
    /// Links that resolve nothing locally and nothing relative.
    #[cfg(test)]
    pub(crate) fn none() -> Links<'static> {
        Links {
            base: None,
            local: &|_| None,
            issue: &|_| None,
        }
    }
}

/// Where a link or image points after the policy is applied.
enum Target {
    /// Inside the site (a local copy).
    Local(String),
    /// A remote `http(s)`/`mailto` URL.
    Remote(String),
}

impl Target {
    fn into_string(self) -> String {
        match self {
            Target::Local(s) | Target::Remote(s) => s,
        }
    }
}

/// The scheme of `url`, lower-cased, as a browser would see it (browsers
/// ignore ASCII tabs and newlines anywhere and leading control characters and
/// spaces, so `java\tscript:` is `javascript:`); `None` for a relative URL.
fn scheme_of(url: &str) -> Option<String> {
    let cleaned: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let colon = cleaned.find(':')?;
    let scheme = &cleaned[..colon];
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    // `a/b:c` or `?x:y` is a relative path, not a scheme.
    valid.then(|| scheme.to_ascii_lowercase())
}

/// Applies the link policy to `dest`; `None` when the link must not be kept.
fn resolve(dest: &str, links: &Links) -> Option<Target> {
    let dest = dest.trim();
    if dest.is_empty() {
        return None;
    }
    if dest.starts_with('#') {
        // A fragment: only meaningful within the same text, and harmless.
        return Some(Target::Remote(dest.to_string()));
    }
    let absolute = match scheme_of(dest).as_deref() {
        Some("http" | "https") => Url::parse(dest).ok()?,
        Some("mailto") => return Some(Target::Remote(dest.to_string())),
        Some(_) => return None,
        None => links.base?.join(dest).ok()?,
    };
    if !matches!(absolute.scheme(), "http" | "https") {
        return None;
    }
    let local = (links.local)(dest).or_else(|| (links.local)(absolute.as_str()));
    Some(match local {
        Some(href) => Target::Local(href),
        None => Target::Remote(absolute.into()),
    })
}

/// Renders `markdown` to HTML under the policy described in the module docs.
pub(crate) fn render(markdown: &str, links: &Links) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut events: Vec<Event> = Vec::new();
    // For every open link and image: whether its start tag was emitted (and
    // what as), so the matching end tag can follow suit.
    let mut open: Vec<Option<TagEnd>> = Vec::new();
    let mut in_code = false;
    let mut html_block: Option<String> = None;

    for event in coalesce_text(Parser::new_ext(markdown, options)) {
        match event {
            Event::Start(Tag::HtmlBlock) => html_block = Some(String::new()),
            Event::End(TagEnd::HtmlBlock) => {
                let raw = html_block.take().unwrap_or_default();
                let trimmed = raw.trim();
                if !(trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
                    let kind = pulldown_cmark::CodeBlockKind::Fenced(CowStr::from("html"));
                    events.push(Event::Start(Tag::CodeBlock(kind)));
                    events.push(Event::Text(CowStr::from(raw)));
                    events.push(Event::End(TagEnd::CodeBlock));
                }
            }
            Event::Html(raw) | Event::InlineHtml(raw) if html_block.is_some() => {
                if let Some(block) = html_block.as_mut() {
                    block.push_str(&raw);
                }
            }
            Event::InlineHtml(raw) | Event::Html(raw) => {
                let tag = raw.trim().to_ascii_lowercase();
                if matches!(tag.as_str(), "<br>" | "<br/>" | "<br />") {
                    events.push(Event::HardBreak);
                } else if !(tag.starts_with("<!--") && tag.ends_with("-->")) {
                    events.push(Event::Text(raw));
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                events.push(Event::Start(Tag::CodeBlock(kind)));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                events.push(Event::End(TagEnd::CodeBlock));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                // `<user@example.com>`: the renderer adds the `mailto:`.
                let target = if link_type == LinkType::Email {
                    Some(dest_url.to_string())
                } else {
                    resolve(&dest_url, links).map(Target::into_string)
                };
                match target {
                    Some(dest) => {
                        events.push(Event::Start(Tag::Link {
                            link_type,
                            dest_url: CowStr::from(dest),
                            title,
                            id,
                        }));
                        open.push(Some(TagEnd::Link));
                    }
                    None => open.push(None),
                }
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => match resolve(&dest_url, links) {
                Some(Target::Local(src)) => {
                    events.push(Event::Start(Tag::Image {
                        link_type,
                        dest_url: CowStr::from(src),
                        title,
                        id,
                    }));
                    open.push(Some(TagEnd::Image));
                }
                Some(Target::Remote(url)) => {
                    events.push(Event::Start(Tag::Link {
                        link_type: LinkType::Inline,
                        dest_url: CowStr::from(url),
                        title,
                        id,
                    }));
                    events.push(Event::Text(CowStr::from("Image: ")));
                    open.push(Some(TagEnd::Link));
                }
                None => {
                    events.push(Event::Text(CowStr::from("Image: ")));
                    open.push(None);
                }
            },
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some(Some(end)) = open.pop() {
                    events.push(Event::End(end));
                }
            }
            Event::Text(text) if !in_code && open.is_empty() => linkify(text, links, &mut events),
            other => events.push(other),
        }
    }
    let mut html = String::with_capacity(markdown.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

/// Joins adjacent text events (the parser splits text at characters that
/// might have been emphasis, such as `_`), so a bare URL is seen whole.
fn coalesce_text<'a>(events: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut out: Vec<Event<'a>> = Vec::new();
    for event in events {
        match (out.last_mut(), event) {
            (Some(Event::Text(prev)), Event::Text(next)) => {
                *prev = CowStr::from(format!("{prev}{next}"));
            }
            (_, event) => out.push(event),
        }
    }
    out
}

/// `true` for characters that end a bare URL.
fn ends_url(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`')
}

/// Length of the bare URL at the start of `rest`, without trailing
/// punctuation and unbalanced closing parentheses (as GitHub does).
fn url_len(rest: &str) -> usize {
    let mut end = rest.find(ends_url).unwrap_or(rest.len());
    loop {
        let candidate = &rest[..end];
        let Some(last) = candidate.chars().last() else {
            return 0;
        };
        let unbalanced_paren =
            last == ')' && candidate.matches(')').count() > candidate.matches('(').count();
        if matches!(
            last,
            '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '*' | '_' | '~'
        ) || unbalanced_paren
        {
            end -= last.len_utf8();
        } else {
            return end;
        }
    }
}

/// Emits `text`, with bare URLs and `#123` references turned into links.
fn linkify<'a>(text: CowStr<'a>, links: &Links, events: &mut Vec<Event<'a>>) {
    let mut plain_from = 0;
    let mut i = 0;
    let bytes = text.as_bytes();
    let mut out: Vec<Event<'a>> = Vec::new();
    while i < text.len() {
        let after_boundary = i == 0 || {
            let prev = text[..i].chars().last().unwrap_or(' ');
            !prev.is_alphanumeric() && prev != '/' && prev != '&'
        };
        let rest = &text[i..];
        let mut link: Option<(usize, String)> = None;
        if after_boundary {
            let lower = rest.get(..8).unwrap_or(rest).to_ascii_lowercase();
            if lower.starts_with("https://") || lower.starts_with("http://") {
                let len = url_len(rest);
                let scheme_len = if lower.starts_with("https://") { 8 } else { 7 };
                if len > scheme_len {
                    if let Some(target) = resolve(&rest[..len], links) {
                        link = Some((len, target.into_string()));
                    }
                }
            } else if bytes[i] == b'#' {
                let digits = rest[1..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len() - 1);
                let next = rest[1 + digits..].chars().next();
                if digits > 0 && !next.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    if let Some(href) = rest[1..=digits]
                        .parse()
                        .ok()
                        .and_then(|n: u64| (links.issue)(n))
                    {
                        link = Some((1 + digits, href));
                    }
                }
            }
        }
        match link {
            Some((len, href)) => {
                if plain_from < i {
                    out.push(Event::Text(CowStr::from(text[plain_from..i].to_string())));
                }
                out.push(Event::Start(Tag::Link {
                    link_type: LinkType::Autolink,
                    dest_url: CowStr::from(href),
                    title: CowStr::from(""),
                    id: CowStr::from(""),
                }));
                out.push(Event::Text(CowStr::from(text[i..i + len].to_string())));
                out.push(Event::End(TagEnd::Link));
                i += len;
                plain_from = i;
            }
            None => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    if plain_from == 0 {
        events.push(Event::Text(text));
        return;
    }
    if plain_from < text.len() {
        out.push(Event::Text(CowStr::from(text[plain_from..].to_string())));
    }
    events.extend(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::assert_inert;

    fn md(s: &str) -> String {
        render(s, &Links::none())
    }

    #[test]
    fn raw_html_is_shown_as_text_never_run() {
        for input in [
            "<script>alert(1)</script>",
            "hello <img src=x onerror=alert(1)> world",
            "<iframe src=\"https://evil.example\"></iframe>",
            "<svg onload=alert(1)>",
            "<a href=\"javascript:alert(1)\">x</a>",
            "<style>body{display:none}</style>",
            "<form action=https://evil.example><input></form>",
            "<div\nonclick=\"x\">multi\nline</div>",
        ] {
            let html = md(input);
            assert_inert(&html);
            assert!(html.contains("&lt;"), "{input} -> {html}");
        }
    }

    #[test]
    fn html_comments_vanish_and_br_breaks() {
        let html = md("<!-- template hint -->\n\ntext<br>more <!-- x -->");
        assert!(!html.contains("template hint"), "{html}");
        assert!(!html.contains("&lt;!--"), "{html}");
        assert!(html.contains("text<br />"), "{html}");
    }

    #[test]
    fn dangerous_link_schemes_become_text() {
        for input in [
            "[x](javascript:alert(1))",
            "[x](JaVaScRiPt:alert(1))",
            "[x](java\tscript:alert(1))",
            "[x]( javascript:alert(1))",
            "[x](data:text/html;base64,PHNjcmlwdD4=)",
            "[x](vbscript:msgbox)",
            "[x](file:///etc/passwd)",
            "<javascript:alert(1)>",
            "[x][r]\n\n[r]: javascript:alert(1)",
            "![x](javascript:alert(1))",
            "![x](data:image/svg+xml;base64,PHN2Zz4=)",
        ] {
            let html = md(input);
            assert_inert(&html);
            assert!(!html.contains("<a"), "{input} -> {html}");
        }
    }

    #[test]
    fn safe_links_are_kept_and_attributes_escaped() {
        let html = md("[a](https://example.com/?q=\"><script>) and <https://x.example/a>");
        assert_inert(&html);
        assert!(
            html.contains("href=\"https://example.com/?q=%22%3E%3Cscript%3E\""),
            "{html}"
        );
        assert!(html.contains("href=\"https://x.example/a\""), "{html}");
        let mail = md("<user@example.com>");
        assert!(mail.contains("href=\"mailto:user@example.com\""), "{mail}");
    }

    #[test]
    fn remote_images_become_links_and_local_ones_stay() {
        let html = md("![a cat](https://example.com/cat.png)");
        assert!(!html.contains("<img"), "{html}");
        assert!(
            html.contains("<a href=\"https://example.com/cat.png\">Image: a cat</a>"),
            "{html}"
        );
        let local = |u: &str| (u == "https://example.com/cat.png").then(|| "files/cat.png".into());
        let links = Links {
            base: None,
            local: &local,
            issue: &|_| None,
        };
        let html = render("![a cat](https://example.com/cat.png)", &links);
        assert!(
            html.contains("<img src=\"files/cat.png\" alt=\"a cat\" />"),
            "{html}"
        );
    }

    #[test]
    fn relative_links_resolve_against_the_base_or_are_dropped() {
        let base = Url::parse("https://github.com/o/r/issues/1").unwrap();
        let links = Links {
            base: Some(&base),
            local: &|_| None,
            issue: &|_| None,
        };
        let html = render("[f](../blob/main/README.md) [g](/o/r/wiki)", &links);
        assert!(
            html.contains("href=\"https://github.com/o/r/blob/main/README.md\""),
            "{html}"
        );
        assert!(
            html.contains("href=\"https://github.com/o/r/wiki\""),
            "{html}"
        );
        let html = md("[f](docs/x.md)");
        assert!(!html.contains("<a"), "{html}");
    }

    #[test]
    fn bare_urls_and_issue_references_become_links() {
        let issue = |n: u64| (n == 7).then(|| "7.html".to_string());
        let links = Links {
            base: None,
            local: &|_| None,
            issue: &issue,
        };
        let html = render(
            "see https://example.com/a_(b). and #7, not #8 or a#7 or #7x; `https://code.example`",
            &links,
        );
        assert!(
            html.contains("<a href=\"https://example.com/a_(b)\">https://example.com/a_(b)</a>."),
            "{html}"
        );
        assert!(html.contains("<a href=\"7.html\">#7</a>,"), "{html}");
        assert_eq!(html.matches("<a ").count(), 2, "{html}");
        assert!(html.contains("<code>https://code.example</code>"), "{html}");
    }

    #[test]
    fn gfm_extensions_render() {
        let html = md("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n\n~~old~~");
        assert!(
            html.contains("<table>") && html.contains("<th>a</th>"),
            "{html}"
        );
        assert!(html.contains("type=\"checkbox\""), "{html}");
        assert!(html.contains("<del>old</del>"), "{html}");
    }

    #[test]
    fn schemes_are_detected_like_a_browser() {
        assert_eq!(
            scheme_of(" \u{1}JavaScript:x").as_deref(),
            Some("javascript")
        );
        assert_eq!(scheme_of("jav\nascript:x").as_deref(), Some("javascript"));
        assert_eq!(scheme_of("a/b:c"), None);
        assert_eq!(scheme_of("./x:y"), None);
        assert_eq!(scheme_of("x.md"), None);
    }
}
