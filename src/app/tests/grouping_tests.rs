//! Slice A end to end: the engine orders `diff_files`, the sidebar renders the
//! groups, and the session carries the grouping back out of JSON.

use crate::app::*;
use crate::model::{DiffFile, FileStatus};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};

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

fn commit_message_file() -> DiffFile {
    DiffFile {
        is_commit_message: true,
        ..make_file("COMMIT_MSG")
    }
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
        _path: &Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
        _start: u32,
        _end: u32,
    ) -> crate::error::Result<Vec<crate::model::DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _path: &Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        Ok(0)
    }
}

/// A real `App` with grouping on, exactly as the binary starts it.
fn grouped_app(files: Vec<DiffFile>) -> App {
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
    app.enable_grouping();
    app.expand_all_dirs();
    app
}

fn grouped_paths(paths: &[&str]) -> App {
    grouped_app(paths.iter().map(|path| make_file(path)).collect())
}

fn file_order(app: &App) -> Vec<String> {
    app.diff_files
        .iter()
        .map(|file| file.display_path().to_string_lossy().to_string())
        .collect()
}

fn visible_files(app: &App) -> Vec<String> {
    app.build_visible_items()
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::File { file_idx, .. } => Some(
                app.diff_files[*file_idx]
                    .display_path()
                    .to_string_lossy()
                    .to_string(),
            ),
            _ => None,
        })
        .collect()
}

fn group_rows(app: &App) -> Vec<(String, usize, usize)> {
    app.build_visible_items()
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::Group {
                label,
                reviewed,
                total,
                ..
            } => Some((label.clone(), *reviewed, *total)),
            _ => None,
        })
        .collect()
}

