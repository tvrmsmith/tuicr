//! Order scoring, for `GROUPING.md` rules 2, 3, 4 and 8 — the half of the
//! rules pairwise co-membership F1 cannot see.
//!
//! # What is measured, and why it is two numbers
//!
//! A grouping induces a total order on the changeset's files: groups in reading
//! order, and within each group its files in their listed order. Both arms
//! produce one (`GROUPS_CONTRACT.md`: reading order **is** the `Vec` position),
//! and the partition is total over real files (`TOTAL_COVERAGE.md` Decision 3),
//! so the computed and expected sequences always range over the *same* file set.
//! That is what makes a rank correlation well defined here with no matching
//! step: there is nothing to align, only to compare.
//!
//! Kendall's tau over that sequence is the metric, split in two by which pairs
//! it runs over:
//!
//! - [`OrderScore::tau_group`] — pairs of files the **expected** grouping puts
//!   in *different* groups. This is group reading order (rules 2, 3, 8).
//! - [`OrderScore::tau_within`] — pairs inside one expected group that a rule
//!   actually constrains (rule 4 and the three rules added with this metric).
//!   Unconstrained pairs are excluded rather than scored, so neither fixture's
//!   incidental path sort is graded as if it were intent.
//!
//! The two pair sets are exactly the split pairwise F1 already makes: F1 asks
//! *are these two files together?* of same-group pairs; tau asks *are they in
//! the right order?*. Different failures, complementary pair universes, so this
//! **combines with** F1 and replaces nothing.
//!
//! # Why tau and not a displacement score
//!
//! Tau reads on a fixed scale whatever the changeset size: `1.0` every pair in
//! the right order, `0.0` no better than shuffling, `-1.0` exactly reversed.
//! Negative numbers carrying "actively backwards" is the reason for keeping the
//! raw `[-1, 1]` range instead of rescaling to `[0, 1]`, where reversal and
//! chance would both read as "low".
//!
//! It also degrades in proportion to how far a group moved, which is the
//! property the ticket asked for: moving one group `k` slots inverts only the
//! pairs between it and the groups it crossed, so the penalty scales with the
//! files crossed, while a full reversal inverts every pair. `one group moved`
//! and `reversed` are therefore different numbers, not both "wrong" — see
//! `one_group_moved_scores_far_above_a_reversal`.
//!
//! # Bias
//!
//! `tau_group` flatters the refine arm **by construction** and every report of
//! it says so. The refine prompt asks in words for the groups in the order a
//! reviewer should read them (rule 2); the heuristic arm's order is
//! `gd-26r.12` Decision 3's *proxy* — member count descending, `dir:` groups
//! pinned last — which never claimed to be intent-centrality. A gap on
//! `tau_group` is partly the gap between an attempt and an admitted proxy.
//!
//! `tau_within` is not biased that way: the refine prompt says nothing about
//! within-group file order, so both arms are unprompted there.

use std::collections::{BTreeMap, BTreeSet};

use super::changeset::{ChangedFile, Changeset};
use super::score::Partition;

/// Prefix of the directory-fallback groups `gd-26r.12` Decision 3 pins last.
const FALLBACK_PREFIX: &str = "dir:";

#[derive(Debug, Clone)]
pub struct OrderScore {
    /// Kendall's tau over cross-group pairs: group reading order.
    /// `None` when the expected grouping has fewer than two groups.
    pub tau_group: Option<f64>,
    /// How many pairs `tau_group` ran over.
    pub group_pairs: u64,
    /// Kendall's tau over rule-constrained within-group pairs.
    /// `None` when no rule constrains any pair — which is a real answer about
    /// the fixture, not a zero.
    pub tau_within: Option<f64>,
    /// How many pairs `tau_within` ran over.
    pub within_pairs: u64,
    /// Satisfied and broken pair counts per rule.
    ///
    /// Reported alongside the pooled `tau_within` because the rules do not
    /// share a ceiling: scoring the *expected* order against its own rules
    /// gives less than 1.0 on both fixtures, so a pooled number alone would
    /// blame an arm for a constraint the ground truth also breaks.
    pub within_by_rule: BTreeMap<Rule, (u64, u64)>,
}

