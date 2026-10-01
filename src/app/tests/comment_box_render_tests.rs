//! Comment boxes drawn through a full `ui::render` frame: review comments and
//! PR conversation comments show rendered markdown, and each box's annotation
//! count matches the rows it draws.

use std::path::PathBuf;

use ratatui::style::{Color, Modifier};

use super::grouping_tests::{build_app_over, empty_session, stub_vcs_info};
use super::pr_info_tests::{build_pr_app, draw_app, find_text, rows_of};
use crate::app::*;
use crate::model::{Comment, CommentType, DiffFile, DiffHunk, DiffLine, LineOrigin, LineRange};
use crate::ui::comment_panel::{self, CommentTypePresentation};

const PATH: &str = "src/notes.rs";
/// Content of new-side line 2, the diff row drawn right after a comment
/// anchored on line 1.
const LINE_TWO_MARKER: &str = "zq_line_two_marker";

fn context_line(lineno: u32, content: &str) -> DiffLine {
    DiffLine {
        origin: LineOrigin::Context,
        content: content.to_string(),
        old_lineno: Some(lineno),
        new_lineno: Some(lineno),
        highlighted_spans: None,
    }
}

/// An app over one three-line file with a line comment holding `content`
/// anchored on new-side line 1.
fn app_with_line_comment(content: &str) -> App {
    let lines = vec![
        context_line(1, "fn first() {}"),
        context_line(2, LINE_TWO_MARKER),
        context_line(3, "fn third() {}"),
    ];
    let hunks = vec![DiffHunk {
        header: "@@ -1,3 +1,3 @@".to_string(),
        lines,
        old_start: 1,
        old_count: 3,
        new_start: 1,
        new_count: 3,
    }];
    let content_hash = DiffFile::compute_content_hash(&hunks);
    let file = DiffFile {
        old_path: Some(PathBuf::from(PATH)),
        new_path: Some(PathBuf::from(PATH)),
        status: crate::model::FileStatus::Modified,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    };
    let mut session = empty_session(&stub_vcs_info());
    session.add_diff_file(&file);
    let review = session
        .get_file_mut(&PathBuf::from(PATH))
        .expect("file review");
    let mut comment = Comment::new(
        content.to_string(),
        CommentType::from_id("note"),
        Some(LineSide::New),
    );
    comment.line_range = Some(LineRange::single(1));
    review.add_line_comment(1, comment);
    let mut app = build_app_over(vec![file], session);
    app.rebuild_annotations();
    app
}

fn any_row(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|row| row.contains(needle))
}

