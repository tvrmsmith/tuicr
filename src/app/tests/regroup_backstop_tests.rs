//! The auto-regroup backstop (`gd-26r.36`): the full pass a drifted review
//! takes on its own, the crossings that must not fire it, and the promise that
//! it never costs anything.
//!
//! Drift is reached the way a review reaches it — files arriving into a session
//! that already holds a group table, so incremental assignment opens groups for
//! them — rather than by setting a percentage. A test that set the number by
//! hand would pass over a backstop wired to a number nothing produces.

use std::sync::Arc;
use std::time::Duration;

use super::grouping_tests::{grouped_paths, make_file};
use crate::app::App;
use crate::app::FileTreeItem;
use crate::app::grouping::GroupingStatus;
use crate::app::refine::RefineConfig;
use crate::grouping::GroupSource;

const PATHS: &[&str] = &[
    "src/auth/login.rs",
    "src/auth/session.rs",
    "src/auth/token.rs",
    "tests/auth_test.rs",
    "docs/auth.md",
    "Cargo.lock",
];

/// Arrivals with little in common with `PATHS`, so incremental assignment opens
/// groups for them instead of filing them into ranked ones. Six of these over
/// the six above is a partition that is 42% drift — five files in opened groups
/// over twelve, the sixth having found a group that already existed, which is
/// the group-derived reading of drift doing exactly what `gd-26r.37` pinned.
const ARRIVALS: &[&str] = &[
    "infra/terraform/network.tf",
    "vendor/legacy/report.cob",
    "assets/logo.svg",
    "scripts/deploy.sh",
    "proto/billing.proto",
    "config/nginx.conf",
];

/// What `ARRIVALS` comes to, asserted in the fixture rather than assumed: every
/// threshold below is set relative to it.
const DRIFTED: u32 = 42;

/// One more arrival, for the tests that need drift to move a second time.
const LATER: &[&str] = &["migrations/0007_add_index.sql"];

fn app() -> App {
    grouped_paths(PATHS)
}

/// A grouped review carrying `count` arrivals that incremental assignment
/// placed, and the backstop's own first poll already taken: the latch is what
/// keeps a review from being regrouped for drift it opened on, and every test
/// below is about a crossing *after* the review opened.
fn drifted_app() -> App {
    let mut app = app();
    app.poll_regroup_backstop();
    drift(&mut app, ARRIVALS);
    assert_eq!(
        percent(&app),
        DRIFTED,
        "the fixture the thresholds read off"
    );
    app
}

/// The reload a human drives, minus the backend: paths appear, and the session's
/// group table is carried onto the new changeset by `grouping_for`.
fn drift(app: &mut App, arrivals: &[&str]) {
    for path in arrivals {
        app.diff_files.push(make_file(path));
    }
    app.sort_files_by_directory(false);
    assert!(
        app.grouping
            .as_ref()
            .expect("grouped")
            .groups()
            .iter()
            .any(|group| group.drifted()),
        "the setup depends on the arrivals opening groups, not joining them"
    );
}

fn percent(app: &App) -> u32 {
    app.grouping.as_ref().expect("grouped").drift_percent()
}

fn identities(app: &App) -> Vec<String> {
    app.grouping
        .as_ref()
        .expect("grouped")
        .groups()
        .iter()
        .map(|group| group.id.as_str().to_string())
        .collect()
}

fn visible_files_after_expanding(app: &mut App) -> Vec<String> {
    for group in identities(app) {
        app.expanded_groups.insert(group);
    }
    super::grouping_tests::visible_files(app)
}