impl OrderScore {
    pub fn tau_for(&self, rule: Rule) -> Option<f64> {
        let (ok, broken) = self.within_by_rule.get(&rule).copied()?;
        tau(ok, broken)
    }
}

/// A grouping's groups in reading order. Held separately from [`Partition`] so
/// the pairwise scorer stays order-blind and its published numbers stay
/// comparable across this change.
#[derive(Debug, Clone)]
pub struct Ordered {
    pub partition: Partition,
    /// Group names, reading order first. A name the partition does not hold is
    /// ignored; a group the order omits is appended, name-sorted, so a
    /// malformed order degrades rather than dropping files from the sequence.
    pub order: Vec<String>,
}

impl Ordered {
    pub fn new(partition: Partition, order: Vec<String>) -> Self {
        Self { partition, order }
    }

    /// `gd-26r.12` Decision 3, which is what the heuristic arm ships: concern
    /// groups by member count descending, `dir:` fallback groups pinned last as
    /// a class and size-descending among themselves. Name breaks size ties so
    /// the order is total and reproducible.
    pub fn heuristic(partition: Partition) -> Self {
        let mut names: Vec<&String> = partition.groups.keys().collect();
        names.sort_by_key(|name| {
            (
                name.starts_with(FALLBACK_PREFIX),
                std::cmp::Reverse(partition.groups[*name].len()),
                name.as_str(),
            )
        });
        let order = names.into_iter().cloned().collect();
        Self { partition, order }
    }

    /// The same groups and the same reading order, with each group's files
    /// re-sorted to satisfy the within-group rules.
    ///
    /// This is a **pure sort over paths the engine already holds**: no model
    /// call, no token similarity, nothing nondeterministic. It exists because
    /// the measurement showed both arms breaking rule 4 on every pair — both
    /// build their groups from a path-keyed map, and `x.test.ts` sorts before
    /// `x.ts`, so a plain path sort puts every test ahead of the code it covers.
    ///
    /// Key, in order: mechanical files last (rule 8 inside a group), then
    /// directory, then stem — which is what puts a production file and its test
    /// adjacent, since `stem` strips the test marker — then production before
    /// test, then unit test before broad test, then path for a total order.
    pub fn sorted_within_groups(mut self, changeset: &Changeset) -> Self {
        let by_path: BTreeMap<&str, &ChangedFile> = changeset
            .files
            .iter()
            .map(|file| (file.path.as_str(), file))
            .collect();
        for members in self.partition.groups.values_mut() {
            members.sort_by_key(|path| match by_path.get(path.as_str()) {
                Some(file) => (
                    file.is_mechanical(),
                    file.dir().to_string(),
                    file.stem(),
                    file.is_test(),
                    file.is_broad_test(),
                    path.clone(),
                ),
                None => (
                    false,
                    String::new(),
                    String::new(),
                    false,
                    false,
                    path.clone(),
                ),
            });
        }
        self
    }

    /// The induced file sequence: groups in reading order, files within a group
    /// in their listed order.
    pub fn sequence(&self) -> Vec<&str> {
        let ordered = self.order.iter().chain(self.partition.groups.keys());
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut out: Vec<&str> = Vec::new();
        for name in ordered {
            if !seen.insert(name.as_str()) {
                continue;
            }
            let Some(members) = self.partition.groups.get(name) else {
                continue;
            };
            out.extend(members.iter().map(String::as_str));
        }
        out
    }

    /// Position of each path in [`Self::sequence`].
    fn ranks(&self) -> BTreeMap<&str, usize> {
        self.sequence()
            .into_iter()
            .enumerate()
            .map(|(index, path)| (path, index))
            .collect()
    }
}

