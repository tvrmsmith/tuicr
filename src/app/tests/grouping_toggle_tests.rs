//! `<leader>g` and `:set groups!`: switching a live session between the
//! grouped list and the plain directory tree.
//!
//! The question these tests exist for is the two key spaces. `expanded_dirs`
//! held directory paths and group ids at different times in a session's life
//! and was safe because the two were never populated at once
//! (`docs/SIDEBAR_MODEL.md` point 3) — and a toggle is the first thing in tuicr
//! that makes one session hold both. The answer this slice took is to split
//! them: `expanded_dirs` for the tree, `expanded_groups` for the groups, and a
//! toggle touches neither. So every test below toggles at least twice, and the
//! arrangement a reader made in one sidebar has to survive a trip through the
//! other.

use std::collections::HashSet;
use std::path::PathBuf;

use super::grouping_tests::{
    PATHS, commit_message_file, file_order, grouped_app, grouped_paths,
    grouped_paths_with_commit_message, make_file, ungrouped_app, visible_files,
};
use crate::app::{App, FileTreeItem};

fn group_row_ids(app: &App) -> Vec<String> {
    app.build_visible_items()
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::Group { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn directory_row_paths(app: &App) -> Vec<String> {
    app.build_visible_items()
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::Directory { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn selected_row(app: &App) -> FileTreeItem {
    let items = app.build_visible_items();
    items
        .get(app.file_list_state.selected())
        .unwrap_or_else(|| {
            panic!(
                "selection {} is off the end",
                app.file_list_state.selected()
            )
        })
        .clone()
}

fn current_path(app: &App) -> String {
    app.diff_files[app.diff_state.current_file_idx]
        .display_path()
        .to_string_lossy()
        .to_string()
}

fn sorted(keys: &HashSet<String>) -> Vec<String> {
    let mut keys: Vec<String> = keys.iter().cloned().collect();
    keys.sort();
    keys
}

/// Everything a reader could tell apart: the order of the review, both key
/// spaces, the rows on screen, and where both cursors are.
#[derive(Debug, PartialEq, Eq)]
struct Session {
    order: Vec<String>,
    expanded_dirs: Vec<String>,
    expanded_groups: Vec<String>,
    rows: Vec<FileTreeItem>,
    selected: usize,
    current_file: usize,
}

fn snapshot(app: &App) -> Session {
    Session {
        order: file_order(app),
        expanded_dirs: sorted(&app.expanded_dirs),
        expanded_groups: sorted(&app.expanded_groups),
        rows: app.build_visible_items(),
        selected: app.file_list_state.selected(),
        current_file: app.diff_state.current_file_idx,
    }
}

/// Puts the reader on a file row rather than the overview, which is the state
/// every "selection survives" claim below is made about.
fn put_reader_on(app: &mut App, path: &str) {
    let idx = app
        .diff_files
        .iter()
        .position(|file| file.display_path().to_str() == Some(path))
        .expect("the review holds the file");
    app.jump_to_file(idx);
}

/// The id of the group holding `path`, which only exists while grouping is on.
fn group_of(app: &App, path: &str) -> String {
    app.group_key_of_file(&PathBuf::from(path))
        .expect("the file is in the partition")
}

#[test]
fn toggling_off_replaces_the_group_rows_with_the_directory_tree() {
    let mut app = grouped_paths(PATHS);
    assert!(!group_row_ids(&app).is_empty(), "it starts grouped");
    assert!(directory_row_paths(&app).is_empty(), "and has no dir rows");

    app.toggle_grouping();

    assert!(group_row_ids(&app).is_empty(), "no group row is left");
    assert!(
        !directory_row_paths(&app).is_empty(),
        "the plain directory tree is back"
    );
    let mut shown = visible_files(&app);
    shown.sort();
    let mut expected: Vec<String> = PATHS.iter().map(|path| path.to_string()).collect();
    expected.sort();
    assert_eq!(shown, expected, "no file was lost in the switch");
    assert_eq!(file_order(&app), file_order(&never_grouped()));
}

/// The same paths through an app that was never grouped, which is what the
/// toggled-off sidebar has to be indistinguishable from.
fn never_grouped() -> App {
    let mut app = ungrouped_app(PATHS.iter().map(|path| make_file(path)).collect());
    app.expand_all_dirs();
    app
}

#[test]
fn toggling_off_keeps_the_grouping_so_toggling_back_never_regroups() {
    let mut app = grouped_paths(PATHS);
    let before: Vec<(String, String)> = identities(&app);

    app.toggle_grouping();
    assert!(
        app.grouping.is_some(),
        "the grouping is kept while the tree is on screen \
         (`docs/REGROUPING_STATE.md`: reopening never regroups)"
    );
    let order_off = file_order(&app);

    app.toggle_grouping();

    // A fresh pass would mint fresh ids, so identical ids are the proof that
    // no pass ran — not merely that the same shape came back.
    assert_eq!(identities(&app), before, "the very same groups came back");
    assert_ne!(file_order(&app), order_off, "and the group order with them");

    // And again, in case the first round-trip happened to be the cheap one.
    app.toggle_grouping();
    app.toggle_grouping();
    assert_eq!(identities(&app), before);
}

fn identities(app: &App) -> Vec<(String, String)> {
    app.grouping
        .as_ref()
        .map(|grouping| {
            grouping
                .groups()
                .iter()
                .map(|group| (group.id.as_str().to_string(), group.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn toggling_twice_leaves_a_session_indistinguishable_from_never_having_toggled() {
    let mut app = grouped_paths(PATHS);
    // A reader who has arranged the sidebar, not a freshly opened one: an
    // arrangement is the only thing a reseeding toggle could quietly destroy.
    let collapsed = group_of(&app, "docs/auth.md");
    app.toggle_directory(&collapsed);
    // Collapsing parks the sidebar cursor on the row it collapsed, so the
    // reader is put back on their file afterwards. A toggle re-derives sidebar
    // selection from the current file, as every reorder does
    // (`ensure_valid_tree_selection`), so a cursor parked away from that file
    // is the one thing it cannot promise to leave alone.
    put_reader_on(&mut app, "src/auth/token.rs");
    let before = snapshot(&app);

    app.toggle_grouping();
    app.toggle_grouping();

    assert_eq!(snapshot(&app), before, "one round trip changed nothing");

    app.toggle_grouping();
    app.toggle_grouping();

    assert_eq!(snapshot(&app), before, "and neither did a second");
}

#[test]
fn each_sidebar_keeps_the_arrangement_its_own_reader_made() {
    let mut app = grouped_paths(PATHS);
    let collapsed_group = group_of(&app, "src/auth/token.rs");
    app.toggle_directory(&collapsed_group);

    app.toggle_grouping();
    let collapsed_dir = directory_row_paths(&app)
        .into_iter()
        .find(|path| path == "src/auth")
        .expect("the tree has the directory the files live in");
    app.toggle_directory(&collapsed_dir);
    assert!(
        !visible_files(&app).contains(&"src/auth/token.rs".to_string()),
        "the collapsed directory hides its files"
    );

    app.toggle_grouping();
    assert!(
        !app.expanded_groups.contains(&collapsed_group),
        "the group the reader collapsed is still collapsed"
    );
    assert!(
        app.expanded_groups.len() + 1 == identities(&app).len(),
        "and every other group is still open"
    );

    app.toggle_grouping();
    assert!(
        !app.expanded_dirs.contains(&collapsed_dir),
        "and the directory they collapsed is still collapsed too"
    );
}

#[test]
fn the_file_under_the_cursor_keeps_its_row_in_both_directions() {
    let mut app = grouped_paths(PATHS);
    put_reader_on(&mut app, "tests/auth_test.rs");
    assert!(matches!(selected_row(&app), FileTreeItem::File { .. }));

    app.toggle_grouping();

    assert_eq!(current_path(&app), "tests/auth_test.rs");
    match selected_row(&app) {
        FileTreeItem::File { file_idx, .. } => assert_eq!(
            app.diff_files[file_idx].display_path().to_str(),
            Some("tests/auth_test.rs")
        ),
        other => panic!("selection landed on {other:?}, not the file's row"),
    }

    app.toggle_grouping();

    assert_eq!(current_path(&app), "tests/auth_test.rs");
    match selected_row(&app) {
        FileTreeItem::File { file_idx, .. } => assert_eq!(
            app.diff_files[file_idx].display_path().to_str(),
            Some("tests/auth_test.rs")
        ),
        other => panic!("selection landed on {other:?}, not the file's row"),
    }
}

#[test]
fn a_cursor_whose_row_is_hidden_falls_back_the_way_each_sidebar_requires() {
    let mut app = grouped_paths(PATHS);
    put_reader_on(&mut app, "src/auth/token.rs");
    let group = group_of(&app, "src/auth/token.rs");
    app.toggle_directory(&group);
    assert!(
        matches!(selected_row(&app), FileTreeItem::Group { ref id, .. } if *id == group),
        "collapsing parks the cursor on the group row"
    );

    app.toggle_grouping();

    // The tree hides nothing here, so the fallback is not needed and the file
    // gets its row back.
    assert_eq!(current_path(&app), "src/auth/token.rs");
    assert!(matches!(selected_row(&app), FileTreeItem::File { .. }));

    // Now hide it in the tree instead, and toggle back into the sidebar that
    // still has the group collapsed: each side falls back to its own container.
    app.toggle_directory("src/auth");
    assert!(
        !visible_files(&app).contains(&"src/auth/token.rs".to_string()),
        "the tree now hides the file the reader is on"
    );

    app.toggle_grouping();

    assert_eq!(current_path(&app), "src/auth/token.rs");
    assert!(
        matches!(selected_row(&app), FileTreeItem::Group { ref id, .. } if *id == group),
        "and the grouped sidebar parks it back on the group row"
    );
}

#[test]
fn toggling_with_the_cursor_on_the_commit_message_row_keeps_it_there() {
    let mut app = grouped_paths_with_commit_message(PATHS);
    put_reader_on(&mut app, "COMMIT_MSG");
    assert!(
        app.diff_files[0].is_commit_message,
        "the pseudo-file is pinned at index 0 grouped"
    );
    assert!(
        app.group_key_of_file(&PathBuf::from("COMMIT_MSG"))
            .is_none()
    );

    for direction in ["off", "on"] {
        app.toggle_grouping();
        assert!(
            app.diff_files[0].is_commit_message,
            "the pseudo-file stays pinned at index 0 with grouping {direction}"
        );
        assert_eq!(
            current_path(&app),
            "COMMIT_MSG",
            "and the reader stays on it, though it is in no group at all"
        );
        match selected_row(&app) {
            FileTreeItem::File { file_idx, .. } => {
                assert!(app.diff_files[file_idx].is_commit_message)
            }
            other => panic!("selection landed on {other:?}, not the pseudo-file's row"),
        }
    }
}

#[test]
fn toggling_on_a_review_that_was_never_grouped_computes_the_grouping_and_opens_it() {
    // `--no-grouping`, or any startup that never called `enable_grouping`: the
    // toggle is the first thing that asks for a grouping at all.
    let mut app = never_grouped();
    assert!(app.grouping.is_none() && !app.grouping_enabled);
    assert!(
        app.expanded_groups.is_empty(),
        "nothing to seed it from yet"
    );

    app.toggle_grouping();

    let ids: HashSet<String> = app
        .grouping
        .as_ref()
        .expect("the toggle computed one")
        .groups()
        .iter()
        .map(App::group_row_key)
        .collect();
    assert_eq!(
        app.expanded_groups, ids,
        "a grouping with no sidebar state yet opens expanded, as startup seeds it"
    );
    let mut shown = visible_files(&app);
    shown.sort();
    let mut expected: Vec<String> = PATHS.iter().map(|path| path.to_string()).collect();
    expected.sort();
    assert_eq!(shown, expected);

    // And it is a grouping like any other from here: toggling off and back
    // keeps it rather than computing a second one.
    let before = identities(&app);
    app.toggle_grouping();
    app.toggle_grouping();
    assert_eq!(identities(&app), before);
}

#[test]
fn a_review_with_nothing_to_group_toggles_without_inventing_a_sidebar() {
    // The commit-message pseudo-file sits outside the partition, so a review of
    // nothing else is a review with an empty changeset.
    let mut app = grouped_app(vec![commit_message_file()]);
    assert!(app.grouping.is_none(), "there was nothing to group");

    app.toggle_grouping();
    assert!(!app.grouping_enabled);
    assert_eq!(visible_files(&app), vec!["COMMIT_MSG".to_string()]);

    app.toggle_grouping();
    assert!(app.grouping_enabled);
    assert!(app.grouping.is_none());
    assert_eq!(
        app.message.as_ref().expect("a message").content,
        "Grouping: on (nothing to group)"
    );
    assert_eq!(visible_files(&app), vec!["COMMIT_MSG".to_string()]);
}

#[test]
fn the_set_groups_command_is_the_same_toggle() {
    let mut app = grouped_paths(PATHS);
    let grouped = snapshot(&app);

    set_groups(&mut app);
    assert!(!app.grouping_enabled);
    assert!(group_row_ids(&app).is_empty());
    assert_eq!(
        app.message.as_ref().expect("a message").content,
        "Grouping: off"
    );

    set_groups(&mut app);
    assert!(app.grouping_enabled);
    assert_eq!(snapshot(&app), grouped, "and back to exactly where it was");
    assert_eq!(
        app.message.as_ref().expect("a message").content,
        format!("Grouping: on ({} groups)", identities(&app).len())
    );
}

fn set_groups(app: &mut App) {
    app.enter_command_mode();
    app.command_buffer = "set groups!".to_string();
    crate::handler::handle_command_action(app, crate::input::Action::SubmitInput);
    assert_eq!(app.input_mode, crate::app::InputMode::Normal);
}
