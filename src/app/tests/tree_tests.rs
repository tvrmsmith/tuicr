use crate::app::*;
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};

/// The 161-file changeset the sidebar row costs in `docs/SIDEBAR_MODEL.md`
/// were measured against. Lines are `<status>\t<path>`.
const ROW_COST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/grouping/orca-971b16754.files");

fn make_file(path: &str) -> DiffFile {
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks: vec![],
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 0,
    }
}

/// Drives the real tree code on a throwaway `App`, so what the tests assert
/// is what the sidebar ships.
struct TreeTestHarness {
    app: App,
}

impl TreeTestHarness {
    fn new(paths: &[&str]) -> Self {
        Self::with_mode(FileTreeMode::Nested, paths)
    }

    fn with_mode(mode: FileTreeMode, paths: &[&str]) -> Self {
        Self::from_files(mode, paths.iter().map(|p| make_file(p)).collect())
    }

    fn from_files(mode: FileTreeMode, files: Vec<DiffFile>) -> Self {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".into(),
            branch_name: Some("main".into()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );
        let mut app = App::build(
            Box::new(StubVcs(vcs_info.clone())),
            vcs_info,
            crate::theme::Theme::dark(),
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
        app.file_tree_mode = mode;
        app.expand_all_dirs();
        Self { app }
    }

    fn expand_all(&mut self) {
        self.app.expand_all_dirs();
    }

    fn collapse_all(&mut self) {
        self.app.collapse_all_dirs();
    }

    fn toggle(&mut self, dir: &str) {
        self.app.toggle_directory(dir);
    }

    fn build_visible_items(&self) -> Vec<FileTreeItem> {
        self.app.build_visible_items()
    }

    fn visible_file_count(&self) -> usize {
        self.build_visible_items()
            .iter()
            .filter(|i| matches!(i, FileTreeItem::File { .. }))
            .count()
    }

    fn visible_dir_count(&self) -> usize {
        self.build_visible_items()
            .iter()
            .filter(|i| matches!(i, FileTreeItem::Directory { .. }))
            .count()
    }

    fn dir_labels(&self) -> Vec<String> {
        self.build_visible_items()
            .iter()
            .filter_map(|item| match item {
                FileTreeItem::Directory { label, .. } => Some(label.clone()),
                FileTreeItem::File { .. } | FileTreeItem::Group { .. } => None,
            })
            .collect()
    }

    fn file_labels(&self) -> Vec<String> {
        self.build_visible_items()
            .iter()
            .filter_map(|item| match item {
                FileTreeItem::File { label, .. } => Some(label.clone()),
                FileTreeItem::Directory { .. } | FileTreeItem::Group { .. } => None,
            })
            .collect()
    }
}

/// The hand grouping the row costs in `docs/SIDEBAR_MODEL.md` were drawn
/// against: 13 groups in intent-centrality order.
const ROW_COST_GROUPS: &str =
    include_str!("../../../tests/fixtures/grouping/orca-971b16754.groups");

fn row_cost_files() -> Vec<DiffFile> {
    let files: Vec<DiffFile> = ROW_COST_FIXTURE
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| make_file(line.split_once('\t').map_or(line, |(_status, path)| path)))
        .collect();
    assert_eq!(files.len(), 161, "fixture changed size");
    files
}

fn row_cost_harness(mode: FileTreeMode) -> TreeTestHarness {
    TreeTestHarness::from_files(mode, row_cost_files())
}

/// The same fixture with the same hand grouping the mockups were drawn from,
/// driven through the real grouped sidebar: the record's partition and reading
/// order, the shipped within-group sort, and per-run directory emission.
fn grouped_row_cost_app(mode: FileTreeMode) -> App {
    grouped_app_from(mode, row_cost_files(), ROW_COST_GROUPS)
}

