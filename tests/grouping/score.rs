//! Pairwise co-membership scoring, per tests/fixtures/grouping/README.md.
//! Group names are ignored; only the partition is judged.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Partition {
    /// Group name to member paths. Names carry no weight in scoring; they are
    /// kept only so a report can show what a pass called a group.
    pub groups: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Score {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub group_count: usize,
    pub largest_share: f64,
}

impl Partition {
    /// Parses the fixture's expected grouping: `[name]` headers, indented paths.
    pub fn parse(text: &str) -> Self {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut current = String::from("ungrouped");
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(name) = trimmed
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                current = name.to_string();
                groups.entry(current.clone()).or_default();
            } else {
                groups
                    .entry(current.clone())
                    .or_default()
                    .push(trimmed.to_string());
            }
        }
        Self { groups }
    }

    pub fn from_assignments<I>(assignments: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (path, group) in assignments {
            groups.entry(group).or_default().push(path);
        }
        Self { groups }
    }

    pub fn file_count(&self) -> usize {
        self.groups.values().map(Vec::len).sum()
    }

    fn co_membership_pairs(&self) -> u64 {
        self.groups
            .values()
            .map(|members| pairs(members.len()))
            .sum()
    }

    /// Pairwise precision/recall/F1 of this partition against `expected`,
    /// plus the shape numbers a bare F1 hides.
    pub fn score_against(&self, expected: &Partition) -> Score {
        let expected_group: BTreeMap<&str, &str> = expected
            .groups
            .iter()
            .flat_map(|(name, members)| {
                members
                    .iter()
                    .map(move |path| (path.as_str(), name.as_str()))
            })
            .collect();

        let true_positives: u64 = self
            .groups
            .values()
            .map(|members| {
                let mut overlap: BTreeMap<&str, usize> = BTreeMap::new();
                for path in members {
                    if let Some(group) = expected_group.get(path.as_str()) {
                        *overlap.entry(group).or_default() += 1;
                    }
                }
                overlap.values().map(|&n| pairs(n)).sum::<u64>()
            })
            .sum();

        let computed_pairs = self.co_membership_pairs();
        let expected_pairs = expected.co_membership_pairs();
        let precision = ratio(true_positives, computed_pairs);
        let recall = ratio(true_positives, expected_pairs);
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };

        let total = self.file_count().max(1) as f64;
        let largest = self.groups.values().map(Vec::len).max().unwrap_or(0) as f64;
        Score {
            precision,
            recall,
            f1,
            group_count: self.groups.len(),
            largest_share: largest / total,
        }
    }
}

fn pairs(n: usize) -> u64 {
    let n = n as u64;
    n.saturating_sub(1) * n / 2
}

/// Zero rather than undefined when a partition co-groups nothing: the
/// one-file-per-group baseline has no true positives to be precise about.
fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
