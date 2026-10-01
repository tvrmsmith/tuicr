//! Rendered `.md` diffs, observed through the whole app drawn into a
//! `TestBackend`. The mock VCS serves a synthetic markdown file and counts
//! fetches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::app::*;
use crate::error::TuicrError;
use crate::forge::traits::{ForgeBackend, ForgeFileLinesRequest};
use crate::model::{DiffLine, FilePatch, FileStatus};
use crate::theme::Theme;
use crate::vcs::slice_context_lines;
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

/// State shared between the test and the mock backends, which the `App`
/// owns once built.
#[derive(Default)]
struct Served {
    /// Full new side per path. A path with no entry serves nothing.
    files: HashMap<String, String>,
    fail: bool,
    /// What a reload of the working tree diff returns.
    patches: Vec<FilePatch>,
    /// Paths of every `fetch_context_lines` call. Line counts are not recorded.
    fetches: Vec<String>,
}

type SharedServed = Arc<Mutex<Served>>;

fn serve(served: &SharedServed, path: &Path) -> crate::error::Result<String> {
    let state = served.lock().unwrap();
    if state.fail {
        return Err(TuicrError::UnsupportedOperation(
            "mock fetch failure".into(),
        ));
    }
    Ok(state
        .files
        .get(&path.display().to_string())
        .cloned()
        .unwrap_or_default())
}

fn fetch_count(served: &SharedServed) -> usize {
    served.lock().unwrap().fetches.len()
}

fn fetch_count_for(served: &SharedServed, path: &str) -> usize {
    served
        .lock()
        .unwrap()
        .fetches
        .iter()
        .filter(|fetched| fetched.as_str() == path)
        .count()
}

struct MockVcs {
    info: VcsInfo,
    served: SharedServed,
}

impl VcsBackend for MockVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        let patches = self.served.lock().unwrap().patches.clone();
        crate::vcs::diff_parser::parse_file_patches(patches, highlighter)
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        self.served
            .lock()
            .unwrap()
            .fetches
            .push(file_path.display().to_string());
        let content = serve(&self.served, file_path)?;
        Ok(slice_context_lines(&content, start_line, end_line))
    }

    fn file_line_count(
        &self,
        file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(serve(&self.served, file_path)?.lines().count() as u32)
    }
}

