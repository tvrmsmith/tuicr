use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{AnnotatedLine, App};
use crate::forge::traits::{PullRequestCheckStatus, PullRequestInfo, PullRequestReviewStatus};
use crate::media::{MediaKind, MediaLine, MediaRef, detect::find_media};
use crate::model::CommentType;
use crate::syntax::SyntaxHighlighter;
use crate::syntax::markdown_render;
use crate::theme::Theme;
use crate::ui::comment_panel::{self, CommentTypePresentation};
use crate::ui::diff_view::{HEADER_RULE, cursor_indicator, cursor_indicator_spaced};
use crate::ui::styles;

/// What Enter does on a PR-info row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowAction {
    /// Open the media at this index into [`pr_body_media`].
    Media(usize),
    /// Toggle the `<details>` section with this number.
    Details(usize),
}

/// How the PR description renders.
#[derive(Clone, Copy)]
pub(crate) enum BodyRender<'a> {
    /// Highlighted markdown source.
    Source,
    /// Rendered markdown; `toggled` lists the `<details>` sections shown
    /// opposite to their default.
    Markdown { toggled: &'a BTreeSet<usize> },
}

/// One rendered row of the PR-info panel body.
pub(crate) struct PrInfoRow {
    pub line: Line<'static>,
    /// Read by `pr_info_action_at_cursor`, the Enter handler's lookup.
    pub action: Option<RowAction>,
}

/// Every rendered PR-info line is prefixed with a two-column cursor indicator
/// (`cursor_indicator_spaced`), so the wrapped body has that much less room.
/// The annotation/height counters and the renderer MUST wrap
/// `pr_info_rows` at this same width — otherwise `line_annotations`
/// desyncs from the rendered `Vec<Line>` and cursor↔line mapping drifts for
/// every row below the panel.
pub(crate) const PR_INFO_INDICATOR_WIDTH: usize = 2;

/// Width available for wrapped PR-info content at the given viewport width.
pub(crate) fn pr_info_content_width(viewport_width: usize) -> usize {
    viewport_width
        .saturating_sub(PR_INFO_INDICATOR_WIDTH)
        .max(1)
}

/// Rows and the inputs they were built from. The panel is rebuilt several
/// times per frame (and once per `PrInfoLine` under wrap), so `pr_info_rows`
/// reuses these while every input is unchanged. Tests assign `App` fields
/// directly, so the key compares by value, and by `Arc` identity for the
/// theme (a new `Theme` builds a new highlighter).
pub(crate) struct PrInfoRowsCache {
    info: PullRequestInfo,
    width: usize,
    render: CachedRender,
    highlighter: Arc<SyntaxHighlighter>,
    rows: Rc<Vec<PrInfoRow>>,
}

/// Owned [`BodyRender`] for the cache key.
#[derive(Clone, PartialEq, Eq)]
enum CachedRender {
    Source,
    Markdown { toggled: BTreeSet<usize> },
}

impl CachedRender {
    fn of(app: &App) -> Self {
        if app.render_markdown {
            Self::Markdown {
                toggled: app.pr_details_toggled.clone(),
            }
        } else {
            Self::Source
        }
    }

    fn as_body_render(&self) -> BodyRender<'_> {
        match self {
            Self::Source => BodyRender::Source,
            Self::Markdown { toggled } => BodyRender::Markdown { toggled },
        }
    }
}

/// The one place the panel's rows are built from `App` state, so height,
/// annotations, search, and drawing always see the same rows. Empty when
/// there is no PR.
pub(crate) fn pr_info_rows(app: &App) -> Rc<Vec<PrInfoRow>> {
    let Some(info) = app.pr_info.as_ref() else {
        return Rc::default();
    };
    let width = pr_info_content_width(app.diff_state.viewport_width);
    let highlighter = app.theme.syntax_highlighter_arc();
    let render = CachedRender::of(app);
    let mut cache = app.pr_info_rows_cache.borrow_mut();
    if let Some(hit) = cache.as_ref().filter(|hit| {
        hit.width == width
            && hit.render == render
            && Arc::ptr_eq(&hit.highlighter, &highlighter)
            && hit.info == *info
    }) {
        return Rc::clone(&hit.rows);
    }
    let rows = Rc::new(build_pr_info_rows(
        info,
        width,
        &app.theme,
        render.as_body_render(),
    ));
    *cache = Some(PrInfoRowsCache {
        info: info.clone(),
        width,
        render,
        highlighter,
        rows: Rc::clone(&rows),
    });
    rows
}