/// The landing policy `:regroup` and the backstop share
/// (`docs/MID_SESSION_REGROUP.md`), asserted here as well because the backstop
/// is the landing nobody asked for and is the one a reader is least ready for.
fn assert_landed_on_group_of_current_file(app: &App) {
    let current = app.diff_files[app.diff_state.current_file_idx]
        .display_path()
        .clone();
    let expected = app
        .group_key_of_file(&current)
        .expect("the current file is in the partition");
    let items = app.build_visible_items();
    match items.get(app.file_list_state.selected()) {
        Some(FileTreeItem::Group { id, .. }) => assert_eq!(id, &expected),
        other => panic!("selection landed on {other:?}, not the group row"),
    }
    assert!(
        app.expanded_groups.is_empty(),
        "every group collapses on a landing"
    );
}

fn put_reader_inside(app: &mut App, path: &str) {
    let idx = app
        .diff_files
        .iter()
        .position(|file| file.display_path().to_str() == Some(path))
        .expect("the review holds the file");
    app.jump_to_file(idx);
}

/// A review that would refine if anything asked it to. Nothing here does: the
/// backstop takes no call and cannot be handed one.
fn refining_app() -> App {
    let mut app = drifted_app();
    app.refine_config = Some(RefineConfig {
        timeout: Duration::from_secs(30),
        settings: crate::grouping::vertex::Settings::default(),
    });
    app
}

/// The crossing: drift reaches the threshold and the review regroups itself
/// through slice C's landing.
#[test]
fn drift_crossing_the_threshold_lands_a_full_regroup() {
    let mut app = drifted_app();
    app.regroup_threshold = DRIFTED as usize;
    put_reader_inside(&mut app, "src/auth/token.rs");
    let before = identities(&app);

    assert!(app.poll_regroup_backstop(), "the sidebar moved, so redraw");

    assert_eq!(
        percent(&app),
        0,
        "a full pass is the last full pass, so nothing has drifted from it"
    );
    assert!(
        identities(&app).iter().all(|id| !before.contains(id)),
        "a full pass mints fresh identities; nothing was carried over"
    );
    assert_eq!(
        app.diff_files[app.diff_state.current_file_idx]
            .display_path()
            .to_str(),
        Some("src/auth/token.rs"),
        "the diff pane is undisturbed"
    );
    assert_landed_on_group_of_current_file(&app);
    assert_eq!(
        visible_files_after_expanding(&mut app).len(),
        PATHS.len() + ARRIVALS.len(),
        "and no file left the sidebar"
    );
}

/// The non-crossing. One arrival among seven is 14%, and a backstop that fired
/// there would be the reshuffle the indicator exists to make unnecessary.
#[test]
fn drift_below_the_threshold_leaves_the_review_alone() {
    let mut app = app();
    app.poll_regroup_backstop();
    drift(&mut app, &ARRIVALS[..1]);
    app.regroup_threshold = DRIFTED as usize;
    let before = identities(&app);
    assert_eq!(percent(&app), 14, "one arrival among seven");

    assert!(!app.poll_regroup_backstop(), "nothing moved");

    assert_eq!(identities(&app), before, "the same groups, the same ids");
    assert_eq!(
        app.grouping_status(),
        Some(GroupingStatus::Drift { percent: 14 }),
        "the chip goes on counting instead"
    );
}

/// The number that fires the backstop is the number the chip renders — one
/// function, read in one unit (`gd-26r.37`). At the threshold it fires; one
/// percent above the chip it does not.
#[test]
fn the_backstop_fires_on_the_same_number_the_chip_reports() {
    let shown = match drifted_app().grouping_status() {
        Some(GroupingStatus::Drift { percent }) => percent,
        other => panic!("expected a drift chip, got {other:?}"),
    };

    let mut above = drifted_app();
    above.regroup_threshold = shown as usize + 1;
    assert!(
        !above.poll_regroup_backstop(),
        "a threshold above what the reader is watching must not fire"
    );

    let mut at = drifted_app();
    at.regroup_threshold = shown as usize;
    assert!(
        at.poll_regroup_backstop(),
        "and the value on screen is exactly the value that fires"
    );
}

