//! Find the images, videos and attachment links in a markdown body.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::{MediaKind, MediaLine, MediaRef};

/// Media in a markdown body, in document order.
/// `repo_host` enables the bare-URL rule for `https://<repo_host>/user-attachments/assets/<id>`.
pub fn find_media(body: &str, repo_host: Option<&str>) -> Vec<MediaLine> {
    let lines = LineIndex::new(body);
    let found = scan(body, &lines, repo_host);
    found
        .iter()
        .map(|item| {
            let covered = lines.covered(&item.span);
            MediaLine {
                line: covered.start,
                start: item.span.start,
                media_only: is_media_only(&lines.text_without(body, &covered, &found)),
                lines: covered,
                media: item.media.clone(),
            }
        })
        .collect()
}

/// Whether what is left of the covered lines, once the media is cut out, is
/// only decoration the renderer can drop.
fn is_media_only(rest: &str) -> bool {
    without_link_wrappers(&without_html_tags(rest))
        .lines()
        .all(|line| without_line_markers(line).is_empty())
}

/// `text` with its HTML tags cut out; a `<` that opens no tag stays.
pub(crate) fn without_html_tags(text: &str) -> String {
    let mut kept = String::new();
    let mut at = 0;
    for tag in tag_spans(text, 0..text.len()) {
        if HtmlTag::parse(&text[tag.clone()]).is_some() {
            kept.push_str(&text[at..tag.start]);
            at = tag.end;
        }
    }
    kept.push_str(&text[at..]);
    kept
}

/// `line` trimmed, without the blockquote and list markers that open it.
fn without_line_markers(line: &str) -> &str {
    let mut rest = line.trim();
    while let Some(after) = rest.strip_prefix('>').or_else(|| strip_list_marker(rest)) {
        rest = after.trim_start();
    }
    rest
}

/// `line` after a leading `-`, `*`, `+` or `N.` list marker, which must be
/// followed by whitespace or end the line.
fn strip_list_marker(line: &str) -> Option<&str> {
    let after = match line.strip_prefix(['-', '*', '+']) {
        Some(after) => after,
        None => {
            let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits == 0 {
                return None;
            }
            line[digits..].strip_prefix('.')?
        }
    };
    (after.is_empty() || after.starts_with(char::is_whitespace)).then_some(after)
}

/// `text` without link brackets and the `(url)` after a closing bracket, so a
/// link whose text was media leaves nothing behind.
fn without_link_wrappers(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(bracket) = rest.find(['[', ']']) {
        kept.push_str(&rest[..bracket]);
        let closing = rest.as_bytes()[bracket] == b']';
        rest = &rest[bracket + 1..];
        if closing && rest.starts_with('(') {
            rest = rest.find(')').map_or("", |paren| &rest[paren + 1..]);
        }
    }
    kept.push_str(rest);
    kept
}

/// A media reference and the byte range of its markup in the body.
struct Found {
    span: Range<usize>,
    media: MediaRef,
}

fn scan(body: &str, lines: &LineIndex, repo_host: Option<&str>) -> Vec<Found> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut found = Vec::new();
    let mut html = HtmlMedia::default();
    let mut in_code_block = false;
    // A line's text can arrive as several events; test the line once.
    let mut attachment_line = None;
    let mut image: Option<(Range<usize>, String, String)> = None;
    for (event, range) in Parser::new_ext(body, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                image = Some((range, dest_url.to_string(), String::new()));
            }
            Event::Text(text) => match image.as_mut() {
                Some((_, _, alt)) => alt.push_str(&text),
                None if !in_code_block => {
                    let line = lines.line_of(range.start);
                    if attachment_line != Some(line)
                        && let Some(item) = attachment(body, lines.span(body, line), repo_host)
                    {
                        attachment_line = Some(line);
                        found.push(item);
                    }
                }
                None => {}
            },
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, alt)) = image.as_mut() {
                    alt.push(' ');
                }
            }
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::InlineHtml(_) => html.tag(body, range, &mut found),
            Event::Start(Tag::HtmlBlock) => {
                for tag in tag_spans(body, range) {
                    html.tag(body, tag, &mut found);
                }
                html.end_block(&mut found);
            }
            Event::End(TagEnd::Image) => {
                if let Some((span, url, alt)) = image.take() {
                    found.push(Found {
                        span,
                        media: media_ref(MediaKind::Image, url, &alt),
                    });
                }
            }
            Event::End(end) if !is_inline(&end) => html.end_block(&mut found),
            _ => {}
        }
    }
    html.end_block(&mut found);
    // A video is recorded when its block ends, after media that follow it.
    found.sort_by_key(|item| item.span.start);
    found
}

