//! Incremental assignment: placing the files that appeared or moved since the
//! last full pass, without disturbing the ones that did not.
//!
//! This is the **only** automatic change to a grouping (`docs/REGROUPING_STATE.md`).
//! It runs on `:reload` and on a commit-selection change, it is heuristics-only,
//! and it only ever adds. The invariant the whole decision rests on is negative:
//! *unchanged files never move*. Between one full pass and the next, the sidebar
//! being read does not rearrange itself.
//!
//! What that buys is group **identity**. A full pass mints a fresh [`GroupId`]
//! for every group, so recomputing one because a single file appeared would
//! renumber every group under a reader who has been working through them. Here
//! the kept groups arrive with their persisted ids and leave with them.
//!
//! Three routes place a file, tried in this order and stopping at the first that
//! answers:
//!
//! 1. **The name the heuristics would file it under is already a group.** A new
//!    lockfile is `mechanical`, a new root markdown file is `docs`, a new file
//!    in an established token cluster carries that cluster's key. Joining by
//!    name is what makes the common case land where a full pass would have put
//!    it, at no cost.
//! 2. **It is similar enough to something already placed.** The same
//!    [`similarity`](super::passes::similarity) the absorb pass uses, scored
//!    against the kept members only, so one weak guess cannot chain onto
//!    another.
//! 3. **Neither, so it opens a new group** — appended last, marked
//!    [`GroupSource::Incremental`] and `new_since_full_pass`, because
//!    intent-centrality order is a property of a full pass and an unranked group
//!    belongs at the bottom next to the drive-bys rather than interleaved with
//!    groups that were ranked.

use std::collections::{BTreeMap, BTreeSet};

use super::changeset::{ChangedFile, Changeset};
use super::order;
use super::passes::{self, GroupingConfig, PassClaim};
use super::{GroupId, GroupSource, PresentedGroup};

/// Placed by joining a group that already existed.
const JOINED: &str = "incremental-join";
/// Placed into a group this assignment opened.
const OPENED: &str = "incremental-new";

/// The groups a grouping of `changeset` is built from, given the groups that
/// survived, and the claim per file the builder records as provenance.
///
/// `kept` is in reading order and carries only the members that kept their
/// persisted group id. Everything else in `changeset` is unplaced and is placed
/// here. Members `changeset` no longer holds are dropped: a group whose files
/// were deleted keeps its order slot and its id, so widening the range back
/// restores it rather than minting a second group for the same files.
pub(super) fn place(
    changeset: &Changeset,
    mut kept: Vec<PresentedGroup>,
    config: GroupingConfig,
) -> (Vec<PresentedGroup>, BTreeMap<String, PassClaim>) {
    let known: BTreeSet<&str> = changeset
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    for group in &mut kept {
        group.members.retain(|path| known.contains(path.as_str()));
    }

    let placed: BTreeSet<&str> = kept
        .iter()
        .flat_map(|group| group.members.iter().map(String::as_str))
        .collect();
    let unplaced: Vec<&ChangedFile> = changeset
        .files
        .iter()
        .filter(|file| !placed.contains(file.path.as_str()))
        .collect();
    if unplaced.is_empty() {
        return (kept, BTreeMap::new());
    }

    // The passes run over the **whole** changeset and only the arrivals' claims
    // are read. Running them over the arrivals alone would be cheaper and much
    // worse: a lone file carries no cluster, so a test file rejoining the
    // concern it has always belonged to would come back as a directory
    // fallback and be torn out of its group for being edited. Reading one
    // claim out of a full pass is not a full pass — nothing an unchanged file
    // was claimed for is acted on, which is the invariant that matters.
    let claimed: BTreeMap<String, PassClaim> = passes::assign(changeset, config)
        .into_iter()
        .map(|claim| (claim.path.clone(), claim))
        .collect();
    let by_path = order::files_by_path(changeset);

    let by_name: BTreeMap<&str, usize> = kept
        .iter()
        .enumerate()
        // First wins: two groups can share a name — the refine arm's repair
        // table only makes names unique within one answer — and reading order
        // is the tie-break everything else here uses.
        .map(|(index, group)| (group.name.as_str(), index))
        .fold(BTreeMap::new(), |mut names, (name, index)| {
            names.entry(name).or_insert(index);
            names
        });

    let mut claims: BTreeMap<String, PassClaim> = BTreeMap::new();
    let mut joined: Vec<(usize, String)> = Vec::new();
    let mut opened: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for file in &unplaced {
        let claim = claimed.get(file.path.as_str());
        let name = claim.map(|claim| claim.group.as_str());

        let target = name
            .and_then(|name| by_name.get(name).copied())
            .or_else(|| nearest_group(file, &kept, &by_path, config));

        match target {
            Some(index) => {
                claims.insert(
                    file.path.clone(),
                    PassClaim {
                        path: file.path.clone(),
                        group: kept[index].name.clone(),
                        pass: JOINED,
                        runner_up: None,
                    },
                );
                joined.push((index, file.path.clone()));
            }
            None => {
                // Every file the passes see is claimed — the directory fallback
                // claims unconditionally — so the `unwrap_or` is the total
                // partition's belt and braces, not a routine path.
                let name = name.unwrap_or(order::FALLBACK_PREFIX).to_string();
                claims.insert(
                    file.path.clone(),
                    PassClaim {
                        path: file.path.clone(),
                        group: name.clone(),
                        pass: OPENED,
                        runner_up: None,
                    },
                );
                opened.entry(name).or_default().push(file.path.clone());
            }
        }
    }

    for (index, path) in joined {
        kept[index].members.push(path);
    }

    // New groups are ranked among *themselves* by the same size-descending
    // proxy a full pass uses, and as a block they sit after every group that
    // was ranked by one. Existing group order is never reshuffled.
    for name in order::heuristic_group_order(&opened) {
        let members = opened.remove(&name).unwrap_or_default();
        kept.push(PresentedGroup {
            id: GroupId::new(),
            name,
            source: GroupSource::Incremental,
            new_since_full_pass: true,
            members,
        });
    }

    (kept, claims)
}