pub fn pr_info_render_height(app: &App) -> usize {
    pr_info_rows(app).len()
}

pub fn issue_comments_render_height(app: &App) -> usize {
    let Some(info) = app.pr_info.as_ref() else {
        return 0;
    };
    if info.issue_comments.is_empty() {
        return 0;
    }
    let mut height = if app.is_single_file_view { 0 } else { 1 };
    let width = app.diff_state.viewport_width.max(1);
    for comment in &info.issue_comments {
        height += app.comment_display_lines(&comment.body, width);
    }
    height
}

pub fn is_cursor_in_pr_info(app: &App) -> bool {
    pr_info_render_height(app) > 0 && app.diff_state.cursor_line < pr_info_render_height(app)
}

pub fn is_cursor_in_issue_comments(app: &App) -> bool {
    let start = app.issue_comments_start_line();
    let end = start + issue_comments_render_height(app);
    end > start && (start..end).contains(&app.diff_state.cursor_line)
}

pub fn append_pr_info_section(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    line_idx: &mut usize,
    current_line_idx: usize,
) {
    for row in pr_info_rows(app).iter() {
        let mut pr_line = row.line.clone();
        let indicator = cursor_indicator_spaced(*line_idx, current_line_idx);
        pr_line.spans.insert(
            0,
            Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
        );
        lines.push(pr_line);
        *line_idx += 1;
    }
}

/// `visible` is the half-open logical-row range the caller is fully building
/// this frame; boxes outside it are replaced with blank placeholder rows. These
/// comments sit at the very top of the document, so for most of a review they
/// are scrolled past — and formatting them costs the same markdown pass as any
/// other comment box. `width` is the column width, cursor-indicator column
/// included, as every comment box takes.
pub fn append_issue_comments_section(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    width: usize,
    visible: (usize, usize),
) {
    let Some(info) = app.pr_info.as_ref() else {
        return;
    };
    if info.issue_comments.is_empty() {
        return;
    }

    if !app.is_single_file_view {
        let header = format!("═══ PR #{} Comments ", info.details.number);
        lines.push(Line::from(vec![
            Span::styled(
                cursor_indicator_spaced(*line_idx, current_line_idx),
                styles::current_line_indicator_style(&app.theme),
            ),
            Span::styled(header, styles::file_header_style(&app.theme)),
            Span::styled(HEADER_RULE, styles::file_header_style(&app.theme)),
        ]));
        *line_idx += 1;
    }

    let note_type = CommentType::from_id("note");
    let presentation = CommentTypePresentation {
        label: app.comment_type_label(&note_type),
        color: app.comment_type_color(&note_type),
    };
    for comment in &info.issue_comments {
        let rows = app.comment_display_lines(&comment.body, width);
        if !crate::ui::diff_view::comment_box_visible(*line_idx, rows, visible) {
            crate::ui::diff_view::skip_comment_box(lines, line_idx, rows);
            continue;
        }
        for mut comment_line in app.comment_boxes().lines(
            presentation.clone(),
            &comment.body,
            None,
            width,
            comment.author.as_deref(),
        ) {
            let indicator = cursor_indicator(*line_idx, current_line_idx);
            comment_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            );
            lines.push(comment_line);
            *line_idx += 1;
        }
    }
}

/// The media referenced from the PR body, in document order. The one list
/// both [`RowAction::Media`] and the media viewer index into, so repeated
/// URLs (e.g. badges) stay distinct entries.
pub(crate) fn pr_body_media(info: &PullRequestInfo) -> Vec<MediaLine> {
    find_media(&info.details.body, Some(&info.details.repository.host))
}

