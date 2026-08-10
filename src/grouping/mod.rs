//! The grouping engine: a changeset in, an ordered strict partition out.
//!
//! The design records this implements are `docs/GROUPING.md` (the 14 rules),
//! `docs/GROUPS_CONTRACT.md` (this module's types), `docs/TOTAL_COVERAGE.md`
//! (why the partition is total and where the commit-message row sits) and
//! `docs/GROUPING_PASSES.md` (every number the passes are held to). The fixture
//! harness in `tests/grouping_prototype.rs` scores this code directly.
//!
//! What is **not** here yet, named rather than implied: the refine arm and its
//! blocking startup screen, and `:regroup` with incremental assignment. Both
//! are decided (`docs/MID_SESSION_REGROUP.md`, `docs/REGROUPING_STATE.md`) and
//! land in later slices of `gd-26r.28`. Until then every group is
//! [`GroupSource::Heuristics`] and no group is ever new since the last full
//! pass.

pub mod changeset;
pub mod order;
pub mod passes;

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

impl Grouping {
    /// **The one constructor for a presented grouping**, and the only place the
    /// within-group sort is applied.
    ///
    /// Taking the changeset is deliberate: `gd-26r.27` made the sort part of
    /// *producing* a grouping rather than a variant of reporting one, so there
    /// is no path that builds a grouping and skips it — not the heuristic arm,
    /// not the refine arm, not a grouping restored from a session.
    ///
    /// `order` names the groups in reading order. A name it does not mention is
    /// appended name-sorted rather than dropped, so a malformed order degrades
    /// the reading order instead of losing files.
    pub fn present(
        changeset: &Changeset,
        claims: Vec<PassClaim>,
        order: &[String],
        source: GroupSource,
    ) -> Self {
        let mut members: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut claim_by_path: BTreeMap<String, PassClaim> = BTreeMap::new();
        for claim in claims {
            members
                .entry(claim.group.clone())
                .or_default()
                .push(claim.path.clone());
            claim_by_path.insert(claim.path.clone(), claim);
        }

        let by_path = order::files_by_path(changeset);
        for (name, paths) in &mut members {
            order::sort_group_files(name, paths, &by_path, CentralFirst::On);
        }

        let mut names: Vec<String> = order
            .iter()
            .filter(|name| members.contains_key(*name))
            .cloned()
            .collect();
        names.extend(members.keys().filter(|name| !order.contains(name)).cloned());

        let mut groups = Vec::with_capacity(names.len());
        let mut assignments = Vec::new();
        for name in names {
            let id = GroupId::new();
            for path in members.remove(&name).unwrap_or_default() {
                let claim = claim_by_path.remove(&path);
                assignments.push(Assignment {
                    path: PathBuf::from(&path),
                    group_id: id.clone(),
                    pass: claim.as_ref().map_or("restored", |claim| claim.pass),
                    runner_up: claim.and_then(|claim| claim.runner_up),
                });
            }
            groups.push(Group {
                id,
                name,
                source,
                new_since_full_pass: false,
            });
        }

        Self {
            groups,
            assignments,
        }
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
    let mut members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for claim in &claims {
        members
            .entry(claim.group.clone())
            .or_default()
            .push(claim.path.clone());
    }
    let order = order::heuristic_group_order(&members);
    Grouping::present(changeset, claims, &order, GroupSource::Heuristics)
}
