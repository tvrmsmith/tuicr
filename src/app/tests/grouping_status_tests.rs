//! The grouping status the sidebar header reports (`gd-26r.15`): which of the
//! four states wins, when the slot is empty, and what the drift number counts.
//!
//! The states are asserted through the app rather than by setting the fields
//! they read, because three of the four are *derived* — in flight is a pending
//! `:regroup`, heuristics-only is a partition with no refined group in it, and
//! drift is the partition's own arithmetic. A test that set them by hand would
//! pass over a header that never lights up.

use std::sync::Arc;
use std::time::Duration;

use super::grouping_tests::{grouped_paths, make_file};
use crate::app::App;
use crate::app::grouping::GroupingStatus;
use crate::app::refine::{CancelKeys, RefineCall, RefineConfig, Screen};
use crate::grouping::GroupSource;

const PATHS: &[&str] = &[
    "src/auth/login.rs",
    "src/auth/session.rs",
    "src/auth/token.rs",
    "tests/auth_test.rs",
    "docs/auth.md",
    "Cargo.lock",
];

/// A well-formed answer over `PATHS`, so a test can reach a partition that
/// really is refined rather than one asserted to be.
const ANSWER: &str = r#"{"groups": [
    {"name": "auth-token-rotation", "files": [
        "src/auth/token.rs", "tests/auth_test.rs", "docs/auth.md"]},
    {"name": "session-plumbing", "files": [
        "src/auth/login.rs", "src/auth/session.rs", "Cargo.lock"]}
]}"#;

const PATIENT: Duration = Duration::from_secs(30);

fn app() -> App {
    grouped_paths(PATHS)
}

/// A grouped review whose config asked for refine. The settings are never read:
/// every test supplies its own call.
fn configured_app() -> App {
    let mut app = app();
    app.refine_config = Some(RefineConfig {
        timeout: PATIENT,
        settings: crate::grouping::vertex::Settings::default(),
    });
    app
}

fn answering(body: &'static str) -> RefineCall {
    Arc::new(move |_prompt: &str, _timeout: Duration| Ok(body.to_string()))
}

/// A call that parks, which is what an in-flight regroup and a cancel are both
/// waiting through.
fn never() -> RefineCall {
    Arc::new(|_prompt: &str, _timeout: Duration| {
        std::thread::sleep(Duration::from_secs(60));
        Err("unreachable in these tests".to_string())
    })
}

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

/// A keyboard that presses cancel on its first read, and a screen that records
/// nothing: this module is about what the header says afterwards.
struct Cancelling;

impl CancelKeys for Cancelling {
    fn cancelled(&mut self, within: Duration) -> bool {
        std::thread::sleep(within.min(Duration::from_millis(1)));
        true
    }
}

struct Blind;

impl Screen for Blind {
    fn waiting(&mut self, _elapsed: Duration, _attempt: u32) {}
    fn finish(&mut self) {}
}

/// The review after the human cancelled a startup refine wait, reached through
/// the wait itself.
fn cancelled_app() -> App {
    let mut app = crate::app::tests::grouping_tests::ungrouped_app(
        PATHS.iter().map(|path| make_file(path)).collect(),
    );
    app.refine_config = Some(RefineConfig {
        timeout: PATIENT,
        settings: crate::grouping::vertex::Settings::default(),
    });
    app.refine_grouping_with(PATIENT, never(), &mut Blind, &mut Cancelling);
    app.enable_grouping();
    app
}

/// A review that drifted: one file appeared and incremental assignment opened a
/// group for it, which is the only way a `~` and a drift percentage arise.
fn drifted_app() -> App {
    let mut app = app();
    let arrival = make_file("infra/terraform/network.tf");
    app.diff_files.push(arrival);
    // The session's table is what `grouping_for` carries onto the new
    // changeset, and the arrival is the one path it does not name.
    app.sort_files_by_directory(false);
    let grouping = app.grouping.as_ref().expect("grouped");
    assert!(
        grouping.groups().iter().any(|group| group.drifted()),
        "the setup depends on the arrival opening a group, not joining one"
    );
    app
}

#[test]
fn an_undrifted_heuristic_review_says_nothing() {
    assert_eq!(app().grouping_status(), None);
}

/// The header is grouped-view chrome. Toggled off the sidebar is the plain file
/// tree, and the staleness of a grouping nobody is looking at is not news.
#[test]
fn the_ungrouped_sidebar_reports_no_grouping_status() {
    let mut app = drifted_app();
    assert!(app.grouping_status().is_some(), "grouped, it reports drift");

    app.toggle_grouping();

    assert_eq!(app.grouping_status(), None);
    assert!(
        app.grouping.is_some(),
        "the grouping is kept across the off state; only the chrome went"
    );
}