pub(crate) fn build_pr_info_rows(
    info: &PullRequestInfo,
    width: usize,
    theme: &Theme,
    render: BodyRender,
) -> Vec<PrInfoRow> {
    let mut rows = Vec::new();
    let content_width = width.max(1);
    let details = &info.details;

    push_section_header(
        &mut rows,
        theme,
        format!("═══ PR #{} {} ", details.number, details.title),
    );

    let body = if details.body.trim().is_empty() {
        "(no description)".to_string()
    } else {
        details.body.clone()
    };
    push_body_rows(&mut rows, info, theme, &body, content_width, render);

    push_blank(&mut rows);
    push_section_header(
        &mut rows,
        theme,
        format!("═══ PR #{} Status ", details.number),
    );

    let mut status_parts = Vec::new();
    if details.merged_at.is_some() {
        status_parts.push("Merged".to_string());
    } else if details.closed {
        status_parts.push("Closed".to_string());
    } else {
        status_parts.push(details.state.clone());
    }
    if details.is_draft {
        status_parts.push("Draft".to_string());
    }
    if let Some(decision) = &info.review_decision {
        status_parts.push(humanize_token(decision));
    }
    if let Some(state) = &info.merge_state {
        status_parts.push(format!("merge {}", humanize_token(state)));
    }
    if let Some(mergeable) = &info.mergeable {
        status_parts.push(humanize_token(mergeable));
    }
    push_wrapped_line(
        &mut rows,
        format!("Status: {}", status_parts.join(" · ")),
        content_width,
    );

    let head_short = details.head_sha.chars().take(8).collect::<String>();
    let mut branch_line = format!(
        "{} → {} · {}",
        details.head_ref_name, details.base_ref_name, head_short
    );
    if let Some(author) = &details.author {
        branch_line.push_str(&format!(" · @{author}"));
    }
    if let Some(updated) = details.updated_at {
        branch_line.push_str(&format!(
            " · updated {}",
            updated.format("%Y-%m-%d %H:%M UTC")
        ));
    }
    push_wrapped_line(&mut rows, branch_line, content_width);

    if !info.requested_reviewers.is_empty() {
        push_wrapped_line(
            &mut rows,
            format!("Requested: {}", format_users(&info.requested_reviewers)),
            content_width,
        );
    }

    let approved = reviews_by_state(&info.latest_reviews, "APPROVED");
    let changes = reviews_by_state(&info.latest_reviews, "CHANGES_REQUESTED");
    let commented = reviews_by_state(&info.latest_reviews, "COMMENTED");
    if !approved.is_empty() {
        push_wrapped_line(
            &mut rows,
            format!("Approved: {}", format_users(&approved)),
            content_width,
        );
    }
    if !changes.is_empty() {
        push_wrapped_line(
            &mut rows,
            format!("Changes requested: {}", format_users(&changes)),
            content_width,
        );
    }
    if !commented.is_empty() {
        push_wrapped_line(
            &mut rows,
            format!("Commented: {}", format_users(&commented)),
            content_width,
        );
    }

    if !info.checks.is_empty() {
        push_blank(&mut rows);
        push_wrapped_line(&mut rows, "Checks".to_string(), content_width);
        for check in &info.checks {
            push_row(&mut rows, format_check_line(check, content_width, theme));
        }
    }

    rows
}

/// The Enter action of the cursor row, when the cursor is on a rendered
/// PR-info line. `handle_diff_action`'s `SelectFile` arm dispatches on it.
pub(crate) fn pr_info_action_at_cursor(app: &App) -> Option<RowAction> {
    if !is_cursor_in_pr_info(app) {
        return None;
    }
    let &AnnotatedLine::PrInfoLine { line_idx } =
        app.line_annotations.get(app.diff_state.cursor_line)?
    else {
        return None;
    };
    pr_info_rows(app).get(line_idx)?.action
}

/// One merged run of media-only source lines, replaced by one placeholder
/// row per index in `media`.
struct MediaBlock {
    lines: Range<usize>,
    media: Range<usize>,
}

/// Splits `media` (document order) into merged, non-overlapping placeholder
/// blocks and the inline media that ride after their prose line. Media
/// starting inside a block's covered lines (an `<img>` within a multi-line
/// `<video>`) joins the block, and inline media on a media-only line joins it
/// too; skipping either would stall every later entry.
fn plan_media(media: &[MediaLine]) -> (Vec<MediaBlock>, Vec<(usize, usize)>) {
    let mut blocks = Vec::new();
    let mut inline = Vec::new();
    let mut idx = 0usize;
    while idx < media.len() {
        let line = media[idx].line;
        let same_line_end = idx
            + media[idx..]
                .iter()
                .take_while(|entry| entry.line == line)
                .count();
        let covered_end = media[idx..same_line_end]
            .iter()
            .filter(|entry| entry.media_only)
            .map(|entry| entry.lines.end)
            .max();
        let Some(mut covered_end) = covered_end else {
            inline.extend((idx..same_line_end).map(|i| (line, i)));
            idx = same_line_end;
            continue;
        };
        covered_end = covered_end.max(line + 1);
        let mut end = same_line_end;
        while end < media.len() && media[end].line < covered_end {
            covered_end = covered_end.max(media[end].lines.end);
            end += 1;
        }
        blocks.push(MediaBlock {
            lines: line..covered_end,
            media: idx..end,
        });
        idx = end;
    }
    (blocks, inline)
}

