//! The grouping engine: a changeset in, an ordered strict partition out.
//!
//! The design records this implements are `docs/GROUPING.md` (the 14 rules),
//! `docs/GROUPS_CONTRACT.md` (this module's types), `docs/TOTAL_COVERAGE.md`
//! (why the partition is total and where the commit-message row sits) and
//! `docs/GROUPING_PASSES.md` (every number the passes are held to). The fixture
//! harness in `tests/grouping_prototype.rs` scores this code directly.
//!
//! Two arms produce a grouping. [`group_changeset`] is the heuristic one:
//! deterministic, instant, free, and the fallback every other path lands on.
//! [`refine`] is the model one, opt-in behind `[grouping].refine`, blocking at
//! startup, and worth the block for its partition (`docs/GROUPING_PASSES.md`).
//!
//! What is **not** here yet, named rather than implied: `:regroup` with
//! incremental assignment, decided in `docs/REGROUPING_STATE.md` and landing in
//! a later slice of `gd-26r.28`. Until then no group is ever new since the last
//! full pass.

pub mod changeset;
pub mod order;
pub mod passes;
pub mod refine;
pub mod vertex;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use changeset::Changeset;
use order::CentralFirst;
use passes::{GroupingConfig, PassClaim};

/// A group's stable identity (`gd-26r.8`). Opaque: not the name, never the
/// name, because a refine run may rename a group it otherwise leaves intact and
/// a name key makes that indistinguishable from every file in it moving.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId(String);

impl GroupId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Rehydrates an id persisted on a previous run. The only other way to get
    /// one, and the reason the inner string is not public: an id is minted or
    /// restored, never derived from anything a rename could change.
    pub fn from_persisted(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for GroupId {
    fn default() -> Self {
        Self::new()
    }
}

/// What produced a group. Per-group, not global: a refine response that omits
/// paths leaves them in their heuristic groups, so one partition can mix
/// sources (`docs/TOTAL_COVERAGE.md` Decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupSource {
    Heuristics,
    Refined,
    Incremental,
}