fn grouped_app_from(mode: FileTreeMode, files: Vec<DiffFile>, groups_text: &str) -> App {
    use crate::grouping::changeset::Changeset;
    use crate::grouping::{GroupId, GroupSource, Grouping, PresentedGroup};

    let mut presented: Vec<PresentedGroup> = Vec::new();
    for line in groups_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            Some(name) => presented.push(PresentedGroup {
                id: GroupId::new(),
                name: name.to_string(),
                source: GroupSource::Heuristics,
                new_since_full_pass: false,
                members: Vec::new(),
            }),
            None => presented
                .last_mut()
                .expect("a path before any [group] header")
                .members
                .push(line.to_string()),
        }
    }
    assert!(!presented.is_empty(), "no [group] header in the fixture");

    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".into(),
        branch_name: Some("main".into()),
        vcs_type: VcsType::Git,
    };
    let mut session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    for file in &files {
        session.add_file(file.display_path().clone(), file.status, file.content_hash);
    }
    // Recorded on the session so `enable_grouping` restores it verbatim
    // instead of computing a heuristic one: the table measures the record's
    // grouping, not the engine's.
    session.record_grouping(&Grouping::restore(
        &Changeset::from_diff_files(&files),
        presented,
    ));

    let mut app = App::build(
        Box::new(StubVcs(vcs_info.clone())),
        vcs_info,
        crate::theme::Theme::dark(),
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
    app.file_tree_mode = mode;
    app.enable_grouping();
    app.expand_all_dirs();
    app
}

#[test]
fn test_expand_all_shows_all_files() {
    let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/ui/help.rs", "src/main.rs"]);
    h.expand_all();

    assert_eq!(h.visible_file_count(), 3);
}

#[test]
fn test_collapse_all_hides_all_files() {
    let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/main.rs"]);
    h.expand_all();
    h.collapse_all();

    assert_eq!(h.visible_file_count(), 0);
    assert_eq!(h.visible_dir_count(), 1); // only "src" visible
}

#[test]
fn test_collapse_parent_hides_nested_dirs() {
    let mut h = TreeTestHarness::new(&["src/ui/components/button.rs"]);
    h.expand_all();
    assert_eq!(h.visible_dir_count(), 3); // src, src/ui, src/ui/components

    h.toggle("src");
    let items = h.build_visible_items();
    assert_eq!(items.len(), 1); // only collapsed "src" dir
    assert!(matches!(
        &items[0],
        FileTreeItem::Directory {
            expanded: false,
            ..
        }
    ));
}

#[test]
fn test_root_files_always_visible() {
    let mut h = TreeTestHarness::new(&["README.md", "Cargo.toml"]);
    h.collapse_all();

    assert_eq!(h.visible_file_count(), 2);
}

#[test]
fn test_tree_depth_correct() {
    let mut h = TreeTestHarness::new(&["a/b/c/file.rs"]);
    h.expand_all();

    let items = h.build_visible_items();
    assert!(matches!(&items[0], FileTreeItem::Directory { depth: 0, path, .. } if path == "a"));
    assert!(matches!(&items[1], FileTreeItem::Directory { depth: 1, path, .. } if path == "a/b"));
    assert!(matches!(&items[2], FileTreeItem::Directory { depth: 2, path, .. } if path == "a/b/c"));
    assert!(matches!(&items[3], FileTreeItem::File { depth: 3, .. }));
}

#[test]
fn test_toggle_expands_collapsed_dir() {
    let mut h = TreeTestHarness::new(&["src/main.rs"]);
    h.collapse_all();
    assert_eq!(h.visible_file_count(), 0);

    h.toggle("src");
    assert_eq!(h.visible_file_count(), 1);
}

#[test]
fn test_sibling_dirs_independent() {
    let mut h = TreeTestHarness::new(&["src/app.rs", "tests/test.rs"]);
    h.expand_all();
    h.toggle("src"); // collapse src

    assert_eq!(h.visible_file_count(), 1); // only tests/test.rs
}

struct StubVcs(VcsInfo);
impl VcsBackend for StubVcs {
    fn info(&self) -> &VcsInfo {
        &self.0
    }
    fn get_working_tree_diff(
        &self,
        _hl: &crate::syntax::SyntaxHighlighter,
    ) -> crate::error::Result<Vec<DiffFile>> {
        Ok(Vec::new())
    }
    fn fetch_context_lines(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
        _start: u32,
        _end: u32,
    ) -> crate::error::Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        Ok(0)
    }
}

