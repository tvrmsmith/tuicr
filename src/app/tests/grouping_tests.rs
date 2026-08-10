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

#[test]
fn a_group_lists_its_files_flat_at_depth_one_by_full_path() {
    // `docs/SIDEBAR_MODEL.md`: inside a group there are no directory rows at
    // all, whatever the tree mode says. A group's members sit directly beneath
    // it, each labelled with its full relative path.
    for mode in [
        FileTreeMode::Nested,
        FileTreeMode::Compact,
        FileTreeMode::Flat,
    ] {
        let mut app = grouped_paths(PATHS);
        app.file_tree_mode = mode;
        let items = app.build_visible_items();

        assert!(
            !items
                .iter()
                .any(|item| matches!(item, FileTreeItem::Directory { .. })),
            "{mode:?} emitted a directory row under grouping"
        );
        for item in &items {
            let FileTreeItem::File {
                file_idx,
                label,
                depth,
            } = item
            else {
                continue;
            };
            let file = &app.diff_files[*file_idx];
            let path = file.display_path().to_string_lossy().to_string();
            assert_eq!(label, &path, "{mode:?} labelled {path} by file name");
            // The commit-message pseudo-file is outside the partition and
            // pinned above every group, so it alone sits at depth 0.
            let expected = usize::from(!file.is_commit_message);
            assert_eq!(*depth, expected, "{mode:?}: {path}");
        }
        assert_eq!(visible_files(&app).len(), PATHS.len(), "{mode:?}");
    }
}

#[test]
fn expanding_seeds_group_ids_and_nothing_else() {
    // `expanded_dirs` holds group ids only while grouping is on, which is what
    // lets one flat string set serve both sidebars without colliding.
    let app = grouped_paths(PATHS);
    let ids: HashSet<String> = app
        .grouping
        .as_ref()
        .expect("grouping computed")
        .groups()
        .iter()
        .map(|group| group.id.as_str().to_string())
        .collect();

    assert_eq!(app.expanded_dirs, ids);
}

#[test]
fn jumping_to_a_hidden_file_reveals_it_by_opening_its_group_alone() {
    let mut app = grouped_paths(PATHS);
    app.collapse_all_dirs();
    assert!(visible_files(&app).is_empty(), "everything starts hidden");

    let target = app
        .diff_files
        .iter()
        .position(|file| !file.is_commit_message)
        .expect("a real file");
    let path = app.diff_files[target]
        .display_path()
        .to_string_lossy()
        .to_string();
    let group_id = app
        .group_of_file(app.diff_files[target].display_path())
        .expect("assigned to a group")
        .id
        .as_str()
        .to_string();

    app.jump_to_file(target);

    assert!(visible_files(&app).contains(&path));
    assert_eq!(
        app.expanded_dirs,
        HashSet::from([group_id]),
        "the group id is the only key that reveals anything"
    );
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
