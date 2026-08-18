//! Where the grouping engine meets the sidebar.
//!
//! The engine (`crate::grouping`) produces a grouping and nothing else. This
//! module owns the seam: it turns `diff_files` into a [`Changeset`], decides
//! whether the session already holds a grouping of it, flattens the result back
//! into `diff_files` order, and hands the sidebar the keys it expands rows
//! with. Nothing here re-decides anything the engine settled.
//!
//! The refine arm meets the seam at four fields, and `crate::app::refine` runs
//! on both sides of the TUI. Before the alternate screen exists it parks its
//! answer in [`App::pending_grouping`]; after it, the same wait parks it and
//! reorders immediately. Either way the seam treats a parked answer as one more
//! source of a grouping, ranked ahead of the session's own. It does **not**
//! re-sort what arrives: `Grouping::build` is the sole constructor and sorts
//! within groups by construction, so a refined group order lands already
//! ordered inside.
//!
//! The other three fields are the dispatch decision. [`App::refine_target_picked`]
//! is armed by [`App::reorder_for_load`] and consumed here; what it becomes is
//! [`App::refine_wanted`], which the main loop reads, and
//! [`App::refine_over_saved_grouping`], which tells the wait whether the
//! grouping it is about to replace was the session's or this load's.
//!
//! Drift is not a fresh pass. When the persisted table no longer describes the
//! changeset exactly, [`ReviewSession::grouping_for`] places what moved and
//! keeps every group identity the reader has been working through
//! (`docs/REGROUPING_STATE.md`); only a review the session never grouped, and
//! `:regroup` itself (`crate::app::regroup`), start from the heuristics.

use unicode_width::UnicodeWidthStr;

use super::*;
use crate::grouping::changeset::Changeset;
use crate::grouping::passes::GroupingConfig;

/// What a finished diff load means for the refine gate.
///
/// The target-pick sites — confirming a commit or a working-tree tab in the
/// selector, opening a PR from the picker — pass [`Self::NewTarget`]. Every
/// other load reorders the same review with different bytes and passes
/// [`Self::SameReview`], because each of them would otherwise cost a blocking
/// billed call of up to the full timeout over a live review
/// (`docs/REGROUPING_STATE.md`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TargetPick {
    /// The human just said what to review; this load may dispatch one refine.
    NewTarget,
    /// The file set moved inside a review already open; never refines.
    SameReview,
}

/// The grouping state the sidebar header is reporting, and the text it reports
/// it with (`gd-26r.15`).
///
/// The header is a border title, so the text is plain: a coloured run inside a
/// border line reads as a rendering artefact rather than as a badge. "Not
/// silent" is met by the slot being permanently present, not by being loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupingStatus {
    /// A `:regroup` refine call is in flight. Only `:regroup` reaches here:
    /// both startup waits block the TUI behind a screen of their own, so there
    /// is no header to put this in until they are over.
    Refining,
    /// The human pressed the cancel key. Worded as a choice, deliberately not
    /// as [`Self::HeuristicsOnly`]'s statement about the world.
    RefineCancelled,
    /// `[grouping].refine` is on and no group came back refined — offline, no
    /// credentials, rate-limited, timed out, or an envelope that would not
    /// parse. Said out loud so weak groups are not blamed on the heuristics
    /// when the refine pass never ran.
    HeuristicsOnly,
    /// The share of files placed by incremental assignment since the last full
    /// pass, hidden entirely at zero.
    Drift { percent: u32 },
}

impl GroupingStatus {
    /// The header suffix, given the cells the header has left for it.
    ///
    /// `room` only decides the drift chip's advisory tail: `· :regroup` renders
    /// when the whole of it fits and is dropped when it does not, because a
    /// truncated `· :regr` is worse than no advice at all. The other three are
    /// short enough that a header too narrow for them is too narrow for the
    /// counts as well.
    pub(crate) fn chip(self, room: usize) -> String {
        match self {
            GroupingStatus::Refining => "\u{00b7} refining ".to_string(),
            GroupingStatus::RefineCancelled => "\u{00b7} refine cancelled ".to_string(),
            GroupingStatus::HeuristicsOnly => "\u{00b7} heuristics only ".to_string(),
            GroupingStatus::Drift { percent } => {
                // Advisory only, naming the command. No conditional binding: a
                // key that exists only while a condition holds is a key nobody
                // learns.
                let advised = format!("\u{00b7} {percent}% new \u{00b7} :regroup ");
                if advised.width() <= room {
                    advised
                } else {
                    format!("\u{00b7} {percent}% new ")
                }
            }
        }
    }
}

