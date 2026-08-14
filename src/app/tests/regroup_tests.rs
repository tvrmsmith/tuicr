//! Slice C end to end: `:regroup` landing under a reader, and the incremental
//! assignment a reload makes when the file set has moved.
//!
//! Every test here asserts two things beyond whatever it is about: selection is
//! still on a real row, and no file left the sidebar. Those are the two ways a
//! regroup can quietly ruin a review — the cursor pointing at a row that no
//! longer exists, and a file that is in the diff but in no group and therefore
//! on no screen.

use std::sync::Arc;
use std::time::Duration;

use super::grouping_tests::{grouped_app, make_file, visible_files};
use crate::app::App;
use crate::app::FileTreeItem;
use crate::app::refine::{RefineCall, RefineConfig};
use crate::grouping::GroupSource;

const PATHS: &[&str] = &[
    "src/auth/login.rs",
    "src/auth/session.rs",
    "src/auth/token.rs",
    "tests/auth_test.rs",
    "docs/auth.md",
    "Cargo.lock",
];

/// A well-formed answer over `PATHS` whose group names are nothing like the
/// heuristic ones, so "the group the reader was in did not survive" is a state
/// a test can actually reach.
const ANSWER: &str = r#"{"groups": [
    {"name": "auth-token-rotation", "files": [
        "src/auth/token.rs", "tests/auth_test.rs", "docs/auth.md"]},
    {"name": "session-plumbing", "files": [
        "src/auth/login.rs", "src/auth/session.rs", "Cargo.lock"]}
]}"#;

fn app() -> App {
    grouped_app(PATHS.iter().map(|path| make_file(path)).collect())
}

/// An app that would refine if a call were handed to it. The settings are never
/// read: every test supplies its own call.
fn refining_app(timeout: Duration) -> App {
    let mut app = app();
    app.refine_config = Some(RefineConfig {
        timeout,
        settings: crate::grouping::vertex::Settings::default(),
    });
    app
}

fn answering(body: &'static str) -> RefineCall {
    Arc::new(move |_prompt: &str, _timeout: Duration| Ok(body.to_string()))
}

fn refusing() -> RefineCall {
    Arc::new(|_prompt: &str, _timeout: Duration| Err("vertex said no".to_string()))
}

/// A call that never answers, which is what a timeout waits through.
fn silent() -> RefineCall {
    Arc::new(|_prompt: &str, timeout: Duration| {
        std::thread::sleep(timeout);
        Err("unreachable in these tests".to_string())
    })
}

