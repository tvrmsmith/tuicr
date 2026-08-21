//! Grouping feedback (`gd-26r.42`): the verdict prompt at `:send`/`:w`, its
//! once-per-session gate, and what it yields.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::app::grouping_feedback::{
    FeedbackField, GROUPING_FEEDBACK_TAGS, GroupingFeedbackDraft, GroupingVerdict,
};
use crate::app::*;

use super::grouping_tests::{PATHS, grouped_paths, make_file, ungrouped_app};

/// A draft with no marks, for the pure `into_feedback` tests, which need no
/// `App` at all. Every field is `pub`, so a struct literal is the whole
/// fixture.
fn bare_draft() -> GroupingFeedbackDraft {
    GroupingFeedbackDraft {
        verdict: None,
        tags: BTreeSet::new(),
        note: String::new(),
        note_cursor: 0,
        focus: FeedbackField::Verdict,
        tag_cursor: 0,
        marked_files: Vec::new(),
        marked_groups: Vec::new(),
    }
}

fn first_group_id(app: &App) -> String {
    app.build_visible_items()
        .iter()
        .find_map(|item| match item {
            FileTreeItem::Group { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("a group row")
}

fn file_idx_at(app: &App, n: usize) -> usize {
    app.build_visible_items()
        .iter()
        .filter_map(|item| match item {
            FileTreeItem::File { file_idx, .. } => Some(*file_idx),
            _ => None,
        })
        .nth(n)
        .expect("that many file rows")
}

#[test]
fn the_tag_list_is_the_fixed_seven_from_the_spec() {
    assert_eq!(
        GROUPING_FEEDBACK_TAGS,
        [
            "too coarse",
            "too fine",
            "grouped by layer, not concern",
            "junk-drawer group",
            "bad group names",
            "wrong reading order",
            "tests split from their source",
        ]
    );
}

#[test]
fn a_review_with_no_grouping_is_never_prompted() {
    let mut app = ungrouped_app(PATHS.iter().map(|path| make_file(path)).collect());
    assert!(app.grouping.is_none(), "the fixture has no grouping at all");

    app.maybe_prompt_grouping_feedback();

    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.grouping_feedback.is_none());
}

#[test]
fn a_grouped_review_including_a_heuristics_only_one_is_prompted() {
    // `grouped_paths` enables grouping with no refine call configured, which
    // is the free path most reviews use — the ticket requires it to open the
    // prompt exactly like a refined grouping would.
    let mut app = grouped_paths(PATHS);
    assert!(app.grouping.is_some());

    app.maybe_prompt_grouping_feedback();

    assert_eq!(app.input_mode, InputMode::GroupingFeedback);
    assert!(app.grouping_feedback.is_some());
}

#[test]
fn a_submitted_verdict_silences_the_rest_of_the_session() {
    let mut app = grouped_paths(PATHS);
    app.maybe_prompt_grouping_feedback();
    app.feedback_set_verdict(GroupingVerdict::Useful);
    app.submit_grouping_feedback();
    assert_eq!(app.input_mode, InputMode::Normal);

    app.maybe_prompt_grouping_feedback();

    assert_eq!(
        app.input_mode,
        InputMode::Normal,
        "given feedback, the session never asks again"
    );
    assert!(app.grouping_feedback.is_none());
}

#[test]
fn a_skip_silences_the_rest_of_the_session() {
    let mut app = grouped_paths(PATHS);
    app.maybe_prompt_grouping_feedback();
    app.skip_grouping_feedback();
    assert_eq!(app.input_mode, InputMode::Normal);

    app.maybe_prompt_grouping_feedback();

    assert_eq!(
        app.input_mode,
        InputMode::Normal,
        "a skip silences the session just like a submitted verdict"
    );
    assert!(app.grouping_feedback.is_none());
}

#[test]
fn on_demand_feedback_reopens_after_a_skip() {
    let mut app = grouped_paths(PATHS);
    app.maybe_prompt_grouping_feedback();
    app.skip_grouping_feedback();

    app.open_grouping_feedback();

    assert_eq!(
        app.input_mode,
        InputMode::GroupingFeedback,
        "`:grouping feedback` is the only path back in after a skip"
    );
    assert!(app.grouping_feedback.is_some());
}

#[test]
fn a_missing_verdict_leaves_the_prompt_open_and_warns() {
    let mut app = grouped_paths(PATHS);
    app.open_grouping_feedback();

    app.submit_grouping_feedback();

    assert_eq!(
        app.input_mode,
        InputMode::GroupingFeedback,
        "the verdict is required, so submit refuses to close the prompt"
    );
    assert!(app.grouping_feedback.is_some());
    assert!(app.message.is_some(), "submit without a verdict must warn");
}

#[test]
fn tags_resolve_in_constant_order_not_click_order() {
    let draft_tags_after_selecting = |order: &[usize]| -> Vec<&'static str> {
        let mut draft = bare_draft();
        draft.verdict = Some(GroupingVerdict::Mixed);
        for &idx in order {
            draft.tags.insert(idx);
        }
        draft.into_feedback().expect("verdict was set").tags
    };

    // Selected in this call order: 4, then 1, then 6.
    let tags = draft_tags_after_selecting(&[4, 1, 6]);

    assert_eq!(
        tags,
        vec![
            "too fine",
            "bad group names",
            "tests split from their source"
        ]
    );
}

#[test]
fn an_empty_or_blank_note_yields_none() {
    let mut empty = bare_draft();
    empty.verdict = Some(GroupingVerdict::Useless);
    empty.note = String::new();
    assert!(
        empty
            .into_feedback()
            .expect("verdict was set")
            .note
            .is_none()
    );

    let mut blank = bare_draft();
    blank.verdict = Some(GroupingVerdict::Useless);
    blank.note = "   ".to_string();
    assert!(
        blank
            .into_feedback()
            .expect("verdict was set")
            .note
            .is_none()
    );
}

#[test]
fn a_note_yields_its_trimmed_text() {
    let mut draft = bare_draft();
    draft.verdict = Some(GroupingVerdict::Mixed);
    draft.note = "  the auth group is a junk drawer  ".to_string();

    let feedback = draft.into_feedback().expect("verdict was set");

    assert_eq!(
        feedback.note.as_deref(),
        Some("the auth group is a junk drawer")
    );
}

#[test]
fn the_draft_shows_the_accumulated_marks_not_a_picker() {
    let mut app = grouped_paths(PATHS);
    let group_id = first_group_id(&app);
    let file_a = file_idx_at(&app, 0);
    let file_b = file_idx_at(&app, 1);
    let path_a = app.diff_files[file_a].display_path().clone();
    let path_b = app.diff_files[file_b].display_path().clone();
    let group_name = app
        .session
        .groups
        .iter()
        .find(|group| group.id == group_id)
        .expect("the group exists")
        .name
        .clone();

    app.toggle_mark_for_file_idx(file_a);
    app.toggle_mark_for_file_idx(file_b);
    app.toggle_group_mark_by_id(&group_id);

    app.open_grouping_feedback();

    let draft = app.grouping_feedback.as_ref().expect("the prompt is open");
    assert_eq!(
        draft.marked_files,
        vec![PathBuf::from(&path_a), PathBuf::from(&path_b)]
    );
    assert_eq!(
        draft.marked_groups,
        vec![group_name],
        "the draft carries the group's name, not its id"
    );
}

#[test]
fn feedback_off_disables_the_prompt_and_the_marks() {
    let mut app = grouped_paths(PATHS);
    app.grouping_feedback_enabled = false;
    let file_idx = file_idx_at(&app, 0);
    let group_id = first_group_id(&app);

    app.maybe_prompt_grouping_feedback();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.grouping_feedback.is_none());

    app.open_grouping_feedback();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.grouping_feedback.is_none());
    assert!(
        app.message.is_some(),
        "open_grouping_feedback must warn when off"
    );

    app.toggle_mark_for_file_idx(file_idx);
    app.toggle_group_mark_by_id(&group_id);

    assert!(app.session.marked_files().is_empty());
    assert!(app.session.marked_group_ids().is_empty());
}

