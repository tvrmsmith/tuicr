//! GitHub's collapsible `<details>` sections, found in a body's HTML blocks.

use std::collections::BTreeSet;
use std::ops::Range;

use crate::media::detect::{HtmlTag, tag_spans, without_html_tags};

/// One `<details>` section with a matching `</details>`, in source bytes.
pub(super) struct Section {
    /// The opening tag's order among every `<details>` tag in the source, from 0.
    pub id: usize,
    /// The `<details>` tag's start through the end of the `</summary>` tag
    /// that follows it, or through the `<details>` tag alone without one.
    pub header: Range<usize>,
    /// The matching `</details>` tag.
    pub close: Range<usize>,
    /// The summary's text, tags stripped and whitespace collapsed, or
    /// `Details` when it is missing or empty.
    pub label: String,
    /// The tag carries `open`, so the section starts expanded.
    pub open: bool,
    /// The id of the section this one sits inside. A parent missing from
    /// the result never closed, and neither did any section around it.
    pub parent: Option<usize>,
}

impl Section {
    /// Whether the section renders expanded: open by default unless
    /// `toggled` names it, or collapsed by default and named.
    pub fn expanded(&self, toggled: &BTreeSet<usize>) -> bool {
        self.open != toggled.contains(&self.id)
    }
}

/// The sections opened by tags in `blocks`, the byte ranges of the source's
/// HTML blocks in order, sorted by id. A `<details>` with no matching close
/// is no section, so a stray tag never hides the rest of the body.
pub(super) fn sections(src: &str, blocks: &[Range<usize>]) -> Vec<Section> {
    let tags: Vec<(Range<usize>, HtmlTag)> = blocks
        .iter()
        .filter(|block| !is_opaque(&src[(*block).clone()]))
        .flat_map(|block| outside_comments(src, block.clone()))
        .flat_map(|part| tag_spans(src, part))
        .filter_map(|span| HtmlTag::parse(&src[span.clone()]).map(|tag| (span, tag)))
        .collect();
    let mut found = Vec::new();
    // Sections still waiting for their `</details>`.
    let mut pending: Vec<Section> = Vec::new();
    let mut next_id = 0;
    let mut at = 0;
    while let Some((span, tag)) = tags.get(at) {
        at += 1;
        if tag.name != "details" {
            continue;
        }
        if tag.closing {
            if let Some(section) = pending.pop() {
                found.push(Section {
                    close: span.clone(),
                    ..section
                });
            }
            continue;
        }
        let (header_end, label) = match summary(src, span.end, &tags[at..]) {
            Some((consumed, end, label)) => {
                at += consumed;
                (end, label)
            }
            None => (span.end, String::new()),
        };
        let label = if label.is_empty() {
            "Details".to_string()
        } else {
            label
        };
        pending.push(Section {
            id: next_id,
            header: span.start..header_end,
            // Replaced when the matching `</details>` arrives.
            close: header_end..header_end,
            label,
            open: tag.attr("open").is_some(),
            parent: pending.last().map(|section| section.id),
        });
        next_id += 1;
    }
    found.sort_by_key(|section| section.id);
    found
}

/// The `<summary>` element at the head of `tags`, when only whitespace
/// separates it from `after`, the end of its `<details>` tag: how many tags
/// it spans, the end of its `</summary>`, and its label.
fn summary(
    src: &str,
    after: usize,
    tags: &[(Range<usize>, HtmlTag)],
) -> Option<(usize, usize, String)> {
    let (open, tag) = tags.first()?;
    let adjacent = src
        .get(after..open.start)
        .is_some_and(|gap| gap.trim().is_empty());
    if tag.name != "summary" || tag.closing || !adjacent {
        return None;
    }
    // A `<details>` before any `</summary>` means this summary never closed.
    let close = tags
        .iter()
        .skip(1)
        .take_while(|(_, tag)| tag.name != "details")
        .position(|(_, tag)| tag.name == "summary" && tag.closing)?
        + 1;
    let end = &tags[close].0;
    let text = without_html_tags(&src[open.end..end.start]);
    let label = text.split_whitespace().collect::<Vec<_>>().join(" ");
    Some((close + 1, end.end, label))
}

/// The parts of `block` outside HTML comments, whose tags are text. Found
/// before splitting tags because a `>` inside a comment would end it early.
/// A comment with no `-->` runs to the block's end.
fn outside_comments(src: &str, block: Range<usize>) -> Vec<Range<usize>> {
    let mut parts = Vec::new();
    let mut at = block.start;
    while let Some(open) = src[at..block.end].find("<!--").map(|offset| at + offset) {
        parts.push(at..open);
        let text = open + "<!--".len();
        let Some(close) = src[text..block.end].find("-->") else {
            return parts;
        };
        at = text + close + "-->".len();
    }
    parts.push(at..block.end);
    parts
}

/// Whether `block` is a comment or a `<pre>`, `<script>`, `<style>` or
/// `<textarea>` block, whose tags are text rather than markup.
fn is_opaque(block: &str) -> bool {
    let Some(rest) = block.trim_start().strip_prefix('<') else {
        return false;
    };
    rest.starts_with("!--")
        || ["pre", "script", "style", "textarea"].iter().any(|name| {
            rest.get(..name.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(name))
                && !rest[name.len()..].starts_with(|c: char| c.is_ascii_alphanumeric())
        })
}