/// One ordered pair a rule requires: `before` must precede `after`, and the
/// rule that says so, kept for the report so a low score can be attributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Constraint<'a> {
    pub before: &'a str,
    pub after: &'a str,
    pub rule: Rule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    /// Rule 4: a test follows the production file it covers.
    TestFollowsProduction,
    /// Rule 4, extended: broader tests come after unit tests.
    UnitBeforeBroadTest,
    /// Rule 8, applied inside a group: a mechanical file that landed in a
    /// concern group sorts to the group's tail.
    MechanicalLast,
    /// The group's central file — the one the group is named for — leads it.
    CentralFirst,
}

impl Rule {
    pub fn label(self) -> &'static str {
        match self {
            Rule::TestFollowsProduction => "test-follows-production",
            Rule::UnitBeforeBroadTest => "unit-before-broad-test",
            Rule::MechanicalLast => "mechanical-last",
            Rule::CentralFirst => "central-first",
        }
    }
}

/// The within-group orderings the rules require of `expected`, derived from the
/// **rules** rather than read off the fixture's own file order.
///
/// This is the deliberate choice that keeps the metric honest. Fixture 2's
/// members are a plain path sort in all 14 groups, so grading a computed order
/// against the fixture's literal sequence would score an accident. Grading
/// against the rules scores only what the rules actually say, and reports the
/// pair count so an empty constraint set is visible rather than silently
/// producing a perfect number.
pub fn within_group_constraints<'a>(
    changeset: &'a Changeset,
    expected: &'a Partition,
) -> Vec<Constraint<'a>> {
    let by_path: BTreeMap<&str, &ChangedFile> = changeset
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();

    let mut constraints: BTreeSet<Constraint<'a>> = BTreeSet::new();
    for (name, members) in &expected.groups {
        let files: Vec<&ChangedFile> = members
            .iter()
            .filter_map(|path| by_path.get(path.as_str()).copied())
            .collect();

        for (index, left) in files.iter().enumerate() {
            for right in files.iter().skip(index + 1) {
                for (before, after) in [(*left, *right), (*right, *left)] {
                    // Rule 4: the test follows **its** production file. Stem
                    // equality alone is not that relationship — fixture 1 holds
                    // four unrelated `github.ts` / `github.test.ts` pairs in
                    // different directories, which a stem-only match would
                    // constrain against each other and grade the fixture's own
                    // order as broken. Requiring the same directory, or a
                    // `__tests__` directory directly beneath it, is what "its"
                    // means.
                    if !before.is_test()
                        && after.is_test()
                        && before.stem() == after.stem()
                        && covers(before, after)
                    {
                        constraints.insert(Constraint {
                            before: before.path.as_str(),
                            after: after.path.as_str(),
                            rule: Rule::TestFollowsProduction,
                        });
                    }
                    // A broad test reads after the unit tests it sits above.
                    if before.is_test() && !before.is_broad_test() && after.is_broad_test() {
                        constraints.insert(Constraint {
                            before: before.path.as_str(),
                            after: after.path.as_str(),
                            rule: Rule::UnitBeforeBroadTest,
                        });
                    }
                    // Rule 8 inside a group. Rule 8 proper puts a *burst* of
                    // generated files in their own group; this catches the
                    // straggler that landed in a concern group anyway.
                    if !before.is_mechanical() && after.is_mechanical() {
                        constraints.insert(Constraint {
                            before: before.path.as_str(),
                            after: after.path.as_str(),
                            rule: Rule::MechanicalLast,
                        });
                    }
                }
            }
        }

        if let Some(central) = central_file(name, &files) {
            for file in &files {
                if file.path != central.path {
                    constraints.insert(Constraint {
                        before: central.path.as_str(),
                        after: file.path.as_str(),
                        rule: Rule::CentralFirst,
                    });
                }
            }
        }
    }
    constraints.into_iter().collect()
}

/// Whether `test` is plausibly the test *of* `production`: same directory, or a
/// `__tests__` directory directly beneath it. Callers have already matched the
/// stems; this is the locality half of "its production file".
fn covers(production: &ChangedFile, test: &ChangedFile) -> bool {
    let home = production.dir();
    test.dir() == home || test.dir() == format!("{home}/__tests__")
}

