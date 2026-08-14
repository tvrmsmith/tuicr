//! The blocking startup refine, driven end to end with a canned call, a fake
//! keyboard and a recording screen.
//!
//! Every test here ends the same way — `enable_grouping`, then assert the
//! session is *working*. That is the point. Cancel, timeout and a body that is
//! not JSON are the paths most likely to be written once and never run, and the
//! failure they hide is not a crash but a half-grouped sidebar: a partition with
//! a file missing, or no partition at all. So each of them is followed all the
//! way into the grouped sidebar rather than stopping at the [`RefineOutcome`].

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::grouping_tests::{
    make_file, ungrouped_app, ungrouped_app_serving, ungrouped_app_with_working_tree,
};
use crate::app::App;
use crate::app::TargetPick;
use crate::app::refine::{CancelKeys, RefineCall, RefineConfig, RefineOutcome, Screen, Skipped};
use crate::grouping::GroupSource;
use crate::model::DiffFile;
use crate::vcs::traits::CommitInfo;

const PATHS: &[&str] = &[
    "src/auth/login.rs",
    "src/auth/session.rs",
    "src/auth/token.rs",
    "tests/auth_test.rs",
    "docs/auth.md",
    "Cargo.lock",
];

/// A generous timeout for the tests that are not about timing out. Nothing
/// here touches a socket, so no passing test ever waits on it.
const PATIENT: Duration = Duration::from_secs(30);

fn app() -> App {
    ungrouped_app(PATHS.iter().map(|path| make_file(path)).collect())
}

/// An app whose config asked for refine, which is what makes a diff load ask
/// for one. The settings are never read: every test supplies its own call.
fn configured_app() -> App {
    let mut app = app();
    app.refine_config = Some(RefineConfig {
        timeout: PATIENT,
        settings: crate::grouping::vertex::Settings::default(),
    });
    app
}

/// A configured app already showing a grouped review, whose backend serves
/// `working_tree` to the working-tree loaders. Picking the working-tree tab in
/// the target selector is [`App::load_staged_and_unstaged_selection`], so this
/// is the seam that reaches the arming decision the way the binary does —
/// including the load that finds nothing and returns before any reorder.
fn app_serving(working_tree: &[&str]) -> App {
    let mut app = ungrouped_app_with_working_tree(
        PATHS.iter().map(|path| make_file(path)).collect(),
        working_tree.iter().map(|path| make_file(path)).collect(),
    );
    app.refine_config = Some(RefineConfig {
        timeout: PATIENT,
        settings: crate::grouping::vertex::Settings::default(),
    });
    app.enable_grouping();
    assert!(!app.refine_wanted, "the app under test starts unarmed");
    app
}

fn commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.to_string(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: None,
        author: "someone".to_string(),
        time: chrono::Utc::now(),
    }
}

/// Puts the review into the state an inline commit selector needs, then toggles
/// it, which is the reshuffle the gate must never spend a call on. The whole
/// range is selected and cached, so this reaches the reorder without a backend.
fn toggle_inline_selection(app: &mut App) {
    app.review_commits = vec![commit("newer"), commit("older")];
    app.commit_selection_range = Some((0, 1));
    app.range_diff_files = Some(app.diff_files.clone());
    app.reload_inline_selection()
        .expect("the cached full range needs no backend");
}

/// A well-formed answer over `PATHS`: two groups, in an order the heuristics do
/// not produce, so "the model's order was honoured" is a checkable claim.
const ANSWER: &str = r#"{"groups": [
    {"name": "auth-token-rotation", "files": [
        "src/auth/token.rs", "tests/auth_test.rs", "docs/auth.md"]},
    {"name": "session-plumbing", "files": [
        "src/auth/login.rs", "src/auth/session.rs", "Cargo.lock"]}
]}"#;

/// Records what the screen was told, so the wait's honesty about the file count
/// and the retry is asserted rather than assumed.
#[derive(Default)]
struct Recorder {
    frames: Vec<(Duration, u32)>,
    finished: bool,
}

impl Screen for Recorder {
    fn waiting(&mut self, elapsed: Duration, attempt: u32) {
        self.frames.push((elapsed, attempt));
    }
    fn finish(&mut self) {
        self.finished = true;
    }
}

/// A keyboard that presses cancel on the nth poll and nothing on the others.
struct Keys {
    cancel_on: Option<usize>,
    polls: usize,
}

impl Keys {
    fn silent() -> Self {
        Self {
            cancel_on: None,
            polls: 0,
        }
    }
    fn cancelling() -> Self {
        Self {
            cancel_on: Some(1),
            polls: 0,
        }
    }
}