fn is_inline(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

/// The attachment on the source line at `line`, when its trimmed text is
/// exactly `https://<repo_host>/user-attachments/assets/<id>`.
fn attachment(body: &str, line: Range<usize>, repo_host: Option<&str>) -> Option<Found> {
    let text = &body[line.clone()];
    let url = text.trim();
    let id = url
        .strip_prefix("https://")?
        .strip_prefix(repo_host?)?
        .strip_prefix("/user-attachments/assets/")?;
    if id.is_empty() || id.contains(['/', '?', '#']) || id.contains(char::is_whitespace) {
        return None;
    }
    let start = line.start + (text.len() - text.trim_start().len());
    Some(Found {
        span: start..start + url.len(),
        media: media_ref(MediaKind::Attachment, url.to_string(), ""),
    })
}

/// Turns HTML tags, fed one at a time in document order, into media. A
/// `<video>` stays open across tags until its `</video>` or the end of the
/// block, because its source may come from a later `<source>` tag.
#[derive(Default)]
struct HtmlMedia {
    video: Option<OpenVideo>,
}

/// A `<video>` tag whose close tag has not been seen yet.
struct OpenVideo {
    tag: Range<usize>,
    src: Option<String>,
}

impl HtmlMedia {
    /// Records the media, if any, of the HTML tag at `span`.
    fn tag(&mut self, body: &str, span: Range<usize>, found: &mut Vec<Found>) {
        let Some(tag) = HtmlTag::parse(&body[span.clone()]) else {
            return;
        };
        match (tag.name.as_str(), tag.closing) {
            ("img", false) => {
                if let Some(src) = tag.attr("src") {
                    let alt = tag.attr("alt").unwrap_or("");
                    let media = media_ref(MediaKind::Image, src.to_string(), alt);
                    found.push(Found { span, media });
                }
            }
            ("video", false) => {
                self.end_block(found);
                self.video = Some(OpenVideo {
                    tag: span,
                    src: tag.attr("src").map(str::to_string),
                });
            }
            ("source", false) => {
                if let Some(video) = self.video.as_mut().filter(|video| video.src.is_none()) {
                    video.src = tag.attr("src").map(str::to_string);
                }
            }
            ("video", true) => {
                if let Some(video) = self.video.take() {
                    video.emit(span.end, found);
                }
            }
            _ => {}
        }
    }

    /// Closes any open `<video>` at the end of its block, spanning its open tag.
    fn end_block(&mut self, found: &mut Vec<Found>) {
        if let Some(video) = self.video.take() {
            let end = video.tag.end;
            video.emit(end, found);
        }
    }
}

impl OpenVideo {
    /// Records the video, spanning its open tag through byte `end`; a video
    /// that never got a source stays prose.
    fn emit(self, end: usize, found: &mut Vec<Found>) {
        if let Some(src) = self.src {
            let media = media_ref(MediaKind::Video, src, "");
            found.push(Found {
                span: self.tag.start..end,
                media,
            });
        }
    }
}

/// The byte ranges of the tags inside `block`, each running `<` through the
/// `>` that closes it outside any quoted attribute value.
pub(crate) fn tag_spans(body: &str, block: Range<usize>) -> Vec<Range<usize>> {
    let bytes = body.as_bytes();
    let mut spans = Vec::new();
    let mut at = block.start;
    while let Some(open) = body[at..block.end].find('<').map(|offset| at + offset) {
        let mut quote = None;
        let mut close = None;
        for (index, &byte) in bytes.iter().enumerate().take(block.end).skip(open + 1) {
            match (quote, byte) {
                (Some(q), _) if byte == q => quote = None,
                (Some(_), _) => {}
                (None, b'"' | b'\'') => quote = Some(byte),
                (None, b'>') => {
                    close = Some(index);
                    break;
                }
                (None, _) => {}
            }
        }
        let Some(close) = close else { break };
        spans.push(open..close + 1);
        at = close + 1;
    }
    spans
}

/// One HTML tag, lowercased name and attributes as written.
pub(crate) struct HtmlTag<'a> {
    pub(crate) name: String,
    /// A close tag such as `</video>`.
    pub(crate) closing: bool,
    attrs: Vec<(String, &'a str)>,
}

impl<'a> HtmlTag<'a> {
    /// Parses `text` when it is a single tag such as `<img src=x alt="y">`.
    pub(crate) fn parse(text: &'a str) -> Option<Self> {
        let inner = text.trim().strip_prefix('<')?.strip_suffix('>')?;
        let inner = inner.strip_suffix('/').unwrap_or(inner);
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, inner),
        };
        let name_len = inner
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(inner.len());
        if name_len == 0 {
            return None;
        }
        let name = inner[..name_len].to_ascii_lowercase();
        let attrs = parse_attrs(&inner[name_len..]);
        Some(Self {
            name,
            closing,
            attrs,
        })
    }

    pub(crate) fn attr(&self, name: &str) -> Option<&'a str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|&(_, value)| value)
    }
}

