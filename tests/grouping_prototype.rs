//! Prototype grouping engine scored against the hand-grouped fixture.
//!
//! Run the report with:
//!   cargo test --test grouping_prototype -- --nocapture report
//!
//! The fixture is paths-only and provisional (tests/fixtures/grouping/README.md),
//! so these numbers compare passes against each other and catch regressions.
//! They do not declare a pass good.

mod grouping;

use grouping::changeset::Changeset;
use grouping::passes::{self, GroupingConfig};
use grouping::score::{Partition, Score};

const FILES: &str = include_str!("fixtures/grouping/orca-971b16754.files");
const GROUPS: &str = include_str!("fixtures/grouping/orca-971b16754.groups");

fn fixture() -> (Changeset, Partition) {
    (Changeset::parse(FILES), Partition::parse(GROUPS))
}

fn row(label: &str, score: Score) {
    println!(
        "{label:<34} F1 {:.3}   P {:.3}   R {:.3}   groups {:>3}   largest {:>5.1}%",
        score.f1,
        score.precision,
        score.recall,
        score.group_count,
        score.largest_share * 100.0
    );
}

#[test]
fn fixture_is_well_formed() {
    let (changeset, expected) = fixture();
    assert_eq!(changeset.len(), 161, "fixture changeset size");
    assert_eq!(
        expected.file_count(),
        changeset.len(),
        "every file is grouped exactly once"
    );
}

#[test]
fn report() {
    let (changeset, expected) = fixture();

    println!("\n=== baselines ===");
    row(
        "all-one-group",
        passes::baseline::all_one_group(&changeset).score_against(&expected),
    );
    row(
        "one-file-per-group",
        passes::baseline::one_file_per_group(&changeset).score_against(&expected),
    );
    row(
        "top-level-directory",
        passes::baseline::top_level_directory(&changeset).score_against(&expected),
    );
    row(
        "parent-directory",
        passes::baseline::parent_directory(&changeset).score_against(&expected),
    );

    println!("\n=== heuristic passes ===");
    let default = GroupingConfig::default();
    let grouping = passes::group(&changeset, default);
    row(
        "default config",
        grouping.partition().score_against(&expected),
    );

    println!("\n=== alternative clustering strategy ===");
    for target in [10, 13, 16, 20] {
        row(
            &format!("agglomerative, {target} groups"),
            passes::agglomerative_grouping(&changeset, target).score_against(&expected),
        );
    }

    println!("\n=== ablations ===");
    for (label, config) in ablations(default) {
        row(
            &label,
            passes::group(&changeset, config)
                .partition()
                .score_against(&expected),
        );
    }

    println!("\n=== ubiquity threshold sweep ===");
    for threshold in [0.10, 0.15, 0.20, 0.25, 0.35, 0.50, 1.00] {
        let config = GroupingConfig {
            ubiquity_threshold: threshold,
            ..default
        };
        row(
            &format!("ubiquity >= {threshold:.2}"),
            passes::group(&changeset, config)
                .partition()
                .score_against(&expected),
        );
    }

    println!("\n=== min cluster sweep ===");
    for min_cluster in [2, 3, 4, 5] {
        let config = GroupingConfig {
            min_cluster,
            ..default
        };
        row(
            &format!("min cluster {min_cluster}"),
            passes::group(&changeset, config)
                .partition()
                .score_against(&expected),
        );
    }

    println!("\n=== which passes fired (default config) ===");
    for (pass, count) in grouping.pass_hits() {
        println!("  {pass:<20} {count:>3} files");
    }

    println!("\n=== groups produced (default config) ===");
    let partition = grouping.partition();
    let mut groups: Vec<_> = partition.groups.iter().collect();
    groups.sort_by_key(|(_, members)| std::cmp::Reverse(members.len()));
    for (name, members) in groups {
        println!("  {name:<40} {:>3} files", members.len());
    }

    println!("\n=== can a filename token even find each expected group? ===");
    let mut explainability = passes::best_key_per_expected_group(&changeset, &expected, default);
    explainability.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    for (group, key, f1, size) in &explainability {
        println!("  {group:<26} {size:>3} files   best key `{key}` F1 {f1:.2}");
    }

    println!("\n=== close calls (chosen / runner-up) ===");
    let close_calls = grouping.close_calls();
    println!(
        "  {} of {} files flagged",
        close_calls.len(),
        changeset.len()
    );
    for assignment in close_calls.iter().take(12) {
        let runner_up = assignment.runner_up.as_ref().unwrap();
        println!(
            "  {}\n    {} <- {}: {}",
            assignment.path, assignment.group, runner_up.group, runner_up.reason
        );
    }
}

