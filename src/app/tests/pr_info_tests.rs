use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::{Color, Modifier};

use crate::app::{App, DiffSource, InputMode, PullRequestDiffSource};
use crate::forge::traits::{
    ForgeRepository, PrSessionKey, PullRequestCheckStatus, PullRequestDetails, PullRequestInfo,
    PullRequestIssueComment, PullRequestReviewStatus,
};
use crate::model::{DiffFile, FileStatus, ReviewSession, SessionDiffSource};
use crate::syntax::markdown_render::{GLOW_CHIP_BG, GLOW_CHIP_FG};
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
    let rows = crate::ui::pr_info_panel::build_pr_info_rows(
        &sample_pr_info(),
        80,
        &Theme::dark(),
        crate::ui::pr_info_panel::BodyRender::Markdown {
            toggled: &std::collections::BTreeSet::new(),
        },
    );
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
        crate::ui::pr_info_panel::pr_info_action_at_cursor(&app),
        Some(crate::ui::pr_info_panel::RowAction::Media(0))
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
        crate::ui::pr_info_panel::pr_info_action_at_cursor(&app),
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
        crate::ui::pr_info_panel::pr_info_action_at_cursor(&app),
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
        draw_app(&mut app);

        assert_panel_annotations_match_drawn_rows(&mut app);
    }
}

#[test]
fn should_keep_pr_info_annotations_in_sync_with_rendered_lines_at_wrap_boundary() {
    // A description line exactly `width` columns wide fits on one line at
    // `width` but wraps at `width - 1`. The counter (line_annotations) and
    // the renderer must agree on the wrap width, or every row below the
    // panel maps to the wrong annotation. Regression guard for the desync.
    let mut app = build_pr_app();
    app.pr_info = Some(sample_pr_info());
    draw_app(&mut app);
    let width = crate::ui::pr_info_panel::pr_info_content_width(app.diff_state.viewport_width);
    let (xs, ys) = ("X".repeat(30), "Y".repeat(width - 31));
    app.pr_info.as_mut().unwrap().details.body = format!("{xs} {ys}");
    draw_app(&mut app);

    let body_row = drawn_cursor_row(&mut app, 1);
    assert!(body_row.contains(&format!("{xs} {ys}")), "{body_row:?}");
    assert_panel_annotations_match_drawn_rows(&mut app);
}

/// The drawn row carrying the cursor marker after moving the cursor to
/// annotation `line`. Drawing places the marker by counting drawn rows, so
/// annotations that drift from the drawn rows put it on the wrong text.
fn drawn_cursor_row(app: &mut App, line: usize) -> String {
    app.diff_state.cursor_line = line;
    app.ensure_cursor_visible();
    let buffer = draw_once(app);
    let (title_x, _) = find_text(&buffer, "═══ PR #42 ");
    let marker_x = title_x - 2;
    let y = (0..buffer.area.height)
        .find(|&y| buffer[(marker_x, y)].symbol() == "▶")
        .expect("cursor marker drawn");
    row_text(&buffer, y)
}

