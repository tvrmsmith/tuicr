//! Where the grouping engine meets the sidebar.
//!
//! The engine (`crate::grouping`) produces a grouping and nothing else. This
//! module owns the seam: it turns `diff_files` into a [`Changeset`], decides
//! whether the session already holds a grouping of it, flattens the result back
//! into `diff_files` order, and hands the sidebar the keys it expands rows
//! with. Nothing here re-decides anything the engine settled.
//!
//! What is **not** here, named rather than implied (`gd-26r.28` slice A): the
//! refine arm and its blocking startup screen, `:regroup`, and incremental
//! assignment. Until those land the grouping is recomputed by the heuristics
//! whenever the persisted one no longer describes the changeset, which is
//! deterministic, instant and free.

use super::*;
use crate::grouping::changeset::Changeset;
use crate::grouping::passes::GroupingConfig;

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
    pub(in crate::app) fn group_of_file(&self, path: &Path) -> Option<&crate::grouping::Group> {
        let grouping = self.grouping.as_ref()?;
        grouping.group(grouping.group_of(path)?)
    }

    /// Restores the session's grouping, or computes a fresh heuristic one, and
    /// reorders `diff_files` to match: the commit-message pseudo-file first,
    /// then the groups in reading order, and inside each group the engine's
    /// within-group order.
    ///
    /// The pseudo-file sits outside the partition entirely and keeps index 0
    /// (`docs/TOTAL_COVERAGE.md` Decision 1).
    pub(in crate::app) fn order_files_by_group(&mut self) {
        let changeset = Changeset::from_diff_files(&self.diff_files);
        if changeset.is_empty() {
            self.grouping = None;
            self.order_files_by_directory();
            return;
        }

        let grouping = self.session.grouping_for(&changeset).unwrap_or_else(|| {
            crate::grouping::group_changeset(&changeset, GroupingConfig::default())
        });

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