fn app_with(paths: &[&str]) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".into(),
        branch_name: Some("main".into()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    App::build(
        Box::new(StubVcs(vcs_info.clone())),
        vcs_info,
        crate::theme::Theme::dark(),
        None,
        false,
        paths.iter().map(|p| make_file(p)).collect(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app")
}

/// Renders the visible tree as (label, depth) pairs, with directories
/// suffixed by '/' so a mis-parented file is obvious in the assertion.
fn rendered_tree(app: &App) -> Vec<(String, usize)> {
    app.build_visible_items()
        .iter()
        .map(|item| match item {
            FileTreeItem::Directory { path, depth, .. } => (format!("{path}/"), *depth),
            FileTreeItem::File {
                file_idx, depth, ..
            } => (
                app.diff_files[*file_idx]
                    .display_path()
                    .to_string_lossy()
                    .to_string(),
                *depth,
            ),
            FileTreeItem::Group { label, .. } => (format!("[{label}]"), 0),
        })
        .collect()
}

#[test]
fn test_interleaved_paths_stay_under_own_directory() {
    // A sibling directory sharing a prefix used to sort between a directory
    // and its own subdirectory, because '.' sorts before '/'. That split the
    // ChronoStream subtree in two, and since build_visible_items emits a
    // directory header only once, ChronoStream/Subdir rendered indented under
    // ChronoStream.BuildTests.
    let mut app = app_with(&[
        "ChronoStream/Subdir/file.cs",
        "ChronoStream.BuildTests/test.cs",
        "ChronoStream/root.cs",
    ]);
    app.sort_files_by_directory(true);
    app.expand_all_dirs();

    assert_eq!(
        rendered_tree(&app),
        vec![
            ("ChronoStream/".to_string(), 0),
            ("ChronoStream/root.cs".to_string(), 1),
            ("ChronoStream/Subdir/".to_string(), 1),
            ("ChronoStream/Subdir/file.cs".to_string(), 2),
            ("ChronoStream.BuildTests/".to_string(), 0),
            ("ChronoStream.BuildTests/test.cs".to_string(), 1),
        ]
    );
}

#[test]
fn nested_is_the_default_mode() {
    assert_eq!(FileTreeMode::default(), FileTreeMode::Nested);
}

#[test]
fn default_mode_emits_one_row_per_ancestor() {
    let h = TreeTestHarness::new(&["a/b/c/file.rs"]);

    assert_eq!(h.dir_labels(), vec!["a/", "b/", "c/"]);
    assert_eq!(h.file_labels(), vec!["file.rs"]);
}

#[test]
fn compact_joins_a_single_child_chain_into_one_row() {
    let h = TreeTestHarness::with_mode(FileTreeMode::Compact, &["a/b/c/file.rs"]);
    let items = h.build_visible_items();

    assert_eq!(h.dir_labels(), vec!["a/b/c/"]);
    assert!(matches!(
        &items[0],
        FileTreeItem::Directory { path, depth: 0, .. } if path == "a/b/c"
    ));
    assert!(matches!(&items[1], FileTreeItem::File { depth: 1, .. }));
}

#[test]
fn compact_stops_joining_where_the_tree_branches() {
    let h = TreeTestHarness::with_mode(
        FileTreeMode::Compact,
        &["src/main/github/a.rs", "src/renderer/lib/b.rs"],
    );

    assert_eq!(
        h.dir_labels(),
        vec!["src/", "main/github/", "renderer/lib/"]
    );
}

#[test]
fn compact_does_not_join_a_directory_holding_its_own_files() {
    let h = TreeTestHarness::with_mode(FileTreeMode::Compact, &["a/own.rs", "a/b/nested.rs"]);

    assert_eq!(h.dir_labels(), vec!["a/", "b/"]);
}

#[test]
fn compact_chain_toggles_as_one_unit_keyed_by_the_joined_path() {
    let mut h = TreeTestHarness::with_mode(FileTreeMode::Compact, &["a/b/c/file.rs"]);
    assert_eq!(h.visible_file_count(), 1);

    h.toggle("a/b/c");
    assert_eq!(h.visible_file_count(), 0);
    assert_eq!(h.visible_dir_count(), 1);

    // The joined chain has no per-segment rows, so its intermediate paths are
    // not toggle keys at all.
    h.toggle("a/b");
    assert_eq!(h.visible_file_count(), 0);
}

#[test]
fn flat_emits_no_directory_rows_and_labels_files_with_the_full_path() {
    let h = TreeTestHarness::with_mode(FileTreeMode::Flat, &["a/b/c/file.rs", "README.md"]);

    assert_eq!(h.visible_dir_count(), 0);
    assert_eq!(h.file_labels(), vec!["README.md", "a/b/c/file.rs"]);
    assert!(
        h.build_visible_items()
            .iter()
            .all(|item| matches!(item, FileTreeItem::File { depth: 0, .. }))
    );
}

#[test]
fn flat_keeps_every_file_visible_when_everything_is_collapsed() {
    let mut h = TreeTestHarness::with_mode(FileTreeMode::Flat, &["a/b/c/file.rs", "src/main.rs"]);
    h.collapse_all();

    assert_eq!(h.visible_file_count(), 2);
}

// Row costs from the table in `docs/SIDEBAR_MODEL.md`, fully expanded.

#[test]
fn nested_costs_197_rows_on_the_fixture() {
    let h = row_cost_harness(FileTreeMode::Nested);

    assert_eq!(h.build_visible_items().len(), 197);
}

#[test]
fn compact_costs_191_rows_on_the_fixture() {
    let h = row_cost_harness(FileTreeMode::Compact);

    assert_eq!(h.build_visible_items().len(), 191);
}

#[test]
fn flat_costs_161_rows_on_the_fixture() {
    let h = row_cost_harness(FileTreeMode::Flat);

    assert_eq!(h.build_visible_items().len(), 161);
}

// The grouped half of the same table, re-measured by `gd-26r.31` against the
// shipped code. The record's numbers had been drawn from mockups and nothing
// held them to the sidebar; two of the three had drifted.

fn grouped_row_count(mode: FileTreeMode) -> usize {
    grouped_row_cost_app(mode).build_visible_items().len()
}

#[test]
fn the_chosen_grouped_layout_costs_174_rows_on_the_fixture() {
    // 161 files plus 13 group rows, no directory rows. `gd-26r.34` makes this
    // the only grouped layout; today it is what `flat` already produces.
    assert_eq!(grouped_row_count(FileTreeMode::Flat), 174);
}

#[test]
fn every_group_collapses_to_a_thirteen_row_overview() {
    // The strongest claim in `docs/SIDEBAR_MODEL.md`, and the one the whole
    // sidebar case rests on. A collapsed group emits nothing beneath it, so no
    // in-group layout decision can move this — hence all three modes.
    for mode in [
        FileTreeMode::Nested,
        FileTreeMode::Compact,
        FileTreeMode::Flat,
    ] {
        let mut app = grouped_row_cost_app(mode);
        app.collapse_all_dirs();

        assert_eq!(app.build_visible_items().len(), 13, "{mode:?}");
    }
}

#[test]
fn per_run_directory_rows_cost_five_rows_over_one_row_per_directory() {
    // The record published 300 for `groups + nested`, measured at one row per
    // directory per group. That model still reproduces exactly — 126 distinct
    // directories, 161 files, 13 groups — so the whole of the +5 is the
    // per-run emission `gd-26r.28` slice A shipped, not an error in the
    // original arithmetic.
    let items = grouped_row_cost_app(FileTreeMode::Nested).build_visible_items();
    let dir_rows: Vec<&String> = items
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::Directory { path, .. } => Some(path),
            FileTreeItem::File { .. } | FileTreeItem::Group { .. } => None,
        })
        .collect();
    let distinct: HashSet<&&String> = dir_rows.iter().collect();

    assert_eq!(distinct.len(), 126, "distinct (group, directory) keys");
    assert_eq!(distinct.len() + 161 + 13, 300, "the published cost model");
    assert_eq!(dir_rows.len(), 131, "five directories span two runs");
    assert_eq!(items.len(), 305);
}

#[test]
fn grouped_compact_costs_285_rows_on_the_fixture() {
    // Published as 251, which assumed chain joins decided against a group's
    // own files. `TreeLayout` (`tree.rs:106`) decides them once over the whole
    // of `diff_files`, so joins that pay inside a group are never made.
    // `gd-26r.34` deletes the question with the rows.
    assert_eq!(grouped_row_count(FileTreeMode::Compact), 285);
}
