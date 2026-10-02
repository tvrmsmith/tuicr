//! Rendered `.md` diffs, observed through the whole app drawn into a
//! `TestBackend`. The diff files carry the full text the loaders read, and
//! rendering must never fetch: the mock VCS panics on a fetch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::app::*;
use crate::model::{DiffLine, FilePatch, FileStatus, FullText};
use crate::theme::Theme;
use crate::vcs::traits::VcsType;

const GUIDE: &str = "docs/guide.md";

const GUIDE_NEW: [&str; 15] = [
    "# Guide",
    "",
    "| Name | Notes |",
    "| --- | --- |",
    "| a | first |",
    "| b | second |",
    "",
    "```python",
    "x = 1",
    "y = 3",
    "```",
    "",
    "## Setup",
    "",
    "Run **fast** with [docs](https://example.com) and `cargo`.",
];

const GUIDE_OLD: [&str; 12] = [
    "# Guide",
    "",
    "| Name | Notes |",
    "| --- | --- |",
    "| a | first |",
    "| b | old |",
    "",
    "```python",
    "x = 1",
    "y = 2",
    "```",
    "",
];

const GUIDE_PATCH: &str = "\
@@ -5,3 +5,3 @@
 | a | first |
-| b | old |
+| b | second |
 
@@ -9,3 +9,3 @@
 x = 1