impl CancelKeys for Keys {
    fn cancelled(&mut self, within: Duration) -> bool {
        self.polls += 1;
        // The real key read sleeps out its slice; this one must not, or the
        // suite pays the wall clock the arm was measured at. It does sleep a
        // sliver, because a poll that consumes nothing turns the timeout test
        // into a hot loop that records millions of frames.
        std::thread::sleep(within.min(Duration::from_millis(1)));
        self.cancel_on == Some(self.polls)
    }
}

/// A call that answers immediately with whatever it is handed, in order.
fn answering(bodies: &[&str]) -> (RefineCall, Arc<AtomicUsize>) {
    let bodies: Vec<String> = bodies.iter().map(|body| body.to_string()).collect();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let call: RefineCall = Arc::new(move |_prompt: &str, _timeout: Duration| {
        let index = counter.fetch_add(1, Ordering::SeqCst);
        Ok(bodies
            .get(index)
            .cloned()
            .unwrap_or_else(|| "no further canned answer".to_string()))
    });
    (call, calls)
}

/// A call that never answers, which is what a timeout and a cancel are waiting
/// through. It parks rather than returning so the wait has to reach its own
/// deadline or its own key read to end.
fn never() -> RefineCall {
    Arc::new(|_prompt: &str, _timeout: Duration| {
        std::thread::sleep(Duration::from_secs(60));
        Err("never answered".to_string())
    })
}

/// The claim every fallback test makes: this is a working grouped session, not
/// a degraded one. Total partition, every file placed exactly once, every group
/// still the heuristics' own.
fn assert_heuristic_session(app: &App) {
    let grouping = app.grouping.as_ref().expect("a grouping exists");
    assert!(
        grouping
            .groups()
            .iter()
            .all(|group| group.source == GroupSource::Heuristics),
        "a fallback leaves no group claiming to be refined"
    );
    assert_eq!(
        grouping.assignments().len(),
        PATHS.len(),
        "the heuristic partition is total: falling back cannot lose a file"
    );
    for path in PATHS {
        assert!(
            grouping.group_of(std::path::Path::new(path)).is_some(),
            "{path} is unassigned after the fallback"
        );
    }
    assert_eq!(app.diff_files.len(), PATHS.len());
    assert!(app.pending_refined.is_none(), "nothing stale is parked");
}

fn group_names(app: &App) -> Vec<String> {
    app.grouping
        .as_ref()
        .expect("a grouping exists")
        .groups()
        .iter()
        .map(|group| group.name.clone())
        .collect()
}

#[test]
fn a_clean_answer_is_adopted_in_the_order_the_model_returned_it() {
    let mut app = app();
    let (call, calls) = answering(&[ANSWER]);
    let mut screen = Recorder::default();

    let outcome = app.refine_grouping_with(PATIENT, call, &mut screen, &mut Keys::silent());
    assert!(
        matches!(outcome, RefineOutcome::Refined { repairs: 0 }),
        "{outcome:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one call, never a vote");
    assert!(screen.finished, "the progress line is wiped before the TUI");
    assert!(outcome.warning().is_none(), "a refine that worked is quiet");

    app.enable_grouping();
    assert_eq!(
        group_names(&app),
        ["auth-token-rotation", "session-plumbing"]
    );
    let grouping = app.grouping.as_ref().expect("a grouping exists");
    assert!(
        grouping
            .groups()
            .iter()
            .all(|group| group.source == GroupSource::Refined)
    );
    assert_eq!(grouping.assignments().len(), PATHS.len());
    assert!(
        app.pending_refined.is_none(),
        "the parked answer is taken, not copied: a later reorder must not \
         re-apply it over a changed changeset"
    );
    // The groups run in the model's order, and inside each one the files are in
    // the engine's within-group order — neither the model's listing order nor
    // path order (rule 4 puts `auth_test.rs` after `token.rs`). `Grouping::build`
    // is the sole constructor and sorts by construction, so a refined grouping
    // arrives sorted and the seam re-sorts nothing.
    let files: Vec<String> = app
        .diff_files
        .iter()
        .map(|file| file.display_path().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        files,
        [
            "docs/auth.md",
            "src/auth/token.rs",
            "tests/auth_test.rs",
            "src/auth/session.rs",
            "src/auth/login.rs",
            "Cargo.lock",
        ]
    );
}