fn ablations(default: GroupingConfig) -> Vec<(String, GroupingConfig)> {
    vec![
        (
            "tie-break: cluster keeps test".into(),
            GroupingConfig {
                tests_follow_production: false,
                ..default
            },
        ),
        (
            "with directory tokens".into(),
            GroupingConfig {
                use_dir_tokens: true,
                ..default
            },
        ),
        (
            "no test-burst boost".into(),
            GroupingConfig {
                test_burst_boost: 0.0,
                ..default
            },
        ),
        (
            "no ubiquity stoplist".into(),
            GroupingConfig {
                ubiquity_threshold: 1.01,
                ..default
            },
        ),
        (
            "no leftover absorption".into(),
            GroupingConfig {
                absorb_leftovers: false,
                ..default
            },
        ),
        (
            "absorb everything (sim >= 0)".into(),
            GroupingConfig {
                absorb_min_similarity: 0.0,
                ..default
            },
        ),
        (
            "absorb only on strong match (>= 3)".into(),
            GroupingConfig {
                absorb_min_similarity: 3.0,
                ..default
            },
        ),
    ]
}

#[test]
fn heuristic_beats_every_baseline() {
    let (changeset, expected) = fixture();
    let heuristic = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected);

    for (label, baseline) in [
        ("all-one-group", passes::baseline::all_one_group(&changeset)),
        (
            "one-file-per-group",
            passes::baseline::one_file_per_group(&changeset),
        ),
        (
            "top-level-directory",
            passes::baseline::top_level_directory(&changeset),
        ),
        (
            "parent-directory",
            passes::baseline::parent_directory(&changeset),
        ),
    ] {
        let score = baseline.score_against(&expected);
        assert!(
            heuristic.f1 > score.f1,
            "heuristic F1 {:.3} must beat {label} F1 {:.3}",
            heuristic.f1,
            score.f1
        );
    }
}

#[test]
fn scoring_harness_is_calibrated() {
    let (_, expected) = fixture();
    let self_score = expected.score_against(&expected);
    assert!(
        (self_score.f1 - 1.0).abs() < 1e-9,
        "a partition must score 1.0 against itself"
    );
    assert_eq!(self_score.group_count, 13);
    assert!((self_score.largest_share - 38.0 / 161.0).abs() < 1e-9);
}

/// Locks the numbers recorded in docs/GROUPING_PASSES.md so a change to the
/// passes has to move them deliberately.
#[test]
fn recorded_numbers_hold() {
    let (changeset, expected) = fixture();
    let score = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected);

    assert!(
        (0.38..0.40).contains(&score.f1),
        "F1 drifted: {:.3}",
        score.f1
    );
    assert!(
        score.precision > 0.50,
        "precision drifted: {:.3}",
        score.precision
    );
    assert!(
        score.largest_share < 0.25,
        "largest group share drifted: {:.3}",
        score.largest_share
    );

    let no_stoplist = passes::group(
        &changeset,
        GroupingConfig {
            ubiquity_threshold: 1.01,
            ..GroupingConfig::default()
        },
    )
    .partition()
    .score_against(&expected);
    assert!(
        score.f1 - no_stoplist.f1 > 0.10,
        "the ubiquity stoplist is load-bearing; without it F1 {:.3} vs {:.3}",
        no_stoplist.f1,
        score.f1
    );
}

#[test]
fn grouping_is_a_strict_partition() {
    let (changeset, _) = fixture();
    let grouping = passes::group(&changeset, GroupingConfig::default());
    let partition = grouping.partition();

    assert_eq!(
        partition.file_count(),
        changeset.len(),
        "every file assigned exactly once"
    );
    let mut seen = std::collections::BTreeSet::new();
    for members in partition.groups.values() {
        for path in members {
            assert!(seen.insert(path.clone()), "{path} appears in two groups");
        }
    }
}