fn row_with(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not drawn: {rows:#?}"))
}

fn annotations_matching(app: &App, pred: impl Fn(&AnnotatedLine) -> bool) -> usize {
    app.line_annotations
        .iter()
        .filter(|line| pred(line))
        .count()
}

fn draw_line_comment(content: &str) -> Vec<String> {
    let mut app = app_with_line_comment(content);
    rows_of(&draw_app(&mut app))
}

#[test]
fn review_comment_renders_heading_bullets_and_code_block() {
    let rows = draw_line_comment("## Plan\n\n- first\n- second\n\n```rust\nlet x = 1;\n```");

    assert!(any_row(&rows, "Plan"), "{rows:#?}");
    assert!(!any_row(&rows, "## Plan"), "{rows:#?}");
    assert!(any_row(&rows, "• first"), "{rows:#?}");
    assert!(any_row(&rows, "• second"), "{rows:#?}");
    assert!(!any_row(&rows, "- first"), "{rows:#?}");
    assert!(any_row(&rows, "│ let x = 1;"), "{rows:#?}");
    assert!(!any_row(&rows, "```"), "{rows:#?}");
}

#[test]
fn review_comment_box_fits_its_rendered_rows() {
    let mut app = app_with_line_comment("```\ncode\n```\n\n\n\ntail");
    let rows = rows_of(&draw_app(&mut app));

    let tail = row_with(&rows, "tail");
    assert!(rows[tail + 1].contains('╰'), "{rows:#?}");
    assert!(rows[tail + 2].contains(LINE_TWO_MARKER), "{rows:#?}");

    let top = row_with(&rows, "├");
    let drawn = tail + 1 - top + 1;
    let annotated = annotations_matching(&app, |line| {
        matches!(line, AnnotatedLine::LineComment { .. })
    });
    assert_eq!(annotated, drawn, "{rows:#?}");
    assert!(annotated - 2 < 7, "{rows:#?}");
}

#[test]
fn pr_conversation_comment_renders_a_table_in_a_box_that_fits() {
    let mut app = build_pr_app();
    app.pr_info.as_mut().expect("PR info").issue_comments[0].body =
        "| Name | Qty |\n| --- | ---: |\n| apple | 3 |\n| kiwi | 12 |".to_string();
    let buffer = draw_app(&mut app);
    let rows = rows_of(&buffer);

    let header = row_with(&rows, "Name  │ Qty");
    assert!(rows[header + 1].contains("apple │   3"), "{rows:#?}");
    assert!(rows[header + 2].contains("kiwi  │  12"), "{rows:#?}");
    assert!(rows[header + 3].contains('╰'), "{rows:#?}");
    assert!(!any_row(&rows, "---"), "{rows:#?}");
    let (x, y) = find_text(&buffer, "Name  │ Qty");
    let name = &buffer[(x, y)];
    assert!(name.modifier.contains(Modifier::BOLD), "{name:?}");
    assert!(name.modifier.contains(Modifier::UNDERLINED), "{name:?}");

    let top = (0..header)
        .rev()
        .find(|&y| rows[y].contains('╭'))
        .expect("box top border");
    let drawn = header + 3 - top + 1;
    let annotated = annotations_matching(&app, |line| {
        matches!(line, AnnotatedLine::IssueComment { .. })
    });
    assert_eq!(annotated, drawn, "{rows:#?}");
}

#[test]
fn editing_a_review_comment_shows_its_raw_markdown() {
    let mut app = app_with_line_comment("**bold** text");
    let rows = rows_of(&draw_app(&mut app));
    assert!(any_row(&rows, "bold text"), "{rows:#?}");
    assert!(!any_row(&rows, "**bold**"), "{rows:#?}");

    app.diff_state.cursor_line = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, AnnotatedLine::LineComment { .. }))
        .expect("comment annotation")
        + 1;
    assert!(app.enter_edit_mode(false));
    let rows = rows_of(&draw_app(&mut app));
    assert!(any_row(&rows, "**bold** text"), "{rows:#?}");
}

#[test]
fn export_keeps_a_comment_as_raw_markdown() {
    let app = app_with_line_comment("use `code` here");

    let exported = crate::output::markdown::generate_export_content(
        &app.session,
        &app.diff_source,
        &app.comment_types,
        &app.export,
        &[],
        None,
    )
    .expect("export");

    assert!(exported.contains("`code`"), "{exported}");
}

#[test]
fn rendering_off_draws_a_comment_as_source() {
    let mut app = app_with_line_comment("## Note");
    app.render_markdown = false;

    let rows = rows_of(&draw_app(&mut app));

    assert!(any_row(&rows, "## Note"), "{rows:#?}");
}

/// The renderer replaces off-screen comment boxes with exactly
/// `App::comment_display_lines` blank rows, and the annotation builder sizes
/// every comment the same way. If that count ever drifted from what
/// `format_comment_lines` actually emits, the document would desync — the
/// cursor would land on the wrong row and culled boxes would leave the wrong
/// number of gaps. Pin the two together.
#[test]
fn comment_display_lines_matches_rendered_box_height() {
    let app = app_with_line_comment("");
    assert!(app.render_markdown);
    let bodies = [
        "",
        "single line",
        "first\nsecond\nthird",
        "trailing newline\n",
        "\n\nleading blanks",
        &"x".repeat(300),
        &"日本語のテキストです ".repeat(20),
        "`code` **bold** and a very long tail that will need to wrap at least once or twice",
        "# Title\n\npara",
        "| a | b |\n| - | - |\n| 1 | 2 |",
        "```rust\nfn x() {}\n```",
        "- one\n  - two",
        "a\n\n\n\nb",
    ];
    // Viewport widths, including degenerate ones narrower than the box chrome.
    for viewport_width in [9usize, 12, 40, 80, 120] {
        for body in bodies {
            // What every call site passes: the viewport minus the
            // cursor-indicator column.
            let box_width = viewport_width.saturating_sub(1);
            let rendered = comment_panel::format_comment_lines(
                &app.theme,
                CommentTypePresentation {
                    label: "NOTE".to_string(),
                    color: Color::Blue,
                },
                &comment_panel::comment_body_rows(&app.theme, body, box_width, true),
                None,
                box_width,
                None,
            );
            assert_eq!(
                app.comment_display_lines(body, viewport_width),
                rendered.len(),
                "width={viewport_width} body={body:?}"
            );
        }
    }
}
