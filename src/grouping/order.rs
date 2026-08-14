//! Reading order, at both levels: which group comes first, and which file
//! leads a group.
//!
//! Both are engine code and both are pure. The group order is `gd-26r.12`
//! Decision 3's size-descending proxy, which the refine arm replaces with a
//! real intent-centrality order when it runs. The within-group order is
//! `gd-26r.27`'s deterministic sort, which **no** arm replaces: the refine
//! response carries nothing about file order, so this runs identically on the
//! refined and the heuristic path, and on the cancelled, timed-out and
//! `refine = false` paths too.
//!
//! Nothing here can fail, and nothing here is configurable.

use std::collections::{BTreeMap, BTreeSet};

use super::changeset::{ChangedFile, Changeset};

/// Prefix of the directory-fallback groups `gd-26r.12` Decision 3 pins last.
pub const FALLBACK_PREFIX: &str = "dir:";

/// Whether the sort applies rule 13 — the group's central file leads it.
///
/// A knob rather than a constant because rule 13 is the one within-group rule
/// both fixtures' own file order *contradicts* (`tau` −0.25 and −0.29 against
/// themselves), so the measurement can price it but not settle it: on both
/// fixtures it is worth nothing and costs nothing. `gd-26r.27` ruled it **on**
/// anyway, because `GROUPING.md` rule 13 is normative and a sort that
/// implements the measurable subset of a written spec is a sort nobody can read
/// the spec to check. See `docs/GROUPING_PASSES.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CentralFirst {
    /// Kept to price rule 13, which is otherwise invisible: both fixtures score
    /// the same with it and without it.
    Off,
    /// What ships.
    On,
}

/// The changeset indexed by path, for the sort and for the harness's rule
/// scorer, which has to resolve a path to a file the same way.
pub fn files_by_path(changeset: &Changeset) -> BTreeMap<&str, &ChangedFile> {
    changeset
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect()
}

/// `gd-26r.12` Decision 3's group order: concern groups by member count
/// descending, `dir:` fallback groups pinned last as a class and
/// size-descending among themselves. Name breaks size ties so the order is
/// total and reproducible.
pub fn heuristic_group_order(groups: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut names: Vec<&String> = groups.keys().collect();
    names.sort_by_key(|name| {
        (
            name.starts_with(FALLBACK_PREFIX),
            std::cmp::Reverse(groups[*name].len()),
            name.as_str(),
        )
    });
    names.into_iter().cloned().collect()
}

/// One group's files in reading order.
///
/// A **pure sort over paths the engine already holds**: no model call, no token
/// similarity, nothing nondeterministic. It exists because the measurement
/// showed both arms breaking rule 4 on every pair — both build their groups
/// from a path-keyed map, and `x.test.ts` sorts before `x.ts`, so a plain path
/// sort puts every test ahead of the code it covers.
///
/// Key, outermost first:
///
/// 1. the central file, when rule 13 is on (`GROUPING.md` rule 13);
/// 2. mechanical stragglers last (rule 14);
/// 3. broad tests after everything narrower (rule 12) — a group-wide band,
///    not a per-file adjustment, because the rule says integration and
///    end-to-end tests follow *the* unit tests, not their own;
/// 4. directory, then stem — which is what puts a production file and its
///    test adjacent, since `stem` strips the test marker;
/// 5. production before test (rule 4);
/// 6. path, for a total and stable order where no rule speaks.
pub fn sort_group_files(
    name: &str,
    members: &mut [String],
    by_path: &BTreeMap<&str, &ChangedFile>,
    central_first: CentralFirst,
) {
    let files: Vec<&ChangedFile> = members
        .iter()
        .filter_map(|path| by_path.get(path.as_str()).copied())
        .collect();
    let central = match central_first {
        CentralFirst::On => central_file(name, &files).map(|file| file.path.clone()),
        CentralFirst::Off => None,
    };
    members.sort_by_key(|path| {
        let file = by_path.get(path.as_str()).copied();
        (
            file.is_none() || central.as_deref() != Some(path.as_str()),
            file.is_some_and(|file| file.is_mechanical()),
            file.is_some_and(|file| file.is_broad_test()),
            file.map(|file| file.dir().to_string()).unwrap_or_default(),
            file.map(|file| file.stem()).unwrap_or_default(),
            file.is_some_and(|file| file.is_test()),
            path.clone(),
        )
    });
}