#[test]
fn submitting_feedback_writes_no_session_state() {
    let mut app = grouped_paths(PATHS);
    // `ReviewSession` carries no `PartialEq` (nested `Comment` values don't
    // need one for anything else), so the "unchanged" assertion compares its
    // serialized form — the same shape the session file itself is written in.
    let before = serde_json::to_value(&app.session).expect("session serializes");

    app.open_grouping_feedback();
    app.feedback_set_verdict(GroupingVerdict::Mixed);
    app.feedback_toggle_tag();
    app.feedback_note_insert_char('x');
    app.submit_grouping_feedback();

    let after = serde_json::to_value(&app.session).expect("session serializes");
    assert_eq!(
        after, before,
        "record_grouping_feedback writes no file and touches no session state; \
         gd-26r.43 owns the log"
    );
}

#[test]
fn the_prompt_never_moves_drift() {
    let mut app = grouped_paths(PATHS);
    let before = app.grouping.as_ref().expect("grouping computed").clone();

    app.maybe_prompt_grouping_feedback();
    app.feedback_set_verdict(GroupingVerdict::Useless);
    app.submit_grouping_feedback();

    let after_submit = app.grouping.as_ref().expect("grouping computed");
    assert_eq!(after_submit.drift(), before.drift());
    assert_eq!(after_submit.drift_percent(), before.drift_percent());

    app.open_grouping_feedback();
    app.skip_grouping_feedback();

    let after_skip = app.grouping.as_ref().expect("grouping computed");
    assert_eq!(after_skip.drift(), before.drift());
    assert_eq!(after_skip.drift_percent(), before.drift_percent());
}

