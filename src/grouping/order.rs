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
    members.sort_by_key(|path| match by_path.get(path.as_str()) {
        Some(file) => (
            central.as_deref() != Some(path.as_str()),
            file.is_mechanical(),
            file.is_broad_test(),
            file.dir().to_string(),
            file.stem(),
            file.is_test(),
            path.clone(),
        ),
        None => (
            true,
            false,
            false,
            String::new(),
            String::new(),
            false,
            path.clone(),
        ),
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
