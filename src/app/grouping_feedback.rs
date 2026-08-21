//! Grouping feedback (`gd-26r.42`): the verdict prompt shown at `:send` and
//! `:w`, or opened on demand with `:grouping feedback`.
//!
//! `gd-26r.41` let the reader mark a group or file as wrong while they read.
//! This ticket asks the question the marks alone cannot answer: was the
//! grouping as a whole worth the time it saved? The prompt shows the
//! accumulated marks rather than offering a picker — at 400 files nothing can
//! offer a list to tick, which is exactly why marking happens as you read.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::{App, InputMode};
use crate::text_edit::{
    delete_char_before, delete_word_before, next_char_boundary, prev_char_boundary,
};

/// The reader's verdict on the grouping as a whole. Exactly three points
/// (`gd-26r.18`): a binary turns "mostly fine, one junk-drawer group" into
/// noise, and five invites false precision on a two-second judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupingVerdict {
    Useful,
    Mixed,
    Useless,
}

/// A source constant, never a config key: a user-editable list makes the
/// log unaggregatable over time (`gd-26r.18`, following `gd-26r.38`'s call on
/// the size caps).
///
/// Deliberately absent: "group too big" (the caps already own it, and the `!`
/// marker already reports it) and "files in the wrong group" (that is
/// `gd-26r.41`'s file marker, not a tag).
pub const GROUPING_FEEDBACK_TAGS: &[&str] = &[
    "too coarse",
    "too fine",
    "grouped by layer, not concern",
    "junk-drawer group",
    "bad group names",
    "wrong reading order",
    "tests split from their source",
];

/// Which part of the prompt has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackField {
    Verdict,
    Tags,
    Note,
}

/// The prompt's in-progress state. Built when the prompt opens and consumed
/// (via [`Self::into_feedback`]) when it submits.
pub struct GroupingFeedbackDraft {
    pub verdict: Option<GroupingVerdict>,
    /// Indices into [`GROUPING_FEEDBACK_TAGS`].
    pub tags: BTreeSet<usize>,
    pub note: String,
    pub note_cursor: usize,
    pub focus: FeedbackField,
    pub tag_cursor: usize,
    /// Snapshot of `session.marked_files()` taken when the prompt opens.
    pub marked_files: Vec<PathBuf>,
    /// Snapshot of the marked groups' *names* (not ids — see
    /// [`App::open_feedback_draft`]) taken when the prompt opens.
    pub marked_groups: Vec<String>,
}

/// What the prompt yields on submit. `gd-26r.43` logs it; nothing reads it
/// today.
pub struct GroupingFeedback {
    pub verdict: GroupingVerdict,
    pub tags: Vec<&'static str>,
    pub note: Option<String>,
}

impl GroupingFeedbackDraft {
    fn new(marked_files: Vec<PathBuf>, marked_groups: Vec<String>) -> Self {
        Self {
            verdict: None,
            tags: BTreeSet::new(),
            note: String::new(),
            note_cursor: 0,
            focus: FeedbackField::Verdict,
            tag_cursor: 0,
            marked_files,
            marked_groups,
        }
    }

    /// Consumes the draft into the value the log wants, or `None` when no
    /// verdict was given. Pure and testable without an `App`; callers that
    /// need to enforce the required-verdict warning check `verdict` before
    /// calling this, not after.
    ///
    /// Tags resolve in constant order (ascending index), never click order,
    /// and the note is `None` when empty or whitespace-only.
    pub fn into_feedback(self) -> Option<GroupingFeedback> {
        let verdict = self.verdict?;
        let tags = self
            .tags
            .into_iter()
            .map(|idx| GROUPING_FEEDBACK_TAGS[idx])
            .collect();
        let trimmed = self.note.trim();
        let note = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        Some(GroupingFeedback {
            verdict,
            tags,
            note,
        })
    }
}

impl App {
    /// Opens the prompt if it hasn't already run its course for this
    /// session. Fired by `:send` and `:w`. Silent when it declines: a review
    /// with no grouping, feedback turned off, or a session that already
    /// submitted or skipped has nothing new to ask.
    pub fn maybe_prompt_grouping_feedback(&mut self) {
        if !self.grouping_feedback_enabled
            || self.grouping.is_none()
            || self.grouping_feedback_settled
        {
            return;
        }
        self.open_feedback_draft();
    }

    /// The `:grouping feedback` on-demand path. Ignores
    /// [`Self::grouping_feedback_settled`] — this is the only way back in
    /// after a skip — but still refuses when the feature is off or there is
    /// no grouping to vote on, warning either way.
    pub fn open_grouping_feedback(&mut self) {
        if !self.grouping_feedback_enabled {
            self.set_warning(
                "Grouping feedback is off (`[grouping].feedback = false`); nothing to open.",
            );
            return;
        }
        if self.grouping.is_none() {
            self.set_warning("This review has no grouping; there is nothing to vote on.");
            return;
        }
        self.open_feedback_draft();
    }