-y = 2
+y = 3
 ```
@@ -12,1 +12,4 @@
 
+## Setup
+
+Run **fast** with [docs](https://example.com) and `cargo`.
";

const LIB_PATCH: &str = "\
@@ -1,0 +1,1 @@
+fn added() {}
";

/// A VCS that answers line counts, as the real ones do for the end-of-file
/// gap, and panics on a fetch: rendering reads only `DiffFile::full_text`.
struct NoFetchVcs {
    info: VcsInfo,
}

impl VcsBackend for NoFetchVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        unimplemented!("these tests never reload")
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        panic!("rendering fetched {}", file_path.display())
    }

    fn file_line_count(
        &self,
        file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        match file_path.to_str() {
            Some(GUIDE) => Ok(GUIDE_NEW.len() as u32),
            _ => Ok(1),
        }
    }
}

fn owned(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| line.to_string()).collect()
}

fn guide_text() -> Arc<FullText> {
    Arc::new(FullText {
        old: Some(owned(&GUIDE_OLD)),
        new: Some(owned(&GUIDE_NEW)),
    })
}

fn file_patch(path: &str, status: FileStatus, patch: &str) -> FilePatch {
    let (old_path, new_path) = match status {
        FileStatus::Added => (None, Some(PathBuf::from(path))),
        FileStatus::Deleted => (Some(PathBuf::from(path)), None),
        _ => (Some(PathBuf::from(path)), Some(PathBuf::from(path))),
    };
    FilePatch::new(old_path, new_path, status, patch)
}

/// Parses patches the way the real loaders do, so hunk lines carry syntect spans.
fn parse(patches: Vec<FilePatch>) -> Vec<DiffFile> {
    crate::vcs::diff_parser::parse_file_patches(patches, Theme::dark().syntax_highlighter())
        .expect("parse fixture patches")
}

/// `files` with `text` attached to the guide, as a loader leaves it.
fn with_guide_text(mut files: Vec<DiffFile>, text: Option<Arc<FullText>>) -> Vec<DiffFile> {
    for file in &mut files {
        if file.display_path() == Path::new(GUIDE) {
            file.full_text = text.clone();
        }
    }
    files
}

fn guide_patches() -> Vec<FilePatch> {
    vec![
        file_patch(GUIDE, FileStatus::Modified, GUIDE_PATCH),
        file_patch("src/lib.rs", FileStatus::Modified, LIB_PATCH),
    ]
}

fn guide_files(text: Option<Arc<FullText>>) -> Vec<DiffFile> {
    with_guide_text(parse(guide_patches()), text)
}

fn vcs_info() -> VcsInfo {
    VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "abc123".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    }
}

fn build_app(files: Vec<DiffFile>) -> App {
    let vcs_info = vcs_info();
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    let mut app = App::build(
        Box::new(NoFetchVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        files,
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app");
    app.diff_view_mode = DiffViewMode::Unified;
    app.diff_state.wrap_lines = false;
    app
}

/// Turns rendering on the way startup does: the flag, then a rebuild.
fn render_markdown_diffs(app: &mut App) {
    app.render_markdown_diffs = true;
    app.rebuild_annotations();
}

fn rendered_app(files: Vec<DiffFile>) -> App {
    let mut app = build_app(files);
    render_markdown_diffs(&mut app);
    app
}

fn rendered_guide_app() -> App {
    rendered_app(guide_files(Some(guide_text())))
}

fn draw(app: &mut App) -> Buffer {
    let backend = TestBackend::new(140, 60);
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

/// One drawn diff row: the gutter number, the text after the gutter
/// (trailing spaces trimmed), and the buffer cells that text sits on.
struct DrawnRow {
    number: Option<u32>,
    text: String,
    y: u16,
    text_x: u16,
}

impl DrawnRow {
    /// The cells under the trimmed text, one per character.
    fn text_cells<'a>(&self, buffer: &'a Buffer) -> Vec<&'a ratatui::buffer::Cell> {
        (self.text_x..self.text_x + self.text.chars().count() as u16)
            .map(|x| &buffer[(x, self.y)])
            .collect()
    }

    /// The cell showing the first occurrence of `needle`'s first character.
    fn cell_of<'a>(&self, buffer: &'a Buffer, needle: &str) -> &'a ratatui::buffer::Cell {
        let offset = self.text.find(needle).expect("needle on row");
        let col = self.text[..offset].chars().count() as u16;
        &buffer[(self.text_x + col, self.y)]
    }
}

fn cells_of_row(buffer: &Buffer, y: u16) -> Vec<String> {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_string())
        .collect()
}

/// Leftmost column of the diff panel's content (right of its left border).
fn diff_panel_x(buffer: &Buffer) -> u16 {
    let top = cells_of_row(buffer, 1);
    let corners: Vec<usize> = top
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.as_str() == "\u{250c}")
        .map(|(x, _)| x)
        .collect();
    corners.last().copied().expect("diff panel corner") as u16 + 1
}

/// Parses a unified-view row: `<spaces><number> <marker> <text>`.
fn parse_row(buffer: &Buffer, y: u16, panel_x: u16) -> DrawnRow {
    let cells = cells_of_row(buffer, y);
    let mut x = panel_x as usize;
    let end = cells.len() - 1;
    // Leading blanks, and the cursor marker on the cursor row.
    while x < end && (cells[x] == " " || cells[x] == "\u{25b6}") {
        x += 1;
    }
    let digits_start = x;
    while x < end && cells[x].chars().all(|c| c.is_ascii_digit()) && cells[x] != " " {
        x += 1;
    }
    let number = if x > digits_start {
        cells[digits_start..x].concat().parse().ok()
    } else {
        None
    };
    // Marker column and the space after it.
    let text_x = (x + 3).min(end);
    let text: String = cells[text_x..end].concat();
    DrawnRow {
        number,
        text: text.trim_end().to_string(),
        y,
        text_x: text_x as u16,
    }
}

/// The rows of `path`'s block: from its file header to the next file header.
fn file_block(buffer: &Buffer, path: &str) -> Vec<DrawnRow> {
    let panel_x = diff_panel_x(buffer);
    let rows: Vec<DrawnRow> = (2..buffer.area.height - 2)
        .map(|y| parse_row(buffer, y, panel_x))
        .collect();
    let is_header = |row: &DrawnRow| row_text(buffer, row.y).contains("\u{2550}\u{2550}\u{2550}");
    let start = rows
        .iter()
        .position(|row| is_header(row) && row_text(buffer, row.y).contains(path))
        .unwrap_or_else(|| panic!("no header for {path}"));
    let len = rows[start + 1..]
        .iter()
        .position(is_header)
        .unwrap_or(rows.len() - start - 1);
    rows.into_iter().skip(start).take(len + 1).collect()
}

/// Rows of `path`'s block that carry a gutter number, in drawn order.
fn numbered_rows(buffer: &Buffer, path: &str) -> Vec<DrawnRow> {
    file_block(buffer, path)
        .into_iter()
        .filter(|row| row.number.is_some())
        .collect()
}

fn guide_numbered_rows(buffer: &Buffer) -> Vec<DrawnRow> {
    numbered_rows(buffer, GUIDE)
}

/// The row of the new side's line `n`: the last row drawn with that number,
/// since a deletion is drawn before the addition that replaces it.
fn row_for_new(buffer: &Buffer, n: u32) -> DrawnRow {
    guide_numbered_rows(buffer)
        .into_iter()
        .rev()
        .find(|row| row.number == Some(n))
        .unwrap_or_else(|| panic!("no row for new {n}"))
}

/// The deletion row of old line `n`: the first row drawn with that number.
fn row_for_old(buffer: &Buffer, n: u32) -> DrawnRow {
    guide_numbered_rows(buffer)
        .into_iter()
        .find(|row| row.number == Some(n))
        .unwrap_or_else(|| panic!("no row for old {n}"))
}

fn draw_sized(app: &mut App, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .expect("draw frame");
    terminal.backend().buffer().clone()
}

/// The first drawn row carrying gutter number `n`, for layouts where the file
/// header's rule row confuses `file_block` (narrow terminals, PR mode).
fn drawn_row_numbered(buffer: &Buffer, n: u32) -> DrawnRow {
    let panel_x = diff_panel_x(buffer);
    (2..buffer.area.height - 2)
        .map(|y| parse_row(buffer, y, panel_x))
        .find(|row| row.number == Some(n))
        .unwrap_or_else(|| panic!("no row numbered {n}"))
}

// 9. With the full text attached, contract B's scenarios 1-8 hold.

#[test]
fn should_draw_one_row_per_hunk_line_when_markdown_diffs_render() {
    let mut off = build_app(guide_files(Some(guide_text())));
    let mut on = rendered_guide_app();

    let off_buffer = draw(&mut off);
    let on_buffer = draw(&mut on);

    let numbers: Vec<u32> = guide_numbered_rows(&on_buffer)
        .iter()
        .filter_map(|row| row.number)
        .collect();
    assert_eq!(numbers, vec![5, 6, 6, 7, 9, 10, 10, 11, 12, 13, 14, 15]);
    assert_eq!(
        file_block(&on_buffer, GUIDE).len(),
        file_block(&off_buffer, GUIDE).len()
    );
}

#[test]
fn should_render_table_rows_with_the_header_columns_above_the_hunk() {
    let mut app = rendered_guide_app();

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 5).text, "a    \u{2502} first");
    assert_eq!(row_for_old(&buffer, 6).text, "b    \u{2502} old");
    assert_eq!(row_for_new(&buffer, 6).text, "b    \u{2502} second");
    for row in guide_numbered_rows(&buffer)
        .iter()
        .filter(|row| matches!(row.number, Some(5..=7)))
    {
        assert!(
            !row.text.contains('|'),
            "row still has a pipe: {}",
            row.text
        );
    }
}

#[test]
fn should_render_code_block_rows_when_a_hunk_starts_inside_the_fence() {
    let mut app = rendered_guide_app();

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 9).text, "\u{2502} x = 1");
    assert_eq!(row_for_old(&buffer, 10).text, "\u{2502} y = 2");
    assert_eq!(row_for_new(&buffer, 10).text, "\u{2502} y = 3");
    assert_eq!(row_for_new(&buffer, 11).text, "");
    assert!(!row_for_new(&buffer, 15).text.contains('\u{2502}'));
}

fn is_underlined(cell: &ratatui::buffer::Cell) -> bool {
    cell.modifier.contains(Modifier::UNDERLINED)
}

#[test]
fn should_render_headings_and_inline_markup_without_their_markers() {
    let mut app = rendered_guide_app();

    let buffer = draw(&mut app);

    let heading = row_for_new(&buffer, 13);
    assert_eq!(heading.text, "Setup");
    assert!(is_underlined(heading.cell_of(&buffer, "S")));
    assert!(!heading.text.contains('#'));

    let prose = row_for_new(&buffer, 15);
    assert_eq!(prose.text, "Run fast with docs and  cargo .");
    assert!(is_underlined(prose.cell_of(&buffer, "docs")));
    assert!(!prose.text.contains("https"));
}

#[test]
fn should_keep_the_diff_background_on_rendered_rows() {
    let mut app = rendered_guide_app();
    let theme = Theme::dark();

    let buffer = draw(&mut app);

    let lib_row = file_block(&buffer, "src/lib.rs")
        .into_iter()
        .find(|row| row.number == Some(1))
        .expect("lib.rs added row");
    assert_eq!(lib_row.cell_of(&buffer, "fn").bg, theme.syntax_add_bg);

    for n in [6, 13, 15] {
        let row = row_for_new(&buffer, n);
        for cell in row.text_cells(&buffer) {
            assert_eq!(cell.bg, theme.syntax_add_bg, "new {n}: {}", row.text);
        }
    }
    for n in [6, 10] {
        let row = row_for_old(&buffer, n);
        for cell in row.text_cells(&buffer) {
            assert_eq!(cell.bg, theme.syntax_del_bg, "old {n}: {}", row.text);
        }
    }
}

#[test]
fn should_render_an_added_file_from_its_hunk() {
    let added = file_patch(
        "docs/new.md",
        FileStatus::Added,
        "@@ -0,0 +1,5 @@\n+Intro\n+```python\n+print(1)\n+```\n+After\n",
    );
    let mut app = rendered_app(parse(vec![added]));

    let buffer = draw(&mut app);

    let rows = numbered_rows(&buffer, "docs/new.md");
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts, ["Intro", "python", "\u{2502} print(1)", "", "After"]);
    assert_eq!(rows[1].cell_of(&buffer, "python").fg, Theme::dark().fg_dim);
}

#[test]
fn should_render_a_deleted_file_from_its_hunk() {
    let deleted = file_patch(
        "docs/old.md",
        FileStatus::Deleted,
        "@@ -1,2 +0,0 @@\n-## Gone\n-Text\n",
    );
    let mut app = rendered_app(parse(vec![deleted]));

    let buffer = draw(&mut app);

    let rows = numbered_rows(&buffer, "docs/old.md");
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts, ["Gone", "Text"]);
}

#[test]
fn should_render_markdown_rows_on_both_sides_in_side_by_side_view() {
    let mut app = rendered_guide_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.rebuild_annotations();

    let buffer = draw(&mut app);

    let panel_x = diff_panel_x(&buffer) as usize;
    let rows: Vec<String> = (2..buffer.area.height - 2)
        .map(|y| cells_of_row(&buffer, y)[panel_x..].concat())
        .collect();

    let pair = rows
        .iter()
        .find(|row| row.contains("b    \u{2502} old"))
        .expect("row pairing old 6 with new 6");
    let left = pair.find("b    \u{2502} old").unwrap();
    let right = pair
        .find("b    \u{2502} second")
        .expect("new 6 on the right");
    assert!(left < right);

    let setup: Vec<&String> = rows.iter().filter(|row| row.contains("Setup")).collect();
    assert_eq!(setup.len(), 1);
    assert!(setup[0].contains("13"));
    assert!(rows.iter().all(|row| !row.contains("## Setup")));
}

// 10. Off by default.
#[test]
fn should_keep_source_text_when_markdown_diffs_are_off() {
    let mut app = build_app(guide_files(Some(guide_text())));

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 13).text, "## Setup");
    assert_eq!(row_for_new(&buffer, 6).text, "| b | second |");
}

fn annotation_of_new_line(app: &App, new: u32) -> usize {
    app.line_annotations
        .iter()
        .position(|annotation| {
            matches!(
                annotation,
                AnnotatedLine::DiffLine { new_lineno: Some(n), .. } if *n == new
            )
        })
        .unwrap_or_else(|| panic!("no annotation for new {new}"))
}

// 11. No full text (a failed read) falls back to source.
#[test]
fn should_fall_back_to_source_when_the_full_text_is_missing() {
    let mut app = rendered_app(guide_files(None));

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 13).text, "## Setup");
    assert_eq!(row_for_new(&buffer, 6).text, "| b | second |");
    app.diff_state.cursor_line = annotation_of_new_line(&app, 13);
    app.cursor_down(1);
    assert_eq!(app.diff_state.cursor_line, annotation_of_new_line(&app, 14));
}

// 12. Text that disagrees with the hunks falls back to source.
#[test]
fn should_fall_back_to_source_when_the_full_text_disagrees_with_the_hunks() {
    let mut new = owned(&GUIDE_NEW);
    new[5] = "| b | other |".to_string();
    let text = Arc::new(FullText {
        old: Some(owned(&GUIDE_OLD)),
        new: Some(new),
    });
    let mut app = rendered_app(guide_files(Some(text)));

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 6).text, "| b | second |");
    assert_eq!(row_for_new(&buffer, 13).text, "## Setup");
}

// 13. New text for the same hunks re-renders.
#[test]
fn should_re_render_when_a_reload_brings_new_full_text_for_the_same_hunks() {
    let mut app = rendered_guide_app();
    let buffer = draw(&mut app);
    assert_eq!(row_for_new(&buffer, 6).text, "b    \u{2502} second");

    let widen = |lines: &[&str]| {
        let mut lines = owned(lines);
        lines[2] = "| Label | Notes |".to_string();
        lines
    };
    let widened = Arc::new(FullText {
        old: Some(widen(&GUIDE_OLD)),
        new: Some(widen(&GUIDE_NEW)),
    });
    app.diff_files = guide_files(Some(widened));
    app.rebuild_annotations();

    let buffer = draw(&mut app);
    assert_eq!(row_for_new(&buffer, 6).text, "b     \u{2502} second");
}

const BASE_SHA: &str = "1234567890abcdef";
const HEAD_SHA: &str = "abcdef0123456789";

fn joined(lines: &[&str]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// A forge serving the guide PR: its patches, and the guide's content at the
/// base and head shas. Records every `fetch_file_content` request.
struct GuideForge {
    content_requests: Arc<std::sync::Mutex<Vec<(String, PathBuf)>>>,
}

impl crate::forge::traits::ForgeBackend for GuideForge {
    fn list_pull_requests(
        &self,
        _query: crate::forge::traits::PullRequestListQuery,
    ) -> Result<crate::forge::traits::PagedPullRequests> {
        unimplemented!()
    }
    fn get_pull_request(
        &self,
        _target: crate::forge::traits::PullRequestTarget,
    ) -> Result<crate::forge::traits::PullRequestDetails> {
        Ok(crate::forge::traits::PullRequestDetails {
            repository: crate::forge::traits::ForgeRepository::github(
                "github.com",
                "owner",
                "repo",
            ),
            number: 42,
            title: "Guide".to_string(),
            url: "https://github.com/owner/repo/pull/42".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: HEAD_SHA.to_string(),
            base_sha: BASE_SHA.to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        })
    }
    fn get_pull_request_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<FilePatch>> {
        Ok(guide_patches())
    }
    fn fetch_file_lines(
        &self,
        request: crate::forge::traits::ForgeFileLinesRequest,
    ) -> Result<Vec<DiffLine>> {
        panic!("rendering fetched lines of {}", request.path.display())
    }
    fn fetch_file_content(
        &self,
        request: crate::forge::traits::ForgeFileLinesRequest,
    ) -> Result<String> {
        let key = (request.sha().to_string(), request.path.clone());
        self.content_requests.lock().unwrap().push(key.clone());
        match (key.0.as_str(), key.1.to_str()) {
            (BASE_SHA, Some(GUIDE)) => Ok(joined(&GUIDE_OLD)),
            (HEAD_SHA, Some(GUIDE)) => Ok(joined(&GUIDE_NEW)),
            _ => Err(crate::error::TuicrError::Forge(format!(
                "no content for {key:?}"
            ))),
        }
    }
    fn list_review_threads(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
        Ok(Vec::new())
    }
    fn list_pull_request_commits(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
        Ok(Vec::new())
    }
    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _start_sha: &str,
        _end_sha: &str,
    ) -> Result<Vec<FilePatch>> {
        unimplemented!()
    }
    fn create_review(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _request: crate::forge::traits::CreateReviewRequest<'_>,
    ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
        unimplemented!()
    }
}

// 14. PR mode renders at load.
#[test]
fn should_render_a_pr_markdown_file_on_the_first_draw_after_opening() {
    let _reviews = super::target_selector_tests::TestReviewsDir::new();
    let mut app = build_app(Vec::new());
    app.render_markdown_diffs = true;
    let content_requests = Arc::default();
    let summary = crate::forge::traits::PullRequestSummary {
        repository: crate::forge::traits::ForgeRepository::github("github.com", "owner", "repo"),
        number: 42,
        title: "Guide".to_string(),
        author: None,
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        updated_at: None,
        url: "https://github.com/owner/repo/pull/42".to_string(),
        state: "OPEN".to_string(),
        is_draft: false,
    };

    app.open_pr_with_backend(
        &summary,
        Box::new(GuideForge {
            content_requests: Arc::clone(&content_requests),
        }),
        None,
    )
    .expect("open PR");
    app.diff_view_mode = DiffViewMode::Unified;
    let buffer = draw(&mut app);

    assert!(matches!(app.diff_source, DiffSource::PullRequest(_)));
    assert_eq!(drawn_row_numbered(&buffer, 13).text, "Setup");
    let requested: Vec<(String, PathBuf)> = content_requests.lock().unwrap().clone();
    assert_eq!(
        requested,
        [
            (BASE_SHA.to_string(), PathBuf::from(GUIDE)),
            (HEAD_SHA.to_string(), PathBuf::from(GUIDE)),
        ]
    );
}

// 15. Contract C's scenarios hold: wrap height, theme change, search.

const LINK_LINE: &str =
    "See [docs](https://example.com/a/very/long/path/that/keeps/going/and/going/on/forever).";

/// The guide with `LINK_LINE` added after line 15, as new line 16, its full
/// text attached.
fn link_line_files() -> Vec<DiffFile> {
    let patch =
        GUIDE_PATCH.replace("@@ -12,1 +12,4 @@", "@@ -12,1 +12,5 @@") + &format!("+{LINK_LINE}\n");
    let mut new = owned(&GUIDE_NEW);
    new.push(LINK_LINE.to_string());
    let text = Arc::new(FullText {
        old: Some(owned(&GUIDE_OLD)),
        new: Some(new),
    });
    with_guide_text(
        parse(vec![
            file_patch(GUIDE, FileStatus::Modified, &patch),
            file_patch("src/lib.rs", FileStatus::Modified, LIB_PATCH),
        ]),
        Some(text),
    )
}

#[test]
fn should_size_a_wrapped_unified_row_by_its_rendered_text() {
    let mut app = build_app(link_line_files());
    app.diff_state.wrap_lines = true;
    render_markdown_diffs(&mut app);

    let buffer = draw_sized(&mut app, 60, 60);

    let row = drawn_row_numbered(&buffer, 16);
    assert_eq!(row.text, "See docs.");
    let idx = annotation_of_new_line(&app, 16);
    assert_eq!(crate::ui::row_height::annotation_row_height(&app, idx), 1);
    assert!(matches!(
        app.line_annotations[idx + 1],
        AnnotatedLine::Spacing
    ));
    assert!(matches!(
        app.line_annotations[idx + 2],
        AnnotatedLine::FileHeader { .. }
    ));
    let spacing = parse_row(&buffer, row.y + 1, diff_panel_x(&buffer));
    assert_eq!((spacing.number, spacing.text.as_str()), (None, ""));
    let header = row_text(&buffer, row.y + 2);
    assert!(header.contains("src/lib.rs"), "row two below: {header}");
}

#[test]
fn should_size_a_wrapped_side_by_side_row_by_its_rendered_text() {
    let sbs_app = |render: bool| {
        let mut app = build_app(link_line_files());
        app.diff_state.wrap_lines = true;
        app.diff_view_mode = DiffViewMode::SideBySide;
        if render {
            render_markdown_diffs(&mut app);
        }
        draw_sized(&mut app, 100, 60);
        app
    };
    let paired_with_new_16 = |app: &App| {
        app.line_annotations
            .iter()
            .position(|annotation| match annotation {
                AnnotatedLine::SideBySideLine {
                    file_idx,
                    hunk_idx,
                    add_line_idx: Some(add),
                    ..
                } => app.diff_files[*file_idx].hunks[*hunk_idx].lines[*add].new_lineno == Some(16),
                _ => false,
            })
            .expect("row pairing new 16")
    };

    let rendered = sbs_app(true);
    let idx = paired_with_new_16(&rendered);
    assert_eq!(
        crate::ui::row_height::annotation_row_height(&rendered, idx),
        1
    );

    let raw = sbs_app(false);
    assert!(
        crate::ui::row_height::annotation_row_height(&raw, paired_with_new_16(&raw)) > 1,
        "the raw source must wrap at this width for the test to mean anything"
    );
}

/// Opens the picker and previews the built-in theme `name`, leaving it open.
fn preview_in_picker(app: &mut App, name: &str) {
    app.enter_theme_picker_mode();
    let position = app
        .theme_picker
        .candidates
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("no theme {name}"));
    app.theme_picker.select(position);
    app.preview_theme(name);
}

const OTHER_THEME: &str = "light";

#[test]
fn should_keep_the_render_and_recolor_it_when_a_theme_is_confirmed() {
    let mut app = rendered_guide_app();
    let dark_fg = {
        let buffer = draw(&mut app);
        let heading = drawn_row_numbered(&buffer, 13);
        heading.cell_of(&buffer, "S").fg
    };

    preview_in_picker(&mut app, OTHER_THEME);
    app.confirm_theme_picker();

    let buffer = draw(&mut app);
    let heading = drawn_row_numbered(&buffer, 13);
    assert_eq!(heading.text, "Setup");
    assert_ne!(heading.cell_of(&buffer, "S").fg, dark_fg);
}

#[test]
fn should_keep_the_current_markdown_file_rendered_when_a_theme_is_previewed() {
    let mut app = rendered_guide_app();
    assert_eq!(app.diff_state.current_file_idx, 0);
    // A drawn frame first, so the preview cannot lean on a layout rebuild.
    draw(&mut app);

    app.preview_theme(OTHER_THEME);

    let buffer = draw(&mut app);
    assert_eq!(drawn_row_numbered(&buffer, 13).text, "Setup");
}

#[test]
fn should_keep_the_render_under_the_original_theme_when_the_picker_is_cancelled() {
    let mut app = rendered_guide_app();
    let original_fg = {
        let buffer = draw(&mut app);
        let heading = drawn_row_numbered(&buffer, 13);
        heading.cell_of(&buffer, "S").fg
    };

    preview_in_picker(&mut app, OTHER_THEME);
    app.cancel_theme_picker();

    let buffer = draw(&mut app);
    let heading = drawn_row_numbered(&buffer, 13);
    assert_eq!(heading.text, "Setup");
    assert_eq!(heading.cell_of(&buffer, "S").fg, original_fg);
}

#[test]
fn should_paint_search_matches_on_rendered_cells() {
    let mut app = rendered_guide_app();
    app.search_buffer = "Setup".to_string();
    assert!(app.search_in_diff_from_cursor());
    assert!(app.search_highlight_visible);
    let match_style = crate::ui::styles::search_match_style(&app.theme);

    let buffer = draw(&mut app);

    let heading = drawn_row_numbered(&buffer, 13);
    assert_eq!(heading.text, "Setup");
    let painted: Vec<bool> = heading
        .text_cells(&buffer)
        .iter()
        .map(|cell| cell.bg == match_style.bg.unwrap())
        .collect();
    assert_eq!(painted, vec![true; 5]);
    let outside = (heading.text_x + 5..buffer.area.width - 1)
        .filter(|&x| buffer[(x, heading.y)].bg == match_style.bg.unwrap())
        .count();
    assert_eq!(outside, 0);
}
