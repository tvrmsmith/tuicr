//! The fixture harness's own half of the grouping work: scoring, baselines and
//! the refine arms.
//!
//! The **engine** lives in `tuicr::grouping` and is scored from here
//! (`gd-26r.28`). What stays test-side is measurement — pairwise F1, Kendall
//! tau, the rule constraints, the controls the passes are compared against —
//! plus the refine harness, which ships in a later slice.
pub mod arms;
pub mod baselines;
pub mod order_score;
pub mod refine;
pub mod score;