impl App {
    /// Turns grouping on for the session and rebuilds the sidebar around it.
    ///
    /// Called once from the binary after config is read, mirroring how
    /// `file_tree_mode` is applied. `App::build` deliberately leaves grouping
    /// off: it is the shared constructor for every mode and test, and the
    /// grouped sidebar is a startup decision the binary makes.
    pub fn enable_grouping(&mut self) {
        self.grouping_enabled = true;
        self.sort_files_by_directory(true);
    }

    /// Turns grouping off and back on mid-session (`<leader>g`,
    /// `:set groups!`), switching between the grouped list and the plain
    /// directory tree.
    ///
    /// **The grouping is kept across the off state.** Toggling off is a change
    /// of sidebar, not a discard: `App::grouping` stays populated, so toggling
    /// back re-sorts `diff_files` from the grouping already in hand and never
    /// costs a pass, let alone a refine (`docs/REGROUPING_STATE.md`).
    ///
    /// Neither `expanded_dirs` nor `expanded_groups` comes out changed. They are
    /// separate sets precisely so that a toggle needs no remapping and no
    /// reseeding: each sidebar comes back arranged the way its reader left it,
    /// and toggling twice is the identity. The one exception is the first
    /// toggle *on* of a session that never had a grouping — `--no-grouping`, or
    /// a changeset that had nothing to group — where the grouping computed here
    /// has no sidebar state at all yet and is seeded expanded, exactly as
    /// startup would have seeded it.
    ///
    /// Selection is carried by the reorder itself, with `reset_position =
    /// false`: the file under the cursor is re-found by path in the new order,
    /// or `ensure_valid_tree_selection` falls back the way it already does for
    /// a file no row shows. Expanded hunk gaps go, as they do on every reorder
    /// (`gd-26r.26`).
    pub fn toggle_grouping(&mut self) {
        let first_grouped_view = self.grouping.is_none();
        // Both sets are put back byte for byte after the reorder, because the
        // reorder itself opens rows: it re-finds the reader's file by path and
        // `jump_to_file` reveals it, which would silently re-expand the very
        // group or directory the reader had collapsed around it. `:regroup`
        // has the same problem and answers it by collapsing everything
        // afterwards (`src/app/regroup.rs`); a toggle answers it by restoring,
        // because a toggle is not a new grouping and has nothing to re-read.
        // Selection then falls back to the collapsed container, which is what
        // `ensure_valid_tree_selection` is for.
        let dirs = self.expanded_dirs.clone();
        let groups = self.expanded_groups.clone();
        self.grouping_enabled = !self.grouping_enabled;
        self.sort_files_by_directory(false);
        self.expanded_dirs = dirs;
        self.expanded_groups = groups;
        if self.grouping_enabled && first_grouped_view {
            self.expand_all_group_keys();
        }
        self.ensure_valid_tree_selection();

        let status = match self.active_grouping() {
            Some(grouping) => format!("on ({} groups)", grouping.groups().len()),
            None if self.grouping_enabled => "on (nothing to group)".to_string(),
            None => "off".to_string(),
        };
        self.set_message(format!("Grouping: {status}"));
    }

    /// The grouping the sidebar is rendering, which is the session's grouping
    /// only while grouping is switched on.
    ///
    /// Every site that used to ask `self.grouping.is_some()` asks this instead.
    /// The two stopped meaning the same thing when `<leader>g` started leaving
    /// a grouping populated behind a directory tree.
    pub(in crate::app) fn active_grouping(&self) -> Option<&crate::grouping::Grouping> {
        self.grouping.as_ref().filter(|_| self.grouping_enabled)
    }