fn group_ids(app: &App) -> Vec<String> {
    app.build_visible_items()
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::Group { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

const PATHS: &[&str] = &[
    "src/auth/login.rs",
    "src/auth/session.rs",
    "src/auth/token.rs",
    "tests/auth_test.rs",
    "docs/auth.md",
    "Cargo.lock",
    ".github/workflows/ci.yml",
];

#[test]
fn grouping_reorders_diff_files_into_group_runs() {
    let app = grouped_paths(PATHS);
    let grouping = app.grouping.as_ref().expect("grouping computed");

    assert_eq!(
        file_order(&app).len(),
        PATHS.len(),
        "the partition is total: no file may be dropped by the reorder"
    );

    // Every group is one unbroken run of `diff_files`, which is what lets the
    // sidebar read a group as a single span.
    let mut runs: Vec<&str> = Vec::new();
    for file in &app.diff_files {
        let id = grouping
            .group_of(file.display_path())
            .expect("every real file is assigned")
            .as_str();
        if runs.last() != Some(&id) {
            assert!(!runs.contains(&id), "group {id} is split across runs");
            runs.push(id);
        }
    }
    assert_eq!(
        runs.len(),
        grouping.groups().len(),
        "every group occupies exactly one run"
    );

    // And the runs are in the engine's reading order, not the diff's.
    let expected: Vec<&str> = grouping
        .groups()
        .iter()
        .map(|group| group.id.as_str())
        .collect();
    assert_eq!(runs, expected);
}

#[test]
fn the_commit_message_row_stays_first_and_outside_every_group() {
    let mut files = vec![make_file("src/auth/login.rs")];
    files.push(commit_message_file());
    files.push(make_file("Cargo.lock"));
    let app = grouped_app(files);

    assert!(app.diff_files[0].is_commit_message, "hoisted to index 0");
    let grouping = app.grouping.as_ref().expect("grouping computed");
    assert!(
        grouping
            .group_of(app.diff_files[0].display_path())
            .is_none(),
        "the pseudo-file sits outside the partition"
    );
    assert_eq!(
        visible_files(&app).first().map(String::as_str),
        Some("COMMIT_MSG"),
        "and above every group row"
    );
}

#[test]
fn the_sidebar_shows_one_row_per_group_with_its_counts() {
    let mut app = grouped_paths(PATHS);
    let rows = group_rows(&app);
    assert_eq!(
        rows.len(),
        app.grouping.as_ref().unwrap().groups().len(),
        "one row per group"
    );
    assert_eq!(
        rows.iter().map(|(_, _, total)| total).sum::<usize>(),
        PATHS.len(),
        "the counts partition the changeset"
    );
    assert!(rows.iter().all(|(_, reviewed, _)| *reviewed == 0));

    let first = app.diff_files[0].display_path().clone();
    app.session
        .get_file_mut(&first)
        .expect("tracked file")
        .reviewed = true;
    assert_eq!(
        group_rows(&app)
            .iter()
            .map(|(_, reviewed, _)| reviewed)
            .sum::<usize>(),
        1,
        "the group row counts its own reviewed files"
    );
}

#[test]
fn collapsing_a_group_hides_its_files_and_nothing_else() {
    let mut app = grouped_paths(PATHS);
    let all = visible_files(&app);
    let ids = group_ids(&app);
    let target = ids.first().expect("at least one group").clone();

    let hidden: Vec<String> = {
        let grouping = app.grouping.as_ref().unwrap();
        let id = crate::grouping::GroupId::from_persisted(target.clone());
        grouping
            .files_in(&id)
            .map(|path| path.to_string_lossy().to_string())
            .collect()
    };
    assert!(!hidden.is_empty());

    app.toggle_directory(&target);
    let after = visible_files(&app);

    for path in &hidden {
        assert!(!after.contains(path), "{path} should be hidden");
    }
    for path in &all {
        assert!(
            hidden.contains(path) || after.contains(path),
            "{path} belongs to another group and must stay visible"
        );
    }
    assert_eq!(
        group_ids(&app),
        ids,
        "every group row survives its own collapse"
    );
}

/// A changeset the heuristics put `auth` on as one group spanning two
/// directories, with a broad test in the first of them — so the within-group
/// sort's broad-test band splits `src/alpha` into two non-adjacent runs.
const SPLIT_DIR_PATHS: &[&str] = &[
    "src/alpha/auth_helper.rs",
    "src/zeta/auth_store.rs",
    "src/alpha/auth.e2e.test.rs",
    // Filler with unrelated tokens: `auth` is only a cluster key once the
    // changeset is large enough that three carriers are not ubiquitous.
    "pkg0/unique0.rs",
    "pkg1/unique1.rs",
    "pkg2/unique2.rs",
    "pkg3/unique3.rs",
    "pkg4/unique4.rs",
    "pkg5/unique5.rs",
    "pkg6/unique6.rs",
    "pkg7/unique7.rs",
    "pkg8/unique8.rs",
    "pkg9/unique9.rs",
    "pkg10/unique10.rs",
    "pkg11/unique11.rs",
    "pkg12/unique12.rs",
];

fn split_directory_group(app: &App) -> (String, String) {
    let grouping = app.grouping.as_ref().expect("grouping computed");
    for group in grouping.groups() {
        let dirs: Vec<String> = grouping
            .files_in(&group.id)
            .map(|path| {
                path.parent()
                    .map(|dir| dir.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .collect();
        let mut runs: Vec<String> = Vec::new();
        for dir in dirs {
            if runs.last() == Some(&dir) {
                continue;
            }
            if runs.contains(&dir) {
                return (group.id.as_str().to_string(), dir);
            }
            runs.push(dir);
        }
    }
    panic!("fixture no longer splits a directory inside a group");
}

#[test]
fn a_directory_split_into_two_runs_inside_one_group_keeps_every_row() {
    // The within-group sort bands a group by central file, then mechanical and
    // broad-test tails, so one directory can appear in two non-adjacent runs
    // inside a single group. Directory rows are emitted per run for exactly
    // this reason: `docs/SIDEBAR_MODEL.md` assumed one run per group, which
    // would swallow the second run's files (handed back as `gd-26r.31`).
    let mut app = grouped_paths(SPLIT_DIR_PATHS);
    let (group_id, split_dir) = split_directory_group(&app);

    assert_eq!(
        visible_files(&app).len(),
        SPLIT_DIR_PATHS.len(),
        "fully expanded, every file has a row"
    );

    let key = format!("{group_id}\u{1f}{split_dir}");
    let rows = app
        .build_visible_items()
        .iter()
        .filter(|item| matches!(item, FileTreeItem::Directory { path, .. } if *path == key))
        .count();
    assert_eq!(rows, 2, "the split directory is emitted once per run");

    let in_split_dir: Vec<String> = SPLIT_DIR_PATHS
        .iter()
        .filter(|path| path.starts_with(&format!("{split_dir}/")))
        .map(|path| path.to_string())
        .collect();
    assert!(in_split_dir.len() >= 2);

    app.toggle_directory(&key);
    let after = visible_files(&app);
    for path in &in_split_dir {
        assert!(
            !after.contains(path),
            "{path} is in the collapsed directory, in either run"
        );
    }
    assert_eq!(
        after.len(),
        SPLIT_DIR_PATHS.len() - in_split_dir.len(),
        "and nothing else moved"
    );
}

#[test]
fn collapsing_a_directory_in_one_group_leaves_it_open_in_another() {
    // `src/` lives in both groups here, so an unscoped `expanded_dirs` key
    // would collapse both at once.
    let app_paths = &[
        "src/auth/login.rs",
        "src/auth/session.rs",
        "src/render/paint.rs",
        "src/render/canvas.rs",
    ];
    let mut app = grouped_paths(app_paths);
    let ids = group_ids(&app);
    assert!(
        ids.len() >= 2,
        "the fixture must span two groups to be a test"
    );

    let dir_keys: Vec<String> = app
        .build_visible_items()
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::Directory { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    let scoped = dir_keys
        .iter()
        .find(|key| key.starts_with(&format!("{}\u{1f}", ids[0])))
        .expect("directory keys are group-scoped")
        .clone();

    let before = visible_files(&app).len();
    app.toggle_directory(&scoped);
    let after = visible_files(&app);

    assert!(after.len() < before, "the collapse hid something");
    let other_group_files: Vec<String> = {
        let grouping = app.grouping.as_ref().unwrap();
        let id = crate::grouping::GroupId::from_persisted(ids[1].clone());
        grouping
            .files_in(&id)
            .map(|path| path.to_string_lossy().to_string())
            .collect()
    };
    for path in other_group_files {
        assert!(
            after.contains(&path),
            "{path} is in another group and its directory is still open"
        );
    }
}

#[test]
fn a_session_reopened_shows_yesterdays_groups() {
    let app = grouped_paths(PATHS);
    let before: Vec<String> = app
        .grouping
        .as_ref()
        .unwrap()
        .groups()
        .iter()
        .map(|group| format!("{}:{}", group.id.as_str(), group.name))
        .collect();

    let json = serde_json::to_string(&app.session).expect("session serializes");
    let session: ReviewSession = serde_json::from_str(&json).expect("session parses");
    assert_eq!(session.groups.len(), before.len());
    assert!(
        session
            .files
            .values()
            .all(|review| review.group_id.is_some()),
        "every file carries the group it was placed in"
    );

    let changeset = crate::grouping::changeset::Changeset::from_diff_files(&app.diff_files);
    let restored = session
        .grouping_for(&changeset)
        .expect("the table still describes this changeset");
    let after: Vec<String> = restored
        .groups()
        .iter()
        .map(|group| format!("{}:{}", group.id.as_str(), group.name))
        .collect();
    assert_eq!(after, before, "identities and order survive the round trip");
}

#[test]
fn a_changed_changeset_falls_back_to_a_fresh_pass() {
    let app = grouped_paths(PATHS);
    let mut files: Vec<DiffFile> = app.diff_files.clone();
    files.push(make_file("src/auth/refresh.rs"));
    let changeset = crate::grouping::changeset::Changeset::from_diff_files(&files);

    assert!(
        app.session.grouping_for(&changeset).is_none(),
        "a table that does not cover the changeset is not a grouping of it"
    );
}

#[test]
fn a_session_saved_before_grouping_existed_still_parses() {
    let json = r#"{
        "id": "old",
        "version": "1.3",
        "repo_path": "/tmp",
        "base_commit": "head",
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "files": {
            "src/main.rs": {
                "path": "src/main.rs",
                "reviewed": true,
                "status": "modified",
                "file_comments": [],
                "line_comments": {}
            }
        },
        "session_notes": null
    }"#;

    let session: ReviewSession = serde_json::from_str(json).expect("legacy session parses");
    assert!(session.groups.is_empty());
    assert!(
        session
            .files
            .values()
            .all(|review| review.group_id.is_none()),
        "no group is invented for a session that never had one"
    );
}