/// Asserts through the drawn frame that the `PrInfoLine` annotations cover
/// exactly the drawn panel: the last one lands on the panel's last row (the
/// check) and the next one on the comments header below it.
fn assert_panel_annotations_match_drawn_rows(app: &mut App) {
    let panel = app
        .line_annotations
        .iter()
        .filter(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
        .count();
    assert!(panel > 0, "expected PR-info annotations");
    let last = drawn_cursor_row(app, panel - 1);
    assert!(last.contains("✓ build"), "{last:?}");
    let next = drawn_cursor_row(app, panel);
    assert!(next.contains("═══ PR #42 Comments"), "{next:?}");
}

/// Draws twice around `rebuild_annotations`: the first frame records the
/// real panel width, which the annotations need before the second frame.
pub(super) fn draw_app(app: &mut App) -> Buffer {
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

pub(super) fn rows_of(buffer: &Buffer) -> Vec<String> {
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
pub(super) fn find_text(buffer: &Buffer, needle: &str) -> (u16, u16) {
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

/// Columns of each `│` on row `y` from `x`, short of the panel's right border.
fn bars_from(buffer: &Buffer, x: u16, y: u16) -> Vec<u16> {
    (x..buffer.area.width - 1)
        .filter(|&col| buffer[(col, y)].symbol() == "│")
        .collect()
}

#[test]
fn should_keep_wide_table_columns_aligned_in_the_panel() {
    let long = "Rotate the signing keys for every staging service, then confirm each \
                consumer picks up the new key set before the old one expires at the end \
                of the maintenance window next week";
    let body = format!(
        "| Change | Owner | Risk | Done |\n| --- | --- | --- | --- |\n\
         | {long} | infra | low | yes |\n| Short row | app | none | no |"
    );
    let mut info = sample_pr_info();
    info.details.body = body;
    let mut app = build_pr_app();
    app.pr_info = Some(info);
    let buffer = draw_app(&mut app);
    let rows = rows_of(&buffer);

    let (x, head) = find_text(&buffer, "Change");
    for cell in ["Change", "Owner", "Risk", "Done"] {
        let (cx, cy) = find_text(&buffer, cell);
        assert_eq!(cy, head, "{cell} off the header row");
        assert!(buffer[(cx, cy)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(cx, cy)].modifier.contains(Modifier::UNDERLINED));
    }
    let bars = bars_from(&buffer, x, head);
    assert_eq!(bars.len(), 3, "{:?}", rows[head as usize]);
    for &bar in &bars {
        assert!(buffer[(bar, head)].modifier.contains(Modifier::UNDERLINED));
    }

    let (_, last) = find_text(&buffer, "Short row");
    assert!(last > head + 2, "the long cell wraps onto several rows");
    for y in head..=last {
        assert_eq!(bars_from(&buffer, x, y), bars, "{:?}", rows[y as usize]);
    }
    let first_column: String = (head + 1..last)
        .map(|y| {
            (x..bars[0])
                .map(|col| buffer[(col, y)].symbol())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(first_column, long);
    assert_panel_annotations_match_drawn_rows(&mut app);
}

/// The cell holding the first `needle` character of a chip, and its pads.
fn chip_cells(buffer: &Buffer, needle: &str) -> (Cell, Cell, Cell) {
    let (x, y) = find_text(buffer, needle);
    let end = x + needle.chars().count() as u16;
    (
        buffer[(x - 1, y)].clone(),
        buffer[(x, y)].clone(),
        buffer[(end, y)].clone(),
    )
}

#[test]
fn should_draw_inline_code_as_the_glow_chip_on_a_dark_theme() {
    let buffer = draw_body("run make_check before merging", true);
    let plain = chip_cells(&buffer, "make_check").1;
    let buffer = draw_body("run `make_check` before merging", true);
    let (left, chip, right) = chip_cells(&buffer, "make_check");

    assert_eq!(chip.fg, GLOW_CHIP_FG);
    assert_eq!(chip.bg, GLOW_CHIP_BG);
    for pad in [&left, &right] {
        assert_eq!(pad.symbol(), " ");
        assert_eq!(pad.bg, GLOW_CHIP_BG);
    }
    assert_ne!(plain.bg, GLOW_CHIP_BG);
}

#[test]
fn should_draw_a_legible_chip_on_a_light_theme_with_a_transparent_background() {
    let mut info = sample_pr_info();
    info.details.body = "run `make_check` before merging".to_string();
    let mut app = build_pr_app();
    // `transparent_background` defaults to on, and `main` applies it so.
    app.theme = Theme::light();
    app.theme.panel_bg = Color::Reset;
    app.pr_info = Some(info);
    let buffer = draw_app(&mut app);
    let (left, chip, right) = chip_cells(&buffer, "make_check");

    let light = Theme::light();
    assert_eq!(chip.bg, light.bg_highlight);
    assert_ne!(chip.bg, light.panel_bg);
    assert_ne!(chip.fg, chip.bg);
    assert_ne!(chip.fg, Color::Reset);
    assert_eq!((left.bg, right.bg), (chip.bg, chip.bg));
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

    assert!(drawn_cursor_row(&mut app, 1).contains(" T "));
    assert_panel_annotations_match_drawn_rows(&mut app);

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

const COVERAGE_TABLE_ROWS: usize = 40;

fn coverage_table() -> String {
    let mut table = String::from("| File | Lines |\n|------|-------|\n");
    for i in 0..COVERAGE_TABLE_ROWS {
        table.push_str(&format!("| f{i:02}.rs | {i} |\n"));
    }
    table.trim_end().to_string()
}

fn coverage_body(open: bool) -> String {
    let tag = if open { "<details open>" } else { "<details>" };
    format!(
        "Intro\n\n{tag}\n<summary>Coverage report</summary>\n\n{}\n\n</details>\n\nOutro",
        coverage_table()
    )
}

fn coverage_info(open: bool) -> PullRequestInfo {
    let mut info = sample_pr_info();
    info.details.body = coverage_body(open);
    info
}

fn coverage_app(open: bool) -> App {
    let mut app = build_pr_app();
    app.pr_info = Some(coverage_info(open));
    draw_app(&mut app);
    app
}

fn press_enter(app: &mut App) {
    crate::handler::handle_diff_action(app, crate::input::Action::SelectFile);
    draw_app(app);
}

fn press_enter_at(app: &mut App, line: usize) {
    app.diff_state.cursor_line = line;
    press_enter(app);
}

#[test]
fn enter_on_the_summary_row_rebuilds_annotations_before_the_next_frame() {
    let mut app = coverage_app(false);
    let panel_annotations = |app: &App| {
        app.line_annotations
            .iter()
            .filter(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
            .count()
    };
    let collapsed = panel_annotations(&app);

    app.diff_state.cursor_line = 3;
    crate::handler::handle_diff_action(&mut app, crate::input::Action::SelectFile);

    let expanded = panel_annotations(&app);
    assert!(expanded > collapsed, "{collapsed} -> {expanded}");
    assert_eq!(
        expanded,
        crate::ui::pr_info_panel::pr_info_render_height(&app)
    );
}

#[test]
fn details_section_is_collapsed_by_default() {
    let mut app = coverage_app(false);

    assert!(drawn_cursor_row(&mut app, 3).contains("▶ Coverage report"));
    assert!(
        !rows_of(&draw_app(&mut app))
            .iter()
            .any(|row| row.contains("f00.rs"))
    );
    assert!(drawn_cursor_row(&mut app, 5).contains("Outro"));
    assert_panel_annotations_match_drawn_rows(&mut app);
}

#[test]
fn enter_on_the_summary_row_expands_the_section() {
    let mut app = coverage_app(false);
    press_enter_at(&mut app, 3);

    assert_eq!(app.diff_state.cursor_line, 3);
    assert!(drawn_cursor_row(&mut app, 3).contains("▼ Coverage report"));
    let rows = rows_of(&draw_app(&mut app));
    let table_row = rows
        .iter()
        .find(|row| row.contains("f00.rs"))
        .expect("table row drawn");
    assert!(table_row.contains('│'), "{table_row:?}");

    let table_height = crate::syntax::markdown_render::render_block(
        &Theme::dark(),
        &coverage_table(),
        crate::ui::pr_info_panel::pr_info_content_width(app.diff_state.viewport_width),
        &[],
        &std::collections::BTreeSet::new(),
    )
    .len();
    assert!(drawn_cursor_row(&mut app, 6 + table_height).contains("Outro"));
    assert_panel_annotations_match_drawn_rows(&mut app);
}

#[test]
fn enter_on_an_expanded_summary_row_collapses_the_section() {
    let mut app = coverage_app(false);
    press_enter_at(&mut app, 3);
    press_enter_at(&mut app, 3);

    assert_eq!(app.diff_state.cursor_line, 3);
    assert!(drawn_cursor_row(&mut app, 3).contains("▶ Coverage report"));
    assert!(
        !rows_of(&draw_app(&mut app))
            .iter()
            .any(|row| row.contains("f00.rs"))
    );
    assert!(drawn_cursor_row(&mut app, 5).contains("Outro"));
    assert_panel_annotations_match_drawn_rows(&mut app);
}

#[test]
fn details_open_section_starts_expanded() {
    let mut app = coverage_app(true);

    assert!(drawn_cursor_row(&mut app, 3).contains("▼ Coverage report"));
    assert!(
        rows_of(&draw_app(&mut app))
            .iter()
            .any(|row| row.contains("f00.rs"))
    );
}

#[test]
fn nested_sections_toggle_independently() {
    let body = "<details>\n<summary>Outer</summary>\n\nOuter body\n\n<details>\n<summary>Inner</summary>\n\nInner body\n\n</details>\n\n</details>";
    let mut info = sample_pr_info();
    info.details.body = body.to_string();
    let mut app = build_pr_app();
    app.pr_info = Some(info);
    draw_app(&mut app);
    let drawn = |app: &mut App, line: usize| drawn_cursor_row(app, line);
    let has =
        |app: &mut App, needle: &str| rows_of(&draw_app(app)).iter().any(|r| r.contains(needle));

    assert!(drawn(&mut app, 1).contains("▶ Outer"));

    press_enter_at(&mut app, 1);
    assert!(drawn(&mut app, 1).contains("▼ Outer"));
    assert!(drawn(&mut app, 3).contains("Outer body"));
    assert!(drawn(&mut app, 5).contains("▶ Inner"));
    assert!(!has(&mut app, "Inner body"));
    assert_panel_annotations_match_drawn_rows(&mut app);

    press_enter_at(&mut app, 5);
    assert!(drawn(&mut app, 5).contains("▼ Inner"));
    assert!(drawn(&mut app, 7).contains("Inner body"));
    assert!(drawn(&mut app, 1).contains("▼ Outer"));
    assert_panel_annotations_match_drawn_rows(&mut app);

    // Enter in the inner body closes only the inner section.
    press_enter_at(&mut app, 7);
    assert_eq!(app.diff_state.cursor_line, 5);
    assert!(drawn(&mut app, 5).contains("▶ Inner"));
    assert!(!has(&mut app, "Inner body"));
    assert!(drawn(&mut app, 1).contains("▼ Outer"));
    assert_panel_annotations_match_drawn_rows(&mut app);

    // Enter in the outer body closes the outer section.
    press_enter_at(&mut app, 3);
    assert_eq!(app.diff_state.cursor_line, 1);
    assert!(drawn(&mut app, 1).contains("▶ Outer"));
    assert!(!has(&mut app, "Outer body"));
    assert_panel_annotations_match_drawn_rows(&mut app);
}

#[test]
fn enter_inside_an_expanded_body_collapses_it_onto_the_summary_row() {
    let mut app = coverage_app(false);
    press_enter_at(&mut app, 3);
    let panel = crate::ui::pr_info_panel::pr_info_render_height(&app);
    let table_row = (0..panel)
        .find(|&line| drawn_cursor_row(&mut app, line).contains("f39.rs"))
        .expect("last table row");

    press_enter_at(&mut app, table_row);

    assert_eq!(app.diff_state.cursor_line, 3);
    assert!(drawn_cursor_row(&mut app, 3).contains("▶ Coverage report"));
    assert!(
        !rows_of(&draw_app(&mut app))
            .iter()
            .any(|row| row.contains("f00.rs"))
    );
    assert!(drawn_cursor_row(&mut app, 5).contains("Outro"));
    assert_panel_annotations_match_drawn_rows(&mut app);
}

#[test]
fn enter_outside_every_section_does_nothing() {
    let mut app = coverage_app(true);
    app.diff_state.cursor_line = 1;
    let before = rows_of(&draw_app(&mut app));

    press_enter(&mut app);

    assert_eq!(app.diff_state.cursor_line, 1);
    assert_eq!(rows_of(&draw_app(&mut app)), before);
}

#[test]
fn rendering_off_shows_details_source_and_ignores_enter() {
    let mut app = coverage_app(false);
    app.render_markdown = false;
    let drawn = rows_of(&draw_app(&mut app));
    for needle in ["<details>", "<summary>Coverage report</summary>", "f00.rs"] {
        assert!(drawn.iter().any(|row| row.contains(needle)), "{needle}");
    }
    // The gutter cursor also draws `▶`, so look for the marker with its label.
    assert!(
        !drawn
            .iter()
            .any(|row| row.contains("▶ Coverage report") || row.contains("▼ Coverage report"))
    );

    let panel = crate::ui::pr_info_panel::pr_info_render_height(&app);
    for line in 0..panel {
        app.diff_state.cursor_line = line;
        assert_eq!(
            crate::ui::pr_info_panel::pr_info_action_at_cursor(&app),
            None
        );
    }

    let summary_line = (0..panel)
        .find(|&line| drawn_cursor_row(&mut app, line).contains("<summary>"))
        .expect("summary source row");
    app.diff_state.cursor_line = summary_line;
    let before = rows_of(&draw_app(&mut app));
    press_enter(&mut app);
    assert_eq!(rows_of(&draw_app(&mut app)), before);
}

#[test]
fn action_at_cursor_finds_the_summary_row_only() {
    use crate::ui::pr_info_panel::{RowAction, pr_info_action_at_cursor, pr_info_render_height};
    let mut app = coverage_app(false);

    let actions: Vec<(usize, RowAction)> = (0..=pr_info_render_height(&app))
        .filter_map(|line| {
            app.diff_state.cursor_line = line;
            pr_info_action_at_cursor(&app).map(|action| (line, action))
        })
        .collect();
    assert_eq!(actions, [(3, RowAction::Details(0))]);
}

/// Queues a background reload result fetched from a forge holding
/// `details`, as `spawn_pr_reload`'s thread would, for a reload started at
/// `started_at_head`.
fn queue_pr_reload(app: &mut App, details: PullRequestDetails, started_at_head: &str) {
    use super::target_selector_tests::{FakeForgeBackend, two_file_patch};
    let request = crate::app::PrReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: started_at_head.to_string(),
        started_at: std::time::Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };
    let target = crate::forge::traits::PullRequestTarget::with_repository(
        details.repository.clone(),
        details.number,
        details.number.to_string(),
    );
    let backend = FakeForgeBackend::open_pr_details(details, two_file_patch("new changed"));
    let fetched = crate::forge::pr_open::fetch_pr_data(&backend, target).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::app::PrReloadEvent::Done {
        request: request.clone(),
        result: Ok(fetched),
    })
    .unwrap();
    app.pr_reload_state = Some(request);
    app.pr_reload_rx = Some(rx);
}

#[test]
fn toggled_sections_reset_on_reload_but_not_on_gap_clearing() {
    use super::target_selector_tests::{
        FakeForgeBackend, TestReviewsDir, build_app, sample_pr, test_pr_details, two_file_patch,
    };
    let _reviews = TestReviewsDir::new();
    let mut details = test_pr_details(42, "Add panel");
    details.body = coverage_body(false);
    let forge = || {
        Box::new(FakeForgeBackend::open_pr_details(
            details.clone(),
            two_file_patch("new changed"),
        ))
    };
    let mut app = build_app();
    app.open_pr_with_backend(&sample_pr(42, "Add panel"), forge(), None)
        .unwrap();
    draw_app(&mut app);
    let summary = |app: &mut App| drawn_cursor_row(app, 3);

    press_enter_at(&mut app, 3);
    app.clear_expanded_gaps();
    app.rebuild_annotations();
    assert!(summary(&mut app).contains("▼ Coverage report"));

    app.reload_pull_request_with_backend(forge(), None).unwrap();
    assert!(summary(&mut app).contains("▶ Coverage report"));

    // A background reload lands on the same head, then on a new one.
    for head in [details.head_sha.as_str(), "bbbbbbbbbbbbbbbb"] {
        press_enter_at(&mut app, 3);
        assert!(summary(&mut app).contains("▼ Coverage report"), "{head}");
        let mut fetched = details.clone();
        fetched.head_sha = head.to_string();
        queue_pr_reload(&mut app, fetched, &details.head_sha);
        app.poll_pr_reload_events();
        assert!(summary(&mut app).contains("▶ Coverage report"), "{head}");
    }
}

#[test]
fn toggling_a_section_rebuilds_the_cached_rows() {
    use crate::ui::pr_info_panel::{RowAction, pr_info_rows};
    let summary = |app: &App| -> Option<String> {
        let rows = pr_info_rows(app);
        let row = rows
            .iter()
            .find(|row| row.action == Some(RowAction::Details(0)))?;
        Some(row.line.spans.iter().map(|s| s.content.as_ref()).collect())
    };
    let mut app = coverage_app(false);
    assert_eq!(summary(&app).as_deref(), Some("▶ Coverage report"));

    app.toggle_pr_details(0);
    assert_eq!(summary(&app).as_deref(), Some("▼ Coverage report"));
}
