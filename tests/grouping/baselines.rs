//! Baselines and diagnostics the fixture harness scores the engine against.
//!
//! Measurement only: nothing here ships in the binary. They live beside the
//! scorer rather than in `src/grouping/` because their only caller is the
//! report — but they share the engine's own [`similarity`] and
//! [`candidate_keys`], so "the passes are weak" and "paths alone cannot do
//! better" stay comparable claims about one similarity model.

use std::collections::{BTreeMap, BTreeSet};

use tuicr::grouping::changeset::{ChangedFile, Changeset};
use tuicr::grouping::passes::{GroupingConfig, candidate_keys, similarity};

use super::score::{Partition, f1, free_name};

/// An alternative to greedy token clustering, run to tell "this pass is weak"
/// apart from "paths alone cannot do better": average-linkage agglomerative
/// clustering over the same token similarity, merged down to `target_groups`.
pub fn agglomerative_grouping(changeset: &Changeset, target_groups: usize) -> Partition {
    let files: Vec<&ChangedFile> = changeset.files.iter().collect();
    let mut clusters: Vec<Vec<usize>> = (0..files.len()).map(|i| vec![i]).collect();
    let matrix: Vec<Vec<f64>> = files
        .iter()
        .map(|file| files.iter().map(|other| similarity(file, other)).collect())
        .collect();

    while clusters.len() > target_groups {
        let mut best: Option<(usize, usize, f64)> = None;
        for a in 0..clusters.len() {
            for b in (a + 1)..clusters.len() {
                let linkage = average_linkage(&matrix, &clusters[a], &clusters[b]);
                if best.is_none_or(|(_, _, score)| linkage > score) {
                    best = Some((a, b, linkage));
                }
            }
        }
        let Some((a, b, score)) = best else { break };
        if score <= 0.0 {
            break;
        }
        let merged = clusters.remove(b);
        clusters[a].extend(merged);
    }

    // `Partition` buckets by name, so two clusters that derive the same name
    // would be unioned and the result would quietly be smaller than the
    // baseline the caller asked for. Colliding names are suffixed instead.
    let mut assignments = Vec::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for members in &clusters {
        let name = free_name(&derive_group_name(&files, members), &used);
        used.insert(name.clone());
        assignments.extend(
            members
                .iter()
                .map(|&i| (files[i].path.clone(), name.clone())),
        );
    }
    Partition::from_assignments(assignments)
}

fn average_linkage(matrix: &[Vec<f64>], left: &[usize], right: &[usize]) -> f64 {
    let total: f64 = left
        .iter()
        .map(|&a| right.iter().map(|&b| matrix[a][b]).sum::<f64>())
        .sum();
    total / (left.len() * right.len()) as f64
}

/// For each expected group, the single filename token that best reproduces it.
/// Answers which of the human's groups a token pass could ever find at all.
pub fn best_key_per_expected_group(
    changeset: &Changeset,
    expected: &Partition,
    config: GroupingConfig,
) -> Vec<(String, String, f64, usize)> {
    let files: Vec<&ChangedFile> = changeset.files.iter().collect();
    let mut carriers: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for file in &files {
        for key in candidate_keys(file, config) {
            carriers.entry(key).or_default().insert(file.path.as_str());
        }
    }

    expected
        .groups
        .iter()
        .map(|(name, members)| {
            let members: BTreeSet<&str> = members.iter().map(String::as_str).collect();
            let best = carriers
                .iter()
                .map(|(key, holders)| {
                    let hits = holders.intersection(&members).count() as f64;
                    let precision = hits / holders.len() as f64;
                    let recall = hits / members.len() as f64;
                    (key.clone(), f1(precision, recall))
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap_or_default();
            (name.clone(), best.0, best.1, members.len())
        })
        .collect()
}

/// GROUPING.md rule 10 wants the changeset's own vocabulary: the token most
/// members share, preferring the longer one on a tie.
pub fn derive_group_name(files: &[&ChangedFile], members: &[usize]) -> String {
    let mut frequency: BTreeMap<String, usize> = BTreeMap::new();
    for &index in members {
        let tokens = files[index].name_tokens();
        for token in tokens.iter().collect::<BTreeSet<_>>() {
            *frequency.entry(token.clone()).or_default() += 1;
        }
        for pair in tokens.windows(2) {
            *frequency
                .entry(format!("{}-{}", pair[0], pair[1]))
                .or_default() += 1;
        }
    }
    frequency
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(a.0.len().cmp(&b.0.len())))
        .map(|(token, _)| token)
        .unwrap_or_else(|| "unnamed".to_string())
}

pub mod baseline {
    use super::*;

    pub fn all_one_group(changeset: &Changeset) -> Partition {
        Partition::from_assignments(
            changeset
                .files
                .iter()
                .map(|f| (f.path.clone(), "all".to_string())),
        )
    }

    pub fn one_file_per_group(changeset: &Changeset) -> Partition {
        Partition::from_assignments(
            changeset
                .files
                .iter()
                .map(|f| (f.path.clone(), f.path.clone())),
        )
    }

    pub fn top_level_directory(changeset: &Changeset) -> Partition {
        Partition::from_assignments(
            changeset
                .files
                .iter()
                .map(|f| (f.path.clone(), f.top_level_dir().to_string())),
        )
    }

    /// Not required by the README, but reported because on this changeset the
    /// top-level cut is only two buckets and behaves like all-one-group.
    pub fn parent_directory(changeset: &Changeset) -> Partition {
        Partition::from_assignments(
            changeset
                .files
                .iter()
                .map(|f| (f.path.clone(), f.dir().to_string())),
        )
    }
}