#[test]
fn a_cancelled_wait_opens_a_working_heuristic_session() {
    let mut app = app();
    let mut screen = Recorder::default();
    let mut keys = Keys::cancelling();

    let outcome = app.refine_grouping_with(PATIENT, never(), &mut screen, &mut keys);
    assert!(matches!(outcome, RefineOutcome::Cancelled), "{outcome:?}");
    assert!(
        outcome
            .warning()
            .expect("a cancel says so")
            .contains("heuristics")
    );
    assert!(screen.finished);
    assert!(
        !screen.frames.is_empty(),
        "the human saw the wait before cancelling it"
    );

    app.enable_grouping();
    assert_heuristic_session(&app);
}

#[test]
fn a_timeout_opens_a_working_heuristic_session() {
    let mut app = app();
    let mut screen = Recorder::default();

    // Short enough that the test does not sit through it, and the call parked
    // for a minute, so only the deadline can end this wait.
    let outcome = app.refine_grouping_with(
        Duration::from_millis(50),
        never(),
        &mut screen,
        &mut Keys::silent(),
    );
    assert!(matches!(outcome, RefineOutcome::TimedOut), "{outcome:?}");
    assert!(
        outcome
            .warning()
            .expect("a timeout says so")
            .contains("refine_timeout_ms"),
        "the message names the knob that would have waited longer"
    );
    assert!(screen.finished);

    app.enable_grouping();
    assert_heuristic_session(&app);
}

#[test]
fn a_body_that_is_not_json_is_retried_once_and_then_falls_back() {
    let mut app = app();
    let (call, calls) = answering(&["I'm sorry, I can't do that.", "still not JSON"]);
    let mut screen = Recorder::default();

    let outcome = app.refine_grouping_with(PATIENT, call, &mut screen, &mut Keys::silent());
    let RefineOutcome::Failed(reason) = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        reason.as_str(),
        "no JSON object in response",
        "the fallback names what was wrong with the body, not just that there was a body"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "exactly one retry: the identical prompt resent, and no third attempt"
    );

    app.enable_grouping();
    assert_heuristic_session(&app);
}