/// Appends the body rows, replacing media-only blocks with one placeholder
/// row per media entry and following each inline media line's last row with
/// its placeholders. Both render paths produce [`BlockRow`]s, so one walk
/// keyed on source position serves them.
fn push_body_rows(
    rows: &mut Vec<PrInfoRow>,
    info: &PullRequestInfo,
    theme: &Theme,
    body: &str,
    content_width: usize,
    render: BodyRender,
) {
    let media = pr_body_media(info);
    let (blocks, inline) = plan_media(&media);
    let body_rows = if let BodyRender::Markdown { toggled } = render {
        let apart: Vec<Range<usize>> = blocks.iter().map(|block| block.lines.clone()).collect();
        markdown_render::render_block(theme, body, content_width, &apart, toggled)
    } else {
        comment_panel::markdown_body_line_groups(theme, body, content_width)
    };

    let mut inline_after: Vec<Vec<usize>> = vec![Vec::new(); body_rows.len()];
    for (line, media_idx) in inline {
        if let Some(anchor) = body_rows.iter().rposition(|row| row.source.contains(&line)) {
            inline_after[anchor].push(media_idx);
        }
    }

    let mut pending = blocks.iter().peekable();
    let push_block = |rows: &mut Vec<PrInfoRow>, block: &MediaBlock| {
        for idx in block.media.clone() {
            rows.push(placeholder_row(
                theme,
                idx,
                &media[idx].media,
                content_width,
            ));
        }
    };
    let details_ids: Vec<Option<usize>> = body_rows.iter().map(|row| row.details).collect();
    let mut skip_until = 0usize;
    for (pos, (row, after)) in body_rows.into_iter().zip(inline_after).enumerate() {
        // A block no row started inside has nothing to replace: a collapsed
        // `<details>` section hid its lines.
        while pending
            .next_if(|block| block.lines.end <= row.source.start)
            .is_some()
        {}
        if let Some(id) = row.details {
            rows.push(PrInfoRow {
                line: row.line,
                action: Some(RowAction::Details(id)),
            });
            // A media block covering the header lands after its last row.
            if details_ids.get(pos + 1) != Some(&Some(id)) {
                while let Some(block) = pending.next_if(|b| b.lines.start <= row.source.start) {
                    push_block(rows, block);
                    skip_until = block.lines.end;
                }
            }
            for idx in after {
                rows.push(placeholder_row(
                    theme,
                    idx,
                    &media[idx].media,
                    content_width,
                ));
            }
            continue;
        }
        while let Some(block) = pending.next_if(|block| block.lines.start <= row.source.start) {
            push_block(rows, block);
            skip_until = block.lines.end;
        }
        if row.source.start < skip_until {
            continue;
        }
        rows.push(PrInfoRow {
            line: row.line,
            action: None,
        });
        for idx in after {
            rows.push(placeholder_row(
                theme,
                idx,
                &media[idx].media,
                content_width,
            ));
        }
    }
}

fn placeholder_row(theme: &Theme, media_idx: usize, media: &MediaRef, width: usize) -> PrInfoRow {
    let label = format!("[{}: {}]", media_kind_word(&media.kind), media.label);
    let text = truncate_placeholder(&label, width);
    PrInfoRow {
        line: Line::from(Span::styled(text, styles::branch_style(theme))),
        action: Some(RowAction::Media(media_idx)),
    }
}

fn media_kind_word(kind: &MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Attachment => "attachment",
    }
}

/// Truncates `text` to `width` display columns, keeping the longest prefix
/// that fits in `width - 1` columns and appending `…` when it is too wide.
/// Never wraps: the result is always exactly one row.
fn truncate_placeholder(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let limit = width.saturating_sub(1);
    let mut kept = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > limit {
            break;
        }
        kept.push(c);
        used += cw;
    }
    kept.push('…');
    kept
}

fn push_row(rows: &mut Vec<PrInfoRow>, line: Line<'static>) {
    rows.push(PrInfoRow { line, action: None });
}

fn push_section_header(rows: &mut Vec<PrInfoRow>, theme: &Theme, title: String) {
    push_row(
        rows,
        Line::from(vec![
            Span::styled(title, styles::file_header_style(theme)),
            Span::styled(HEADER_RULE, styles::file_header_style(theme)),
        ]),
    );
}