#[test]
fn a_configured_refine_that_never_landed_reports_heuristics_only() {
    let app = configured_app();

    assert_eq!(
        app.grouping_status(),
        Some(GroupingStatus::HeuristicsOnly),
        "bad groups must not be blamed on the heuristics when refine never ran"
    );
}

/// Nothing was asked for, so nothing is reported: a review with refine off is
/// heuristics-only by definition and saying so every session is noise.
#[test]
fn a_review_with_refine_off_never_reports_heuristics_only() {
    assert_eq!(app().grouping_status(), None);
}

#[test]
fn a_refined_partition_stops_reporting_heuristics_only() {
    let mut app = configured_app();
    app.regroup_with(Some(answering(ANSWER)));
    drain(&mut app);
    assert!(
        app.grouping
            .as_ref()
            .expect("grouped")
            .groups()
            .iter()
            .all(|group| group.source == GroupSource::Refined),
        "the answer landed"
    );

    assert_eq!(app.grouping_status(), None);
}

/// Distinct wording on purpose: the reader chose this, and the environment did
/// not fail.
#[test]
fn a_cancelled_refine_is_reported_as_a_choice() {
    let app = cancelled_app();

    assert_eq!(app.grouping_status(), Some(GroupingStatus::RefineCancelled));
}

#[test]
fn asking_for_a_refine_again_stops_calling_it_cancelled() {
    let mut app = cancelled_app();

    app.regroup_with(Some(answering(ANSWER)));
    drain(&mut app);

    assert_eq!(
        app.grouping_status(),
        None,
        "the second refine landed, so neither the cancel nor the heuristics stand"
    );
}

/// In flight outranks unavailable: it is transient, and it is about to answer
/// the very question the other state describes.
#[test]
fn a_regroup_in_flight_outranks_the_heuristics_only_it_is_answering() {
    let mut app = configured_app();
    assert_eq!(app.grouping_status(), Some(GroupingStatus::HeuristicsOnly));

    app.regroup_with(Some(never()));

    assert!(app.pending_regroup.is_some(), "the call went out");
    assert_eq!(app.grouping_status(), Some(GroupingStatus::Refining));
}

#[test]
fn drift_is_reported_as_a_share_of_the_files() {
    let app = drifted_app();

    // One arrival among seven files: 14%.
    assert_eq!(
        app.grouping_status(),
        Some(GroupingStatus::Drift { percent: 14 })
    );
}

/// Unavailable outranks drift: "not the grouping you asked for" outranks "the
/// grouping you asked for has moved".
#[test]
fn heuristics_only_outranks_drift() {
    let mut app = drifted_app();
    app.refine_config = Some(RefineConfig {
        timeout: PATIENT,
        settings: crate::grouping::vertex::Settings::default(),
    });

    assert_eq!(app.grouping_status(), Some(GroupingStatus::HeuristicsOnly));
}

/// Rounding may not reach the `0%` the empty slot means. One file in two
/// hundred is drift a reader can act on, and `0% new · :regroup` reads as a
/// bug.
#[test]
fn a_drift_too_small_to_round_up_is_still_not_reported_as_zero() {
    let mut paths: Vec<String> = (0..200).map(|n| format!("src/api/route_{n}.rs")).collect();
    let mut app = crate::app::tests::grouping_tests::grouped_app(
        paths.iter().map(|path| make_file(path)).collect(),
    );
    paths.push("vendor/legal/NOTICE.txt".to_string());
    app.diff_files.push(make_file("vendor/legal/NOTICE.txt"));
    app.sort_files_by_directory(false);

    let drift = app.grouping.as_ref().expect("grouped").drift();
    assert!(drift > 0.0 && drift < 0.005, "the setup rounds to zero");
    assert_eq!(
        app.grouping_status(),
        Some(GroupingStatus::Drift { percent: 1 })
    );
}

/// The advisory is dropped whole rather than truncated mid-word, and the
/// percentage — the part the slot exists for — survives either way.
#[test]
fn the_regroup_advice_drops_out_of_a_header_too_narrow_for_it() {
    let chip = GroupingStatus::Drift { percent: 9 };

    assert_eq!(chip.chip(40), "\u{00b7} 9% new \u{00b7} :regroup ");
    assert_eq!(chip.chip(20), "\u{00b7} 9% new \u{00b7} :regroup ");
    assert_eq!(chip.chip(19), "\u{00b7} 9% new ");
    assert_eq!(chip.chip(0), "\u{00b7} 9% new ");
}