/// Parses `key="v" key='v' key=v key` pairs; a bare key has an empty value.
fn parse_attrs(mut rest: &str) -> Vec<(String, &str)> {
    let mut attrs = Vec::new();
    loop {
        rest = rest.trim_start();
        let key_len = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        if key_len == 0 {
            match rest.chars().next() {
                Some(stray) => {
                    rest = &rest[stray.len_utf8()..];
                    continue;
                }
                None => return attrs,
            }
        }
        let key = rest[..key_len].to_ascii_lowercase();
        rest = rest[key_len..].trim_start();
        let value = match rest.strip_prefix('=') {
            Some(after) => {
                let (value, remainder) = split_attr_value(after.trim_start());
                rest = remainder;
                value
            }
            None => "",
        };
        attrs.push((key, value));
    }
}

/// Splits a quoted or unquoted attribute value off the front of `text`.
fn split_attr_value(text: &str) -> (&str, &str) {
    match text.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let quoted = &text[1..];
            match quoted.find(quote) {
                Some(close) => (&quoted[..close], &quoted[close + 1..]),
                None => (quoted, ""),
            }
        }
        _ => {
            let end = text.find(char::is_whitespace).unwrap_or(text.len());
            text.split_at(end)
        }
    }
}

/// Builds a reference whose label is `label` with whitespace runs collapsed
/// when it holds any text, else the URL's file name, else the whole URL.
fn media_ref(kind: MediaKind, url: String, label: &str) -> MediaRef {
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    let label = if label.is_empty() {
        file_name(&url).unwrap_or(&url).to_string()
    } else {
        label
    };
    MediaRef { kind, url, label }
}

/// The last path segment of `url`, ignoring any query or fragment.
fn file_name(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().filter(|name| !name.is_empty())
}