fn push_wrapped_line(rows: &mut Vec<PrInfoRow>, text: String, width: usize) {
    let style = Style::default();
    for chunk in wrap_text(&text, width) {
        push_row(rows, Line::from(Span::styled(chunk, style)));
    }
}

fn push_blank(rows: &mut Vec<PrInfoRow>) {
    if rows.last().is_some_and(|row| !row.line.spans.is_empty()) {
        push_row(rows, Line::default());
    }
}

fn format_check_line(
    check: &PullRequestCheckStatus,
    content_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let summary = format!("{} {}", check_glyph(check), format_check_summary(check));
    let mut spans = Vec::new();
    for chunk in wrap_text(&summary, content_width) {
        spans.push(Span::raw(chunk));
    }
    if let Some(url) = check.url.as_deref().filter(|url| !url.is_empty()) {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(url.to_string(), styles::dim_style(theme)));
    }
    Line::from(spans)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let word_width = word.chars().count();
        if current_width == 0 {
            current.push_str(word);
            current_width = word_width;
            continue;
        }
        if current_width + 1 + word_width <= width {
            current.push(' ');
            current.push_str(word);
            current_width += 1 + word_width;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_width = word_width;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn humanize_token(token: &str) -> String {
    token
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_users(users: &[String]) -> String {
    users
        .iter()
        .map(|user| {
            if user.contains('/') {
                user.clone()
            } else {
                format!("@{user}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn reviews_by_state(reviews: &[PullRequestReviewStatus], state: &str) -> Vec<String> {
    reviews
        .iter()
        .filter(|review| review.state == state)
        .filter_map(|review| review.author.clone())
        .collect()
}

fn check_glyph(check: &PullRequestCheckStatus) -> &'static str {
    let outcome = check
        .conclusion
        .as_deref()
        .or(check.status.as_deref())
        .unwrap_or("");
    match outcome {
        "SUCCESS" | "COMPLETED" if check.conclusion.as_deref() == Some("SUCCESS") => "✓",
        "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" => "✗",
        "PENDING" | "IN_PROGRESS" | "QUEUED" | "WAITING" => "○",
        _ => "·",
    }
}

fn format_check_summary(check: &PullRequestCheckStatus) -> String {
    let mut parts = vec![check.name.clone()];
    if let Some(status) = &check.status {
        parts.push(humanize_token(status));
    }
    if let Some(conclusion) = &check.conclusion {
        parts.push(humanize_token(conclusion));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::{ForgeRepository, PullRequestDetails, PullRequestIssueComment};
    use crate::theme::Theme;

    fn sample_info() -> PullRequestInfo {
        PullRequestInfo {
            details: PullRequestDetails {
                repository: ForgeRepository::github("github.com", "owner", "repo"),
                number: 42,
                title: "Add panel".to_string(),
                url: "https://github.com/owner/repo/pull/42".to_string(),
                state: "OPEN".to_string(),
                is_draft: false,
                author: Some("alice".to_string()),
                head_ref_name: "feature".to_string(),
                base_ref_name: "main".to_string(),
                head_sha: "abc1234567890".to_string(),
                base_sha: "def0987654321".to_string(),
                body: "Ship the panel".to_string(),
                updated_at: None,
                closed: false,
                merged_at: None,
                diff_start_sha: None,
            },
            review_decision: Some("REVIEW_REQUIRED".to_string()),
            mergeable: Some("MERGEABLE".to_string()),
            merge_state: Some("BLOCKED".to_string()),
            requested_reviewers: vec!["bob".to_string()],
            latest_reviews: vec![PullRequestReviewStatus {
                author: Some("carol".to_string()),
                state: "APPROVED".to_string(),
                submitted_at: None,
            }],
            checks: vec![PullRequestCheckStatus {
                name: "build".to_string(),
                status: Some("COMPLETED".to_string()),
                conclusion: Some("SUCCESS".to_string()),
                url: Some("https://github.com/owner/repo/actions/runs/1".to_string()),
            }],
            issue_comments: vec![PullRequestIssueComment {
                author: Some("dave".to_string()),
                body: "Looks good".to_string(),
                url: Some("https://github.com/owner/repo/pull/42#issuecomment-1".to_string()),
                created_at: None,
            }],
        }
    }

    /// Row text/media from row 1 (row 0 is the header) up to but excluding
    /// the blank row that precedes the Status header.
    fn body_rows(body: &str, width: usize) -> Vec<(String, Option<RowAction>)> {
        body_rows_with(body, width, false)
    }

    fn body_rows_with(
        body: &str,
        width: usize,
        markdown: bool,
    ) -> Vec<(String, Option<RowAction>)> {
        let mut info = sample_info();
        info.details.body = body.to_string();
        let toggled = BTreeSet::new();
        let render = if markdown {
            BodyRender::Markdown { toggled: &toggled }
        } else {
            BodyRender::Source
        };
        let rows = build_pr_info_rows(&info, width, &Theme::dark(), render);
        let status_header = rows
            .iter()
            .position(|row| row_text(row).starts_with(&format!("═══ PR #{} Status ", 42)))
            .expect("status header row");
        rows[1..status_header - 1]
            .iter()
            .map(|row| (row_text(row), row.action))
            .collect()
    }

    fn row_text(row: &PrInfoRow) -> String {
        row.line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn media_only_line_is_replaced_by_a_placeholder_row() {
        assert_eq!(
            body_rows("Intro\n![shot](https://x.test/a.png)\nOutro", 80),
            vec![
                ("Intro".to_string(), None),
                ("[image: shot]".to_string(), Some(RowAction::Media(0))),
                ("Outro".to_string(), None),
            ]
        );
    }

    #[test]
    fn inline_image_placeholder_follows_the_last_row_of_a_wrapped_line() {
        let rows: Vec<(String, Option<RowAction>)> = body_rows_with(
            "aaaa bbbb cccc dddd eeee ![a](https://x.test/1.png) ffff",
            20,
            true,
        )
        .into_iter()
        .map(|(text, media)| (text.trim_end().to_string(), media))
        .collect();

        assert_eq!(
            rows,
            vec![
                ("aaaa bbbb cccc dddd".to_string(), None),
                ("eeee ▣ a ffff".to_string(), None),
                ("[image: a]".to_string(), Some(RowAction::Media(0))),
            ]
        );
    }

    #[test]
    fn inline_image_keeps_its_prose_row_and_appends_a_placeholder() {
        assert_eq!(
            body_rows("see ![a](https://x.test/1.png) end", 80),
            vec![
                ("see ![a](https://x.test/1.png) end".to_string(), None),
                ("[image: a]".to_string(), Some(RowAction::Media(0))),
            ]
        );
    }

    #[test]
    fn multi_line_media_only_html_block_keeps_lines_outside_its_range() {
        let body = "<p align=\"center\">\n  <img width=\"400\"\n    alt=\"Login page\"\n    src=\"https://x.test/login.png\">\n</p>";
        assert_eq!(
            body_rows(body, 80),
            vec![
                ("<p align=\"center\">".to_string(), None),
                ("[image: Login page]".to_string(), Some(RowAction::Media(0))),
                ("</p>".to_string(), None),
            ]
        );
    }

    #[test]
    fn two_media_only_images_on_one_line_each_get_their_own_placeholder() {
        assert_eq!(
            body_rows("![a](https://x.test/1.png) ![b](https://x.test/2.png)", 80),
            vec![
                ("[image: a]".to_string(), Some(RowAction::Media(0))),
                ("[image: b]".to_string(), Some(RowAction::Media(1))),
            ]
        );
    }

    #[test]
    fn video_and_bare_attachment_url_each_get_a_placeholder_and_their_own_index() {
        let body = "<video src=\"https://x.test/demo.mp4\"></video>\n\nhttps://github.com/user-attachments/assets/abc";
        assert_eq!(
            body_rows(body, 80),
            vec![
                ("[video: demo.mp4]".to_string(), Some(RowAction::Media(0))),
                (String::new(), None),
                ("[attachment: abc]".to_string(), Some(RowAction::Media(1))),
            ]
        );

        let mut info = sample_info();
        info.details.body = body.to_string();
        let media = pr_body_media(&info);
        assert_eq!(media.len(), 2);
        assert_eq!(media[0].media.kind, MediaKind::Video);
        assert_eq!(media[0].media.url, "https://x.test/demo.mp4");
        assert_eq!(media[1].media.kind, MediaKind::Attachment);
        assert_eq!(
            media[1].media.url,
            "https://github.com/user-attachments/assets/abc"
        );
    }

    #[test]
    fn media_starting_inside_a_covered_range_joins_its_placeholder_block() {
        let body = "<video controls>\n<img src=\"https://x.test/i.png\">\n<source src=\"https://x.test/v.mp4\">\n</video>\n\n![after](https://x.test/a.png)";
        assert_eq!(
            body_rows(body, 80),
            vec![
                ("[video: v.mp4]".to_string(), Some(RowAction::Media(0))),
                ("[image: i.png]".to_string(), Some(RowAction::Media(1))),
                (String::new(), None),
                ("[image: after]".to_string(), Some(RowAction::Media(2))),
            ]
        );
    }

    #[test]
    fn inline_media_sharing_a_line_with_media_only_media_keeps_its_placeholder() {
        let body = "<img src=\"https://x.test/a.png\"> <video src=\"https://x.test/b.mp4\">\n</video> more";
        assert_eq!(
            body_rows(body, 80),
            vec![
                ("[image: a.png]".to_string(), Some(RowAction::Media(0))),
                ("[video: b.mp4]".to_string(), Some(RowAction::Media(1))),
                ("</video> more".to_string(), None),
            ]
        );
    }

    #[test]
    fn placeholder_text_truncates_by_display_width_with_an_ellipsis() {
        assert_eq!(
            body_rows("![abcdefghijklmnopqrstuvwxyz](https://x.test/z.png)", 12),
            vec![("[image: abc…".to_string(), Some(RowAction::Media(0)))]
        );
    }

    #[test]
    fn plain_prose_body_is_unaffected_by_media_handling() {
        // Regression guard: `should_render_pr_description_markdown` below
        // must stay green untouched by this change.
        assert_eq!(
            body_rows("plain `code` plain", 80),
            vec![("plain `code` plain".to_string(), None)]
        );
    }

    #[test]
    fn placeholder_style_differs_from_prose_style() {
        let rows = body_rows("Intro\n![shot](https://x.test/a.png)\nOutro", 80);
        assert_eq!(rows.len(), 3);

        let mut info = sample_info();
        info.details.body = "Intro\n![shot](https://x.test/a.png)\nOutro".to_string();
        let full_rows = build_pr_info_rows(&info, 80, &Theme::dark(), BodyRender::Source);
        let intro_row = &full_rows[1];
        let placeholder_row = &full_rows[2];
        assert_eq!(placeholder_row.action, Some(RowAction::Media(0)));
        assert_ne!(
            intro_row.line.spans[0].style, placeholder_row.line.spans[0].style,
            "placeholder style must differ from prose style"
        );
    }

    #[test]
    fn header_status_and_checks_rows_carry_no_media() {
        let rows = build_pr_info_rows(&sample_info(), 80, &Theme::dark(), BodyRender::Source);
        assert!(rows[0].action.is_none());
        for row in &rows {
            let text = row_text(row);
            if text.starts_with("═══") || text.starts_with("Status:") || text.starts_with('✓')
            {
                assert!(row.action.is_none(), "unexpected media on {text:?}");
            }
        }
    }

    #[test]
    fn should_build_title_and_status_sections() {
        let rows = build_pr_info_rows(&sample_info(), 80, &Theme::dark(), BodyRender::Source);
        let rendered = rows.iter().map(row_text).collect::<Vec<_>>();
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("PR #42 Add panel"))
        );
        assert!(rendered.iter().any(|line| line.contains("Ship the panel")));
        assert!(rendered.iter().any(|line| line.contains("PR #42 Status")));
        assert!(
            rendered
                .iter()
                .any(|line| { line.contains("https://github.com/owner/repo/actions/runs/1") })
        );
    }

    #[test]
    fn should_highlight_pr_description_source_when_rendering_is_off() {
        let mut info = sample_info();
        info.details.body = "plain `code` plain".to_string();
        let lines = build_pr_info_rows(&info, 80, &Theme::dark(), BodyRender::Source);
        let description = &lines[1].line;

        assert_eq!(
            description
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            info.details.body
        );
        assert!(
            description.spans.len() > 1,
            "expected markdown description to be highlighted into multiple spans"
        );

        info.details.body = "plain description".to_string();
        let lines = build_pr_info_rows(&info, 80, &Theme::dark(), BodyRender::Source);
        let description = &lines[1].line;

        assert_eq!(
            description
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            info.details.body
        );
        assert_eq!(description.spans.len(), 1);
    }

    #[test]
    fn should_render_pr_description_markdown() {
        let mut info = sample_info();
        info.details.body = "plain `code` plain".to_string();
        let theme = Theme::dark();
        let rows = build_pr_info_rows(
            &info,
            80,
            &theme,
            BodyRender::Markdown {
                toggled: &BTreeSet::new(),
            },
        );

        assert_eq!(row_text(&rows[1]), "plain  code  plain");
        let chip = rows[1]
            .line
            .spans
            .iter()
            .find(|span| span.content.contains("code"))
            .expect("span holding code");
        assert_eq!(chip.style.bg, Some(markdown_render::GLOW_CHIP_BG));
    }

    fn rendered(body: &str) -> Vec<(String, Option<RowAction>)> {
        body_rows_with(body, 80, true)
    }

    fn row(text: &str, media: Option<RowAction>) -> (String, Option<RowAction>) {
        (text.to_string(), media)
    }

    #[test]
    fn rendered_media_only_line_is_replaced_by_a_placeholder_row() {
        assert_eq!(
            rendered("Intro\n![shot](https://x.test/a.png)\nOutro"),
            vec![
                row("Intro", None),
                row("[image: shot]", Some(RowAction::Media(0))),
                row("Outro", None)
            ]
        );
    }

    #[test]
    fn rendered_inline_image_shows_a_glyph_and_appends_a_placeholder() {
        assert_eq!(
            rendered("see ![a](https://x.test/1.png) end"),
            vec![
                row("see ▣ a end", None),
                row("[image: a]", Some(RowAction::Media(0)))
            ]
        );
    }

    #[test]
    fn rendered_inline_image_placeholder_follows_the_reflowed_paragraph() {
        assert_eq!(
            rendered("one\ntwo ![a](https://x.test/1.png)\nthree"),
            vec![
                row("one two ▣ a three", None),
                row("[image: a]", Some(RowAction::Media(0)))
            ]
        );
    }

    #[test]
    fn rendered_multi_line_media_only_html_block_keeps_lines_outside_its_range() {
        let body = "<p align=\"center\">\n  <img width=\"400\"\n    alt=\"Login page\"\n    src=\"https://x.test/login.png\">\n</p>";
        assert_eq!(rendered(body), body_rows(body, 80));
    }

    #[test]
    fn rendered_video_and_attachment_match_the_off_path() {
        let body = "<video src=\"https://x.test/demo.mp4\"></video>\n\nhttps://github.com/user-attachments/assets/abc";
        assert_eq!(rendered(body), body_rows(body, 80));
    }

    #[test]
    fn rendered_media_starting_inside_a_covered_range_matches_the_off_path() {
        let body = "<video controls>\n<img src=\"https://x.test/i.png\">\n<source src=\"https://x.test/v.mp4\">\n</video>\n\n![after](https://x.test/a.png)";
        assert_eq!(rendered(body), body_rows(body, 80));
    }

    #[test]
    fn rendered_blank_rows_around_a_placeholder_stay() {
        assert_eq!(
            rendered("A\n\n![x](https://x.test/x.png)\n\nB"),
            vec![
                row("A", None),
                row("", None),
                row("[image: x]", Some(RowAction::Media(0))),
                row("", None),
                row("B", None)
            ]
        );
    }

    #[test]
    fn rendered_body_has_no_trailing_blank_rows() {
        assert_eq!(rendered("Body\n\n\n"), vec![row("Body", None)]);
    }

    #[test]
    fn media_in_a_collapsed_section_emits_nothing() {
        assert_eq!(
            rendered(
                "<details>\n<summary>Shots</summary>\n\n![one](https://x.test/1.png)\n\n</details>\n\nAfter"
            ),
            vec![
                row("▸ Shots", Some(RowAction::Details(0))),
                row("", None),
                row("After", None)
            ]
        );
    }

    #[test]
    fn media_in_the_summary_follows_the_summary_row() {
        let rows = rendered(
            "<details><summary><img src=\"https://x.test/s.png\"></summary>\n\nBody\n\n</details>",
        );

        assert_eq!(rows[0], row("▸ Details", Some(RowAction::Details(0))));
        assert!(rows[1].0.starts_with("[image:"), "{rows:?}");
        assert_eq!(rows[1].1, Some(RowAction::Media(0)));
        assert!(rows.iter().all(|(text, _)| !text.contains("Body")));
    }

    #[test]
    fn inline_media_in_the_summary_follows_the_summary_row() {
        let rows = rendered(
            "<details><summary>Shot <img src=\"https://x.test/x.png\" alt=\"x\"></summary>\n\nBody\n\n</details>",
        );

        assert_eq!(rows[0], row("▸ Shot", Some(RowAction::Details(0))));
        assert_eq!(rows[1], row("[image: x]", Some(RowAction::Media(0))));
    }
}
