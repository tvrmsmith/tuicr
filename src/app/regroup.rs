//! `:regroup` mid-session: the full recompute the human asks for, and the
//! landing that puts it under a reader without losing their place.
//!
//! **The startup wait blocks and this one does not.** That asymmetry is the
//! ruling in `docs/MID_SESSION_REGROUP.md`, argued rather than assumed: startup
//! blocks because there is nothing else to do, and mid-review there
//! demonstrably is — you are reading a file, and that reading does not depend
//! on the sidebar being re-sorted. Freezing the TUI for 30–70s here would make
//! `:regroup` a command you avoid. So the refine call runs on a detached thread
//! and [`App::poll_regroup`] collects it from the main loop, exactly as the PR
//! pollers do.
//!
//! **One landing per `:regroup`, not two.** With refine off the heuristic pass
//! lands immediately, because it is instant and free. With refine on the
//! grouping on screen is left alone until the answer arrives: landing the
//! heuristic pass first and the refined one 30–70s later would reshuffle the
//! sidebar twice for one command, and the second reshuffle is the one that was
//! asked for. Every way the call can end — a refusal, a timeout, an unreadable
//! body twice — lands the heuristic pass computed at dispatch instead, so the
//! command always ends in a regrouped review.
//!
//! **The auto-regroup backstop shares that landing.** `:regroup` is the human's
//! lever; [`App::poll_regroup_backstop`] is the one that pulls itself, once
//! drift crosses a deliberately high threshold. It is heuristics-only and never
//! refine, so unlike `:regroup` it is free, instant and cannot surprise anyone
//! with a bill — and it lands through the same [`App::land`] rather than a
//! second path, because the two landings behave identically by ruling
//! (`docs/MID_SESSION_REGROUP.md`).
//!
//! What a landing does is `docs/MID_SESSION_REGROUP.md` verbatim: all groups
//! collapse, the diff pane does not move, and sidebar selection lands on the
//! collapsed group row holding the current file. Group-keyed sidebar state
//! needs no remapping across it, and that is not luck — `expanded_groups` is
//! cleared wholesale by the collapse, so the new grouping's ids are the only
//! keys any row can be looking for.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use super::App;
use super::refine::{ATTEMPTS, RefineCall, RefineOutcome, live_call};
use crate::grouping::changeset::Changeset;
use crate::grouping::passes::GroupingConfig;
use crate::grouping::refine;
use crate::grouping::{Grouping, group_changeset};

/// A refine call in flight for a `:regroup`, and everything needed to retry it,
/// to time it out, and to fall back without asking the passes twice.
pub(crate) struct PendingRegroup {
    answers: Receiver<Result<String, String>>,
    call: RefineCall,
    prompt: String,
    timeout: Duration,
    deadline: Instant,
    attempt: u32,
    /// The changeset the call was dispatched over. An answer is only applied
    /// while the review still holds exactly these paths.
    changeset: Changeset,
    /// The full pass run at dispatch: the prompt was built from it, and it is
    /// what lands if the call does not come back usable.
    heuristic: Grouping,
}

impl App {
    /// `:regroup`. The only path to a full recompute, and the only path to
    /// refine after a session's first grouping (`docs/REGROUPING_STATE.md`).
    pub fn regroup(&mut self) {
        let call = self
            .refine_config
            .as_ref()
            .map(|config| live_call(config.settings.clone()));
        self.regroup_with(call);
    }

