//! The synthetic `Commit Message (<sha>)` pseudo-file sits at `diff_files`
//! index 0, and two independent places put it there: `insert_commit_message_if_single`
//! inserts it (`diff_load.rs`), and `sort_files_by_directory` hoists it back
//! ahead of every real file on every reorder (`tree.rs`).
//!
//! Nothing guarded either until now. `gd-26r.12` ruled the pseudo-file lives
//! *outside* the grouping partition, pinned above every group node, so the
//! hoist is a deliberate invariant the grouping work must preserve rather than
//! an accident of the current sort. These tests are what says so out loud.

use chrono::Utc;

use crate::app::*;
use crate::model::{DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{CommitInfo, VcsType};

struct DummyVcs {
    info: VcsInfo,
}

impl VcsBackend for DummyVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }
    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::NoChanges)
    }
    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

fn commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.to_string(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: Some("a body line".to_string()),
        author: "tester".to_string(),
        time: Utc::now(),
    }
}

fn file(path: &str) -> DiffFile {
    let hunks = vec![DiffHunk {
        header: "@@ -1,1 +1,1 @@".to_string(),
        lines: vec![DiffLine {
            origin: LineOrigin::Context,
            content: "line".to_string(),
            old_lineno: Some(1),
            new_lineno: Some(1),
            highlighted_spans: None,
        }],
        old_start: 1,
        old_count: 1,
        new_start: 1,
        new_count: 1,
    }];
    let content_hash = DiffFile::compute_content_hash(&hunks);
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    }
}

fn build_app(files: Vec<DiffFile>, commits: Vec<CommitInfo>) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    let mut app = App::build(
        Box::new(DummyVcs {
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
    .expect("failed to build test app");
    app.review_commits = commits;
    app
}

fn paths(app: &App) -> Vec<String> {
    app.diff_files
        .iter()
        .map(|f| f.display_path().to_string_lossy().to_string())
        .collect()
}

/// `README.md` is the discriminating case: it lives in the repo root, which
/// `sort_files_by_directory` files under `"."`, and `"."` sorts before every
/// real directory in the `BTreeMap`. Without the hoist it, not the commit
/// message, would be index 0.
fn app_with_commit_message() -> App {
    let mut app = build_app(
        vec![
            file("README.md"),
            file("src/app/tree.rs"),
            file("zzz/last.rs"),
        ],
        vec![commit("abc1234")],
    );
    app.insert_commit_message_if_single();
    app
}

#[test]
fn insert_commit_message_puts_the_pseudo_file_at_index_zero() {
    let app = app_with_commit_message();

    assert_eq!(
        paths(&app)[0],
        "Commit Message (abc1234)",
        "the synthetic commit-message file must be diff_files index 0"
    );
    assert!(
        app.diff_files[0].is_commit_message,
        "index 0 must be flagged is_commit_message"
    );
    assert_eq!(
        app.diff_files
            .iter()
            .filter(|f| f.is_commit_message)
            .count(),
        1,
        "exactly one commit-message pseudo-file"
    );
}

#[test]
fn sorting_by_directory_keeps_the_pseudo_file_at_index_zero() {
    let mut app = app_with_commit_message();

    app.sort_files_by_directory(true);

    assert_eq!(
        paths(&app),
        vec![
            "Commit Message (abc1234)".to_string(),
            "README.md".to_string(),
            "src/app/tree.rs".to_string(),
            "zzz/last.rs".to_string(),
        ],
        "the commit message stays ahead of every real file, including repo-root \
         files which sort under \".\" and would otherwise take index 0"
    );
}

/// The hoist has to survive a sort that re-finds the previously current file,
/// which is the branch `:reload` takes (`reset_position = false`).
#[test]
fn the_hoist_survives_a_sort_that_preserves_position() {
    let mut app = app_with_commit_message();
    let target = app
        .diff_files
        .iter()
        .position(|f| f.display_path() == Path::new("src/app/tree.rs"))
        .expect("target file present");
    app.jump_to_file(target);

    app.sort_files_by_directory(false);

    assert_eq!(
        paths(&app)[0],
        "Commit Message (abc1234)",
        "position-preserving sorts hoist the commit message too"
    );
    assert_eq!(
        app.diff_files[app.diff_state.current_file_idx].display_path(),
        Path::new("src/app/tree.rs"),
        "and the current file is still the one the cursor was on"
    );
}

/// No single commit selected means no pseudo-file at all, so nothing is hoisted
/// and index 0 is an ordinary file. Without this the two tests above would pass
/// against an implementation that hoisted something unconditionally.
#[test]
fn no_pseudo_file_when_more_than_one_commit_is_selected() {
    let mut app = build_app(
        vec![file("README.md"), file("src/app/tree.rs")],
        vec![commit("abc1234"), commit("def5678")],
    );

    app.insert_commit_message_if_single();
    app.sort_files_by_directory(true);

    assert!(
        !app.diff_files.iter().any(|f| f.is_commit_message),
        "two commits in range: no commit-message pseudo-file"
    );
    assert_eq!(paths(&app)[0], "README.md");
}
