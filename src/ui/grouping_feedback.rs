//! Render for the grouping-feedback prompt (`gd-26r.42`). Modelled on
//! `submit_modals::render_submit_action_picker`: `Clear`, a centered
//! bordered `Block`, theme colours from `styles`.
//!
//! Nothing here re-picks a group or a file — `gd-26r.18` ruled that out at
//! 400 files — so the marks section only ever reads
//! `GroupingFeedbackDraft::marked_files`/`marked_groups`, the snapshot taken
//! when the prompt opened.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::app::grouping_feedback::{FeedbackField, GROUPING_FEEDBACK_TAGS, GroupingVerdict};
use crate::ui::comment_panel::wrap_segments;
use crate::ui::styles;

/// Marks are shown, never scrolled through, so a long list is truncated
/// rather than growing the modal past the screen.
const MAX_MARKS_SHOWN: usize = 6;

pub fn render_grouping_feedback(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let Some(draft) = app.grouping_feedback.as_ref() else {
        return;
    };

    let area = centered_rect(64, 80, frame.area());
    frame.render_widget(Clear, area);

    let group_count = app.session.groups.len();
    let title = format!(
        " Grouping feedback \u{00b7} {group_count} group{s} ",
        s = if group_count == 1 { "" } else { "s" }
    );
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = vec![
        field_label(theme, "Verdict", draft.focus == FeedbackField::Verdict),
        verdict_line(theme, draft.verdict),
        Line::from(""),
        field_label(
            theme,
            "Tags (space to toggle)",
            draft.focus == FeedbackField::Tags,
        ),
    ];
    for (idx, tag) in GROUPING_FEEDBACK_TAGS.iter().enumerate() {
        lines.push(tag_line(
            theme,
            tag,
            draft.tags.contains(&idx),
            draft.focus == FeedbackField::Tags && draft.tag_cursor == idx,
        ));
    }
    lines.push(Line::from(""));

    lines.push(field_label(
        theme,
        "Note",
        draft.focus == FeedbackField::Note,
    ));
    let note_width = inner.width.saturating_sub(2) as usize;
    lines.extend(note_lines(
        &draft.note,
        draft.note_cursor,
        note_width,
        theme,
    ));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "Marks from this review (not re-picked here):",
        Style::default().fg(theme.fg_secondary),
    )));
    lines.push(marks_line("Groups", &draft.marked_groups));
    lines.push(marks_line(
        "Files",
        &draft
            .marked_files
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
    ));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "Enter: submit   Esc: skip   Tab/Shift-Tab: change field",
        Style::default().fg(theme.fg_secondary),
    )));

    let paragraph = Paragraph::new(lines)
        .style(styles::popup_style(theme))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

/// A field's label line, prefixed with `>` and bolded when it holds focus —
/// the one visible difference between the focused field and its two
/// siblings.
fn field_label(theme: &crate::theme::Theme, text: &str, focused: bool) -> Line<'static> {
    let marker = if focused { "> " } else { "  " };
    let style = if focused {
        Style::default()
            .fg(theme.fg_primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_secondary)
    };
    Line::from(Span::styled(format!("{marker}{text}:"), style))
}

/// The three verdict points, `1`/`2`/`3` as their discoverable keys, with
/// the selected one visibly marked.
fn verdict_line(theme: &crate::theme::Theme, verdict: Option<GroupingVerdict>) -> Line<'static> {
    let points = [
        ("1", "useful", GroupingVerdict::Useful),
        ("2", "mixed", GroupingVerdict::Mixed),
        ("3", "useless", GroupingVerdict::Useless),
    ];
    let mut spans = vec![Span::raw("    ")];
    for (i, (key, label, value)) in points.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        let selected = verdict == Some(value);
        let marker = if selected { "(*)" } else { "( )" };
        let mut style = Style::default();
        if selected {
            style = style.fg(theme.fg_primary).add_modifier(Modifier::BOLD);
        } else {
            style = style.fg(theme.fg_secondary);
        }
        spans.push(Span::styled(format!("{marker} {key} {label}"), style));
    }
    Line::from(spans)
}

fn tag_line(
    theme: &crate::theme::Theme,
    tag: &str,
    selected: bool,
    under_cursor: bool,
) -> Line<'static> {
    let cursor = if under_cursor { ">" } else { " " };
    let checkbox = if selected { "[x]" } else { "[ ]" };
    let mut style = Style::default();
    if under_cursor {
        style = style.add_modifier(Modifier::REVERSED);
    } else if selected {
        style = style.fg(theme.fg_primary);
    } else {
        style = style.fg(theme.fg_secondary);
    }
    Line::from(Span::styled(format!("  {cursor} {checkbox} {tag}"), style))
}