/// Heuristics-only, never refine (`docs/REGROUPING_STATE.md`). The backstop has
/// to be free, instant and deterministic: a threshold crossing that silently
/// spent money mid-review is the surprise the whole grouping ticket exists to
/// prevent.
#[test]
fn the_backstop_never_spends_money_on_a_refine_call() {
    let mut app = refining_app();
    app.regroup_threshold = DRIFTED as usize;

    assert!(app.poll_regroup_backstop());

    // The only spend is a dispatched call, and a dispatched call is a
    // `pending_regroup` waiting to be collected. There is none: the backstop
    // takes no `RefineCall` and reaches no constructor for one.
    assert!(
        app.pending_regroup.is_none(),
        "no call went out, so there is nothing to collect"
    );
    assert!(!app.poll_regroup(), "and nothing is in flight to answer");
    let grouping = app.grouping.as_ref().expect("grouped");
    assert!(
        grouping
            .groups()
            .iter()
            .all(|group| group.source == GroupSource::Heuristics),
        "the landing is a heuristic pass, whatever [grouping].refine says"
    );
    assert_eq!(
        app.grouping_status(),
        Some(GroupingStatus::HeuristicsOnly),
        "and the header says so rather than implying the grouping was refined"
    );
}

/// A `:regroup` the human asked for is already bringing the full pass this
/// would land. Firing underneath it would reshuffle the sidebar twice for one
/// command, and the second reshuffle is the requested one.
#[test]
fn nothing_fires_while_a_regroup_is_in_flight() {
    let mut app = refining_app();
    app.regroup_threshold = DRIFTED as usize;
    app.regroup_with(Some(Arc::new(|_prompt: &str, _timeout: Duration| {
        std::thread::sleep(Duration::from_secs(60));
        Err("unreachable in this test".to_string())
    })));
    assert!(app.pending_regroup.is_some(), "the call went out");
    let before = identities(&app);

    assert!(!app.poll_regroup_backstop());

    assert_eq!(identities(&app), before);
}

/// Chrome and landings alike belong to the grouped view. With `<leader>g` off
/// the reader is on the plain file tree, the chip is silent, and a sidebar that
/// rearranged itself out of sight is one they cannot account for on the way
/// back.
#[test]
fn nothing_fires_while_the_grouped_sidebar_is_toggled_off() {
    let mut app = drifted_app();
    app.regroup_threshold = DRIFTED as usize;
    app.toggle_grouping();
    let before = identities(&app);

    assert!(!app.poll_regroup_backstop());

    assert_eq!(identities(&app), before, "the grouping was left as it was");
}

/// Reopening a session never regroups: yesterday's review shows yesterday's
/// groups (`docs/REGROUPING_STATE.md`). The first poll latches whatever drift
/// the review opened carrying, so a session persisted over the threshold is not
/// regrouped for it — and the very next arrival, which the reader is present
/// for, is.
#[test]
fn a_review_that_opens_over_the_threshold_is_not_regrouped_for_it() {
    let mut app = app();
    drift(&mut app, ARRIVALS);
    app.regroup_threshold = DRIFTED as usize;
    let reopened = identities(&app);

    assert!(!app.poll_regroup_backstop(), "the first poll only latches");
    assert!(!app.poll_regroup_backstop(), "and nothing has moved since");
    assert_eq!(identities(&app), reopened);

    drift(&mut app, LATER);

    assert!(
        app.poll_regroup_backstop(),
        "drift moving under the reader is what the backstop answers"
    );
    assert_eq!(percent(&app), 0);
}

/// `regroup_threshold = 0` is the off switch: no crossing exists, so nothing
/// lands whatever drift reaches.
#[test]
fn a_zero_threshold_turns_the_backstop_off() {
    let mut app = drifted_app();
    app.regroup_threshold = 0;
    let before = identities(&app);

    assert!(!app.poll_regroup_backstop());

    assert_eq!(identities(&app), before);
    assert_eq!(percent(&app), DRIFTED, "drifted, and left drifted");
}