/// The file a group is named for: the production, non-mechanical member sharing
/// the most name tokens with the group name.
///
/// Requires a **unique** maximum and at least one shared token. A group whose
/// name matches two members equally well, or none, constrains nothing — the
/// conservative reading, because a wrong central file would invert every pair
/// in the group at once.
fn central_file<'a>(name: &str, files: &[&'a ChangedFile]) -> Option<&'a ChangedFile> {
    let wanted: BTreeSet<String> = name
        .split(['-', '_', '/', ':', ' '])
        .filter(|token| token.len() > 1)
        .map(str::to_ascii_lowercase)
        .collect();
    if wanted.is_empty() {
        return None;
    }

    let scored: Vec<(usize, &ChangedFile)> = files
        .iter()
        .filter(|file| !file.is_test() && !file.is_mechanical())
        .map(|file| {
            let tokens: BTreeSet<String> = file.name_tokens().into_iter().collect();
            (tokens.intersection(&wanted).count(), *file)
        })
        .filter(|(shared, _)| *shared > 0)
        .collect();

    let best = scored.iter().map(|(shared, _)| *shared).max()?;
    let mut winners = scored.iter().filter(|(shared, _)| *shared == best);
    let (_, winner) = winners.next()?;
    match winners.next() {
        Some(_) => None,
        None => Some(winner),
    }
}

/// Kendall's tau of `computed` against `expected`, split into the group-order
/// and within-group halves.
pub fn score_order(changeset: &Changeset, computed: &Ordered, expected: &Ordered) -> OrderScore {
    let computed_rank = computed.ranks();
    let expected_rank = expected.ranks();
    let expected_group = expected.partition.group_of();

    // Group order: every pair the expected grouping separates. A pair the
    // computed grouping happens to co-group still has a defined relative
    // position in the induced sequence, so no case needs special handling —
    // which is why a partition error degrades this score rather than voiding it.
    let paths: Vec<&str> = expected_rank.keys().copied().collect();
    let (mut concordant, mut discordant) = (0u64, 0u64);
    for (index, left) in paths.iter().enumerate() {
        for right in paths.iter().skip(index + 1) {
            if expected_group.get(left) == expected_group.get(right) {
                continue;
            }
            let (Some(cl), Some(cr)) = (computed_rank.get(left), computed_rank.get(right)) else {
                continue;
            };
            let expected_first = expected_rank[left] < expected_rank[right];
            let computed_first = cl < cr;
            if expected_first == computed_first {
                concordant += 1;
            } else {
                discordant += 1;
            }
        }
    }
    let group_pairs = concordant + discordant;
    let tau_group = tau(concordant, discordant);

    let (mut ok, mut broken) = (0u64, 0u64);
    let mut within_by_rule: BTreeMap<Rule, (u64, u64)> = BTreeMap::new();
    for constraint in within_group_constraints(changeset, &expected.partition) {
        let (Some(before), Some(after)) = (
            computed_rank.get(constraint.before),
            computed_rank.get(constraint.after),
        ) else {
            continue;
        };
        let entry = within_by_rule.entry(constraint.rule).or_default();
        if before < after {
            ok += 1;
            entry.0 += 1;
        } else {
            broken += 1;
            entry.1 += 1;
        }
    }

    OrderScore {
        tau_group,
        group_pairs,
        tau_within: tau(ok, broken),
        within_pairs: ok + broken,
        within_by_rule,
    }
}

/// `(C - D) / (C + D)`: `1.0` all in order, `0.0` chance, `-1.0` reversed.
/// `None` rather than `0.0` on no pairs, because "nothing to measure" and "as
/// good as a shuffle" are answers a report must not confuse.
fn tau(concordant: u64, discordant: u64) -> Option<f64> {
    let total = concordant + discordant;
    if total == 0 {
        return None;
    }
    Some((concordant as f64 - discordant as f64) / total as f64)
}