    /// The seam the tests drive: the same command with the refine call
    /// supplied, and `None` for the review that is not refining at all.
    pub(crate) fn regroup_with(&mut self, call: Option<RefineCall>) {
        if !self.grouping_enabled {
            self.set_error(
                "Grouping is off for this review; turn it on with <leader>g or :set groups!.",
            );
            return;
        }
        let changeset = Changeset::from_diff_files(&self.diff_files);
        if changeset.is_empty() {
            self.set_error("Nothing to regroup.");
            return;
        }

        // A full pass, so every group is minted fresh: `:regroup` is the moment
        // the human asked for a new shape, and keeping the old identities would
        // only preserve sidebar state the landing discards anyway.
        let heuristic = group_changeset(&changeset, GroupingConfig::default());

        let Some(call) = call.filter(|_| self.refine_config.is_some()) else {
            let groups = heuristic.groups().len();
            self.land(heuristic);
            self.set_message(format!("Regrouped: {groups} groups."));
            return;
        };

        let timeout = self
            .refine_config
            .as_ref()
            .map(|config| config.timeout)
            .unwrap_or(Duration::from_secs(0));
        let prompt = refine::prompt(&changeset, &heuristic);
        let files = changeset.len();

        // A refine the human asked for again is no longer a refine the human
        // walked away from, whatever this one goes on to do.
        self.refine_cancelled = false;

        // Replaces whatever was in flight. A second `:regroup` is a second
        // question, and the first answer is dropped with its receiver rather
        // than landing later over the top of this one.
        self.pending_regroup = Some(PendingRegroup {
            answers: dispatch(&call, &prompt, timeout),
            call,
            prompt,
            timeout,
            deadline: Instant::now() + timeout,
            attempt: 1,
            changeset,
            heuristic,
        });
        self.set_message(format!(
            "Refining groups over {files} files; the review stays open."
        ));
    }

    /// Collects a `:regroup` refine call, if one is in flight. Returns whether
    /// anything changed, which is the main loop's redraw signal.
    ///
    /// Nothing is drawn per tick: the wait is not blocking, so there is no
    /// elapsed clock a reader is sitting in front of. The in-flight and
    /// unavailable states are `gd-26r.15`'s to surface.
    pub fn poll_regroup(&mut self) -> bool {
        let Some(mut pending) = self.pending_regroup.take() else {
            return false;
        };

        let outcome = match pending.answers.try_recv() {
            Ok(Ok(body)) => {
                match refine::apply(&body, &pending.changeset, &pending.heuristic) {
                    // Applied only while the answer still describes the review.
                    // A `:reload` or a commit switch during the call leaves a
                    // partition over paths that are no longer on screen; it is
                    // dropped rather than repaired onto a changeset nobody
                    // asked about, and the heuristic pass goes with it because
                    // it is just as stale.
                    Ok(_) if !self.still_holds(&pending.changeset) => {
                        self.set_warning(
                            "The review changed while refine was in flight; the regroup was \
                             discarded.",
                        );
                        return true;
                    }
                    Ok(refined) => {
                        let repairs = refined.repairs.len();
                        self.land(refined.grouping);
                        RefineOutcome::Refined { repairs }
                    }
                    // The one retryable failure, and only once: a body that
                    // yields no JSON. The retry is a fresh one-shot call with
                    // the identical prompt and its own full timeout, so the
                    // single-shot contract holds literally.
                    Err(reason) if pending.attempt < ATTEMPTS => {
                        pending.answers = dispatch(&pending.call, &pending.prompt, pending.timeout);
                        pending.deadline = Instant::now() + pending.timeout;
                        pending.attempt += 1;
                        self.pending_regroup = Some(pending);
                        self.set_message(format!("Refine answer unreadable ({reason}); retrying."));
                        return true;
                    }
                    Err(reason) => self.fall_back(pending, RefineOutcome::Failed(reason)),
                }
            }
            Ok(Err(reason)) => self.fall_back(pending, RefineOutcome::Failed(reason)),
            Err(TryRecvError::Disconnected) => self.fall_back(
                pending,
                RefineOutcome::Failed("the refine call ended without answering".to_string()),
            ),
            Err(TryRecvError::Empty) => {
                if Instant::now() < pending.deadline {
                    self.pending_regroup = Some(pending);
                    return false;
                }
                self.fall_back(pending, RefineOutcome::TimedOut)
            }
        };

        match outcome.warning() {
            Some(warning) => self.set_warning(warning),
            None => self.set_message("Regrouped."),
        }
        true
    }

