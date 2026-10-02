//! Turn a fetched body into readable text.
//!
//! HTML is reduced by scanning: script, style, and similar elements are dropped
//! whole; block-level tags introduce line breaks; every other tag becomes a
//! space. This is not a DOM and does not pretend to be one — it exists so a
//! model reads prose, not markup. Non-HTML bodies pass through `sanitize`.

/// Elements whose entire contents are removed (they are never prose).
const DROPPED: &[&str] = &[
    "script", "style", "noscript", "svg", "template", "head", "nav", "footer", "aside", "form",
    "iframe", "object", "embed",
];

/// Tags that introduce a line break around their content.
const BLOCK: &[&str] = &[
    "p",
    "div",
    "li",
    "tr",
    "br",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "section",
    "article",
    "pre",
    "blockquote",
    "table",
    "ul",
    "ol",
];

/// The largest text a page contributes to the index or a response.
const MAX_TEXT: usize = 200_000;

/// Extract a title and readable text from an HTML document.
pub(super) fn read(body: &str) -> (String, String) {
    let title = title_of(body);
    let text = html_to_text(body);
    (title, text)
}

/// The document title, from the first `<title>` element.
fn title_of(body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    let Some(open) = lower.find("<title") else {
        return String::new();
    };
    let Some(gt) = lower[open..].find('>') else {
        return String::new();
    };
    let start = open + gt + 1;
    let Some(close) = lower[start..].find("</title>") else {
        return String::new();
    };
    collapse(&decode(&body[start..start + close]))
}

/// Reduce HTML to text: drop non-prose elements, break at block tags, collapse
/// whitespace, and cap the length.
fn html_to_text(body: &str) -> String {
    let mut out = String::with_capacity(body.len() / 2);
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // `i < bytes.len()` is the loop condition, so `bytes[i]` is in range.
        if bytes.get(i) == Some(&b'<') {
            let Some(gt) = body[i..].find('>') else {
                break;
            };
            let tag_end = i + gt + 1;
            let raw = &body[i + 1..i + gt];
            let name = tag_name(raw);
            if DROPPED.contains(&name.as_str()) {
                // Skip to the matching close tag; a self-closing or unmatched
                // open is skipped to the end, which is safe (it only drops text).
                let close = format!("</{name}>");
                match body[tag_end..].to_ascii_lowercase().find(&close) {
                    Some(rel) => i = tag_end + rel + close.len(),
                    None => break,
                }
                if BLOCK.contains(&name.as_str()) {
                    out.push('\n');
                }
                continue;
            }
            if BLOCK.contains(&name.as_str()) {
                out.push('\n');
            } else {
                out.push(' ');
            }
            i = tag_end;
        } else {
            let next = body[i..].find('<').map(|n| i + n).unwrap_or(bytes.len());
            out.push_str(&decode(&body[i..next]));
            i = next;
        }
    }
    cap(clean(&out))
}

/// The tag's lowercase element name, ignoring any attributes.
fn tag_name(raw: &str) -> String {
    let end = raw
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(raw.len());
    raw[..end].to_ascii_lowercase()
}

/// Collapse whitespace and trim each line, dropping control characters.
fn clean(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .fold(String::new(), |mut acc, line| {
            if line.is_empty() {
                if !acc.ends_with('\n') && !acc.is_empty() {
                    acc.push('\n');
                }
            } else {
                acc.push_str(&line);
                acc.push('\n');
            }
            acc
        })
        .trim()
        .to_string()
}

/// Make any byte stream valid UTF-8 text and strip control characters, for
/// non-HTML bodies that still need to be safe to store and read.
pub(super) fn sanitize(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' | '\t' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    cap(out)
}

/// Collapse whitespace runs to single spaces and trim.
fn collapse(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cap text at `MAX_TEXT` characters on a character boundary.
fn cap(text: String) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text;
    }
    text.chars().take(MAX_TEXT).collect()
}

/// Decode the entities that appear in prose text.
fn decode(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = value.to_string();
    for (from, to) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
    ] {
        out = out.replace(from, to);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_prose_are_extracted() {
        let html = "<html><head><title>Doc Title</title><style>x{}</style></head>\
                    <body><nav>skip</nav><script>evil()</script>\
                    <main><h1>Heading</h1><p>First paragraph.</p><p>Second paragraph.</p></main></body></html>";
        let (title, text) = read(html);
        assert_eq!(title, "Doc Title");
        assert!(!text.contains("evil"), "script dropped");
        assert!(!text.contains("skip"), "nav dropped");
        assert!(text.contains("First paragraph."));
        assert!(text.contains("Second paragraph."));
    }

    #[test]
    fn block_tags_become_line_breaks() {
        let (_, text) = read("<p>one</p><p>two</p>");
        assert_eq!(text, "one\ntwo");
    }

    #[test]
    fn entities_are_decoded() {
        let (_, text) = read("<p>a &amp; b</p>");
        assert_eq!(text, "a & b");
    }

    #[test]
    fn control_characters_are_stripped_from_non_html() {
        assert_eq!(sanitize(b"a\x00b\nc"), "ab\nc");
    }
}