fn joined(lines: &[&str]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn new_side_with_two_lines_inserted() -> String {
    let mut lines = vec!["intro one", "intro two"];
    lines.extend(GUIDE_NEW);
    joined(&lines)
}

fn file_patch(path: &str, status: FileStatus, patch: &str) -> FilePatch {
    let (old_path, new_path) = match status {
        FileStatus::Added => (None, Some(PathBuf::from(path))),
        FileStatus::Deleted => (Some(PathBuf::from(path)), None),
        _ => (Some(PathBuf::from(path)), Some(PathBuf::from(path))),
    };
    FilePatch::new(old_path, new_path, status, patch)
}

/// `patch` with every `@@` header's starts moved down by `by` lines.
fn shifted(patch: &str, by: u32) -> String {
    patch
        .lines()
        .map(|line| match line.strip_prefix("@@ -") {
            Some(rest) => {
                let (old, rest) = rest.split_once(" +").unwrap();
                let (new, _) = rest.split_once(" @@").unwrap();
                let shift = |range: &str| {
                    let (start, count) = range.split_once(',').unwrap();
                    format!("{},{count}", start.parse::<u32>().unwrap() + by)
                };
                format!("@@ -{} +{} @@", shift(old), shift(new))
            }
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Parses patches the way the real loaders do, so hunk lines carry syntect spans.
fn parse(patches: Vec<FilePatch>) -> Vec<DiffFile> {
    crate::vcs::diff_parser::parse_file_patches(patches, Theme::dark().syntax_highlighter())
        .expect("parse fixture patches")
}

fn guide_patches() -> Vec<FilePatch> {
    vec![
        file_patch(GUIDE, FileStatus::Modified, GUIDE_PATCH),
        file_patch("src/lib.rs", FileStatus::Modified, LIB_PATCH),
    ]
}

fn served_with_guide() -> SharedServed {
    let served = SharedServed::default();
    {
        let mut state = served.lock().unwrap();
        state.files.insert(GUIDE.to_string(), joined(&GUIDE_NEW));
        state.patches = guide_patches();
    }
    served
}

fn vcs_info() -> VcsInfo {
    VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "abc123".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    }
}

fn build_app(files: Vec<DiffFile>, served: &SharedServed) -> App {
    let vcs_info = vcs_info();
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    let mut app = App::build(
        Box::new(MockVcs {
            info: vcs_info.clone(),
            served: Arc::clone(served),
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

fn guide_app(served: &SharedServed) -> App {
    build_app(parse(guide_patches()), served)
}

fn rendered_guide_app(served: &SharedServed) -> App {
    let mut app = guide_app(served);
    app.set_render_markdown_diffs(true);
    app
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

// 1. One row per source line.
#[test]
fn should_draw_one_row_per_hunk_line_when_markdown_diffs_render() {
    let served = served_with_guide();
    let mut off = guide_app(&served);
    let mut on = rendered_guide_app(&served);

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

// 2. Table header above the hunk, removed row in its old context.
#[test]
fn should_render_table_rows_with_the_header_columns_above_the_hunk() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);

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

// 3. Hunk starting inside a code block.
#[test]
fn should_render_code_block_rows_when_a_hunk_starts_inside_the_fence() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);

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

// 4. Headings and inline.
#[test]
fn should_render_headings_and_inline_markup_without_their_markers() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);

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

// 5. Diff colours kept.
#[test]
fn should_keep_the_diff_background_on_rendered_rows() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
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

// 6. Fence rows in a diff (Added file, no fetch).
#[test]
fn should_render_an_added_file_from_its_hunk_without_fetching() {
    let served = served_with_guide();
    let added = file_patch(
        "docs/new.md",
        FileStatus::Added,
        "@@ -0,0 +1,5 @@\n+Intro\n+```python\n+print(1)\n+```\n+After\n",
    );
    let mut app = build_app(parse(vec![added]), &served);
    app.set_render_markdown_diffs(true);

    let buffer = draw(&mut app);

    let rows = numbered_rows(&buffer, "docs/new.md");
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts, ["Intro", "python", "\u{2502} print(1)", "", "After"]);
    assert_eq!(rows[1].cell_of(&buffer, "python").fg, Theme::dark().fg_dim);
    assert_eq!(fetch_count_for(&served, "docs/new.md"), 0);
}

// 7. Deleted file renders from its hunk.
#[test]
fn should_render_a_deleted_file_from_its_hunk_without_fetching() {
    let served = served_with_guide();
    let deleted = file_patch(
        "docs/old.md",
        FileStatus::Deleted,
        "@@ -1,2 +0,0 @@\n-## Gone\n-Text\n",
    );
    let mut app = build_app(parse(vec![deleted]), &served);
    app.set_render_markdown_diffs(true);

    let buffer = draw(&mut app);

    let rows = numbered_rows(&buffer, "docs/old.md");
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts, ["Gone", "Text"]);
    assert_eq!(fetch_count_for(&served, "docs/old.md"), 0);
}

// 8. Side-by-side.
#[test]
fn should_render_markdown_rows_on_both_sides_in_side_by_side_view() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
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

// 9. Off by default.
#[test]
fn should_keep_source_text_and_not_fetch_when_markdown_diffs_are_off() {
    let served = served_with_guide();
    let mut app = guide_app(&served);

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 13).text, "## Setup");
    assert_eq!(row_for_new(&buffer, 6).text, "| b | second |");
    assert_eq!(fetch_count(&served), 0);
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

fn failing_served() -> SharedServed {
    let served = served_with_guide();
    served.lock().unwrap().fail = true;
    served
}

// 10. Fetch failure falls back to source.
#[test]
fn should_fall_back_to_source_when_the_fetch_fails() {
    let served = failing_served();
    let mut app = rendered_guide_app(&served);

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 13).text, "## Setup");
    assert_eq!(row_for_new(&buffer, 6).text, "| b | second |");
    app.diff_state.cursor_line = annotation_of_new_line(&app, 13);
    app.cursor_down(1);
    assert_eq!(app.diff_state.cursor_line, annotation_of_new_line(&app, 14));
}

// 11. A failed fetch is not retried on every rebuild, and an explicit reload retries.
#[test]
fn should_not_retry_a_failed_fetch_until_an_explicit_reload() {
    let served = failing_served();
    let mut app = rendered_guide_app(&served);
    let after_first_attempt = fetch_count(&served);

    for _ in 0..3 {
        app.rebuild_annotations();
    }
    assert_eq!(fetch_count(&served), after_first_attempt);

    served.lock().unwrap().fail = false;
    app.reload_diff_files().expect("reload");

    let buffer = draw(&mut app);
    assert_eq!(row_for_new(&buffer, 13).text, "Setup");
}

// 12. Check failure falls back to source.
#[test]
fn should_fall_back_to_source_when_the_served_file_disagrees_with_the_hunks() {
    let served = served_with_guide();
    let mut lines = GUIDE_NEW.to_vec();
    lines[5] = "| b | other |";
    served
        .lock()
        .unwrap()
        .files
        .insert(GUIDE.to_string(), joined(&lines));
    let mut app = rendered_guide_app(&served);

    let buffer = draw(&mut app);

    assert_eq!(row_for_new(&buffer, 6).text, "| b | second |");
    assert_eq!(row_for_new(&buffer, 13).text, "## Setup");
}

// 13. Success is cached, never refetched per rebuild.
#[test]
fn should_not_refetch_a_rendered_file_on_rebuild_or_comment() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
    let after_render = fetch_count(&served);
    assert!(after_render > 0);

    for _ in 0..3 {
        app.rebuild_annotations();
    }
    app.diff_state.cursor_line = annotation_of_new_line(&app, 13);
    app.enter_comment_mode(false, Some((13, LineSide::New)));
    app.comment_buffer = "note".to_string();
    app.save_comment();
    let buffer = draw(&mut app);
    assert_eq!(fetch_count(&served), after_render);
    assert_eq!(row_for_new(&buffer, 13).text, "Setup");
}

