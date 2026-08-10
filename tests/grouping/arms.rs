//! The heuristic arm as the harness sees it: name-keyed and unordered.
//!
//! The engine's passes return a bare `Vec<PassClaim>`, because opaque identity
//! and reading order are put on by `tuicr::grouping::Grouping` — the one
//! constructor for a presented grouping. The scorer wants neither: it compares
//! *names to names*, and it grades order through [`super::order_score`], which
//! takes a [`Partition`] and an order separately so the pairwise numbers stay
//! order-blind and comparable to every row already published.
//!
//! So this is the missing half a measurement needs and a runtime does not: the
//! claims bucketed by name, with the diagnostics — [`Grouping::pass_hits`],
//! [`Grouping::close_calls`] — the report prints. The passes themselves are the
//! engine's; nothing here re-implements a decision.

use std::collections::BTreeMap;

use tuicr::grouping::changeset::Changeset;
use tuicr::grouping::passes;

pub use tuicr::grouping::RunnerUp;
pub use tuicr::grouping::passes::{GroupingConfig, PassClaim as Assignment};

pub use super::baselines::{agglomerative_grouping, baseline, best_key_per_expected_group};
use super::score::Partition;

#[derive(Debug, Clone)]
pub struct Grouping {
    pub assignments: Vec<Assignment>,
}

impl Grouping {
    pub fn partition(&self) -> Partition {
        Partition::from_assignments(
            self.assignments
                .iter()
                .map(|a| (a.path.clone(), a.group.clone())),
        )
    }

    pub fn pass_hits(&self) -> BTreeMap<&'static str, usize> {
        let mut hits = BTreeMap::new();
        for assignment in &self.assignments {
            *hits.entry(assignment.pass).or_default() += 1;
        }
        hits
    }

    pub fn close_calls(&self) -> Vec<&Assignment> {
        self.assignments
            .iter()
            .filter(|a| a.runner_up.is_some())
            .collect()
    }
}

pub fn group(changeset: &Changeset, config: GroupingConfig) -> Grouping {
    Grouping {
        assignments: passes::assign(changeset, config),
    }
}