/// Polls until the worker has answered, or gives up long before a hung test
/// would hold the suite.
fn drain(app: &mut App) {
    for _ in 0..2000 {
        if app.pending_regroup.is_none() {
            return;
        }
        app.poll_regroup();
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the regroup never finished");
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

/// The row the sidebar cursor is on, which must always exist.
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

/// The landing policy's whole promise, asserted the same way everywhere: the
/// cursor is on the collapsed group row of the file the reader was reading.
fn assert_landed_on_group_of_current_file(app: &App) {
    let current = app.diff_files[app.diff_state.current_file_idx]
        .display_path()
        .clone();
    let expected = app
        .group_key_of_file(&current)
        .expect("the current file is in the partition");
    match selected_row(app) {
        FileTreeItem::Group { id, .. } => assert_eq!(id, expected),
        other => panic!("selection landed on {other:?}, not the group row"),
    }
    assert!(
        app.expanded_dirs.is_empty(),
        "every group collapses on a landing"
    );
}

fn put_reader_inside(app: &mut App, path: &str) {
    let idx = app
        .diff_files
        .iter()
        .position(|file| file.display_path().to_str() == Some(path))
        .expect("the review holds the file");
    // Through `jump_to_file`, not by setting the index: the cursor has to leave
    // the overview or the reorder has no current file to re-find by path.
    app.jump_to_file(idx);
    assert!(
        matches!(selected_row(app), FileTreeItem::File { .. }),
        "the reader starts on a file row inside an expanded group"
    );
}

/// The reader is inside a group the new grouping still produces. They come out
/// on its row, collapsed, with the diff pane still on their file.
#[test]
fn a_regroup_under_a_reader_in_a_surviving_group_lands_on_that_group() {
    let mut app = app();
    put_reader_inside(&mut app, "src/auth/token.rs");
    let before = app.group_key_of_file(&std::path::PathBuf::from("src/auth/token.rs"));

    app.regroup_with(None);

    assert_eq!(
        app.diff_files[app.diff_state.current_file_idx]
            .display_path()
            .to_str(),
        Some("src/auth/token.rs"),
        "the diff pane is undisturbed: the same file, re-found by path"
    );
    // A fresh pass mints fresh ids, so the group is the same *group* and not
    // the same identity — which is exactly why nothing may be remapped.
    assert_ne!(
        app.group_key_of_file(&std::path::PathBuf::from("src/auth/token.rs")),
        before
    );
    assert_landed_on_group_of_current_file(&app);
    assert_eq!(visible_files_after_expanding(&mut app).len(), PATHS.len());
}

/// The reader is inside a group the refined grouping dissolves. Same promise:
/// the cursor is on the row of whatever group now holds their file, and no file
/// was lost in the reshuffle.
#[test]
fn a_regroup_under_a_reader_whose_group_dissolves_lands_on_the_new_one() {
    let mut app = refining_app(Duration::from_secs(30));
    put_reader_inside(&mut app, "docs/auth.md");
    let dissolved = app
        .group_key_of_file(&std::path::PathBuf::from("docs/auth.md"))
        .expect("grouped");

    app.regroup_with(Some(answering(ANSWER)));
    drain(&mut app);

    let names: Vec<String> = app
        .grouping
        .as_ref()
        .expect("grouped")
        .groups()
        .iter()
        .map(|group| group.name.clone())
        .collect();
    assert_eq!(names, vec!["auth-token-rotation", "session-plumbing"]);
    assert!(
        !group_ids_of(&app).contains(&dissolved),
        "the group the reader was in is simply gone — no remapping, no fixup"
    );
    assert_landed_on_group_of_current_file(&app);
    assert_eq!(visible_files_after_expanding(&mut app).len(), PATHS.len());
}

/// A refine arm that refuses still leaves the review regrouped: the heuristic
/// pass computed at dispatch lands instead.
#[test]
fn a_regroup_whose_refine_arm_fails_lands_the_heuristic_pass() {
    let mut app = refining_app(Duration::from_secs(30));
    put_reader_inside(&mut app, "src/auth/login.rs");

    app.regroup_with(Some(refusing()));
    drain(&mut app);

    let grouping = app.grouping.as_ref().expect("grouped");
    assert!(
        grouping
            .groups()
            .iter()
            .all(|group| group.source == GroupSource::Heuristics),
        "the fallback is a full heuristic pass, not a half-applied answer"
    );
    assert_landed_on_group_of_current_file(&app);
    assert_eq!(visible_files_after_expanding(&mut app).len(), PATHS.len());
}

/// And so does one that never answers.
#[test]
fn a_regroup_whose_refine_arm_times_out_lands_the_heuristic_pass() {
    let mut app = refining_app(Duration::from_millis(0));
    put_reader_inside(&mut app, "tests/auth_test.rs");

    app.regroup_with(Some(silent()));
    assert!(app.pending_regroup.is_some(), "the call went out");
    drain(&mut app);

    assert!(app.grouping.is_some());
    assert_landed_on_group_of_current_file(&app);
    assert_eq!(visible_files_after_expanding(&mut app).len(), PATHS.len());
}

/// An answer that arrives after the review moved on is dropped whole. Landing
/// it would partition paths that are no longer on screen.
#[test]
fn an_answer_that_arrives_after_the_review_moved_is_discarded() {
    let mut app = refining_app(Duration::from_secs(30));
    app.regroup_with(Some(answering(ANSWER)));

    app.diff_files.push(make_file("src/auth/refresh.rs"));
    let before: Vec<String> = group_ids_of(&app);
    drain(&mut app);

    assert_eq!(
        group_ids_of(&app),
        before,
        "the grouping on screen was left exactly as it was"
    );
}

/// A file that appears after the last full pass is placed without renaming or
/// renumbering anything the reader has been working through.
#[test]
fn a_file_appearing_after_the_last_pass_leaves_the_groups_alone() {
    let mut app = app();
    let before = identities(&app);

    let arrival = make_file("src/auth/refresh.rs");
    app.session
        .add_file(arrival.display_path().clone(), arrival.status, 0);
    app.diff_files.push(arrival);
    app.sort_files_by_directory(true);

    assert_eq!(
        identities(&app)[..before.len()],
        before[..],
        "every group the reader knows keeps its id, name and slot"
    );
    assert_eq!(
        app.group_key_of_file(&std::path::PathBuf::from("src/auth/refresh.rs")),
        app.group_key_of_file(&std::path::PathBuf::from("src/auth/login.rs")),
        "and the arrival joined the group its neighbours are in"
    );
    assert_eq!(
        visible_files_after_expanding(&mut app).len(),
        PATHS.len() + 1
    );
}

/// A file whose content changed is released and re-placed, and it can land in a
/// different group without either group losing its identity.
#[test]
fn a_file_whose_content_changed_can_move_between_groups() {
    let mut app = refining_app(Duration::from_secs(30));
    // A refined partition puts `Cargo.lock` in a group literally named `docs`
    // and `docs/auth.md` somewhere else, so the passes' own claim for the
    // changed file names a group that already exists and is not its own.
    app.regroup_with(Some(answering(
        r#"{"groups": [
            {"name": "docs", "files": ["Cargo.lock"]},
            {"name": "auth", "files": [
                "src/auth/login.rs", "src/auth/session.rs", "src/auth/token.rs",
                "tests/auth_test.rs", "docs/auth.md"]}
        ]}"#,
    )));
    drain(&mut app);

    let docs_group = app
        .grouping
        .as_ref()
        .expect("grouped")
        .groups()
        .iter()
        .find(|group| group.name == "docs")
        .expect("the refined answer named one")
        .id
        .as_str()
        .to_string();
    let was = app
        .group_key_of_file(&std::path::PathBuf::from("docs/auth.md"))
        .expect("grouped");
    assert_ne!(was, docs_group, "it starts in the other group");

    // The reload the human would drive: the same path with different bytes.
    let changed = app
        .diff_files
        .iter_mut()
        .find(|file| file.display_path().to_str() == Some("docs/auth.md"))
        .expect("still there");
    changed.content_hash = 99;
    let (path, status) = (changed.display_path().clone(), changed.status);
    app.session.add_file(path, status, 99);
    app.sort_files_by_directory(true);

    assert_eq!(
        app.group_key_of_file(&std::path::PathBuf::from("docs/auth.md"))
            .as_deref(),
        Some(docs_group.as_str()),
        "the changed file moved to the group whose name its claim carries"
    );
    assert_eq!(
        identities(&app).len(),
        2,
        "and both groups kept their identity: nothing was opened or dropped"
    );
    assert_eq!(visible_files_after_expanding(&mut app).len(), PATHS.len());
}

