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

/// The same changeset with the commit-message pseudo-file in it, which is the
/// only row that lives outside the partition.
fn grouped_paths_with_commit_message(paths: &[&str]) -> App {
    let mut files: Vec<DiffFile> = paths.iter().map(|path| make_file(path)).collect();
    files.push(commit_message_file());
    grouped_app(files)
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
        let mut app = grouped_paths_with_commit_message(PATHS);
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
        assert!(
            app.diff_files.iter().any(|file| file.is_commit_message),
            "the depth-0 case only runs with a pseudo-file present"
        );
        assert_eq!(visible_files(&app).len(), PATHS.len() + 1, "{mode:?}");
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
fn the_commit_message_row_survives_collapsing_every_group() {
    // It is emitted before the group loop precisely so no collapse can reach
    // it (`docs/TOTAL_COVERAGE.md`): the review always has a way back to the
    // message, however the groups are folded.
    let mut app = grouped_paths_with_commit_message(PATHS);
    app.collapse_all_dirs();

    assert_eq!(
        visible_files(&app),
        vec!["COMMIT_MSG".to_string()],
        "only the pseudo-file survives a full collapse"
    );
}

#[test]
fn collapsing_the_group_holding_the_current_file_parks_the_cursor_on_its_group_row() {
    let mut app = grouped_paths(PATHS);
    // Not the first group: row 0 is what the `select(0)` fallback produces, so
    // a target there would pass whether the grouped arm ran or not.
    let first_group = app
        .grouping
        .as_ref()
        .expect("grouping computed")
        .groups()
        .first()
        .expect("at least one group")
        .id
        .as_str()
        .to_string();
    let target = (0..app.diff_files.len())
        .find(|&idx| {
            let file = &app.diff_files[idx];
            !file.is_commit_message
                && app
                    .group_of_file(file.display_path())
                    .is_some_and(|group| group.id.as_str() != first_group)
        })
        .expect("a file outside the first group");
    app.jump_to_file(target);
    let group_id = app
        .group_of_file(app.diff_files[target].display_path())
        .expect("assigned to a group")
        .id
        .as_str()
        .to_string();

    app.collapse_all_dirs();

    let expected = app
        .build_visible_items()
        .iter()
        .position(|item| matches!(item, FileTreeItem::Group { id, .. } if *id == group_id))
        .expect("the group row is still on screen");
    assert!(
        expected > 0,
        "the target's group row must not be row 0, or the fallback satisfies this test"
    );
    assert_eq!(
        app.file_list_state.selected(),
        expected,
        "the cursor follows the hidden file up to its group row, not to row 0"
    );
}

#[test]
fn searching_the_tree_onto_a_file_inside_a_collapsed_group_moves_the_cursor() {
    // `/` + Enter reveals through whatever hides the match. Under grouping
    // that is the group row, and walking path ancestors instead would expand
    // keys no grouped row answers to, leaving the cursor parked while the
    // status line claimed a hit.
    let mut app = grouped_paths(PATHS);
    app.collapse_all_dirs();
    assert!(visible_files(&app).is_empty(), "everything starts hidden");

    app.begin_file_tree_prompt(FileTreePrompt::Search);
    for ch in "auth.md".chars() {
        app.file_tree_prompt_insert_char(ch);
    }
    app.commit_file_tree_prompt();

    assert!(visible_files(&app).contains(&"docs/auth.md".to_string()));
    let selected = app
        .build_visible_items()
        .get(app.file_list_state.selected())
        .cloned();
    assert!(
        matches!(
            selected,
            Some(FileTreeItem::File { file_idx, .. })
                if app.diff_files[file_idx].display_path() == Path::new("docs/auth.md")
        ),
        "the cursor sits on the match, got {selected:?}"
    );
}

/// The grouping the engine produced for `PATHS` minus `Cargo.lock`, with
/// `Cargo.lock` appended to `diff_files` afterwards. That is the shape
/// `order_files_by_group` sorts an unmentioned file into — last by index,
/// mentioned by no group — without any hand-built partition.
fn app_with_an_orphan_file() -> (App, usize) {
    let grouped: Vec<&str> = PATHS
        .iter()
        .copied()
        .filter(|path| *path != "Cargo.lock")
        .collect();
    let mut app = grouped_paths(&grouped);

    let orphan = make_file("Cargo.lock");
    app.session
        .add_file(orphan.display_path().clone(), orphan.status, 0);
    app.diff_files.push(orphan);

    (app, PATHS.len() - 1)
}

#[test]
fn a_file_the_grouping_does_not_mention_still_gets_a_row() {
    // The partition is total, so this state is only reachable by building it.
    // `order_files_by_group` deliberately sorts an unmentioned file last
    // rather than dropping it; the sidebar has to honour that or the defence
    // is cancelled and the file is lost from the review.
    let (mut app, orphan_idx) = app_with_an_orphan_file();

    let items = app.build_visible_items();
    assert!(
        matches!(
            items.last(),
            Some(FileTreeItem::File { file_idx, depth, .. })
                if *file_idx == orphan_idx && *depth == 0
        ),
        "the orphan is the last row, at top level after the groups: {:?}",
        items.last()
    );
    assert_ascending_by_file_idx(&items);

    app.jump_to_file(orphan_idx);
    let selected = app
        .build_visible_items()
        .get(app.file_list_state.selected())
        .cloned();
    assert!(
        matches!(selected, Some(FileTreeItem::File { file_idx, .. }) if file_idx == orphan_idx),
        "and the cursor can reach it, got {selected:?}"
    );
}

#[test]
fn a_filter_that_hides_the_orphan_takes_its_row_with_it() {
    let (mut app, orphan_idx) = app_with_an_orphan_file();
    assert!(visible_files(&app).contains(&"Cargo.lock".to_string()));

    exclude(&mut app, r"^Cargo\.lock$");

    assert!(
        !visible_files(&app).contains(&"Cargo.lock".to_string()),
        "the orphan row obeys the filter like every other row"
    );
    assert!(!app.build_visible_items().iter().any(
        |item| matches!(item, FileTreeItem::File { file_idx, .. } if *file_idx == orphan_idx)
    ));
}

/// Apply an exclude pattern the way the user does, through the tree prompt.
fn exclude(app: &mut App, pattern: &str) {
    app.begin_file_tree_prompt(FileTreePrompt::Exclude);
    for ch in pattern.chars() {
        app.file_tree_prompt_insert_char(ch);
    }
    app.commit_file_tree_prompt();
}

#[test]
fn a_group_whose_every_file_the_filter_hides_loses_its_row() {
    let mut app = grouped_paths_with_commit_message(PATHS);
    let (doomed, members) = {
        let grouping = app.grouping.as_ref().expect("grouping computed");
        assert!(
            grouping.groups().len() > 1,
            "a sibling group has to survive for this to say anything"
        );
        let group = &grouping.groups()[0];
        let members: Vec<String> = grouping
            .files_in(&group.id)
            .map(|path| regex::escape(&path.to_string_lossy()))
            .collect();
        (group.name.clone(), members)
    };
    let before: Vec<String> = group_rows(&app).into_iter().map(|row| row.0).collect();

    exclude(&mut app, &format!("^({})$", members.join("|")));

    let after: Vec<String> = group_rows(&app).into_iter().map(|row| row.0).collect();
    assert!(
        !after.contains(&doomed),
        "a group with nothing left to show disappears with its files, got {after:?}"
    );
    assert_eq!(
        after.len(),
        before.len() - 1,
        "and only that group: {before:?} -> {after:?}"
    );
    assert!(
        visible_files(&app).contains(&"COMMIT_MSG".to_string()),
        "the pseudo-file is outside the partition and outside the filter's reach here"
    );
}

#[test]
fn the_commit_message_row_obeys_the_filter_like_any_other() {
    let mut app = grouped_paths_with_commit_message(PATHS);
    assert!(visible_files(&app).contains(&"COMMIT_MSG".to_string()));

    exclude(&mut app, "^COMMIT_MSG$");

    assert!(
        !visible_files(&app).contains(&"COMMIT_MSG".to_string()),
        "being pinned above the groups does not exempt it from the filter"
    );
    assert!(
        !group_rows(&app).is_empty(),
        "and the rest of the sidebar is untouched"
    );
}

fn assert_ascending_by_file_idx(items: &[FileTreeItem]) {
    let mut previous: Option<usize> = None;
    for item in items {
        let FileTreeItem::File { file_idx, .. } = item else {
            continue;
        };
        if let Some(previous) = previous {
            assert!(
                *file_idx > previous,
                "row order must ascend: {file_idx} came after {previous}"
            );
        }
        previous = Some(*file_idx);
    }
    assert!(previous.is_some(), "the fixture has rows to check");
}

#[test]
fn the_visible_rows_ascend_by_file_idx() {
    // What group contiguity buys the sidebar (`docs/SIDEBAR_MODEL.md` point
    // 4): `next_file`/`prev_file` step by comparing `file_idx` against the
    // current one, so a row order that ran backwards would skip files.
    let app = grouped_paths_with_commit_message(PATHS);

    assert_ascending_by_file_idx(&app.build_visible_items());
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
