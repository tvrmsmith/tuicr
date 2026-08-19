//! The group size caps (`gd-26r.23`): three constants, and the one split pass
//! that holds a full pass to two of them.
//!
//! **Constants, not configuration.** The human's call, taken against the
//! recommendation to make them config keys: there is no `[grouping]` key, no
//! `KNOWN_KEYS` entry, and adjusting a cap on the evidence of a week of real
//! reviews costs a rebuild. That cost was stated and accepted.
//!
//! [`SOFT_CAP`] has no engine behaviour at all. It is a number in the refine
//! prompt (`super::refine::prompt`) and nothing else — no warning, no marker,
//! no chip. A reader told "this group has 22 files" can only re-run `:regroup`,
//! toggle grouping off, or do nothing, and the flush-right count column already
//! shows the number.
//!
//! [`REFINE_CAP`] and [`HEURISTIC_CAP`] are engine invariants of a **full
//! pass**: no partition a full pass presents holds a group over the cap of the
//! arm that produced it, unless [`enforce`] could not split it — and then the
//! group carries [`PresentedGroup::unbounded`] and the sidebar says so with
//! `!`. Two caps rather than one because a refine group claims to be a concern
//! and is held tight, while a heuristic group is structural and a looser bound
//! avoids shredding a legitimate directory.
//!
//! What is deliberately *not* here: any minimum. A lone generated file or a
//! single doc is a legitimate group of one, so a split that leaves a piece
//! holding one file has done its job.

use std::collections::{BTreeMap, BTreeSet};

use super::refine::free_name;
use super::{GroupId, GroupSource, PresentedGroup};

/// The size a group should not exceed, as the refine prompt is told it. Not an
/// engine rule: nothing in this module or any other compares a group against
/// it.
pub const SOFT_CAP: usize = 20;

/// The hard cap on a group the refine arm produced.
pub const REFINE_CAP: usize = 25;

/// The hard cap on a group the heuristic arm produced. Looser than
/// [`REFINE_CAP`]: a heuristic group is a structural claim, and splitting a
/// coherent directory at 25 shreds it for nothing.
pub const HEURISTIC_CAP: usize = 30;

/// Separator between a split piece's parent name and its directory suffix.
const PIECE_SEPARATOR: &str = " \u{b7} ";

/// The suffix a piece of files that live at the repository root takes.
const ROOT_SUFFIX: &str = "root";

/// The cap a group is held to, keyed on what produced *that group* rather than
/// on the arm the pass ran as. One refine response routinely carries groups of
/// both sources — a group left holding only restored paths presents as
/// heuristics (`docs/GROUPS_CONTRACT.md`) — and holding such a group to the
/// tighter cap would split a partition the refine pass never touched.
pub fn cap_for(source: GroupSource) -> usize {
    match source {
        GroupSource::Refined => REFINE_CAP,
        GroupSource::Heuristics | GroupSource::Incremental => HEURISTIC_CAP,
    }
}