// 14. Stale source refetches.
#[test]
fn should_refetch_once_when_the_hunks_moved_under_a_cached_source() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
    let after_render = fetch_count(&served);

    served
        .lock()
        .unwrap()
        .files
        .insert(GUIDE.to_string(), new_side_with_two_lines_inserted());
    app.diff_files = parse(vec![file_patch(
        GUIDE,
        FileStatus::Modified,
        &shifted(GUIDE_PATCH, 2),
    )]);
    app.rebuild_annotations();

    let buffer = draw(&mut app);
    assert_eq!(row_for_new(&buffer, 15).text, "Setup");
    assert_eq!(fetch_count(&served), after_render + 1);
}

// 14b. A revision change refetches.
#[test]
fn should_refetch_once_when_the_revision_changes() {
    let served = served_with_guide();
    let mut app = guide_app(&served);
    app.diff_source = DiffSource::CommitRange(vec!["rev1".to_string()]);
    app.set_render_markdown_diffs(true);
    let buffer = draw(&mut app);
    assert_eq!(row_for_new(&buffer, 6).text, "b    │ second");
    let after_render = fetch_count(&served);

    let widened = joined(&GUIDE_NEW).replace("| Name | Notes |", "| Label | Notes |");
    served
        .lock()
        .unwrap()
        .files
        .insert(GUIDE.to_string(), widened);
    app.diff_source = DiffSource::CommitRange(vec!["rev2".to_string()]);
    app.rebuild_annotations();

    let buffer = draw(&mut app);
    assert_eq!(row_for_new(&buffer, 6).text, "b     │ second");
    assert_eq!(fetch_count(&served), after_render + 1);
}

/// A forge that serves the shared files through `fetch_file_lines`, so fetches
/// through it land in the same counter as the VCS mock's.
struct MockForge {
    served: SharedServed,
}