#[test]
fn a_retry_that_parses_is_adopted() {
    let mut app = app();
    let (call, calls) = answering(&["(no JSON here)", ANSWER]);
    let mut screen = Recorder::default();

    let outcome = app.refine_grouping_with(PATIENT, call, &mut screen, &mut Keys::silent());
    assert!(
        matches!(outcome, RefineOutcome::Refined { repairs: 0 }),
        "{outcome:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    app.enable_grouping();
    assert_eq!(
        group_names(&app),
        ["auth-token-rotation", "session-plumbing"]
    );
}

#[test]
fn a_transport_failure_falls_back_and_says_why() {
    let mut app = app();
    let call: RefineCall = Arc::new(|_prompt: &str, _timeout: Duration| {
        Err("no application default credentials".to_string())
    });
    let mut screen = Recorder::default();

    let outcome = app.refine_grouping_with(PATIENT, call, &mut screen, &mut Keys::silent());
    match &outcome {
        RefineOutcome::Failed(reason) => assert!(reason.contains("credentials")),
        other => panic!("{other:?}"),
    }
    assert!(
        outcome
            .warning()
            .expect("a failure says so")
            .contains("no application default credentials"),
        "the reason reaches the human rather than a generic apology"
    );

    app.enable_grouping();
    assert_heuristic_session(&app);
}

/// A body the contract calls repairable — an invented path, a dropped one — is
/// applied rather than rejected, and the repairs are counted. There is no
/// rejection threshold: the partition is made total on the way in.
#[test]
fn a_repairable_answer_is_applied_and_stays_total() {
    let mut app = app();
    let (call, _) = answering(&[r#"{"groups": [
        {"name": "auth", "files": [
            "src/auth/token.rs", "src/auth/login.rs", "src/invented.rs"]}
    ]}"#]);
    let mut screen = Recorder::default();

    let outcome = app.refine_grouping_with(PATIENT, call, &mut screen, &mut Keys::silent());
    let warning = outcome
        .warning()
        .expect("a repaired refine is not a quiet one");
    assert!(
        warning.contains("contract violations repaired"),
        "the outcome offers the count as a warning rather than computing and \
         dropping it; whether the binary then displays it is `set_warning`'s \
         business and is not asserted here: {warning}"
    );
    let RefineOutcome::Refined { repairs } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(repairs > 0, "the violations are counted, not swallowed");
    assert!(
        warning.contains(&repairs.to_string()),
        "the human is told how many: {warning}"
    );

    app.enable_grouping();
    let grouping = app.grouping.as_ref().expect("a grouping exists");
    assert_eq!(grouping.assignments().len(), PATHS.len());
    for path in PATHS {
        assert!(
            grouping.group_of(std::path::Path::new(path)).is_some(),
            "{path} is unassigned after a repair"
        );
    }
    assert!(
        !grouping
            .assignments()
            .iter()
            .any(|assignment| assignment.path.ends_with("invented.rs")),
        "the invented path is dropped, not reviewed"
    );
    // One session, two sources: what the model returned is refined, what the
    // repair put back is still the heuristics' answer and says so.
    assert!(
        grouping
            .groups()
            .iter()
            .any(|group| group.source == GroupSource::Refined)
    );
    assert!(
        grouping
            .groups()
            .iter()
            .any(|group| group.source == GroupSource::Heuristics)
    );
}

/// Reopening never refines (`docs/REGROUPING_STATE.md`), so the wait is once per
/// review and a second startup is instant.
#[test]
fn a_session_that_already_holds_a_grouping_is_not_refined_again() {
    let mut app = app();
    app.enable_grouping();
    let names = group_names(&app);

    let (call, calls) = answering(&[ANSWER]);
    let outcome =
        app.refine_grouping_with(PATIENT, call, &mut Recorder::default(), &mut Keys::silent());
    assert!(
        matches!(outcome, RefineOutcome::Skipped(Skipped::AlreadyGrouped)),
        "{outcome:?}"
    );
    assert!(
        outcome
            .warning()
            .expect("a configured refine that will never run says so")
            .contains("saved grouping"),
        "refine on and nothing refined, silently, reads as the setting being ignored"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no request was dispatched");

    app.enable_grouping();
    assert_eq!(group_names(&app), names);
}

/// The keys the screen advertises, and only those. A wrong mapping here is a
/// wait a human cannot get out of, which is the hang the screen exists to avoid
/// looking like.
#[test]
fn the_advertised_keys_cancel_and_nothing_else_does() {
    use crate::app::refine::is_cancel;
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseEvent,
        MouseEventKind,
    };

    let key = |code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind| {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            state: KeyEventState::NONE,
        })
    };
    let press = |code, modifiers| key(code, modifiers, KeyEventKind::Press);

    assert!(is_cancel(&press(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(is_cancel(&press(KeyCode::Char('q'), KeyModifiers::NONE)));
    assert!(
        is_cancel(&press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        "raw mode swallows the signal, so Ctrl-C is read as a key"
    );

    assert!(!is_cancel(&press(KeyCode::Char('c'), KeyModifiers::NONE)));
    assert!(!is_cancel(&press(KeyCode::Enter, KeyModifiers::NONE)));
    assert!(
        !is_cancel(&key(
            KeyCode::Esc,
            KeyModifiers::NONE,
            KeyEventKind::Release
        )),
        "one Esc is one cancel: the release half must not read as a second"
    );
    assert!(!is_cancel(&Event::Resize(80, 24)));
    assert!(!is_cancel(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    })));
}

/// What the shipped screen actually writes, read back from a sink: the file
/// count, the retry, and the erase that stops half a status line ending up
/// under the TUI.
#[test]
fn the_status_line_names_the_wait_and_erases_itself() {
    use crate::app::refine::{Screen, StatusLine};

    let mut sink: Vec<u8> = Vec::new();
    let mut screen = StatusLine::writing_to(161, true, &mut sink);
    screen.waiting(Duration::from_secs(75), 1);
    screen.waiting(Duration::from_secs(75), 2);
    screen.finish();
    let written = String::from_utf8(sink).expect("the screen writes text");

    let frames: Vec<&str> = written.split('\r').filter(|s| !s.is_empty()).collect();
    assert_eq!(frames.len(), 3, "two frames and the erase: {written:?}");
    assert!(
        frames[0].starts_with("\x1b[2K"),
        "each frame erases the line it overwrites: {:?}",
        frames[0]
    );
    assert!(frames[0].contains("161 files"), "{:?}", frames[0]);
    assert!(
        frames[0].contains("1:15 elapsed"),
        "the elapsed clock is minutes and seconds: {:?}",
        frames[0]
    );
    assert!(
        !frames[0].contains("retrying"),
        "a first attempt is not a retry: {:?}",
        frames[0]
    );
    assert!(frames[0].contains("esc cancels"), "{:?}", frames[0]);
    assert!(
        frames[1].contains("retrying (2 of 2)"),
        "a doubled wait reads as a hang unless the retry is legible: {:?}",
        frames[1]
    );
    assert_eq!(
        frames[2], "\x1b[2K",
        "the line is wiped, not left half-drawn"
    );

    // Nothing drawn, nothing to erase: a skipped refine must not emit an escape
    // sequence into a pipeline.
    let mut untouched: Vec<u8> = Vec::new();
    StatusLine::writing_to(0, true, &mut untouched).finish();
    assert!(untouched.is_empty());

    // A run with no controlling terminal has no keyboard behind the wait, and
    // a line that tells the human to press Esc there is a lie the wait would
    // then ignore.
    let mut deaf: Vec<u8> = Vec::new();
    let mut screen = StatusLine::writing_to(161, false, &mut deaf);
    screen.waiting(Duration::from_secs(75), 1);
    let written = String::from_utf8(deaf).expect("the screen writes text");
    assert!(written.contains("161 files"), "{written:?}");
    assert!(
        !written.contains("esc"),
        "a wait that cannot read a key must not advertise one: {written:?}"
    );
}

/// Bare `tuicr` opens on the target selector with nothing to group. That is not
/// a refine that failed, and it is not a refine that will never happen either:
/// the wait runs when the diff loads, so it is quiet here.
#[test]
fn an_empty_changeset_is_not_refined() {
    let mut app = ungrouped_app(Vec::new());
    let (call, calls) = answering(&[ANSWER]);
    let outcome =
        app.refine_grouping_with(PATIENT, call, &mut Recorder::default(), &mut Keys::silent());
    assert!(
        matches!(outcome, RefineOutcome::Skipped(Skipped::NoChangeset)),
        "{outcome:?}"
    );
    assert!(
        outcome.warning().is_none(),
        "the wait is still to come; there is nothing to report yet"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// The whole point of the repair count is that it reaches the human, so the
/// sentence it reaches them in is asserted rather than the number alone —
/// including the singular, which is the branch a plural-only test never runs.
#[test]
fn the_repair_warning_counts_in_the_human_s_grammar() {
    for (repairs, expected) in [
        (1usize, "Refine applied with 1 contract violation repaired."),
        (2, "Refine applied with 2 contract violations repaired."),
    ] {
        assert_eq!(
            RefineOutcome::Refined { repairs }.warning().as_deref(),
            Some(expected)
        );
    }
    assert!(
        RefineOutcome::Refined { repairs: 0 }.warning().is_none(),
        "a clean refine is quiet"
    );
}

/// Records the overlay the in-TUI wait paints, in place of the terminal it
/// would be drawn on.
#[derive(Default)]
struct Overlay {
    frames: Vec<String>,
}

impl Overlay {
    fn paint(&mut self) -> impl FnMut(&str) + '_ {
        move |text: &str| self.frames.push(text.to_string())
    }
}

/// A target picked inside the TUI — bare `tuicr`, or `tuicr pr` before a PR is
/// chosen — has no pre-TUI moment to block in. The wait runs when its diff
/// loads instead, with the same semantics and the alternate screen's own
/// surface.
#[test]
fn a_diff_that_loads_after_the_screen_is_up_is_refined_in_the_tui() {
    let mut app = configured_app();
    // What a target pick does: arm, then load and group the fresh changeset.
    app.reorder_for_load(TargetPick::NewTarget);
    app.enable_grouping();
    assert!(
        app.refine_wanted,
        "the diff of a target the human just picked is what a refine improves"
    );

    // The sidebar is already on screen, so the human is reading a file. The
    // answer reorders every row under them; the file they are on must not move
    // out from under the cursor.
    let anchor = app
        .diff_files
        .iter()
        .position(|file| file.display_path().ends_with("Cargo.lock"))
        .expect("Cargo.lock is in the changeset");
    assert_ne!(
        anchor,
        PATHS.len() - 1,
        "the anchor has to move for the restore to be worth asserting"
    );
    app.rebuild_annotations();
    app.jump_to_file(anchor);
    assert_eq!(
        app.current_file_path().cloned(),
        Some(std::path::PathBuf::from("Cargo.lock")),
        "the cursor has to be on the anchored file, not in the overview"
    );

    let (call, calls) = answering(&[ANSWER]);
    let mut overlay = Overlay::default();
    let outcome = app.refine_loaded_with(PATIENT, call, &mut overlay.paint(), &mut Keys::silent());

    assert!(
        matches!(outcome, RefineOutcome::Refined { repairs: 0 }),
        "{outcome:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one call, never a vote");
    assert!(!app.refine_wanted, "the wait is once per target pick");

    // Adopted, not parked: the sidebar it replaces is already on screen.
    assert_eq!(
        group_names(&app),
        ["auth-token-rotation", "session-plumbing"]
    );
    assert!(app.pending_refined.is_none());
    let grouping = app.grouping.as_ref().expect("a grouping exists");
    assert!(
        grouping
            .groups()
            .iter()
            .all(|group| group.source == GroupSource::Refined)
    );
    assert_eq!(grouping.assignments().len(), PATHS.len());

    assert_eq!(
        app.diff_files
            .get(app.diff_state.current_file_idx)
            .map(|file| file.display_path().clone()),
        Some(std::path::PathBuf::from("Cargo.lock")),
        "the reorder kept the cursor on the file the human was reading"
    );

    // Groups the model invented are rows nobody has ever expanded, so an adopt
    // that did not re-expand would hand back a sidebar of closed folders.
    let ids: std::collections::HashSet<String> = grouping
        .groups()
        .iter()
        .map(App::group_row_key)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(app.expanded_dirs, ids, "every adopted group renders open");

    // Reading order is the model's array order, and each group is one run:
    // a file interleaved between two groups is a group the eye has to hunt for.
    let runs: Vec<String> = app
        .diff_files
        .iter()
        .filter_map(|file| grouping.group_of(file.display_path()))
        .filter_map(|id| grouping.group(id))
        .map(|group| group.name.clone())
        .fold(Vec::new(), |mut runs, name| {
            if runs.last() != Some(&name) {
                runs.push(name);
            }
            runs
        });
    assert_eq!(
        runs,
        ["auth-token-rotation", "session-plumbing"],
        "each group is a contiguous run, in the order the model returned"
    );
}

/// Cancelling the in-TUI wait leaves the same working heuristic session the
/// pre-TUI cancel leaves — and does not ask again, here or on the next reorder.
#[test]
fn a_cancelled_in_tui_wait_keeps_the_heuristic_session_and_is_not_offered_twice() {
    let mut app = configured_app();
    app.reorder_for_load(TargetPick::NewTarget);
    app.enable_grouping();
    let heuristic_names = group_names(&app);

    let mut overlay = Overlay::default();
    let outcome = app.refine_loaded_with(
        PATIENT,
        never(),
        &mut overlay.paint(),
        &mut Keys::cancelling(),
    );

    assert!(matches!(outcome, RefineOutcome::Cancelled), "{outcome:?}");
    assert!(
        overlay
            .frames
            .first()
            .is_some_and(|frame| frame.contains("esc cancels")),
        "the human saw what they cancelled, and how: {:?}",
        overlay.frames
    );
    assert_heuristic_session(&app);
    assert_eq!(group_names(&app), heuristic_names);

    // A reorder of the same review must not re-open the wait the human just
    // walked away from.
    app.enable_grouping();
    assert!(!app.refine_wanted, "one refusal is enough");
}

/// A refine that is off leaves the flag alone, so the main loop never dispatches
/// a wait nobody asked for — even over a target the human just picked.
#[test]
fn a_load_asks_for_no_refine_when_the_config_did_not() {
    let mut app = app();
    app.reorder_for_load(TargetPick::NewTarget);
    app.enable_grouping();
    assert!(!app.refine_wanted);
}

/// Reopening never re-refines (`docs/REGROUPING_STATE.md`): the grouping the
/// human already curated stays. But a configured arm that turns out inert must
/// not be silent about it, so the pick is honoured as far as the refusal and
/// the refusal is reported.
#[test]
fn a_reopened_session_is_told_its_grouping_already_exists() {
    let mut app = configured_app();
    app.reorder_for_load(TargetPick::NewTarget);
    app.enable_grouping();
    let saved = app.session.clone();
    let curated = group_names(&app);

    let mut reopened = configured_app();
    reopened.session = saved;
    reopened.reorder_for_load(TargetPick::NewTarget);
    reopened.enable_grouping();

    let (call, calls) = answering(&[ANSWER]);
    let mut overlay = Overlay::default();
    let outcome =
        reopened.refine_loaded_with(PATIENT, call, &mut overlay.paint(), &mut Keys::silent());

    assert!(
        matches!(outcome, RefineOutcome::Skipped(Skipped::AlreadyGrouped)),
        "{outcome:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing is billed");
    assert!(
        outcome.warning().is_some(),
        "a configured arm that did nothing says so"
    );
    assert_eq!(
        group_names(&reopened),
        curated,
        "the grouping that came back out of the session is the one that stays"
    );
    assert!(!reopened.refine_wanted);
}

/// The gate is the human picking what to review, and nothing else. A reshuffle
/// inside a review already open — an inline commit toggle, a `:reload`, a PR
/// re-fetch — changes the file set without changing the target, and each one
/// would otherwise cost a blocking billed call.
#[test]
fn a_reshuffle_inside_an_open_review_dispatches_no_refine() {
    let mut app = configured_app();
    app.reorder_for_load(TargetPick::NewTarget);
    app.enable_grouping();
    assert!(app.refine_wanted, "the pick is answered by its own load");

    let (call, _) = answering(&[ANSWER]);
    let mut overlay = Overlay::default();
    let outcome = app.refine_loaded_with(PATIENT, call, &mut overlay.paint(), &mut Keys::silent());
    assert!(
        matches!(outcome, RefineOutcome::Refined { repairs: 0 }),
        "{outcome:?}"
    );

    // A different file set under the same review, through the toggle itself.
    app.diff_files = PATHS
        .iter()
        .take(4)
        .map(|path| make_file(path))
        .chain(std::iter::once(make_file("src/auth/refresh.rs")))
        .collect();
    toggle_inline_selection(&mut app);
    assert!(
        !app.refine_wanted,
        "a changeset nobody has seen before is still not a target the human picked"
    );
}

/// The regression the arm-on-success shape exists to make impossible: a target
/// pick whose load never reaches a reorder. Arming at the moment of the pick
/// left the flag set across the early return, and the next toggle inside the
/// review still open spent it on a blocking billed call.
#[test]
fn a_load_that_finds_no_changes_leaves_nothing_armed_for_the_next_reshuffle() {
    let mut app = app_serving(&[]);

    let before: Vec<_> = app
        .diff_files
        .iter()
        .map(DiffFile::display_path)
        .cloned()
        .collect();

    app.load_staged_and_unstaged_selection()
        .expect("an empty working tree is a message, not an error");

    // The branch under test is the one that returns before any reorder, so the
    // test says which branch ran rather than only what it left undone.
    assert_eq!(
        app.message.as_ref().map(|message| message.content.as_str()),
        Some("No staged or unstaged changes")
    );
    assert_eq!(
        app.diff_files
            .iter()
            .map(DiffFile::display_path)
            .cloned()
            .collect::<Vec<_>>(),
        before,
        "a load that found nothing leaves the review it interrupted alone"
    );
    assert!(
        !app.refine_target_picked,
        "a pick whose load found nothing has no load left to answer it"
    );

    toggle_inline_selection(&mut app);
    assert!(
        !app.refine_wanted,
        "the abandoned pick must not be spent on an unrelated reshuffle"
    );
}

/// The other half of the same shape: a load that *does* find changes arms, so
/// moving the arm onto the success path did not disarm the real pick.
#[test]
fn picking_the_working_tree_arms_the_load_that_answers_it() {
    let mut app = app_serving(&["src/auth/login.rs", "src/auth/token.rs"]);

    app.load_staged_and_unstaged_selection()
        .expect("the stub serves a working tree");

    assert!(
        app.refine_wanted,
        "confirming the working-tree tab is a review target being picked"
    );
}

/// The gate is armed by a load and read by the main loop on its next tick, and
/// a reshuffle can land in between. Such a reshuffle is not a pick, so it has
/// nothing to say about a wait already asked for — clearing it there would drop
/// the refine the human paid for by choosing what to review.
#[test]
fn a_reshuffle_between_the_pick_and_the_main_loop_keeps_the_wait() {
    let mut app = configured_app();
    app.reorder_for_load(TargetPick::NewTarget);
    app.enable_grouping();
    assert!(app.refine_wanted);

    toggle_inline_selection(&mut app);

    assert!(
        app.refine_wanted,
        "a reorder that is not a pick must not answer the question a pick asked"
    );
}

/// Confirming a commit selection that opens scoped to a single commit reaches
/// its reorder through the inline-selection reload rather than directly, which
/// is the one caller of that reload that *is* a pick. A gate that only knew
/// about the direct path would drop the refine on every review opened with
/// `initial_commit_selection = oldest`.
#[test]
fn confirming_a_commit_selection_that_opens_scoped_arms_the_refine() {
    let narrowed = ["src/auth/token.rs", "tests/auth_test.rs"];
    let mut app = ungrouped_app_serving(
        PATHS.iter().map(|path| make_file(path)).collect(),
        Vec::new(),
        narrowed.iter().map(|path| make_file(path)).collect(),
    );
    app.refine_config = Some(RefineConfig {
        timeout: PATIENT,
        settings: crate::grouping::vertex::Settings::default(),
    });
    app.commit_selection_start = crate::app::CommitSelectionStart::Oldest;
    app.commit_list = vec![commit("newer"), commit("older")];
    app.commit_selection_range = Some((0, 1));

    app.confirm_commit_selection()
        .expect("the stub serves the commit range");

    assert_eq!(
        app.commit_selection_range,
        Some((1, 1)),
        "opening scoped to the oldest commit is a strict selection"
    );
    let mut loaded: Vec<_> = app
        .diff_files
        .iter()
        .filter(|file| !file.is_commit_message)
        .map(DiffFile::display_path)
        .cloned()
        .collect();
    loaded.sort();
    assert_eq!(
        loaded,
        narrowed.map(std::path::PathBuf::from).to_vec(),
        "the diff under the pick is the narrowed one the human will read"
    );

    app.enable_grouping();
    assert!(
        app.refine_wanted,
        "confirming a commit selection is a review target being picked"
    );
}

/// Esc out of the target selector is leaving the selector, not choosing what to
/// review. It restores the working tree, which is a reorder — and Esc means
/// cancel everywhere else in the TUI, so it must never cost a blocking wait.
#[test]
fn leaving_the_target_selector_with_esc_never_refines() {
    let mut app = app_serving(&["src/auth/login.rs", "src/auth/token.rs"]);
    app.diff_source = crate::app::DiffSource::CommitRange(vec!["abc123".to_string()]);

    app.exit_commit_select_mode()
        .expect("leaving the selector always succeeds");

    // Esc did reorder — it restored the working tree — which is what makes the
    // unarmed flags below evidence rather than an absence.
    assert_eq!(app.diff_source, crate::app::DiffSource::StagedAndUnstaged);
    assert_eq!(
        app.diff_files
            .iter()
            .map(DiffFile::display_path)
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            std::path::PathBuf::from("src/auth/login.rs"),
            std::path::PathBuf::from("src/auth/token.rs"),
        ]
    );
    assert!(
        !app.refine_target_picked,
        "Esc is a cancel, not a pick, so nothing is armed"
    );
    assert!(
        !app.refine_wanted,
        "and the reorder it did dispatches nothing"
    );
}

/// Switching to a second target is a second pick, so it gets its own wait even
/// though the first one already ran.
#[test]
fn picking_a_second_target_refines_again() {
    let mut app = configured_app();
    app.reorder_for_load(TargetPick::NewTarget);
    app.enable_grouping();
    let (call, calls) = answering(&[ANSWER, ANSWER]);
    let mut overlay = Overlay::default();
    app.refine_loaded_with(
        PATIENT,
        Arc::clone(&call),
        &mut overlay.paint(),
        &mut Keys::silent(),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // A different commit range: a new target, a new diff.
    app.diff_files = ["src/render/frame.rs", "src/render/layout.rs"]
        .iter()
        .map(|path| make_file(path))
        .collect();
    // The sidebar is already grouped, so the load's own reorder is what regroups
    // it — and what spends the arming.
    app.reorder_for_load(TargetPick::NewTarget);
    assert!(app.refine_wanted, "a second pick is a second wait");

    let outcome = app.refine_loaded_with(PATIENT, call, &mut overlay.paint(), &mut Keys::silent());
    assert!(
        matches!(outcome, RefineOutcome::Refined { .. }),
        "{outcome:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// A transport that dies mid-call — a panicking `ureq` leg, a poisoned TLS
/// stack — takes the worker thread with it, so the answer channel hangs up
/// without ever sending. That must read as a failure and fall back, not as a
/// wait that never ends.
#[test]
fn a_transport_that_dies_falls_back_to_the_heuristics() {
    let mut app = app();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let call: RefineCall = Arc::new(move |_prompt: &str, _timeout: Duration| {
        counter.fetch_add(1, Ordering::SeqCst);
        panic!("the transport died");
    });
    let mut screen = Recorder::default();

    let outcome = app.refine_grouping_with(PATIENT, call, &mut screen, &mut Keys::silent());
    let RefineOutcome::Failed(reason) = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        reason, "the refine call ended without answering",
        "a hung-up channel reads as a hang-up, not as a spent retry budget"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a dead channel is not retried"
    );
    assert!(
        outcome.warning().is_some(),
        "a failure the human never sees is a silently unrefined sidebar"
    );
    assert!(screen.finished);

    app.enable_grouping();
    assert_heuristic_session(&app);
}
