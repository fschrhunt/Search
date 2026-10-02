//! Small, defensive helpers for reading text out of server-rendered HTML.
//!
//! These are deliberately simple scanners, not a DOM: providers' result markup
//! is small and stable, and a browser-grade parser would be more code than the
//! whole crate. Every helper returns empty on input it cannot classify, so a
//! markup change degrades a provider rather than failing the query.

use std::sync::LazyLock;

/// Compile a pattern known good at build time. A bad literal is a build bug CI
/// catches, not a runtime state, so this is the one place an `expect` is earned.
/// Scoped allow, proof: static pattern behind a `LazyLock`.
#[allow(clippy::expect_used)]
fn static_regex(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("static pattern compiles")
}

/// Whitespace runs, collapsed to a single space.
static SPACE: LazyLock<regex::Regex> = LazyLock::new(|| static_regex(r"\s+"));

/// Any HTML tag.
static TAG: LazyLock<regex::Regex> = LazyLock::new(|| static_regex(r"<[^>]+>"));

/// Numeric character references, `&#123;` and `&#x1F600;`.
static NUMERIC: LazyLock<regex::Regex> = LazyLock::new(|| static_regex(r"&#(x?[0-9A-Fa-f]+);"));

/// Strip tags, unescape entities, collapse whitespace, and trim.
pub(super) fn text(html: &str) -> String {
    let unescaped = unescape(html);
    let stripped = TAG.replace_all(&unescaped, " ");
    SPACE.replace_all(&stripped, " ").trim().to_string()
}

/// Collapse all whitespace runs to single spaces, for snippet text that came
/// from inline markup and may carry doubled spaces or newlines.
pub(super) fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove tags and unescape entities, collapsing whitespace for display.
pub(super) fn strip_tags(html: &str) -> String {
    let unescaped = unescape(html);
    collapse_ws(&TAG.replace_all(&unescaped, " "))
}

/// Cut a string to `n` characters on a word boundary where one is near.
pub(super) fn truncate(value: &str, n: usize) -> String {
    if value.chars().count() <= n {
        return value.to_string();
    }
    let head: String = value.chars().take(n).collect();
    match head.rfind(char::is_whitespace) {
        Some(cut) if cut > n / 2 => format!("{}…", head[..cut].trim_end()),
        _ => format!("{head}…"),
    }
}

/// The value of the first `name="..."` attribute occurrence in `haystack`,
/// where `prefix` includes the `name="` and optional leading space, e.g.
/// `href="`.
pub(super) fn first_attr(haystack: &str, prefix: &str) -> Option<String> {
    let start = haystack.find(prefix)? + prefix.len();
    let rest = &haystack[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The inner HTML of the first element named `tag`, optionally requiring that
/// its opening tag contains every string in `classes`.
pub(super) fn first_tag(haystack: &str, tag: &str, classes: &[&str]) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = haystack;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start..];
        let end = after.find('>')?;
        let head = &after[..=end];
        if classes.iter().all(|c| head.contains(c)) {
            let body = &after[end + 1..];
            return Some(match body.find(&close) {
                Some(stop) => body[..stop].to_string(),
                None => body.to_string(),
            });
        }
        rest = &after[end + 1..];
    }
    None
}

/// Split `html` on each occurrence of `marker`, returning the pieces after the
/// marker up to the next `boundary`. Used by the result-list parsers.
pub(super) fn split_after<'a>(html: &'a str, marker: &str, boundary: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(marker) {
        let block = &rest[at + marker.len()..];
        let block = match block.find(boundary) {
            Some(stop) if stop > 0 => &block[..stop],
            _ => block,
        };
        out.push(block);
        rest = &rest[at + marker.len()..];
    }
    out
}

/// Decode the handful of entities that appear in search-result text. A full
/// entity table is unnecessary here and would be another thing to maintain.
pub(super) fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let named = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        ("&#x2F;", "/"),
        ("&#47;", "/"),
    ];
    let mut out = value.to_string();
    for (from, to) in named {
        out = out.replace(from, to);
    }
    // Numeric forms: &#123; and &#x1F600;
    NUMERIC
        .replace_all(&out, |caps: &regex::Captures| {
            let raw = &caps[1];
            let code = match raw.strip_prefix('x') {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => raw.parse::<u32>().ok(),
            };
            code.and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_default()
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_and_tags_are_decoded() {
        assert_eq!(text("<b>a &amp; b</b>"), "a & b");
        assert_eq!(text("caf&#233;"), "café");
        assert_eq!(text("x&#x2F;y"), "x/y");
    }

    #[test]
    fn attributes_are_read_between_quotes() {
        let html = r#"<a href="https://example.com/x" class="title">"#;
        assert_eq!(
            first_attr(html, "href=\"").as_deref(),
            Some("https://example.com/x")
        );
    }

    #[test]
    fn first_tag_honors_required_classes() {
        let html = r#"<div class="other"><p>x</p></div><div class="want"><p>hit</p></div>"#;
        let found = first_tag(html, "div", &["want"]).unwrap();
        assert!(found.contains("hit"));
        assert!(first_tag(html, "div", &["absent"]).is_none());
    }

    #[test]
    fn truncate_keeps_a_word_boundary() {
        let cut = truncate("one two three four", 9);
        assert!(cut.ends_with('…'));
        assert!(cut.len() < 16);
    }
}