/// A single visible cursor cell inside the note, wrapping through
/// `wrap_segments` (`comment_panel`'s, shared rather than reimplemented)
/// when the buffer is wider than the modal. The note is single-line by
/// design (`Enter` submits, never inserts a newline), so this never has to
/// reason about embedded `\n`s the way the comment editor does.
fn note_lines(
    note: &str,
    cursor: usize,
    width: usize,
    theme: &crate::theme::Theme,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let cursor_style = Style::default()
        .fg(theme.cursor_color)
        .add_modifier(Modifier::REVERSED);
    let segments = wrap_segments(note, width);
    let mut lines = Vec::with_capacity(segments.len());
    let mut byte_offset = 0usize;
    for seg in segments {
        let seg_start = byte_offset;
        let seg_end = seg_start + seg.len();
        byte_offset = seg_end;
        let is_last = seg_end == note.len();
        let cursor_here =
            cursor >= seg_start && (cursor < seg_end || (is_last && cursor == note.len()));
        if cursor_here {
            let rel = cursor - seg_start;
            let (before, after) = seg.split_at(rel);
            let mut spans = vec![Span::raw(format!("  {before}"))];
            let mut chars = after.chars();
            if let Some(c) = chars.next() {
                spans.push(Span::styled(c.to_string(), cursor_style));
                spans.push(Span::raw(chars.as_str().to_string()));
            } else {
                spans.push(Span::styled(" ", cursor_style));
            }
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(format!("  {seg}")));
        }
    }
    lines
}

/// A marks row: a count plus a truncated list, never a picker.
fn marks_line(label: &str, marks: &[String]) -> Line<'static> {
    if marks.is_empty() {
        return Line::from(format!("  {label}: none"));
    }
    let shown: Vec<&str> = marks
        .iter()
        .take(MAX_MARKS_SHOWN)
        .map(String::as_str)
        .collect();
    let mut text = format!("  {label} ({}): {}", marks.len(), shown.join(", "));
    if marks.len() > MAX_MARKS_SHOWN {
        text.push_str(&format!(", +{} more", marks.len() - MAX_MARKS_SHOWN));
    }
    Line::from(text)
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::app::tests::grouping_tests::{
        build_app_over, empty_session, make_file, stub_vcs_info,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    /// A real, grouped `App`, built through the same `pub(crate)` fixture
    /// helpers `grouping_tests` exposes beyond its own module.
    fn grouped_app_for_test() -> App {
        let paths = ["src/auth/login.rs", "src/auth/session.rs", "README.md"];
        let files: Vec<_> = paths.iter().map(|p| make_file(p)).collect();
        let vcs_info = stub_vcs_info();
        let mut session = empty_session(&vcs_info);
        for file in &files {
            session.add_file(file.display_path().clone(), file.status, file.content_hash);
        }
        let mut app = build_app_over(files, session);
        app.enable_grouping();
        app.expand_all_dirs();
        app.open_grouping_feedback();
        app
    }

    fn buffer_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw(app: &mut App) -> Buffer {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    #[test]
    fn the_prompt_renders_verdicts_and_tags_through_the_real_ui_render() {
        // Drives the full `ui::render`, not this module's function directly,
        // so a forgotten `if app.input_mode == InputMode::GroupingFeedback`
        // in `app_layout.rs` would leave the modal missing and fail this
        // test rather than compiling silently.
        let mut app = grouped_app_for_test();
        let buffer = draw(&mut app);
        let text = buffer_text(&buffer);

        assert!(text.contains("useful"), "verdict labels: {text}");
        assert!(text.contains("mixed"), "verdict labels: {text}");
        assert!(text.contains("useless"), "verdict labels: {text}");
        assert!(
            GROUPING_FEEDBACK_TAGS.iter().any(|tag| text.contains(tag)),
            "at least one tag string: {text}"
        );
        assert!(text.contains("Enter: submit"));
        assert!(text.contains("Esc: skip") || text.contains("esc skip"));
    }

    #[test]
    fn marks_are_shown_not_picked() {
        let mut app = grouped_app_for_test();
        // Reopen with a couple of marks recorded first.
        app.skip_grouping_feedback();
        let group_id = app
            .build_visible_items()
            .iter()
            .find_map(|item| match item {
                crate::app::FileTreeItem::Group { id, .. } => Some(id.clone()),
                _ => None,
            })
            .expect("a group row");
        let group_name = app
            .session
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .expect("the group exists")
            .name
            .clone();
        let file_idx = app
            .build_visible_items()
            .iter()
            .find_map(|item| match item {
                crate::app::FileTreeItem::File { file_idx, .. } => Some(*file_idx),
                _ => None,
            })
            .expect("a file row");
        let file_path = app.diff_files[file_idx]
            .display_path()
            .display()
            .to_string();

        app.toggle_group_mark_by_id(&group_id);
        app.toggle_mark_for_file_idx(file_idx);
        app.open_grouping_feedback();

        let buffer = draw(&mut app);
        let text = buffer_text(&buffer);

        assert!(
            text.contains(&group_name),
            "marked group name should be shown: {text}"
        );
        assert!(
            text.contains(&file_path),
            "marked file path should be shown: {text}"
        );
    }
}