/// Splits every over-cap group by directory, in place in the reading order.
///
/// **One shared path for both arms** (`gd-26r.23`): the model answers freely
/// and the heuristics group freely, and whatever either produces is bounded
/// here afterwards. Enforcing the cap through the `GROUPS_CONTRACT` repair path
/// instead was rejected — it would make the cap part of the contract and count
/// a legitimate large answer into the repair warning — and instruction-only
/// enforcement on refine was rejected on direct evidence: the human's 236-file
/// run put 130 files in one group and that group came out of a refine pass.
///
/// **One pass.** A piece still over the cap once the directory split is done is
/// accepted as it is and marked. Recursing until compliant yields oddly
/// specific names from deep paths, and numbered shards as a last resort draw a
/// boundary that means nothing; a marker that says "the engine gave up here" is
/// worth more to a reader than either.
///
/// Callers are the two full passes only. Incremental assignment does not run
/// this: a group row must not split in two under a reader mid-review, so an
/// arrival that pushes a group past its cap is left alone and the drift it adds
/// brings the `gd-26r.36` backstop's own full pass along in time
/// (`docs/REGROUPING_STATE.md`).
pub fn enforce(groups: Vec<PresentedGroup>) -> Vec<PresentedGroup> {
    let mut used: BTreeSet<String> = groups.iter().map(|group| group.name.clone()).collect();
    let mut out = Vec::with_capacity(groups.len());

    for group in groups {
        let cap = cap_for(group.source);
        if group.members.len() <= cap {
            out.push(group);
            continue;
        }

        let buckets = by_directory(&group.members);
        // One directory holding the whole group is the flat case the single
        // pass cannot bound: the only cut left inside it would be an arbitrary
        // one. The group keeps every file it had and says so.
        if buckets.len() < 2 {
            out.push(PresentedGroup {
                unbounded: true,
                ..group
            });
            continue;
        }

        let dirs: Vec<&str> = buckets.keys().copied().collect();
        let suffixes = distinguishing_suffixes(&dirs);
        for (dir, members) in buckets {
            // The parent name is kept rather than discarded: the pass that
            // produced it earned it, and the common prefix of the directories
            // under it carries no information a narrow sidebar row can spare.
            let suffix = &suffixes[dir];
            let name = free_name(&format!("{}{PIECE_SEPARATOR}{suffix}", group.name), &used);
            used.insert(name.clone());
            out.push(PresentedGroup {
                // A split group is not the group that was there before, so no
                // piece inherits its identity. Group-keyed sidebar state points
                // at a group that no longer exists either way, and handing the
                // id to one arbitrary piece would make that piece the survivor
                // for no reason a reader could name.
                id: GroupId::new(),
                name,
                source: group.source,
                new_since_full_pass: group.new_since_full_pass,
                unbounded: members.len() > cap,
                members,
            });
        }
    }

    out
}

/// The members bucketed by their full containing directory, the buckets keyed
/// so they come back in path order. Files at the repository root bucket under
/// `""`.
fn by_directory(members: &[String]) -> BTreeMap<&str, Vec<String>> {
    let mut buckets: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for path in members {
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        buckets.entry(dir).or_default().push(path.clone());
    }
    buckets
}

/// The shortest trailing run of path components that tells each directory apart
/// from the others being split out of the same group.
///
/// `src/agent/transport` and `src/agent/grouping` differ at their last
/// component, so the pieces read `agent-transport · transport` and
/// `agent-transport · grouping`; `a/shared` and `b/shared` need both. The
/// common prefix is what a reader already knows from the parent name, and a
/// sidebar row is narrow.
fn distinguishing_suffixes<'a>(dirs: &[&'a str]) -> BTreeMap<&'a str, String> {
    dirs.iter()
        .map(|dir| {
            let components: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
            if components.is_empty() {
                return (*dir, ROOT_SUFFIX.to_string());
            }
            let suffix = (1..=components.len())
                .map(|len| components[components.len() - len..].join("/"))
                .find(|candidate| {
                    dirs.iter()
                        .filter(|other| *other != dir)
                        .all(|other| !ends_at_component(other, candidate))
                })
                .unwrap_or_else(|| components.join("/"));
            (*dir, suffix)
        })
        .collect()
}