// Assignment 2 (`gd-26r.42`): the key handler resolves a focus-agnostic
// `Action` against `FeedbackField`, so these drive `handle_grouping_feedback_action`
// directly rather than the model methods above.
mod key_handling {
    use super::*;
    use crate::handler::handle_grouping_feedback_action;
    use crate::input::Action;

    #[test]
    fn a_digit_sets_the_verdict_only_when_verdict_is_focused() {
        let mut app = grouped_paths(PATHS);
        app.open_grouping_feedback();
        assert_eq!(
            app.grouping_feedback.as_ref().unwrap().focus,
            FeedbackField::Verdict
        );

        handle_grouping_feedback_action(&mut app, Action::FeedbackChar('2'));

        assert_eq!(
            app.grouping_feedback.as_ref().unwrap().verdict,
            Some(GroupingVerdict::Mixed)
        );
    }

    #[test]
    fn the_same_digit_types_into_the_note_when_note_is_focused() {
        let mut app = grouped_paths(PATHS);
        app.open_grouping_feedback();
        app.feedback_focus_next(); // Verdict -> Tags
        app.feedback_focus_next(); // Tags -> Note

        handle_grouping_feedback_action(&mut app, Action::FeedbackChar('2'));

        let draft = app.grouping_feedback.as_ref().unwrap();
        assert_eq!(draft.note, "2");
        assert!(
            draft.verdict.is_none(),
            "the digit typed into the note must not also set the verdict"
        );
    }

    #[test]
    fn j_then_space_selects_the_second_tag_and_space_again_clears_it() {
        let mut app = grouped_paths(PATHS);
        app.open_grouping_feedback();
        app.feedback_focus_next(); // Verdict -> Tags

        handle_grouping_feedback_action(&mut app, Action::FeedbackChar('j'));
        handle_grouping_feedback_action(&mut app, Action::FeedbackChar(' '));

        let draft = app.grouping_feedback.as_ref().unwrap();
        assert_eq!(draft.tag_cursor, 1);
        assert_eq!(
            draft
                .tags
                .iter()
                .map(|&i| GROUPING_FEEDBACK_TAGS[i])
                .collect::<Vec<_>>(),
            vec!["too fine"]
        );

        handle_grouping_feedback_action(&mut app, Action::FeedbackChar(' '));

        assert!(app.grouping_feedback.as_ref().unwrap().tags.is_empty());
    }

    #[test]
    fn typing_then_backspacing_the_note_leaves_the_cursor_after_the_edit() {
        let mut app = grouped_paths(PATHS);
        app.open_grouping_feedback();
        app.feedback_focus_next();
        app.feedback_focus_next(); // Note

        for c in ['a', 'b', 'c'] {
            handle_grouping_feedback_action(&mut app, Action::FeedbackChar(c));
        }
        handle_grouping_feedback_action(&mut app, Action::FeedbackBackspace);

        let draft = app.grouping_feedback.as_ref().unwrap();
        assert_eq!(draft.note, "ab");
        assert_eq!(draft.note_cursor, 2);
    }

    #[test]
    fn enter_with_a_verdict_closes_the_prompt() {
        let mut app = grouped_paths(PATHS);
        app.open_grouping_feedback();
        handle_grouping_feedback_action(&mut app, Action::FeedbackChar('1'));

        handle_grouping_feedback_action(&mut app, Action::FeedbackSubmit);

        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.grouping_feedback.is_none());
    }

    #[test]
    fn esc_closes_the_prompt_and_silences_the_session() {
        let mut app = grouped_paths(PATHS);
        app.open_grouping_feedback();

        handle_grouping_feedback_action(&mut app, Action::FeedbackSkip);

        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.grouping_feedback.is_none());

        app.maybe_prompt_grouping_feedback();
        assert_eq!(
            app.input_mode,
            InputMode::Normal,
            "esc silences the session the same way skip_grouping_feedback does"
        );
    }
}