    fn open_feedback_draft(&mut self) {
        let marked_files: Vec<PathBuf> = self
            .session
            .marked_files()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let marked_groups: Vec<String> = self
            .session
            .marked_group_ids()
            .into_iter()
            .map(|id| {
                self.session
                    .groups
                    .iter()
                    .find(|group| group.id == id)
                    .map(|group| group.name.clone())
                    .unwrap_or_else(|| id.to_string())
            })
            .collect();
        self.grouping_feedback = Some(GroupingFeedbackDraft::new(marked_files, marked_groups));
        self.input_mode = InputMode::GroupingFeedback;
    }

    /// Enter. Refuses when no verdict was given — the prompt stays open and
    /// warns — otherwise records the feedback, marks the session settled and
    /// closes.
    pub fn submit_grouping_feedback(&mut self) {
        let Some(draft) = self.grouping_feedback.take() else {
            return;
        };
        if draft.verdict.is_none() {
            self.set_warning("A verdict is required: useful, mixed, or useless.");
            self.grouping_feedback = Some(draft);
            return;
        }
        let feedback = draft
            .into_feedback()
            .expect("verdict presence checked above");
        self.grouping_feedback_settled = true;
        self.input_mode = InputMode::Normal;
        self.record_grouping_feedback(feedback);
    }

    /// Esc. Silences the session — no further prompt until reopened or asked
    /// for on demand — and writes nothing.
    pub fn skip_grouping_feedback(&mut self) {
        self.grouping_feedback_settled = true;
        self.grouping_feedback = None;
        self.input_mode = InputMode::Normal;
    }

    /// The seam `gd-26r.43` consumes. Today it only confirms receipt: it
    /// writes no file and touches no session state, so a test asserting
    /// nothing changed under a submit needs nothing beyond this doc comment
    /// to trust.
    fn record_grouping_feedback(&mut self, _feedback: GroupingFeedback) {
        self.set_message("Grouping feedback recorded.");
    }

    /// No-op when the prompt is closed.
    pub fn feedback_focus_next(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.focus = match draft.focus {
            FeedbackField::Verdict => FeedbackField::Tags,
            FeedbackField::Tags => FeedbackField::Note,
            FeedbackField::Note => FeedbackField::Verdict,
        };
    }

    /// No-op when the prompt is closed.
    pub fn feedback_focus_prev(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.focus = match draft.focus {
            FeedbackField::Verdict => FeedbackField::Note,
            FeedbackField::Tags => FeedbackField::Verdict,
            FeedbackField::Note => FeedbackField::Tags,
        };
    }

    /// No-op when the prompt is closed.
    pub fn feedback_set_verdict(&mut self, verdict: GroupingVerdict) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.verdict = Some(verdict);
    }

    /// No-op when the prompt is closed.
    pub fn feedback_tag_cursor_down(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        if draft.tag_cursor + 1 < GROUPING_FEEDBACK_TAGS.len() {
            draft.tag_cursor += 1;
        }
    }

    /// No-op when the prompt is closed.
    pub fn feedback_tag_cursor_up(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.tag_cursor = draft.tag_cursor.saturating_sub(1);
    }

    /// No-op when the prompt is closed.
    pub fn feedback_toggle_tag(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        let idx = draft.tag_cursor;
        if !draft.tags.remove(&idx) {
            draft.tags.insert(idx);
        }
    }

    /// No-op when the prompt is closed.
    pub fn feedback_note_insert_char(&mut self, ch: char) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.note.insert(draft.note_cursor, ch);
        draft.note_cursor += ch.len_utf8();
    }

    /// No-op when the prompt is closed.
    pub fn feedback_note_delete_char_before(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.note_cursor = delete_char_before(&mut draft.note, draft.note_cursor);
    }

    /// No-op when the prompt is closed.
    pub fn feedback_note_delete_word_before(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.note_cursor = delete_word_before(&mut draft.note, draft.note_cursor);
    }

    /// No-op when the prompt is closed.
    pub fn feedback_note_cursor_left(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.note_cursor = prev_char_boundary(&draft.note, draft.note_cursor);
    }

    /// No-op when the prompt is closed.
    pub fn feedback_note_cursor_right(&mut self) {
        let Some(draft) = self.grouping_feedback.as_mut() else {
            return;
        };
        draft.note_cursor = next_char_boundary(&draft.note, draft.note_cursor);
    }
}