impl GroupSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupSource::Heuristics => "heuristics",
            GroupSource::Refined => "refined",
            GroupSource::Incremental => "incremental",
        }
    }

    pub fn from_persisted(text: &str) -> Self {
        match text {
            "refined" => GroupSource::Refined,
            "incremental" => GroupSource::Incremental,
            _ => GroupSource::Heuristics,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Group {
    /// Stable opaque identity (`gd-26r.8`).
    pub id: GroupId,
    /// Display only, rendered verbatim (`docs/SIDEBAR_MODEL.md`): no
    /// prettification, no title-casing. A generic name next to a large count is
    /// the cheapest signal that the grouping is weak there, and papering over
    /// it hides the signal.
    pub name: String,
    pub source: GroupSource,
    /// Set when opened by incremental assignment (`docs/REGROUPING_STATE.md`).
    pub new_since_full_pass: bool,
}

/// Where one file landed. `pass` and `runner_up` are debug-only derived state,
/// recomputed on every regroup and never persisted.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub path: PathBuf,
    pub group_id: GroupId,
    pub pass: &'static str,
    /// Recorded on close calls only (~14% of files). The sidebar deliberately
    /// surfaces none of it until a human can act on it (`gd-26r.18`).
    pub runner_up: Option<RunnerUp>,
}

#[derive(Debug, Clone)]
pub struct RunnerUp {
    pub group: String,
    pub reason: String,
}

/// An ordered set of groups, and the placement of every changeset path into
/// exactly one of them.
///
/// **Reading order is `Vec` position at both levels.** `groups` is in group
/// reading order, and `assignments` is in file reading order — the groups in
/// turn, and inside each group the within-group sort. There is no `order` field
/// at either level to fall out of sync with the slice.
#[derive(Debug, Clone, Default)]
pub struct Grouping {
    groups: Vec<Group>,
    assignments: Vec<Assignment>,
}

/// One group on its way into [`Grouping::build`]: everything except the
/// reading order, which is this value's position in the slice, and the
/// within-group order, which the builder puts on.
pub struct PresentedGroup {
    pub id: GroupId,
    pub name: String,
    pub source: GroupSource,
    pub new_since_full_pass: bool,
    /// Members in any order. The builder sorts them, so no caller — not even a
    /// session file — can hand back a grouping the within-group rules do not
    /// hold on.
    pub members: Vec<String>,
}

impl Grouping {
    /// **The one place a presented grouping is built**, and therefore the only
    /// place the within-group sort is applied.
    ///
    /// `gd-26r.27` made the sort part of *producing* a grouping rather than a
    /// variant of reporting one, so no path can build a grouping and skip it —
    /// not the heuristic arm, not the refine arm, not a session restore. The
    /// two public constructors differ only in where the groups and their
    /// identities come from; both funnel through here.
    fn build(
        changeset: &Changeset,
        mut groups: Vec<PresentedGroup>,
        mut claims: BTreeMap<String, PassClaim>,
    ) -> Self {
        let by_path = order::files_by_path(changeset);
        let mut assignments = Vec::new();
        let mut built = Vec::with_capacity(groups.len());

        for group in &mut groups {
            order::sort_group_files(&group.name, &mut group.members, &by_path, CentralFirst::On);
        }
        for group in groups {
            for path in group.members {
                let claim = claims.remove(&path);
                assignments.push(Assignment {
                    path: PathBuf::from(path),
                    group_id: group.id.clone(),
                    pass: claim.as_ref().map_or("restored", |claim| claim.pass),
                    runner_up: claim.and_then(|claim| claim.runner_up),
                });
            }
            built.push(Group {
                id: group.id,
                name: group.name,
                source: group.source,
                new_since_full_pass: group.new_since_full_pass,
            });
        }

        Self {
            groups: built,
            assignments,
        }
    }

    /// A freshly computed grouping: the passes' claims bucketed by name, put
    /// into `gd-26r.12` Decision 3's group order, with a new identity minted
    /// for every group.
    ///
    /// The order is computed here rather than passed in because it is a
    /// function of the very buckets this method builds, and a caller that
    /// bucketed the claims a second time to ask for it could disagree with what
    /// is presented.
    pub fn present(changeset: &Changeset, claims: Vec<PassClaim>, source: GroupSource) -> Self {
        let mut members: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut claim_by_path: BTreeMap<String, PassClaim> = BTreeMap::new();
        for claim in claims {
            members
                .entry(claim.group.clone())
                .or_default()
                .push(claim.path.clone());
            claim_by_path.insert(claim.path.clone(), claim);
        }
        let order = order::heuristic_group_order(&members);

        let mut names: Vec<String> = order
            .iter()
            .filter(|name| members.contains_key(*name))
            .cloned()
            .collect();
        names.extend(members.keys().filter(|name| !order.contains(name)).cloned());

        let presented = names
            .into_iter()
            .map(|name| PresentedGroup {
                id: GroupId::new(),
                members: members.remove(&name).unwrap_or_default(),
                name,
                source,
                new_since_full_pass: false,
            })
            .collect();

        Self::build(changeset, presented, claim_by_path)
    }

    /// A grouping whose groups and identities came from somewhere other than
    /// the passes, `groups` already in reading order: a persisted session, or
    /// the refine arm's answer.
    ///
    /// Identity is restored rather than minted — the point of an opaque
    /// `group_id` (`docs/REGROUPING_STATE.md`): reopening a session shows
    /// yesterday's groups, with yesterday's group-keyed sidebar state still
    /// pointing at them. From the refine arm the incoming order is the reading
    /// order the model proposed, which is the whole of what blocking startup
    /// buys in ordering terms and the one thing the response is required to
    /// carry (`docs/GROUPS_CONTRACT.md`).
    ///
    /// Within-group order is **not** taken from either source. It arrives here
    /// unsorted and leaves sorted, because [`Grouping::build`] is the sole
    /// constructor: the same rules 4, 12 and 14 that order a heuristic group
    /// order a refined one, for free and identically, so the seam never
    /// re-sorts and the prompt never asks.
    ///
    /// Which pass claimed a file is *not* carried: it is derived debug state,
    /// never persisted, and no heuristic pass placed a refined assignment at
    /// all, so `pass` reads `restored`.
    pub fn restore(changeset: &Changeset, groups: Vec<PresentedGroup>) -> Self {
        Self::build(changeset, groups, BTreeMap::new())
    }

    /// Groups in reading order.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// Every assignment in file reading order.
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }

    pub fn group_of(&self, path: &Path) -> Option<&GroupId> {
        self.assignments
            .iter()
            .find(|assignment| assignment.path == path)
            .map(|assignment| &assignment.group_id)
    }

    pub fn group(&self, id: &GroupId) -> Option<&Group> {
        self.groups.iter().find(|group| &group.id == id)
    }

    /// Files of one group, in reading order.
    pub fn files_in(&self, id: &GroupId) -> impl Iterator<Item = &Path> {
        self.assignments
            .iter()
            .filter(move |assignment| &assignment.group_id == id)
            .map(|assignment| assignment.path.as_path())
    }

    /// How many files each pass claimed. Debug surface and the fixture
    /// harness's `pass_hits`; never rendered.
    pub fn pass_hits(&self) -> BTreeMap<&'static str, usize> {
        let mut hits = BTreeMap::new();
        for assignment in &self.assignments {
            *hits.entry(assignment.pass).or_default() += 1;
        }
        hits
    }

    /// Assignments where a second group was nearly as good a fit.
    pub fn close_calls(&self) -> impl Iterator<Item = &Assignment> {
        self.assignments
            .iter()
            .filter(|assignment| assignment.runner_up.is_some())
    }
}

/// The heuristic arm: the passes, then `gd-26r.12` Decision 3's group order,
/// then the within-group sort. Deterministic, instant, free, and total over
/// every real file in the changeset.
pub fn group_changeset(changeset: &Changeset, config: GroupingConfig) -> Grouping {
    let claims = passes::assign(changeset, config);
    Grouping::present(changeset, claims, GroupSource::Heuristics)
}