/// The brief's standing question, asked rather than assumed: after a regroup,
/// group ids are still the only keys `expanded_dirs` holds.
#[test]
fn expanded_dirs_holds_group_ids_and_nothing_else_after_a_regroup() {
    let mut app = app();
    put_reader_inside(&mut app, "src/auth/session.rs");

    app.regroup_with(None);
    assert!(app.expanded_dirs.is_empty(), "the landing collapsed it");

    app.expand_all_dirs();
    let live = group_ids_of(&app);
    let mut keys: Vec<String> = app.expanded_dirs.iter().cloned().collect();
    keys.sort();
    let mut ids = live;
    ids.sort();
    assert_eq!(keys, ids, "one key per group and no directory paths at all");
}

fn group_ids_of(app: &App) -> Vec<String> {
    app.grouping
        .as_ref()
        .map(|grouping| {
            grouping
                .groups()
                .iter()
                .map(|group| group.id.as_str().to_string())
                .collect()
        })
        .unwrap_or_default()
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

/// Every file the sidebar can reach. A regroup that lost a row fails here even
/// when the partition itself is intact, which is the failure the reader sees.
fn visible_files_after_expanding(app: &mut App) -> Vec<String> {
    app.expand_all_dirs();
    let files = visible_files(app);
    assert_eq!(
        group_ids(app).len(),
        group_ids_of(app).len(),
        "every group in the partition has a row"
    );
    files
}