impl ForgeBackend for MockForge {
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
        unimplemented!()
    }
    fn get_pull_request_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<FilePatch>> {
        unimplemented!()
    }
    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        self.served
            .lock()
            .unwrap()
            .fetches
            .push(request.path.display().to_string());
        let content = serve(&self.served, &request.path)?;
        Ok(slice_context_lines(
            &content,
            request.start_line,
            request.end_line,
        ))
    }
    fn file_line_count(&self, request: ForgeFileLinesRequest) -> Result<u32> {
        Ok(serve(&self.served, &request.path)?.lines().count() as u32)
    }
    fn list_review_threads(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
        unimplemented!()
    }
    fn list_pull_request_commits(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
        unimplemented!()
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

/// A PR-mode app with no forge backend attached yet, as it is before the
/// forge session finishes opening.
fn pr_app_without_forge() -> App {
    use crate::forge::traits::{ForgeRepository, PrSessionKey};

    let pr = PullRequestDiffSource {
        key: PrSessionKey::new(
            ForgeRepository::github("github.com", "owner", "repo"),
            42,
            "abc1234567890",
        ),
        base_sha: "def0987654321".to_string(),
        title: "Guide".to_string(),
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
        Box::new(crate::vcs::PrNoopVcs::new(vcs_info.clone())),
        vcs_info,
        Theme::dark(),
        None,
        false,
        parse(vec![file_patch(GUIDE, FileStatus::Modified, GUIDE_PATCH)]),
        session,
        DiffSource::PullRequest(Box::new(pr)),
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build pr app");
    app.diff_view_mode = DiffViewMode::Unified;
    app.diff_state.wrap_lines = false;
    app
}

// 15. PR mode renders after an early fetch failure.
#[test]
fn should_render_in_pr_mode_once_the_forge_backend_attaches() {
    let served = served_with_guide();
    let mut app = pr_app_without_forge();
    app.set_render_markdown_diffs(true);

    let buffer = draw(&mut app);
    assert_eq!(drawn_row_numbered(&buffer, 13).text, "## Setup");

    app.forge_backend = Some(Box::new(MockForge {
        served: Arc::clone(&served),
    }));
    app.rebuild_annotations();

    let buffer = draw(&mut app);
    assert_eq!(drawn_row_numbered(&buffer, 13).text, "Setup");
}

const LINK_LINE: &str =
    "See [docs](https://example.com/a/very/long/path/that/keeps/going/and/going/on/forever).";

/// The guide with `LINK_LINE` added after line 15, as new line 16.
fn link_line_patches() -> Vec<FilePatch> {
    let patch =
        GUIDE_PATCH.replace("@@ -12,1 +12,4 @@", "@@ -12,1 +12,5 @@") + &format!("+{LINK_LINE}\n");
    vec![
        file_patch(GUIDE, FileStatus::Modified, &patch),
        file_patch("src/lib.rs", FileStatus::Modified, LIB_PATCH),
    ]
}

fn served_with_link_line() -> SharedServed {
    let served = served_with_guide();
    {
        let mut state = served.lock().unwrap();
        let mut lines: Vec<&str> = GUIDE_NEW.to_vec();
        lines.push(LINK_LINE);
        state.files.insert(GUIDE.to_string(), joined(&lines));
        state.patches = link_line_patches();
    }
    served
}

fn draw_sized(app: &mut App, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .expect("draw frame");
    terminal.backend().buffer().clone()
}

/// The first drawn row carrying gutter number `n`, for narrow terminals where
/// the file header's rule row confuses `file_block`.
fn drawn_row_numbered(buffer: &Buffer, n: u32) -> DrawnRow {
    let panel_x = diff_panel_x(buffer);
    (2..buffer.area.height - 2)
        .map(|y| parse_row(buffer, y, panel_x))
        .find(|row| row.number == Some(n))
        .unwrap_or_else(|| panic!("no row numbered {n}"))
}

// 10. Wrapped row height follows the rendered text (unified).
#[test]
fn should_size_a_wrapped_unified_row_by_its_rendered_text() {
    let served = served_with_link_line();
    let mut app = build_app(parse(link_line_patches()), &served);
    app.diff_state.wrap_lines = true;
    app.set_render_markdown_diffs(true);

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

// 11. Wrapped row height follows the rendered text (side-by-side).
#[test]
fn should_size_a_wrapped_side_by_side_row_by_its_rendered_text() {
    let served = served_with_link_line();
    let sbs_app = |render: bool| {
        let mut app = build_app(parse(link_line_patches()), &served);
        app.diff_state.wrap_lines = true;
        app.diff_view_mode = DiffViewMode::SideBySide;
        app.set_render_markdown_diffs(render);
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

// 12. Confirming a theme keeps the render.
#[test]
fn should_keep_the_render_and_recolor_it_when_a_theme_is_confirmed() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
    let dark_fg = {
        let buffer = draw(&mut app);
        let heading = drawn_row_numbered(&buffer, 13);
        heading.cell_of(&buffer, "S").fg
    };
    let fetches = fetch_count(&served);

    preview_in_picker(&mut app, OTHER_THEME);
    app.confirm_theme_picker();

    let buffer = draw(&mut app);
    let heading = drawn_row_numbered(&buffer, 13);
    assert_eq!(heading.text, "Setup");
    assert_ne!(heading.cell_of(&buffer, "S").fg, dark_fg);
    assert_eq!(fetch_count(&served), fetches);
}

// 13. Previewing a theme keeps the current .md file rendered.
#[test]
fn should_keep_the_current_markdown_file_rendered_when_a_theme_is_previewed() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
    let fetches = fetch_count(&served);
    assert_eq!(app.diff_state.current_file_idx, 0);
    // A drawn frame first, so the preview cannot lean on a layout rebuild.
    draw(&mut app);

    app.preview_theme(OTHER_THEME);

    let buffer = draw(&mut app);
    assert_eq!(drawn_row_numbered(&buffer, 13).text, "Setup");
    assert_eq!(fetch_count(&served), fetches);
}

// 14. Cancelling the picker keeps the render under the original theme.
#[test]
fn should_keep_the_render_under_the_original_theme_when_the_picker_is_cancelled() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
    let fetches = fetch_count(&served);
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
    assert_eq!(fetch_count(&served), fetches);
}

// 15. Search paints the rendered cells.
#[test]
fn should_paint_search_matches_on_rendered_cells() {
    let served = served_with_guide();
    let mut app = rendered_guide_app(&served);
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