    /// The auto-regroup backstop (`gd-26r.36`): the full heuristic pass the
    /// review takes on its own once drift crosses
    /// [`App::regroup_threshold`]. Returns whether anything changed, which is
    /// the main loop's redraw signal.
    ///
    /// **Heuristics-only, never refine** (`docs/REGROUPING_STATE.md`). It does
    /// not consult [`App::refine_config`] and cannot reach a call: a threshold
    /// crossing that silently spent \$1.40–2.00 and three minutes and reshuffled
    /// the sidebar mid-review is exactly the surprise the grouping work exists
    /// to prevent. Refine costs money, so refine is something the human asks
    /// for with `:regroup`.
    ///
    /// **It fires on [`crate::grouping::Grouping::drift_percent`]**, the one
    /// drift number the sidebar chip renders. A backstop that fired at a value
    /// the reader never watched approach would be a surprise rather than a
    /// backstop, so it does not compute its own fraction — it inherits the
    /// group-derived reading of drift with it (`gd-26r.37`), which means a file
    /// that joined an established group moves neither the chip nor this.
    ///
    /// Polled from the main loop rather than called from the loads that create
    /// drift, for three reasons. It is a landing under the reader, which is
    /// what the loop already does with [`App::poll_regroup`]; the loads that
    /// drift a grouping are a dozen call sites and one forgetting it would be
    /// a silent gap; and the loop is the line between startup and mid-session,
    /// so a reopened session whose persisted drift is already over the
    /// threshold cannot regroup before its reader has seen it, which
    /// `docs/REGROUPING_STATE.md` forbids outright.
    pub fn poll_regroup_backstop(&mut self) -> bool {
        if self.regroup_threshold == 0 {
            return false;
        }
        // Nothing fires under an in-flight `:regroup`. The human has already
        // asked for the full pass this would land, and the answer to that
        // question is the one they want: landing a heuristic pass first would
        // reshuffle the sidebar twice, and the second reshuffle is the
        // requested one. The fallback arm lands a heuristic pass anyway if the
        // call fails, which is this backstop's outcome by another route.
        if self.pending_regroup.is_some() {
            return false;
        }
        // Asked of the grouping the sidebar is *rendering*: with `<leader>g`
        // off the reader is looking at the directory tree, the chip is silent,
        // and a landing they cannot see is one they cannot understand when
        // they toggle back.
        let Some(grouping) = self.active_grouping() else {
            return false;
        };
        let percent = grouping.drift_percent();
        // The crossing has to happen *under* the reader. A session reopened
        // over the threshold is left alone — "reopening a session never
        // regroups; yesterday's review shows yesterday's groups"
        // (`docs/REGROUPING_STATE.md`) — and a full pass on the first frame
        // would mint fresh ids over exactly the groups the reader came back
        // to. So the first poll latches whatever drift the review opened on
        // and fires nothing; from then on the backstop answers movement.
        let Some(before) = self.drift_when_last_polled.replace(percent) else {
            // The opening value, whatever it is.
            return false;
        };
        if before == percent || (percent as usize) < self.regroup_threshold {
            return false;
        }

        let changeset = Changeset::from_diff_files(&self.diff_files);
        if changeset.is_empty() {
            return false;
        }
        let regrouped = group_changeset(&changeset, GroupingConfig::default());
        let groups = regrouped.groups().len();
        // Slice C's landing, not a second one: all groups collapse, the diff
        // pane does not move, and selection lands on the collapsed group row
        // holding the current file (`docs/MID_SESSION_REGROUP.md`).
        self.land(regrouped);
        // Said out loud, and said as an explanation. A sidebar that rearranged
        // itself is worth a sentence naming what crossed and what it cost,
        // which is nothing.
        self.set_message(format!(
            "{percent}% of the review had drifted; regrouped from the heuristics into {groups} \
             groups."
        ));
        true
    }

    /// Every way the call can fail lands the same thing: the full heuristic
    /// pass computed at dispatch, which is total by construction and already
    /// carries the within-group sort.
    fn fall_back(&mut self, pending: PendingRegroup, outcome: RefineOutcome) -> RefineOutcome {
        if self.still_holds(&pending.changeset) {
            self.land(pending.heuristic);
            return outcome;
        }
        RefineOutcome::Failed(
            "the review changed while refine was in flight; the regroup was discarded".to_string(),
        )
    }

    /// Whether the review is still over exactly the paths a call was dispatched
    /// over. Order is not part of the question: a landing reorders `diff_files`
    /// by definition.
    fn still_holds(&self, changeset: &Changeset) -> bool {
        let now: BTreeSet<String> = Changeset::from_diff_files(&self.diff_files)
            .files
            .into_iter()
            .map(|file| file.path)
            .collect();
        let then: BTreeSet<String> = changeset
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        now == then
    }