    /// What the sidebar header says about the grouping right now, or `None`
    /// when it says nothing: an ungrouped sidebar, and a grouped one that is
    /// refined, current and undrifted.
    ///
    /// **One chip at a time**, in the order below (`gd-26r.15`). In flight
    /// outranks unavailable because it is transient and about to answer the
    /// question the other states describe; unavailable outranks drift because
    /// "not the grouping you asked for" outranks "the grouping you asked for
    /// has moved". Joining them and letting the header truncate was rejected: a
    /// narrow sidebar would cut the file count the header exists for.
    ///
    /// Nothing here is reported while grouping is toggled off. The chrome
    /// belongs to the grouped view, and the staleness of a grouping that is not
    /// on screen is a fact about something the reader is not looking at.
    pub(crate) fn grouping_status(&self) -> Option<GroupingStatus> {
        let grouping = self.active_grouping()?;
        if self.pending_regroup.is_some() {
            return Some(GroupingStatus::Refining);
        }
        if self.refine_cancelled {
            return Some(GroupingStatus::RefineCancelled);
        }
        // Asked of the partition rather than of the last call's outcome, so a
        // reopened session that was refined yesterday keeps quiet and one that
        // never got a refined group says so however long ago it failed.
        let refined = grouping
            .groups()
            .iter()
            .any(|group| group.source == crate::grouping::GroupSource::Refined);
        if self.refine_config.is_some() && !refined {
            return Some(GroupingStatus::HeuristicsOnly);
        }
        // The same function the auto-regroup backstop thresholds on
        // (`src/app/regroup.rs`), in the same units, so the number that trips
        // it is the number this chip counted up to.
        let percent = grouping.drift_percent();
        if percent == 0 {
            return None;
        }
        Some(GroupingStatus::Drift { percent })
    }

    /// Reorders the sidebar for a diff that has just finished loading, arming
    /// the refine gate when that load answers a review target the human picked.
    ///
    /// Arming lives *here*, welded to the reorder that consumes it, rather than
    /// at the moment of the pick. A pick and its load are separated by a fetch
    /// that can fail or come back empty, and every one of those exits returns
    /// before any reorder happens. Arming up front leaves the flag set across
    /// such an exit, and the next reshuffle inside the review that is still
    /// open — an inline commit toggle, a `:reload` — spends it on a blocking
    /// billed call the human never asked for. Arming on the success path
    /// cannot leak that way, and a new early return added later cannot
    /// reintroduce the leak by omission.
    ///
    /// The sidebar keys are re-seeded here too. Every caller did it by hand and
    /// a load that forgot to would render its whole sidebar collapsed, so the
    /// reorder that invalidates the keys is what replaces them.
    pub(in crate::app) fn reorder_for_load(&mut self, pick: TargetPick) {
        if matches!(pick, TargetPick::NewTarget) {
            self.refine_target_picked = true;
        }
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
    }

    /// The `expanded_groups` key of a group row. The group id doubles as the
    /// key so nothing has to be remapped when a regroup renames a group.
    ///
    /// Group ids are the *only* keys `expanded_groups` holds, and directory
    /// paths the only ones `expanded_dirs` holds (`docs/SIDEBAR_MODEL.md`
    /// point 3). One set each rather than one set between them, since
    /// `<leader>g` lets both sidebars be arranged within one session.
    pub(in crate::app) fn group_row_key(group: &crate::grouping::Group) -> String {
        group.id.as_str().to_string()
    }

    /// The group a path belongs to, when the sidebar is grouped. The
    /// commit-message pseudo-file has none: it sits outside the partition.
    #[cfg(test)]
    pub(in crate::app) fn group_of_file(&self, path: &Path) -> Option<&crate::grouping::Group> {
        let grouping = self.active_grouping()?;
        grouping.group(grouping.group_of(path)?)
    }

    /// The `expanded_groups` key of the group a path belongs to. The assignment
    /// already names the id the key is made of, so the sidebar sites ask for
    /// the key rather than scanning the group list back out of it.
    pub(in crate::app) fn group_key_of_file(&self, path: &Path) -> Option<String> {
        let grouping = self.active_grouping()?;
        Some(grouping.group_of(path)?.as_str().to_string())
    }

