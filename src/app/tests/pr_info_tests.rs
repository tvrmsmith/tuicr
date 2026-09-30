use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::app::{App, DiffSource, InputMode, PullRequestDiffSource};
use crate::forge::traits::{
    ForgeRepository, PrSessionKey, PullRequestCheckStatus, PullRequestDetails, PullRequestInfo,
    PullRequestIssueComment, PullRequestReviewStatus,
};
use crate::model::{DiffFile, FileStatus, ReviewSession, SessionDiffSource};
use crate::theme::Theme;
use crate::vcs::traits::VcsType;
use crate::vcs::{PrNoopVcs, VcsInfo};

pub(crate) fn sample_pr_info() -> PullRequestInfo {
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
            body: "Ship it".to_string(),
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

pub(crate) fn build_pr_app() -> App {
    let pr = PullRequestDiffSource {
        key: PrSessionKey::new(
            ForgeRepository::github("github.com", "owner", "repo"),
            42,
            "abc1234567890",
        ),
        base_sha: "def0987654321".to_string(),
        title: "Add panel".to_string(),
        url: "https://github.com/owner/repo/pull/42".to_string(),
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    };
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("forge:github.com/owner/repo"),
        head_commit: pr.key.head_sha.clone(),
        branch_name: Some(pr.head_ref_name.clone()),
        vcs_type: VcsType::File,
    };
    let mut session = ReviewSession::new(
        vcs_info.root_path.clone(),
        pr.key.head_sha.clone(),
        Some(pr.head_ref_name.clone()),
        SessionDiffSource::PullRequest,
    );
    session.pr_session_key = Some(pr.key.clone());
    let mut app = App::build(
        Box::new(PrNoopVcs::new(vcs_info.clone())),
        vcs_info,
        Theme::dark(),
        None,
        false,
        vec![DiffFile {
            old_path: None,
            new_path: Some("src/lib.rs".into()),
            status: FileStatus::Modified,
            hunks: vec![],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }],
        session,
        DiffSource::PullRequest(Box::new(pr)),
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build pr app");
    app.pr_info = Some(sample_pr_info());
    app.rebuild_annotations();
    app
}

#[test]
fn should_not_add_pr_info_to_file_tree() {
    let app = build_pr_app();
    assert!(
        app.line_annotations
            .iter()
            .any(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
    );
    assert!(app.build_visible_items().iter().all(|item| matches!(
        item,
        crate::app::FileTreeItem::Directory { .. } | crate::app::FileTreeItem::File { .. }
    )));
}

#[test]
fn should_order_overview_sections_before_file_diffs() {
    let mut app = build_pr_app();
    // The review-comments header only renders once the section has content.
    app.session.review_comments.push(crate::model::Comment::new(
        "review-level".to_string(),
        crate::model::CommentType::from_id("note"),
        None,
    ));
    app.rebuild_annotations();
    assert!(matches!(
        app.line_annotations.first(),
        Some(crate::app::AnnotatedLine::PrInfoLine { line_idx: 0 })
    ));

    let review_header_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::ReviewCommentsHeader));
    let issue_header_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::IssueCommentsHeader));
    let first_file_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::FileHeader { .. }));

    assert!(review_header_idx.is_some());
    assert!(issue_header_idx.is_some());
    assert!(first_file_idx.is_some());
    assert!(review_header_idx.unwrap() < issue_header_idx.unwrap());
    assert!(issue_header_idx.unwrap() < first_file_idx.unwrap());
}