/// Whether `test` is plausibly the test *of* `production`: same directory, or a
/// `__tests__` directory directly beneath it. Callers have already matched the
/// stems; this is the locality half of "its production file".
///
/// One of rule 4's three gates, alongside [`ChangedFile::is_test`] and stem
/// equality. Shared with the harness's rule scorer so the sort and the metric
/// cannot come to disagree about what "its production file" means.
pub fn covers(production: &ChangedFile, test: &ChangedFile) -> bool {
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
pub fn central_file<'a>(name: &str, files: &[&'a ChangedFile]) -> Option<&'a ChangedFile> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One group sorted, from paths alone.
    ///
    /// The members go in reversed, so a case that passes because the input was
    /// already in the wanted order cannot pass. Every case below is a pair that
    /// the *next* key out would order the other way, which is what makes each
    /// one about the key it names rather than about the sort in general.
    fn ordered(name: &str, paths: &[&str], central_first: CentralFirst) -> Vec<String> {
        let text: String = paths
            .iter()
            .map(|path| format!("M\t{path}"))
            .collect::<Vec<_>>()
            .join("\n");
        let changeset = Changeset::parse(&text);
        let by_path = files_by_path(&changeset);
        let mut members: Vec<String> = paths.iter().rev().map(|path| path.to_string()).collect();
        sort_group_files(name, &mut members, &by_path, central_first);
        members
    }

    /// A group name sharing no token with any member, so rule 13 stands down
    /// and the key under test is the outermost one left.
    const UNRELATED: &str = "quux";

    #[test]
    fn a_mechanical_straggler_sorts_last_however_early_its_directory_is() {
        assert_eq!(
            ordered(
                UNRELATED,
                &["src/auth/login.rs", "Cargo.lock"],
                CentralFirst::On
            ),
            ["src/auth/login.rs", "Cargo.lock"],
            "rule 14 outranks the directory key, which would lead with the repo root"
        );
    }

    #[test]
    fn a_broad_test_follows_the_narrow_ones_whatever_its_stem_sorts_as() {
        assert_eq!(
            ordered(
                UNRELATED,
                &["tests/auth_unit.rs", "tests/auth_integration.rs"],
                CentralFirst::On,
            ),
            ["tests/auth_unit.rs", "tests/auth_integration.rs"],
            "rule 12 outranks the stem key, which would lead with `integration`"
        );
    }

    #[test]
    fn directory_orders_before_stem_and_stem_before_the_rest() {
        assert_eq!(
            ordered(UNRELATED, &["a/beta.rs", "b/alpha.rs"], CentralFirst::On),
            ["a/beta.rs", "b/alpha.rs"],
            "the directory is the outer of the two"
        );
        // `x` sorts before `x-y` by stem and after it by path, so only the stem
        // key can produce this order.
        assert_eq!(
            ordered(UNRELATED, &["a/x.rs", "a/x-y.rs"], CentralFirst::On),
            ["a/x.rs", "a/x-y.rs"],
            "the stem is the inner of the two"
        );
    }

    /// Rule 4, and the reason the sort exists at all: a plain path sort puts
    /// `token.test.rs` ahead of `token.rs`, because `.` sorts below `r`.
    #[test]
    fn a_production_file_leads_its_own_test() {
        assert_eq!(
            ordered(
                UNRELATED,
                &["src/auth/token.rs", "src/auth/token.test.rs"],
                CentralFirst::On,
            ),
            ["src/auth/token.rs", "src/auth/token.test.rs"],
        );
    }

    #[test]
    fn the_path_settles_what_no_rule_speaks_to() {
        assert_eq!(
            ordered(UNRELATED, &["a/x.rs", "a/x.ts"], CentralFirst::On),
            ["a/x.rs", "a/x.ts"],
            "same directory, same stem, neither a test: only the path is left"
        );
    }

    /// Rule 13 is the outermost key and the only one the knob turns off, so the
    /// same group is sorted both ways: with it on the central file leads, with
    /// it off the directory-and-stem order it displaced comes back.
    #[test]
    fn the_central_file_leads_its_group_only_while_rule_13_is_on() {
        let group = [
            "src/auth/token.rs",
            "src/auth/aaa.rs",
            "src/auth/session.rs",
        ];
        assert_eq!(
            ordered("token-rotation", &group, CentralFirst::On),
            [
                "src/auth/token.rs",
                "src/auth/aaa.rs",
                "src/auth/session.rs"
            ],
        );
        assert_eq!(
            ordered("token-rotation", &group, CentralFirst::Off),
            [
                "src/auth/aaa.rs",
                "src/auth/session.rs",
                "src/auth/token.rs"
            ],
        );
    }

    /// A name two members match equally well names neither of them, so nothing
    /// leads and the sort is the one rule 13 would have produced turned off.
    #[test]
    fn a_name_that_fits_two_members_equally_promotes_neither() {
        let group = ["src/a/token.rs", "src/b/token.rs"];
        assert_eq!(
            ordered("token", &group, CentralFirst::On),
            ordered("token", &group, CentralFirst::Off),
        );
        assert_eq!(
            ordered("token", &group, CentralFirst::On),
            ["src/a/token.rs", "src/b/token.rs"],
        );
    }

    /// A test file is never the central one even when it is the best token
    /// match: rule 13 promotes the file the concern is *implemented* in.
    #[test]
    fn a_test_or_a_lockfile_is_never_the_central_file() {
        assert_eq!(
            ordered(
                "rotation",
                &["src/auth/aaa.rs", "src/auth/rotation.test.rs"],
                CentralFirst::On,
            ),
            ["src/auth/aaa.rs", "src/auth/rotation.test.rs"],
        );
    }

    #[test]
    fn groups_run_largest_first_with_the_directory_fallback_pinned_last() {
        let groups: BTreeMap<String, Vec<String>> = [
            ("dir:src/big", vec!["a", "b", "c", "d"]),
            ("small-concern", vec!["e"]),
            ("large-concern", vec!["f", "g"]),
            ("dir:src/small", vec!["h"]),
        ]
        .into_iter()
        .map(|(name, files)| {
            (
                name.to_string(),
                files.into_iter().map(str::to_string).collect(),
            )
        })
        .collect();

        assert_eq!(
            heuristic_group_order(&groups),
            [
                "large-concern",
                "small-concern",
                "dir:src/big",
                "dir:src/small"
            ],
            "a four-file fallback still sorts below a one-file concern"
        );
    }

    /// Size is the only key this entry point can show at work on a tie: the
    /// parameter is a `BTreeMap`, so a tied pair reaches the sort in name order
    /// whatever the caller did, and the name component of the key cannot be
    /// observed from here. What is checked is what is checkable — the order is
    /// total, size-descending, and the same every time.
    #[test]
    fn a_size_tie_between_groups_still_reads_in_one_settled_order() {
        let groups: BTreeMap<String, Vec<String>> = [
            ("zulu", vec!["a", "b"]),
            ("alpha", vec!["c", "d"]),
            ("mike", vec!["e", "f", "g"]),
        ]
        .into_iter()
        .map(|(name, files)| {
            (
                name.to_string(),
                files.into_iter().map(str::to_string).collect(),
            )
        })
        .collect();

        assert_eq!(
            heuristic_group_order(&groups),
            ["mike", "alpha", "zulu"],
            "size decides first; the tied pair reads in name order"
        );
    }
}