    /// Restores the session's grouping, adopts a refined one, or computes a
    /// fresh heuristic one, and reorders `diff_files` to match: the
    /// commit-message pseudo-file first, then the groups in reading order, and
    /// inside each group the engine's within-group order.
    ///
    /// The three sources rank in that order for one reason each. A parked
    /// refine result wins, and is *taken* — a later reorder of a changed
    /// changeset must not re-apply a stale answer. It can only exist when a
    /// refine actually ran, which the wait itself refuses for a session that
    /// already held a grouping, so ranking it first does not weaken
    /// `docs/REGROUPING_STATE.md`; what it does allow is the in-TUI wait to
    /// replace the heuristic grouping this method itself recorded a moment
    /// earlier. The session's grouping comes next, so reopening brings
    /// yesterday's groups and yesterday's group-keyed sidebar state back
    /// intact. The heuristics are last and are the only source that cannot
    /// fail.
    ///
    /// The pseudo-file sits outside the partition entirely and keeps index 0
    /// (`docs/TOTAL_COVERAGE.md` Decision 1).
    pub(in crate::app) fn order_files_by_group(&mut self) {
        // Consumed whatever happens, including on the empty changeset a target
        // pick can produce: an arming that outlived its own load would fire the
        // wait over whatever the next reorder happened to be looking at.
        let picked = std::mem::take(&mut self.refine_target_picked);

        let changeset = Changeset::from_diff_files(&self.diff_files);
        if changeset.is_empty() {
            self.grouping = None;
            self.pending_grouping = None;
            self.refine_wanted = false;
            self.order_files_by_directory();
            return;
        }

        // Asked before the grouping is built, and asked of the *session* rather
        // than of what gets installed: `grouping_for` now answers for any drift
        // by placing the files that moved, so "the session had a table" no
        // longer means "this review was already grouped". A target the session
        // never grouped has to be refinable, and a reopened one must not be.
        let covered = self.session.covers(&changeset);

        let (grouping, from_saved_session) = match self.pending_grouping.take() {
            Some(parked) => (parked, false),
            None => match self.session.grouping_for(&changeset) {
                Some(saved) => (saved, covered),
                None => (
                    crate::grouping::group_changeset(&changeset, GroupingConfig::default()),
                    false,
                ),
            },
        };

        // Picking a target is the whole gate, and the only one. A refine costs
        // a blocking billed call, so it is spent on the one question the human
        // just asked — "review this" — and never on the file set moving under a
        // review already open.
        //
        // A reopened review arms it too. The wait it dispatches will refuse
        // (`Skipped::AlreadyGrouped`) and say so: `[grouping].refine = true`
        // with no wait, no warning and no refined group is indistinguishable
        // from the setting being ignored.
        // Only a pick decides this. Setting it unconditionally would let any
        // reorder that lands between the arming load and the main loop's check
        // — a filter, a `:reload` — quietly clear a wait the human paid for by
        // choosing what to review.
        if picked {
            self.refine_wanted = self.refine_config.is_some();
        }
        // Recorded before `record_grouping` below makes the question
        // unanswerable: from then on the session holds a grouping either way.
        self.refine_over_saved_grouping = from_saved_session;

        let rank: HashMap<PathBuf, usize> = grouping
            .assignments()
            .iter()
            .enumerate()
            .map(|(rank, assignment)| (assignment.path.clone(), rank))
            .collect();

        let mut ordered: Vec<DiffFile> = Vec::with_capacity(self.diff_files.len());
        let mut real: Vec<DiffFile> = Vec::with_capacity(self.diff_files.len());
        for file in self.diff_files.drain(..) {
            if file.is_commit_message {
                ordered.push(file);
            } else {
                real.push(file);
            }
        }
        // A file the grouping does not mention sorts last by path rather than
        // vanishing. The partition is total over real files, so this is
        // unreachable today; it is here because losing a file from the review
        // is a far worse failure than showing one in an odd place.
        real.sort_by_key(|file| {
            let path = file.display_path();
            (
                rank.get(path).copied().unwrap_or(usize::MAX),
                path.to_string_lossy().to_string(),
            )
        });
        ordered.extend(real);
        self.diff_files = ordered;

        self.session.record_grouping(&grouping);
        self.grouping = Some(grouping);
    }
}