#[test]
fn should_start_overview_at_top_of_main_view() {
    let mut app = build_pr_app();
    app.jump_to_file(0);
    assert!(app.diff_state.cursor_line > 0);

    app.diff_state.cursor_line = 0;
    app.ensure_cursor_visible();
    assert!(crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
}

#[test]
fn should_walk_from_overview_to_first_file_with_next_file() {
    let mut app = build_pr_app();
    app.diff_state.cursor_line = 0;
    app.next_file();
    assert_eq!(app.diff_state.current_file_idx, 0);
    assert!(!crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
}

#[test]
fn should_build_pr_info_panel_lines() {
    let rows =
        crate::ui::pr_info_panel::build_pr_info_rows(&sample_pr_info(), 80, &Theme::dark(), true);
    assert!(rows.len() > 5);
}

#[test]
fn media_at_cursor_finds_the_placeholder_row_on_a_media_only_line() {
    let mut info = sample_pr_info();
    info.details.body = "Intro\n![shot](https://x.test/a.png)\nOutro".to_string();

    let mut app = build_pr_app();
    app.pr_info = Some(info);
    app.diff_state.viewport_width = 82; // content width 80 after the indicator columns
    app.rebuild_annotations();

    app.diff_state.cursor_line = 2;
    assert_eq!(
        crate::ui::pr_info_panel::pr_info_media_at_cursor(&app),
        Some(0)
    );
}

#[test]
fn media_at_cursor_is_none_on_a_prose_row() {
    let mut info = sample_pr_info();
    info.details.body = "Intro\n![shot](https://x.test/a.png)\nOutro".to_string();

    let mut app = build_pr_app();
    app.pr_info = Some(info);
    app.diff_state.viewport_width = 82;
    app.rebuild_annotations();

    app.diff_state.cursor_line = 1;
    assert_eq!(
        crate::ui::pr_info_panel::pr_info_media_at_cursor(&app),
        None
    );
}

#[test]
fn media_at_cursor_is_none_below_the_pr_info_panel() {
    let mut info = sample_pr_info();
    info.details.body = "Intro\n![shot](https://x.test/a.png)\nOutro".to_string();

    let mut app = build_pr_app();
    app.pr_info = Some(info);
    app.diff_state.viewport_width = 82;
    app.rebuild_annotations();

    let below = crate::ui::pr_info_panel::pr_info_render_height(&app);
    app.diff_state.cursor_line = below;
    assert!(!crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
    assert_eq!(
        crate::ui::pr_info_panel::pr_info_media_at_cursor(&app),
        None
    );
}

#[test]
fn should_keep_pr_info_annotations_in_sync_with_rendered_lines_for_media_placeholders() {
    // Mirrors `should_keep_pr_info_annotations_in_sync_with_rendered_lines_at_wrap_boundary`:
    // media placeholders change the row count (a media-only line collapses,
    // an inline one grows), so the annotation count must still track the
    // renderer exactly or cursor↔line mapping drifts below the panel.
    let bodies = [
        "<p align=\"center\">\n  <img width=\"400\"\n    alt=\"Login page\"\n    src=\"https://x.test/login.png\">\n</p>",
        "<video src=\"https://x.test/demo.mp4\"></video>\n\nhttps://github.com/user-attachments/assets/abc",
    ];
    for body in bodies {
        let mut info = sample_pr_info();
        info.details.body = body.to_string();

        let mut app = build_pr_app();
        app.pr_info = Some(info);
        app.rebuild_annotations();

        let annotated = app
            .line_annotations
            .iter()
            .filter(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
            .count();

        let mut lines = Vec::new();
        let mut line_idx = 0usize;
        crate::ui::pr_info_panel::append_pr_info_section(
            &app,
            &mut lines,
            &mut line_idx,
            usize::MAX,
        );

        assert!(annotated > 0, "expected PR-info annotations for {body:?}");
        assert_eq!(
            annotated,
            lines.len(),
            "PrInfoLine annotation count must equal the rendered PR-info line count for {body:?}"
        );
    }
}

#[test]
fn should_keep_pr_info_annotations_in_sync_with_rendered_lines_at_wrap_boundary() {
    // A description line exactly `width` columns wide fits on one line at
    // `width` but wraps at `width - 1`. The counter (line_annotations) and
    // the renderer must agree on the wrap width, or every row below the
    // panel maps to the wrong annotation. Regression guard for the desync.
    let width = 60usize;
    let mut info = sample_pr_info();
    info.details.body = format!("{} {}", "X".repeat(30), "Y".repeat(29)); // 30 + 1 + 29 = 60

    let mut app = build_pr_app();
    app.pr_info = Some(info);
    app.diff_state.viewport_width = width;
    app.rebuild_annotations();

    let annotated = app
        .line_annotations
        .iter()
        .filter(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
        .count();

    let mut lines = Vec::new();
    let mut line_idx = 0usize;
    crate::ui::pr_info_panel::append_pr_info_section(&app, &mut lines, &mut line_idx, usize::MAX);

    assert!(annotated > 0, "expected PR-info annotations");
    assert_eq!(
        annotated,
        lines.len(),
        "PrInfoLine annotation count must equal the rendered PR-info line count"
    );
}

/// Draws twice around `rebuild_annotations`: the first frame records the
/// real panel width, which the annotations need before the second frame.
fn draw_app(app: &mut App) -> Buffer {
    draw_once(app);
    app.rebuild_annotations();
    draw_once(app)
}

fn draw_once(app: &mut App) -> Buffer {
    let backend = TestBackend::new(120, 60);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .expect("draw frame");
    terminal.backend().buffer().clone()
}

fn row_text(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_string())
        .collect()
}

fn rows_of(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| row_text(buffer, y))
        .collect()
}

/// Draws a PR app whose description is `body`.
fn draw_body(body: &str, render_markdown: bool) -> Buffer {
    let mut info = sample_pr_info();
    info.details.body = body.to_string();
    let mut app = build_pr_app();
    app.pr_info = Some(info);
    app.render_markdown = render_markdown;
    draw_app(&mut app)
}

/// Column of the first cell of `needle` on the row containing it.
fn find_text(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for y in 0..buffer.area.height {
        let cells: Vec<String> = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        let row = cells.concat();
        if let Some(byte) = row.find(needle) {
            let x = cells.concat()[..byte].chars().count() as u16;
            return (x, y);
        }
    }
    panic!("{needle:?} not drawn");
}

#[test]
fn should_default_render_markdown_to_true() {
    assert!(build_pr_app().render_markdown);
}

#[test]
fn should_draw_headings_without_markers_bold_and_underlined() {
    let buffer = draw_body("# Summary\n\n## Details\n\ntext", true);
    let rows = rows_of(&buffer);
    assert!(rows.iter().any(|row| row.contains("Summary")));
    assert!(!rows.iter().any(|row| row.contains("# Summary")));
    assert!(!rows.iter().any(|row| row.contains("## Details")));

    let (x, y) = find_text(&buffer, "Summary");
    let s = &buffer[(x, y)];
    assert!(s.modifier.contains(Modifier::BOLD));
    assert!(s.modifier.contains(Modifier::UNDERLINED));
    let far = &buffer[(x + 7 + 2, y)];
    assert!(far.modifier.contains(Modifier::UNDERLINED));
    let y_col = x + "Summar".len() as u16;
    assert_eq!(buffer[(y_col, y)].symbol(), "y");
    assert!(
        buffer[(y_col + 10, y)]
            .modifier
            .contains(Modifier::UNDERLINED)
    );

    let (x, y) = find_text(&buffer, "Details");
    let d = &buffer[(x, y)];
    assert!(d.modifier.contains(Modifier::UNDERLINED));
    assert!(!d.modifier.contains(Modifier::BOLD));
}

#[test]
fn should_draw_tables_with_aligned_columns_and_no_delimiter_row() {
    let buffer = draw_body(
        "| Name | Qty |\n| --- | ---: |\n| apple | 3 |\n| kiwi | 12 |",
        true,
    );
    let rows = rows_of(&buffer);
    let at = rows
        .iter()
        .position(|row| row.contains("Name  │ Qty"))
        .expect("header row");
    assert!(rows[at + 1].contains("apple │   3"));
    assert!(rows[at + 2].contains("kiwi  │  12"));
    assert!(!rows.iter().any(|row| row.contains("---")));
}

#[test]
fn should_draw_fenced_code_with_gutter_and_language_tag() {
    let buffer = draw_body("```rust\nfn main() {}\n```", true);
    let rows = rows_of(&buffer);
    assert!(rows.iter().any(|row| row.contains("│ fn main() {}")));
    assert!(rows.iter().any(|row| row.contains("rust")));
    assert!(!rows.iter().any(|row| row.contains("```")));
}

#[test]
fn should_draw_markdown_source_when_render_markdown_is_off() {
    let buffer = draw_body("# Summary\n\n## Details\n\ntext", false);
    assert!(rows_of(&buffer).iter().any(|row| row.contains("# Summary")));
}

#[test]
fn should_keep_annotations_and_drawn_rows_in_step_for_a_mixed_body() {
    let mut info = sample_pr_info();
    info.details.body = "# T\n\npara\n\n\n\n- a\n  - b\n\n| x | y |\n| - | - |\n| 1 | 2 |\n\n```rust\nlet a = 1;\n```\n\n\n".to_string();
    let mut app = build_pr_app();
    app.pr_info = Some(info);
    let rows = rows_of(&draw_app(&mut app));

    let annotated = app
        .line_annotations
        .iter()
        .filter(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
        .count();
    let mut lines = Vec::new();
    let mut line_idx = 0usize;
    crate::ui::pr_info_panel::append_pr_info_section(&app, &mut lines, &mut line_idx, usize::MAX);
    assert_eq!(annotated, lines.len());

    let code = rows
        .iter()
        .position(|row| row.contains("│ let a = 1;"))
        .expect("code row");
    let status = rows
        .iter()
        .position(|row| row.contains("PR #42 Status"))
        .expect("status header");
    assert_eq!(status - code, 2, "exactly one blank row between");
    assert!(!rows[code + 1].chars().any(char::is_alphanumeric));
}

#[test]
fn should_restyle_the_panel_when_the_theme_switches() {
    let mut info = sample_pr_info();
    info.details.body = "`x`".to_string();
    let mut app = build_pr_app();
    app.pr_info = Some(info);
    draw_app(&mut app);

    app.theme = Theme::light();
    let buffer = draw_app(&mut app);
    let (x, y) = (0..buffer.area.height)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .find(|&(x, y)| {
            buffer[(x, y)].symbol() == "x"
                && buffer[(x + 1, y)].symbol() == " "
                && buffer[(x - 1, y)].symbol() == " "
                && buffer[(x - 1, y)].bg == Theme::light().bg_highlight
        })
        .expect("chip cell");
    assert_eq!(buffer[(x, y)].bg, Theme::light().bg_highlight);
}

#[test]
fn should_reuse_pr_info_rows_until_width_or_markdown_mode_changes() {
    let mut app = build_pr_app();
    app.pr_info = Some(sample_pr_info());
    app.diff_state.viewport_width = 80;

    let first = crate::ui::pr_info_panel::pr_info_rows(&app);
    let second = crate::ui::pr_info_panel::pr_info_rows(&app);
    assert!(std::rc::Rc::ptr_eq(&first, &second));

    app.diff_state.viewport_width = 60;
    let narrower = crate::ui::pr_info_panel::pr_info_rows(&app);
    assert!(!std::rc::Rc::ptr_eq(&second, &narrower));

    app.render_markdown = false;
    let plain = crate::ui::pr_info_panel::pr_info_rows(&app);
    assert!(!std::rc::Rc::ptr_eq(&narrower, &plain));

    draw_app(&mut app);
    let drawn = crate::ui::pr_info_panel::pr_info_rows(&app);
    app.pr_info.as_mut().unwrap().details.body = "Refreshed description".to_string();
    let refreshed = crate::ui::pr_info_panel::pr_info_rows(&app);
    assert!(!std::rc::Rc::ptr_eq(&drawn, &refreshed));

    let rows = rows_of(&draw_app(&mut app));
    assert!(rows.iter().any(|row| row.contains("Refreshed description")));
    assert!(!rows.iter().any(|row| row.contains("Ship it")));
}

#[test]
fn should_draw_the_panel_at_full_width_on_the_first_frame() {
    let mut info = sample_pr_info();
    info.details.body = "Readable on the very first frame".to_string();
    let mut app = build_pr_app();
    app.pr_info = Some(info);

    let buffer = draw_once(&mut app);

    assert!(
        rows_of(&buffer)
            .iter()
            .any(|row| row.contains("Readable on the very first frame"))
    );
}