/// Whether `dir` ends with `suffix` on a component boundary, so `src/grouping`
/// is not read as ending with `ouping`.
fn ends_at_component(dir: &str, suffix: &str) -> bool {
    dir == suffix || dir.ends_with(&format!("/{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, source: GroupSource, members: &[&str]) -> PresentedGroup {
        PresentedGroup {
            id: GroupId::new(),
            name: name.to_string(),
            source,
            new_since_full_pass: false,
            unbounded: false,
            members: members.iter().map(|path| (*path).to_string()).collect(),
        }
    }

    fn paths(count: usize, dir: &str) -> Vec<String> {
        (0..count).map(|i| format!("{dir}/file{i:03}.rs")).collect()
    }

    fn spread(dirs: &[(&str, usize)]) -> Vec<String> {
        dirs.iter()
            .flat_map(|(dir, count)| paths(*count, dir))
            .collect()
    }

    fn presented(name: &str, source: GroupSource, members: Vec<String>) -> PresentedGroup {
        PresentedGroup {
            id: GroupId::new(),
            name: name.to_string(),
            source,
            new_since_full_pass: false,
            unbounded: false,
            members,
        }
    }

    fn names(groups: &[PresentedGroup]) -> Vec<&str> {
        groups.iter().map(|group| group.name.as_str()).collect()
    }

    fn sizes(groups: &[PresentedGroup]) -> Vec<usize> {
        groups.iter().map(|group| group.members.len()).collect()
    }

    #[test]
    fn the_two_arms_are_held_to_their_own_caps() {
        let members = spread(&[("src/a", 14), ("src/b", 13)]);
        assert_eq!(members.len(), 27);

        let refined = enforce(vec![presented(
            "auth",
            GroupSource::Refined,
            members.clone(),
        )]);
        assert_eq!(
            names(&refined),
            vec!["auth \u{b7} a", "auth \u{b7} b"],
            "27 files is over the refine cap of 25 and must split"
        );

        let heuristic = enforce(vec![presented("auth", GroupSource::Heuristics, members)]);
        assert_eq!(
            names(&heuristic),
            vec!["auth"],
            "27 files is under the heuristic cap of 30 and must be left alone"
        );
    }

    #[test]
    fn an_over_cap_heuristic_group_splits_by_directory() {
        let split = enforce(vec![presented(
            "github-client",
            GroupSource::Heuristics,
            spread(&[("src/api", 20), ("src/ui", 11)]),
        )]);

        assert_eq!(
            names(&split),
            vec!["github-client \u{b7} api", "github-client \u{b7} ui"]
        );
        assert_eq!(sizes(&split), vec![20, 11]);
        assert!(
            split.iter().all(|group| !group.unbounded),
            "both pieces are under the cap, so neither is marked"
        );
    }

    /// The case the whole ticket is for: the human's 236-file run put 130 files
    /// in one refined group.
    #[test]
    fn the_group_that_held_half_the_review_is_bounded() {
        let split = enforce(vec![presented(
            "the-change",
            GroupSource::Refined,
            spread(&[
                ("src/app", 40),
                ("src/grouping", 35),
                ("src/ui", 30),
                ("src/model", 25),
            ]),
        )]);

        assert_eq!(split.len(), 4, "one piece per directory, one pass only");
        assert_eq!(sizes(&split), vec![40, 35, 25, 30]);
        assert_eq!(
            split
                .iter()
                .filter(|group| group.unbounded)
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "the-change \u{b7} app",
                "the-change \u{b7} grouping",
                "the-change \u{b7} ui"
            ],
            "a piece the one pass left over the cap is marked, not split again"
        );
    }

    #[test]
    fn a_flat_directory_the_pass_cannot_bound_keeps_its_files_and_is_marked() {
        let split = enforce(vec![presented(
            "generated",
            GroupSource::Heuristics,
            paths(44, "src/generated"),
        )]);

        assert_eq!(names(&split), vec!["generated"]);
        assert_eq!(
            sizes(&split),
            vec![44],
            "no file is dropped or shuffled off"
        );
        assert!(
            split[0].unbounded,
            "the engine gave up here and the row has to say so"
        );
    }

    #[test]
    fn a_piece_name_is_the_shortest_suffix_that_distinguishes_it() {
        let split = enforce(vec![presented(
            "transport",
            GroupSource::Refined,
            spread(&[
                ("src/agent/grouping", 10),
                ("src/agent/app", 9),
                ("crates/core/agent/app", 8),
            ]),
        )]);

        assert_eq!(
            names(&split),
            vec![
                "transport \u{b7} core/agent/app",
                "transport \u{b7} src/agent/app",
                "transport \u{b7} grouping",
            ],
            "one component where that is enough, and only as many more as it takes"
        );
    }

    #[test]
    fn a_root_level_piece_is_named_rather_than_left_blank() {
        let mut members = paths(24, "src/app");
        members.push("README.md".to_string());
        members.push("Cargo.toml".to_string());

        let split = enforce(vec![presented("release", GroupSource::Refined, members)]);

        assert_eq!(
            names(&split),
            vec!["release \u{b7} root", "release \u{b7} app"]
        );
    }

    #[test]
    fn a_group_of_one_survives_the_pass_untouched() {
        let groups = enforce(vec![
            group("lockfile", GroupSource::Heuristics, &["Cargo.lock"]),
            group("docs", GroupSource::Refined, &["docs/GROUPING.md"]),
        ]);

        assert_eq!(names(&groups), vec!["lockfile", "docs"]);
        assert!(groups.iter().all(|group| !group.unbounded));
    }

    #[test]
    fn pieces_take_the_parents_slot_in_the_reading_order() {
        let groups = enforce(vec![
            group("first", GroupSource::Refined, &["a/one.rs"]),
            presented(
                "second",
                GroupSource::Refined,
                spread(&[("src/x", 13), ("src/y", 13)]),
            ),
            group("third", GroupSource::Refined, &["z/three.rs"]),
        ]);

        assert_eq!(
            names(&groups),
            vec!["first", "second \u{b7} x", "second \u{b7} y", "third"]
        );
    }

    /// Both arms end in the same split, so what is left to pin is that each
    /// arm reaches it — over a changeset the passes and the reader actually
    /// ran on, not a hand-built `PresentedGroup` list.
    mod through_the_arms {
        use crate::grouping::changeset::Changeset;
        use crate::grouping::passes::GroupingConfig;
        use crate::grouping::{
            GroupSource, Grouping, PresentedGroup, assign_incrementally, group_changeset, refine,
        };

        /// One concern token across two directories, so the token cluster pass
        /// files them all together and the split has a directory boundary to
        /// cut on — inside a changeset big enough that `auth` clears the
        /// ubiquity stoplist, which is the shape a group this size comes in.
        fn changeset_of(per_dir: usize) -> Changeset {
            let mut text = String::new();
            for dir in ["src/api", "src/ui"] {
                for index in 0..per_dir {
                    text.push_str(&format!("M\t{dir}/auth-{index:03}.rs\n"));
                }
            }
            text.push_str(&filler());
            Changeset::parse(&text)
        }

        /// The rest of a large changeset: twelve directories of unrelated files
        /// that cluster among themselves and never near `auth`. Enough of them
        /// that `auth` stays under the ubiquity threshold even once the
        /// arrivals below have grown it.
        fn filler() -> String {
            let mut text = String::new();
            for bucket in 0..12 {
                for index in 0..10 {
                    text.push_str(&format!(
                        "M\tsrc/fill{bucket}/topic{bucket}-{index:03}.rs\n"
                    ));
                }
            }
            text
        }

        /// The groups whose name starts with `prefix`, by name and size. The
        /// filler's own groups are not what any of these tests are about.
        fn sizes(grouping: &Grouping, prefix: &str) -> Vec<(String, usize)> {
            grouping
                .groups()
                .iter()
                .filter(|group| group.name.starts_with(prefix))
                .map(|group| (group.name.clone(), grouping.files_in(&group.id).count()))
                .collect()
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
                    unbounded: group.unbounded,
                    members: grouping
                        .files_in(&group.id)
                        .map(|path| path.to_string_lossy().to_string())
                        .collect(),
                })
                .collect()
        }

        #[test]
        fn a_full_heuristic_pass_splits_a_group_over_the_heuristic_cap() {
            let under = group_changeset(&changeset_of(14), GroupingConfig::default());
            assert_eq!(
                sizes(&under, "auth"),
                vec![("auth".to_string(), 28)],
                "28 files is under the heuristic cap of 30 and the pass leaves it whole"
            );

            let over = group_changeset(&changeset_of(16), GroupingConfig::default());
            assert_eq!(
                sizes(&over, "auth"),
                vec![
                    ("auth \u{b7} api".to_string(), 16),
                    ("auth \u{b7} ui".to_string(), 16)
                ],
                "32 files is over it and the pass splits by directory"
            );
        }

        /// The 130-in-one-group case the ticket exists for, arriving the way it
        /// really arrives: as a refine answer.
        #[test]
        fn a_refine_answer_over_the_refine_cap_is_split_before_it_is_presented() {
            let changeset = changeset_of(14);
            let heuristic = group_changeset(&changeset, GroupingConfig::default());
            // A total answer, as the contract asks for: the `auth` files in one
            // oversized group, everything else in a second, so nothing here is a
            // repair and the split is measured on its own.
            let files: Vec<String> = changeset
                .files
                .iter()
                .filter(|file| file.file_name().starts_with("auth-"))
                .map(|file| format!("\"{}\"", file.path))
                .collect();
            let rest: Vec<String> = changeset
                .files
                .iter()
                .filter(|file| !file.file_name().starts_with("auth-"))
                .map(|file| format!("\"{}\"", file.path))
                .collect();
            let body = format!(
                "{{\"groups\": [{{\"name\": \"the-whole-thing\", \"files\": [{}]}}, \
                 {{\"name\": \"the-rest\", \"files\": [{}]}}]}}",
                files.join(", "),
                rest.join(", ")
            );

            let refined = refine::apply(&body, &changeset, &heuristic).expect("a usable answer");

            assert_eq!(
                sizes(&refined.grouping, "the-whole-thing"),
                vec![
                    ("the-whole-thing \u{b7} api".to_string(), 14),
                    ("the-whole-thing \u{b7} ui".to_string(), 14)
                ],
                "28 files is over the refine cap of 25, where the heuristic arm would have \
                 left it alone"
            );
            assert!(
                refined
                    .grouping
                    .groups()
                    .iter()
                    .filter(|group| group.name.starts_with("the-whole-thing"))
                    .all(|group| group.source == GroupSource::Refined && !group.unbounded),
                "the pieces are the refine pass's own work, and both are bounded"
            );
            assert!(
                refined.repairs.is_empty(),
                "an oversized answer breaks no rule of the contract, so it is not a repair: \
                 {:?}",
                refined.repairs
            );
        }

        /// A cap-forced split is work of the pass, not drift from it. If it
        /// moved `Grouping::drift()` the `gd-26r.36` backstop would read a
        /// number the split itself created, fire a full pass, and split again.
        #[test]
        fn splitting_a_group_does_not_move_drift() {
            let split = group_changeset(&changeset_of(16), GroupingConfig::default());

            assert_eq!(
                sizes(&split, "auth").len(),
                2,
                "the group did split, which is the premise of the rest"
            );
            assert_eq!(split.drift(), 0.0);
            assert_eq!(split.drift_percent(), 0);
            assert!(
                split.groups().iter().all(|group| !group.drifted()),
                "no piece carries the `~` glyph: the drift marker means arrivals since the \
                 last full pass, and this is the full pass"
            );
        }

        /// The invariant is a property of each full pass, not of every frame
        /// (`gd-26r.23`). A group row must not split in two under a reader
        /// mid-review; the drift the arrivals add brings the backstop's own
        /// full pass along in time instead.
        #[test]
        fn an_arrival_that_pushes_a_group_over_the_cap_does_not_split_it() {
            let before = group_changeset(&changeset_of(14), GroupingConfig::default());
            assert_eq!(sizes(&before, "auth"), vec![("auth".to_string(), 28)]);

            let mut text = String::new();
            for dir in ["src/api", "src/ui"] {
                for index in 0..14 {
                    text.push_str(&format!("M\t{dir}/auth-{index:03}.rs\n"));
                }
            }
            for index in 0..6 {
                text.push_str(&format!("A\tsrc/api/auth-new-{index:03}.rs\n"));
            }
            text.push_str(&filler());
            let grown = Changeset::parse(&text);

            let after = assign_incrementally(&grown, as_kept(&before), GroupingConfig::default());

            assert_eq!(
                sizes(&after, "auth"),
                vec![("auth".to_string(), 34)],
                "34 files is over the heuristic cap and the group is still one row"
            );
            assert!(
                after.groups().iter().all(|group| !group.unbounded),
                "and it is not marked either: no split pass ran, so none gave up"
            );
            assert_eq!(
                sizes(&group_changeset(&grown, GroupingConfig::default()), "auth"),
                vec![
                    ("auth \u{b7} api".to_string(), 20),
                    ("auth \u{b7} ui".to_string(), 14)
                ],
                "the full pass the `gd-26r.36` backstop eventually lands is what splits it"
            );
        }
    }

    #[test]
    fn a_piece_name_another_group_already_holds_is_uniquified() {
        let groups = enforce(vec![
            presented(
                "auth",
                GroupSource::Refined,
                spread(&[("src/api", 13), ("src/ui", 13)]),
            ),
            group("auth \u{b7} api", GroupSource::Refined, &["src/api/pin.rs"]),
        ]);

        assert_eq!(
            names(&groups),
            vec!["auth \u{b7} api-2", "auth \u{b7} ui", "auth \u{b7} api"],
            "bucketing is by name, so a collision would silently union two groups"
        );
    }
}