/// The kept group holding the file most like this one, when anything is like it
/// at all. Scored against the kept members as they stood on entry, so
/// absorption order cannot chain one guess onto another.
fn nearest_group(
    file: &ChangedFile,
    kept: &[PresentedGroup],
    by_path: &BTreeMap<&str, &ChangedFile>,
    config: GroupingConfig,
) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, group) in kept.iter().enumerate() {
        for member in &group.members {
            let Some(other) = by_path.get(member.as_str()).copied() else {
                continue;
            };
            let score = passes::similarity(file, other);
            // Strictly greater, so reading order breaks a tie: the earlier
            // group is the more central one and the more likely home.
            if score >= config.absorb_min_similarity
                && best.is_none_or(|(_, current)| score > current)
            {
                best = Some((index, score));
            }
        }
    }
    best.map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grouping::{Grouping, assign_incrementally, group_changeset};

    const CHANGESET: &str = "M\tsrc/auth/login.rs\n\
                             M\tsrc/auth/session.rs\n\
                             M\tsrc/auth/token.rs\n\
                             M\ttests/auth_test.rs\n\
                             M\tdocs/auth.md\n\
                             M\tCargo.lock\n";

    /// The full pass, and the same grouping re-presented as the input to an
    /// incremental one: this is the round trip the session makes on every
    /// `:reload`.
    fn full_pass(text: &str) -> (Changeset, Grouping) {
        let changeset = Changeset::parse(text);
        let grouping = group_changeset(&changeset, GroupingConfig::default());
        (changeset, grouping)
    }

    fn as_kept(grouping: &Grouping) -> Vec<PresentedGroup> {
        grouping
            .groups()
            .iter()
            .map(|group| PresentedGroup {
                id: group.id.clone(),
                name: group.name.clone(),
                source: group.source,
                new_since_full_pass: group.new_since_full_pass,
                members: grouping
                    .files_in(&group.id)
                    .map(|path| path.to_string_lossy().to_string())
                    .collect(),
            })
            .collect()
    }

    fn identities(grouping: &Grouping) -> Vec<(String, String)> {
        grouping
            .groups()
            .iter()
            .map(|group| (group.id.as_str().to_string(), group.name.clone()))
            .collect()
    }

    fn group_of<'a>(grouping: &'a Grouping, path: &str) -> &'a super::super::Group {
        let id = grouping
            .group_of(std::path::Path::new(path))
            .unwrap_or_else(|| panic!("{path} is in the partition"));
        grouping.group(id).expect("the id names a group")
    }

    /// Nothing arrived, so nothing may change — not the identities, not the
    /// order, not the membership. This is the `:reload` that found the same
    /// files, which is most of them.
    #[test]
    fn a_changeset_that_did_not_move_comes_back_untouched() {
        let (changeset, before) = full_pass(CHANGESET);
        let after = assign_incrementally(&changeset, as_kept(&before), GroupingConfig::default());

        assert_eq!(identities(&after), identities(&before));
        assert_eq!(
            after
                .assignments()
                .iter()
                .map(|a| (a.path.clone(), a.group_id.clone()))
                .collect::<Vec<_>>(),
            before
                .assignments()
                .iter()
                .map(|a| (a.path.clone(), a.group_id.clone()))
                .collect::<Vec<_>>()
        );
    }

    /// A file that appears joins the group a full pass would have filed it
    /// under, when that group is already there. No new group, no renumbering.
    #[test]
    fn an_arrival_joins_the_group_it_belongs_to() {
        let (_, before) = full_pass(CHANGESET);
        let grown = Changeset::parse(&format!("{CHANGESET}A\tsrc/auth/refresh.rs\n"));

        let after = assign_incrementally(&grown, as_kept(&before), GroupingConfig::default());

        assert_eq!(
            identities(&after),
            identities(&before),
            "no group was opened and none was renumbered"
        );
        assert_eq!(
            group_of(&after, "src/auth/refresh.rs").id,
            group_of(&before, "src/auth/login.rs").id,
            "it joined the group its neighbours are in"
        );
        assert_eq!(
            after
                .files_in(&group_of(&after, "src/auth/refresh.rs").id.clone())
                .map(|path| path.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec![
                "src/auth/login.rs",
                "src/auth/refresh.rs",
                "src/auth/session.rs",
                "src/auth/token.rs"
            ],
            "and the within-group sort was applied to the group it joined"
        );
    }

    /// A file with nothing in common with anything opens a group — last,
    /// unranked, and marked as both of those things, because intent-centrality
    /// order is a property of a full pass and this file has not had one.
    #[test]
    fn an_arrival_nothing_claims_opens_a_new_group_last() {
        let (_, before) = full_pass(CHANGESET);
        let grown = Changeset::parse(&format!("{CHANGESET}A\tinfra/terraform/main.tf\n"));

        let after = assign_incrementally(&grown, as_kept(&before), GroupingConfig::default());

        let opened = group_of(&after, "infra/terraform/main.tf");
        assert_eq!(opened.source, GroupSource::Incremental);
        assert!(opened.new_since_full_pass);
        assert_eq!(
            after.groups().last().map(|group| group.id.as_str()),
            Some(opened.id.as_str()),
            "an unranked group sits at the bottom next to the drive-bys"
        );
        assert_eq!(
            identities(&after)[..before.groups().len()],
            identities(&before)[..],
            "and every group that was ranked keeps its slot ahead of it"
        );
    }

    /// The partition is strict and total whatever arrives, because the builder
    /// is the same one the full pass goes through.
    #[test]
    fn the_partition_stays_strict_and_total() {
        let (_, before) = full_pass(CHANGESET);
        let grown = Changeset::parse(&format!(
            "{CHANGESET}A\tinfra/terraform/main.tf\nA\tsrc/auth/refresh.rs\nA\tpnpm-lock.yaml\n"
        ));

        let after = assign_incrementally(&grown, as_kept(&before), GroupingConfig::default());

        let mut placed: Vec<&str> = after
            .assignments()
            .iter()
            .map(|assignment| assignment.path.to_str().expect("utf-8 path"))
            .collect();
        let mut expected: Vec<&str> = grown.files.iter().map(|file| file.path.as_str()).collect();
        placed.sort_unstable();
        expected.sort_unstable();
        assert_eq!(placed, expected, "every file exactly once");
    }

    /// A group whose files all left the changeset keeps its identity and its
    /// order slot: narrowing a commit range must not lose the group its files
    /// come back to.
    #[test]
    fn a_group_the_changeset_no_longer_holds_keeps_its_slot() {
        let (_, before) = full_pass(CHANGESET);
        let narrowed = Changeset::parse("M\tsrc/auth/login.rs\nM\tsrc/auth/session.rs\n");

        let after = assign_incrementally(&narrowed, as_kept(&before), GroupingConfig::default());

        assert_eq!(
            identities(&after),
            identities(&before),
            "the emptied groups are still there, in order, with their ids"
        );
        assert_eq!(
            after.assignments().len(),
            2,
            "and hold nothing that is gone"
        );
    }
}