/// Maps byte offsets to 0-based source lines (the body split on `\n`).
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(body: &str) -> Self {
        let starts = std::iter::once(0)
            .chain(body.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Self { starts }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset) - 1
    }

    /// The byte range of `line`, without its `\n`.
    fn span(&self, body: &str, line: usize) -> Range<usize> {
        self.starts[line]
            ..self
                .starts
                .get(line + 1)
                .map_or(body.len(), |&next| next - 1)
    }

    /// The text of `lines` with every media span cut out.
    fn text_without(&self, body: &str, lines: &Range<usize>, found: &[Found]) -> String {
        let end = self.span(body, lines.end - 1).end;
        let mut at = self.starts[lines.start];
        let mut rest = String::new();
        for span in found.iter().map(|item| &item.span) {
            if span.end <= at || span.start >= end {
                continue;
            }
            rest.push_str(&body[at..span.start.max(at)]);
            at = span.end;
        }
        if at < end {
            rest.push_str(&body[at..end]);
        }
        rest
    }

    /// The lines a non-empty span touches, as a half-open range.
    fn covered(&self, span: &Range<usize>) -> Range<usize> {
        self.line_of(span.start)..self.line_of(span.end - 1) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{MediaKind, MediaRef};

    const GITHUB: Option<&str> = Some("github.com");

    /// `find_media`'s result with every `start` zeroed, for the tests that
    /// pin the other fields.
    fn media_lines(body: &str, repo_host: Option<&str>) -> Vec<MediaLine> {
        find_media(body, repo_host)
            .into_iter()
            .map(|media| MediaLine { start: 0, ..media })
            .collect()
    }

    #[test]
    fn start_is_the_byte_where_each_media_markup_starts() {
        let body = "ab ![x](https://x.test/x.png)\n<img src=\"https://x.test/y.png\">";

        let starts: Vec<usize> = find_media(body, GITHUB).iter().map(|m| m.start).collect();

        assert_eq!(starts, [3, 30]);
    }

    fn m(
        line: usize,
        lines: std::ops::Range<usize>,
        media_only: bool,
        kind: MediaKind,
        url: &str,
        label: &str,
    ) -> MediaLine {
        MediaLine {
            line,
            start: 0,
            lines,
            media_only,
            media: MediaRef {
                kind,
                url: url.to_string(),
                label: label.to_string(),
            },
        }
    }

    #[test]
    fn markdown_image_on_its_own_line_is_media_only() {
        assert_eq!(
            media_lines("Intro\n![shot](https://x.test/a.png)\nOutro", GITHUB),
            vec![m(
                1,
                1..2,
                true,
                MediaKind::Image,
                "https://x.test/a.png",
                "shot"
            )]
        );
    }

    #[test]
    fn empty_alt_falls_back_to_the_file_name_without_query() {
        assert_eq!(
            media_lines("![](https://x.test/dir/b.png?raw=1)", GITHUB),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Image,
                "https://x.test/dir/b.png?raw=1",
                "b.png"
            )]
        );
    }

    #[test]
    fn images_inside_prose_are_not_media_only() {
        assert_eq!(
            media_lines(
                "see ![a](https://x.test/1.png) and ![b](https://x.test/2.png) end",
                GITHUB
            ),
            vec![
                m(
                    0,
                    0..1,
                    false,
                    MediaKind::Image,
                    "https://x.test/1.png",
                    "a"
                ),
                m(
                    0,
                    0..1,
                    false,
                    MediaKind::Image,
                    "https://x.test/2.png",
                    "b"
                ),
            ]
        );
    }

    #[test]
    fn images_inside_code_are_not_media() {
        assert_eq!(
            media_lines(
                "```\n![x](https://x.test/c.png)\n```\n`![y](https://x.test/d.png)`",
                GITHUB
            ),
            vec![]
        );
    }

    #[test]
    fn inline_img_tag_takes_its_alt_as_label() {
        assert_eq!(
            media_lines(
                "Before <img src=\"https://x.test/e.png\" alt=\"E\"> after",
                GITHUB
            ),
            vec![m(
                0,
                0..1,
                false,
                MediaKind::Image,
                "https://x.test/e.png",
                "E"
            )]
        );
    }

    #[test]
    fn img_block_with_single_quotes_and_no_alt_uses_the_file_name() {
        assert_eq!(
            media_lines("<img src='https://x.test/f.jpg'>", GITHUB),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Image,
                "https://x.test/f.jpg",
                "f.jpg"
            )]
        );
    }

    #[test]
    fn multi_line_img_in_an_html_block_covers_every_line_of_its_tag() {
        let body = "<p align=\"center\">\n  <img width=\"400\"\n    alt=\"Login page\"\n    src=\"https://x.test/login.png\">\n</p>";
        assert_eq!(
            media_lines(body, GITHUB),
            vec![m(
                1,
                1..4,
                true,
                MediaKind::Image,
                "https://x.test/login.png",
                "Login page"
            )]
        );
    }

    #[test]
    fn video_with_src_runs_through_its_close_tag() {
        assert_eq!(
            media_lines(
                "<video src=\"https://x.test/demo.mp4\" controls></video>",
                GITHUB
            ),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Video,
                "https://x.test/demo.mp4",
                "demo.mp4"
            )]
        );
    }

    #[test]
    fn video_with_a_multi_line_open_tag_covers_through_its_close_tag() {
        assert_eq!(
            media_lines(
                "<video controls\n  src=\"https://x.test/v2.mov\">\n</video>",
                GITHUB
            ),
            vec![m(
                0,
                0..3,
                true,
                MediaKind::Video,
                "https://x.test/v2.mov",
                "v2.mov"
            )]
        );
    }

    #[test]
    fn video_without_src_takes_its_first_source_child() {
        let body = "<video controls>\n<source src=\"https://x.test/s.webm\" type=\"video/webm\">\n</video>";
        assert_eq!(
            media_lines(body, GITHUB),
            vec![m(
                0,
                0..3,
                true,
                MediaKind::Video,
                "https://x.test/s.webm",
                "s.webm"
            )]
        );
    }

    const ATTACHMENT_BODY: &str = "Demo:\n\nhttps://github.com/user-attachments/assets/0f1e2d3c-aaaa-bbbb-cccc-123456789abc\n";

    #[test]
    fn bare_attachment_url_on_the_repo_host_is_an_attachment() {
        assert_eq!(
            media_lines(ATTACHMENT_BODY, GITHUB),
            vec![m(
                2,
                2..3,
                true,
                MediaKind::Attachment,
                "https://github.com/user-attachments/assets/0f1e2d3c-aaaa-bbbb-cccc-123456789abc",
                "0f1e2d3c-aaaa-bbbb-cccc-123456789abc"
            )]
        );
    }

    #[test]
    fn attachment_urls_need_a_repo_host() {
        assert_eq!(media_lines(ATTACHMENT_BODY, None), vec![]);
    }

    #[test]
    fn attachment_urls_on_another_host_are_not_media() {
        assert_eq!(
            media_lines(ATTACHMENT_BODY, Some("ghe.example.com")),
            vec![]
        );
    }

    #[test]
    fn attachment_urls_follow_an_enterprise_repo_host() {
        assert_eq!(
            media_lines(
                "https://ghe.example.com/user-attachments/assets/abc",
                Some("ghe.example.com")
            ),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Attachment,
                "https://ghe.example.com/user-attachments/assets/abc",
                "abc"
            )]
        );
    }

    #[test]
    fn attachment_url_inside_prose_is_not_media() {
        assert_eq!(
            media_lines(
                "see https://github.com/user-attachments/assets/abc here",
                GITHUB
            ),
            vec![]
        );
    }

    #[test]
    fn image_wrapped_in_a_link_is_media_only() {
        assert_eq!(
            media_lines(
                "[![CI](https://x.test/badge.svg)](https://x.test/ci)",
                GITHUB
            ),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Image,
                "https://x.test/badge.svg",
                "CI"
            )]
        );
    }

    #[test]
    fn image_in_a_blockquote_is_media_only() {
        assert_eq!(
            media_lines("> ![q](https://x.test/q.png)", GITHUB),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Image,
                "https://x.test/q.png",
                "q"
            )]
        );
    }

    #[test]
    fn images_in_list_items_are_media_only() {
        assert_eq!(
            media_lines(
                "- ![l](https://x.test/l.png)\n1. ![n](https://x.test/n.png)",
                GITHUB
            ),
            vec![
                m(0, 0..1, true, MediaKind::Image, "https://x.test/l.png", "l"),
                m(1, 1..2, true, MediaKind::Image, "https://x.test/n.png", "n"),
            ]
        );
    }

    #[test]
    fn alt_text_across_lines_collapses_to_single_spaces() {
        assert_eq!(
            media_lines("![two\nlines](https://x.test/g.png)", GITHUB),
            vec![m(
                0,
                0..2,
                true,
                MediaKind::Image,
                "https://x.test/g.png",
                "two lines"
            )]
        );
    }

    #[test]
    fn image_inside_link_text_with_prose_is_not_media_only() {
        assert_eq!(
            media_lines(
                "[see ![i](https://x.test/i.png) here](https://x.test)",
                GITHUB
            ),
            vec![m(
                0,
                0..1,
                false,
                MediaKind::Image,
                "https://x.test/i.png",
                "i"
            )]
        );
    }

    #[test]
    fn img_tag_without_src_stays_prose() {
        assert_eq!(media_lines("<img alt=\"x\">", GITHUB), vec![]);
    }

    #[test]
    fn image_title_is_not_part_of_the_url_or_label() {
        assert_eq!(
            media_lines("![t](https://x.test/t.png \"Title\")", GITHUB),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Image,
                "https://x.test/t.png",
                "t"
            )]
        );
    }

    #[test]
    fn reference_style_image_resolves_its_url() {
        assert_eq!(
            media_lines("![r][ref]\n\n[ref]: https://x.test/r.png", GITHUB),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Image,
                "https://x.test/r.png",
                "r"
            )]
        );
    }

    #[test]
    fn html_tags_sharing_the_line_leave_it_media_only() {
        assert_eq!(
            media_lines("<p><img src=\"https://x.test/p.png\"></p>", GITHUB),
            vec![m(
                0,
                0..1,
                true,
                MediaKind::Image,
                "https://x.test/p.png",
                "p.png"
            )]
        );
    }

    #[test]
    fn empty_and_plain_bodies_hold_no_media() {
        assert_eq!(media_lines("", Some("github.com")), vec![]);
        assert_eq!(media_lines("plain prose", Some("github.com")), vec![]);
    }
}
