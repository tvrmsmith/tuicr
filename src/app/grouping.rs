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

    /// The `expanded_dirs` key of a group row. The group id doubles as the key
    /// so nothing has to be remapped when a regroup renames a group.
    ///
    /// Group ids are the *only* keys `expanded_dirs` holds while grouping is
    /// on, and directory paths the only ones it holds while grouping is off
    /// (`docs/SIDEBAR_MODEL.md` point 3), so the two key spaces are never
    /// populated at once and cannot collide.
    pub(in crate::app) fn group_row_key(group: &crate::grouping::Group) -> String {
        group.id.as_str().to_string()
    }

    /// The group a path belongs to, when there is a grouping. The
    /// commit-message pseudo-file has none: it sits outside the partition.
    #[cfg(test)]
    pub(in crate::app) fn group_of_file(&self, path: &Path) -> Option<&crate::grouping::Group> {
        let grouping = self.grouping.as_ref()?;
        grouping.group(grouping.group_of(path)?)
    }

    /// The `expanded_dirs` key of the group a path belongs to. The assignment
    /// already names the id the key is made of, so the sidebar sites ask for
    /// the key rather than scanning the group list back out of it.
    pub(in crate::app) fn group_key_of_file(&self, path: &Path) -> Option<String> {
        let grouping = self.grouping.as_ref()?;
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