    /// The landing policy of `docs/MID_SESSION_REGROUP.md`, in the order the
    /// three rules have to be applied in.
    ///
    /// The diff pane is undisturbed: the same file at the same line, re-found
    /// by path, which is why the reorder runs with `reset_position = false`.
    /// Every expanded hunk gap is discarded, which `sort_files_by_directory`
    /// does for every reorder since `gd-26r.26`. And **all groups collapse**,
    /// last, because the reorder's own `jump_to_file` reveals the current
    /// file's group on its way past: the collapse is what takes it back down,
    /// and what then leaves selection on the group row rather than on a file
    /// row that is no longer visible.
    ///
    /// Nothing remaps `expanded_groups`. The collapse empties it, so a group
    /// that survived this grouping and one that did not are in exactly the same
    /// position — no stale group id can be left behind to expand a row that no
    /// longer exists. `expanded_dirs` is untouched: the ungrouped tree is not
    /// what the human asked to recompute, and `<leader>g` will show it to them
    /// as they left it.
    ///
    /// **The one landing path.** `:regroup`, a refine answer and the
    /// `gd-26r.36` backstop all reach here rather than any of their own — the
    /// alternative, `record_grouping` (`src/model/review.rs`), is only reached
    /// through `sort_files_by_directory` → `order_files_by_group`, which is
    /// skipped whenever `grouping_enabled` is off, and `<leader>g` can flip
    /// that mid-flight while a `:regroup` refine call is still out. `land` is
    /// the only place a landing is unconditional, which is why the
    /// grouping-feedback bookkeeping (`gd-26r.41`) lives here: counting a
    /// landing and dropping group marks — **not** file marks, which are
    /// path-keyed and stay true of a file that just moved — has to happen
    /// exactly once per landing, on every path that produces one.
    fn land(&mut self, grouping: Grouping) {
        self.session.record_landing();
        self.session.clear_group_marks();
        // `landing_count` and a cleared `marked_groups` are session state like
        // any comment or reviewed flag, so a landing has to be saved like one.
        self.dirty = true;

        let was = self.diff_state.current_file_idx;
        let relative_line = self
            .diff_state
            .cursor_line
            .saturating_sub(self.calculate_file_scroll_offset(was));
        let viewport_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(self.diff_state.scroll_offset);

        self.pending_regroup = None;
        self.pending_grouping = Some(grouping);
        self.sort_files_by_directory(false);
        // `line_annotations` is built by walking `diff_files` in order, so the
        // repartition above left every annotation pointing at the file that
        // used to be at its index.
        self.rebuild_annotations();

        if !self.diff_files.is_empty() {
            let idx = self
                .diff_state
                .current_file_idx
                .min(self.diff_files.len() - 1);
            let start = self.calculate_file_scroll_offset(idx);
            let height = self.file_render_height(idx, &self.diff_files[idx]);
            self.diff_state.current_file_idx = idx;
            self.diff_state.cursor_line = start + relative_line.min(height.saturating_sub(1));

            let max_scroll = self.max_scroll_offset();
            let highest = self.diff_state.viewport_height.max(1).saturating_sub(1);
            self.diff_state.scroll_offset = self
                .diff_state
                .cursor_line
                .saturating_sub(viewport_offset.min(highest))
                .min(max_scroll);
            self.ensure_cursor_visible();
            // The current file is the one the reorder re-found by path, and it
            // stays that way. Re-deriving it from the cursor line instead would
            // hand the answer to `calculate_file_scroll_offset`, which cannot
            // separate files that render to the same offset — an empty diff, a
            // binary, a file whose hunks are all collapsed — and would silently
            // move the reader to whichever of them comes first.
        }

        self.collapse_all_dirs();
    }
}

/// One attempt, on a thread nobody joins. The send fails harmlessly once the
/// receiver is dropped, which is what supersession and a discarded regroup both
/// come down to.
fn dispatch(
    call: &RefineCall,
    prompt: &str,
    timeout: Duration,
) -> Receiver<Result<String, String>> {
    let (answers, answer) = mpsc::channel();
    let worker = Arc::clone(call);
    let sent = prompt.to_string();
    std::thread::spawn(move || {
        let _ = answers.send(worker(&sent, timeout));
    });
    answer
}
