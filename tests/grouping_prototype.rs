//! Prototype grouping engine scored against the hand-grouped fixtures.
//!
//! Run the report with:
//!   cargo test --test grouping_prototype -- --nocapture report
//!
//! Fixture 1 (orca) is checked in, paths-only and provisional
//! (tests/fixtures/grouping/README.md), so its numbers compare passes against
//! each other and catch regressions. They do not declare a pass good.
//!
//! Fixture 2 is hand-grouped but carries file paths from a private work
//! repository, so it is **not** checked in. It is read at runtime from
//! `$TUICR_GROUPING_FIXTURES` (default `~/.local/share/tuicr-fixtures`), and
//! every test that needs it skips with a notice when the directory is absent.
//! docs/GROUPING_PASSES.md records what it produced.

mod grouping;

use std::collections::BTreeMap;
use std::path::PathBuf;

use grouping::changeset::{self, ChangeKind, ChangedFile, Changeset};
use grouping::passes::{self, GroupingConfig};
use grouping::refine::{self, RunRecord, Shape};
use grouping::score::{Partition, Score};

const ORCA_FILES: &str = include_str!("fixtures/grouping/orca-971b16754.files");
const ORCA_GROUPS: &str = include_str!("fixtures/grouping/orca-971b16754.groups");

/// The second, hand-grouped fixture: one .NET-dominant PR, 158 files.
const SECOND_FIXTURE: &str = "meridian-6c22fda02";
/// Paths-only, no expected grouping: exists purely so the three passes that
/// fired on nothing in fixture 1 meet a lockfile, a CI file and a rename.
const FIRE_CHECK_FIXTURE: &str = "meridian-097e2defa-10cc878df";

fn orca() -> (Changeset, Partition) {
    (Changeset::parse(ORCA_FILES), Partition::parse(ORCA_GROUPS))
}

fn fixture_dir() -> PathBuf {
    match std::env::var_os("TUICR_GROUPING_FIXTURES") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".local/share/tuicr-fixtures")
        }
    }
}

/// A changeset plus its hand grouping, or `None` when the external fixture
/// directory is not on this machine.
fn external_fixture(name: &str) -> Option<(Changeset, Partition)> {
    let dir = fixture_dir();
    let files = std::fs::read_to_string(dir.join(format!("{name}.files"))).ok()?;
    let groups = std::fs::read_to_string(dir.join(format!("{name}.groups"))).ok()?;
    Some((Changeset::parse(&files), Partition::parse(&groups)))
}

fn external_changeset(name: &str) -> Option<Changeset> {
    let files = std::fs::read_to_string(fixture_dir().join(format!("{name}.files"))).ok()?;
    Some(Changeset::parse(&files))
}

fn skip_notice(name: &str) {
    println!(
        "skipping {name}: no fixture in {} (set TUICR_GROUPING_FIXTURES)",
        fixture_dir().display()
    );
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
    let (changeset, expected) = orca();
    assert_eq!(changeset.len(), 161, "fixture changeset size");
    assert_eq!(
        expected.file_count(),
        changeset.len(),
        "every file is grouped exactly once"
    );

    let Some((changeset, expected)) = external_fixture(SECOND_FIXTURE) else {
        skip_notice(SECOND_FIXTURE);
        return;
    };
    assert_eq!(changeset.len(), 158, "second fixture changeset size");
    assert_eq!(
        expected.file_count(),
        changeset.len(),
        "every file is grouped exactly once"
    );
}

#[test]
fn report() {
    let (changeset, expected) = orca();
    report_fixture("orca 971b16754 (161 files)", &changeset, &expected);

    match external_fixture(SECOND_FIXTURE) {
        Some((changeset, expected)) => report_fixture(
            &format!("{SECOND_FIXTURE} (158 files)"),
            &changeset,
            &expected,
        ),
        None => skip_notice(SECOND_FIXTURE),
    }
}

fn report_fixture(label: &str, changeset: &Changeset, expected: &Partition) {
    println!("\n\n########## {label} ##########");

    println!("\n=== baselines ===");
    row(
        "all-one-group",
        passes::baseline::all_one_group(changeset).score_against(expected),
    );
    row(
        "one-file-per-group",
        passes::baseline::one_file_per_group(changeset).score_against(expected),
    );
    row(
        "top-level-directory",
        passes::baseline::top_level_directory(changeset).score_against(expected),
    );
    row(
        "parent-directory",
        passes::baseline::parent_directory(changeset).score_against(expected),
    );
    row("expected (self-score)", expected.score_against(expected));

    println!("\n=== heuristic passes ===");
    let default = GroupingConfig::default();
    let grouping = passes::group(changeset, default);
    row(
        "default config",
        grouping.partition().score_against(expected),
    );

    println!("\n=== alternative clustering strategy ===");
    for target in [10, 13, 16, 20] {
        row(
            &format!("agglomerative, {target} groups"),
            passes::agglomerative_grouping(changeset, target).score_against(expected),
        );
    }

    println!("\n=== ablations ===");
    for (label, config) in ablations(default) {
        row(
            &label,
            passes::group(changeset, config)
                .partition()
                .score_against(expected),
        );
    }

    println!("\n=== ubiquity threshold sweep ===");
    for threshold in [0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.50, 1.00] {
        let config = GroupingConfig {
            ubiquity_threshold: threshold,
            ..default
        };
        row(
            &format!("ubiquity >= {threshold:.2}"),
            passes::group(changeset, config)
                .partition()
                .score_against(expected),
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
            passes::group(changeset, config)
                .partition()
                .score_against(expected),
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
    let mut explainability = passes::best_key_per_expected_group(changeset, expected, default);
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

/// The three passes that matched nothing in fixture 1 were kept as unscored
/// bets. This is the changeset that makes them fire or exposes them as dead
/// code: it carries a lockfile, a CI workflow and a rename. There is no hand
/// grouping for it, so nothing here is scored — only whether a pass fires.
#[test]
fn unfired_passes_meet_a_changeset_that_should_fire_them() {
    let Some(changeset) = external_changeset(FIRE_CHECK_FIXTURE) else {
        skip_notice(FIRE_CHECK_FIXTURE);
        return;
    };

    println!("\n\n########## {FIRE_CHECK_FIXTURE} (fire check, no ground truth) ##########");
    println!("  {} files", changeset.len());

    let renamed: Vec<_> = changeset
        .files
        .iter()
        .filter(|file| file.kind == ChangeKind::Renamed)
        .collect();
    println!("  {} renames in the changeset", renamed.len());

    let grouping = passes::group(&changeset, GroupingConfig::default());
    println!("\n=== which passes fired ===");
    for (pass, count) in grouping.pass_hits() {
        println!("  {pass:<20} {count:>3} files");
    }

    let hits = grouping.pass_hits();
    assert!(
        hits.contains_key("mechanical"),
        "the changeset carries a lockfile, so the mechanical pass must fire"
    );
    assert!(
        hits.contains_key("config-ci"),
        "the changeset carries a CI workflow, so the config-ci pass must fire"
    );

    assert!(
        !renamed.is_empty(),
        "the fire-check changeset is supposed to carry a rename"
    );
    assert!(
        !hits.contains_key("rename-pair"),
        "there is no rename pass any more: rule 11 holds by construction"
    );
}

/// GROUPING.md rule 11, as it now reads. Git reports a rename as a single entry
/// keyed by the new path, so a rename's two halves are one file in the diff and
/// there is no pair to reunite — the old `enforce_rename_pairs` looked the old
/// path up in `assigned`, where it could never be a key, and so never executed
/// on any input. What rule 11 asks for is a property of the representation, and
/// this is the test that says so.
#[test]
fn a_rename_is_one_file_in_one_group() {
    let changeset = Changeset::parse(
        "M\tsrc/app/pipeline.ts\n\
         R096\tsrc/app/old-worklist.ts\tsrc/app/claim-worklist.ts\n\
         A\tsrc/app/claim-worklist.test.ts\n",
    );
    let renamed = &changeset.files[1];
    assert_eq!(renamed.path, "src/app/claim-worklist.ts");
    assert_eq!(renamed.kind, ChangeKind::Renamed);
    assert_eq!(
        renamed.rename_from.as_deref(),
        Some("src/app/old-worklist.ts")
    );

    let partition = passes::group(&changeset, GroupingConfig::default()).partition();
    let holders: Vec<&String> = partition
        .groups
        .iter()
        .filter(|(_, members)| members.iter().any(|m| m == &renamed.path))
        .map(|(name, _)| name)
        .collect();
    assert_eq!(holders.len(), 1, "a rename is in exactly one group");
    assert!(
        !partition.groups.values().any(|members| members
            .iter()
            .any(|m| Some(m.as_str()) == renamed.rename_from.as_deref())),
        "the old path is not a file in the changeset and must not be grouped"
    );
}

// --- the model refine pass (`gd-26r.11`) ------------------------------------
//
// The call itself is nondeterministic and costs money, so no test makes one.
// `emit_refine_prompts` writes what would be sent, `scripts/grouping-refine-runs.sh`
// sends it N times per shape and records each answer, and `refine_report` scores
// the recorded answers with the same scorer every other pass is graded by.

/// Prompts land here, regenerable, never committed.
fn prompt_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/grouping-refine/prompts")
}

/// Recorded runs for the checked-in fixture are evidence and live with it.
/// Runs against the private fixture carry its paths in both prompt and answer,
/// so they live beside it, outside the repo.
fn run_dir(fixture: &str) -> PathBuf {
    if fixture == ORCA_FIXTURE {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grouping/refine")
    } else {
        fixture_dir().join("refine")
    }
    .join(fixture)
}

const ORCA_FIXTURE: &str = "orca-971b16754";

/// Writes one prompt per fixture per shape for the run script to send. Not an
/// assertion — run it, then run the script.
#[test]
fn emit_refine_prompts() {
    for (label, changeset, _) in fixtures() {
        let grouping = passes::group(&changeset, GroupingConfig::default());
        let dir = prompt_dir().join(&label);
        std::fs::create_dir_all(&dir).expect("create prompt dir");
        for shape in Shape::ALL {
            let text = refine::prompt(&changeset, &grouping, shape);
            let path = dir.join(format!("{}.txt", shape.slug()));
            std::fs::write(&path, &text).expect("write prompt");
            println!(
                "  {label:<22} {:<12} {:>6} chars -> {}",
                shape.slug(),
                text.len(),
                path.display()
            );
        }
    }
}

struct Run {
    record: RunRecord,
    refined: refine::Refined,
}

fn load_runs(fixture: &str, changeset: &Changeset, shape: Shape) -> Vec<Run> {
    let dir = run_dir(fixture);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            // Recorded runs are named `<shape>-<model>-<nn>.json`, and one shape
            // slug is a prefix of another ("full", "full-coarse"), so match the
            // shape segment exactly rather than by prefix.
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
                .and_then(|stem| stem.rsplitn(3, '-').nth(2))
                .is_some_and(|slug| slug == shape.slug())
        })
        .collect();
    paths.sort();

    let grouping = passes::group(changeset, GroupingConfig::default());
    paths
        .iter()
        .filter_map(|path| {
            let envelope = std::fs::read_to_string(path).ok()?;
            let record = match RunRecord::parse(&envelope) {
                Ok(record) => record,
                Err(error) => {
                    println!("  !! {}: {error}", path.display());
                    return None;
                }
            };
            match refine::apply(&record.body, changeset, &grouping, shape) {
                Ok(refined) => Some(Run { record, refined }),
                Err(error) => {
                    println!("  !! {}: unusable answer: {error}", path.display());
                    None
                }
            }
        })
        .collect()
}

#[test]
fn refine_report() {
    for (label, changeset, expected) in fixtures() {
        println!("\n\n########## refine: {label} ##########");
        let heuristic = passes::group(&changeset, GroupingConfig::default())
            .partition()
            .score_against(&expected);
        let best_baseline = [
            passes::baseline::all_one_group(&changeset),
            passes::baseline::top_level_directory(&changeset),
            passes::baseline::parent_directory(&changeset),
        ]
        .iter()
        .map(|baseline| baseline.score_against(&expected).f1)
        .fold(0.0f64, f64::max);
        let bar = heuristic.f1.max(best_baseline);
        println!(
            "  bar to clear: {bar:.3}  (heuristics {:.3}, best baseline {best_baseline:.3})",
            heuristic.f1
        );

        for shape in Shape::ALL {
            let all = load_runs(&label, &changeset, shape);
            if all.is_empty() {
                println!("\n=== {} (no runs recorded) ===", shape.slug());
                println!("  none on this machine; see scripts/grouping-refine-runs.sh");
                continue;
            }
            // A run is only comparable with runs of the same model, so the
            // arms are reported apart rather than averaged together.
            let mut by_model: BTreeMap<String, Vec<&Run>> = BTreeMap::new();
            for run in &all {
                by_model
                    .entry(run.record.model.clone())
                    .or_default()
                    .push(run);
            }
            for (model, runs) in by_model {
                println!(
                    "\n=== {} · {model} ({} runs recorded) ===",
                    shape.slug(),
                    runs.len()
                );

                let mut partitions = Vec::new();
                for (index, run) in runs.iter().enumerate() {
                    let score = run.refined.partition.score_against(&expected);
                    row(&format!("  run {index}"), score);
                    for repair in &run.refined.repairs {
                        println!("      repair: {repair}");
                    }
                    partitions.push(run.refined.partition.clone());
                }

                let f1s: Vec<f64> = partitions
                    .iter()
                    .map(|p| p.score_against(&expected).f1)
                    .collect();
                let mean = f1s.iter().sum::<f64>() / f1s.len() as f64;
                let worst = f1s.iter().copied().fold(f64::INFINITY, f64::min);
                let best = f1s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                println!("  F1 mean {mean:.3}  worst {worst:.3}  best {best:.3}  bar {bar:.3}");

                match refine::agreement(&partitions) {
                    Some(agreement) => println!(
                        "  run-to-run agreement: mean {:.3}  worst {:.3}  best {:.3}",
                        agreement.mean, agreement.worst, agreement.best
                    ),
                    None => println!("  run-to-run agreement: needs 2+ runs"),
                }
                if let Some(stability) = refine::file_stability(&partitions) {
                    println!(
                        "  per file: {:.1}% identical group-mates in every run, \
                     mean group-mate overlap {:.3}",
                        stability.identical_share * 100.0,
                        stability.mean_overlap
                    );
                }
                // Consensus is the obvious answer to nondeterminism and costs one
                // call per vote, so the cheapest useful number of votes is the
                // decision, not whether consensus works at all.
                for votes in [3, partitions.len()] {
                    if votes < 3 || votes > partitions.len() {
                        continue;
                    }
                    if let Some(consensus) = refine::consensus(&partitions[..votes]) {
                        row(
                            &format!("  consensus of {votes}"),
                            consensus.score_against(&expected),
                        );
                    }
                }

                let calls = runs.len() as f64;
                let sum = |f: fn(&RunRecord) -> f64| runs.iter().map(|r| f(&r.record)).sum::<f64>();
                println!(
                    "  per call: {:.0} input + {:.0} cached tokens, {:.0} output, ${:.3}, {:.1}s",
                    sum(|r| r.input_tokens) / calls,
                    sum(|r| r.cached_tokens) / calls,
                    sum(|r| r.output_tokens) / calls,
                    sum(|r| r.cost_usd) / calls,
                    sum(|r| r.wall_clock_ms) / calls / 1000.0,
                );
                println!(
                    "  repairs per call: {:.1}",
                    runs.iter().map(|r| r.refined.repairs.len()).sum::<usize>() as f64 / calls
                );
                println!(
                    "  names, run 0 reading order: {}",
                    runs[0].refined.order.join(", ")
                );

                if matches!(shape, Shape::Full | Shape::FullCoarse) {
                    println!(
                        "  where it wins and loses, per expected group (heuristic -> refine):"
                    );
                    let heuristic_recall = refine::recall_per_expected_group(
                        &passes::group(&changeset, GroupingConfig::default()).partition(),
                        &expected,
                    );
                    let refined_recall =
                        refine::recall_per_expected_group(&partitions[0], &expected);
                    let mut rows: Vec<_> = heuristic_recall
                        .iter()
                        .zip(&refined_recall)
                        .map(|((name, size, before), (_, _, after))| {
                            (name.clone(), *size, *before, *after)
                        })
                        .collect();
                    rows.sort_by_key(|row| std::cmp::Reverse(row.1));
                    for (name, size, before, after) in rows {
                        println!(
                            "    {name:<28} {size:>3} files   {before:.2} -> {after:.2}  {:+.2}",
                            after - before
                        );
                    }
                }
            }
        }
    }
}

/// The scorer ignores group names, so a shape that only renames cannot move it
/// — not "did not", *cannot*. The value of naming-only is therefore invisible
/// to this harness by construction, and saying so is part of the answer rather
/// than a caveat on it.
#[test]
fn naming_only_cannot_move_the_metric() {
    for (label, changeset, expected) in fixtures() {
        let runs = load_runs(&label, &changeset, Shape::NamingOnly);
        if runs.is_empty() {
            println!("no naming-only runs for {label}");
            continue;
        }
        let heuristic = passes::group(&changeset, GroupingConfig::default())
            .partition()
            .score_against(&expected);
        for run in &runs {
            let score = run.refined.partition.score_against(&expected);
            assert!(
                (score.f1 - heuristic.f1).abs() < 1e-9,
                "{label}: naming-only moved F1 {:.6} from {:.6}, so it changed the partition",
                score.f1,
                heuristic.f1
            );
        }
    }
}

/// Merging whole groups can only coarsen the partition, so it can only trade
/// precision away for recall. Locked because it bounds what the cheap shape is
/// able to buy, whatever a particular run happens to score.
#[test]
fn merge_only_can_only_coarsen() {
    for (label, changeset, _) in fixtures() {
        let runs = load_runs(&label, &changeset, Shape::MergeOnly);
        if runs.is_empty() {
            println!("no merge-only runs for {label}");
            continue;
        }
        let heuristic = passes::group(&changeset, GroupingConfig::default()).partition();
        for run in &runs {
            for members in heuristic.groups.values() {
                let holders: std::collections::BTreeSet<&String> = run
                    .refined
                    .partition
                    .groups
                    .iter()
                    .filter(|(_, refined)| refined.iter().any(|p| members.contains(p)))
                    .map(|(name, _)| name)
                    .collect();
                assert_eq!(
                    holders.len(),
                    1,
                    "{label}: merge-only split a heuristic group across {holders:?}"
                );
            }
        }
    }
}

/// Whatever a run answers, the strict partition of GROUPING.md survives it.
/// This is the property that makes an unreliable pass shippable at all: the
/// repairs in `refine::apply` are load-bearing, not defensive decoration.
#[test]
fn every_recorded_run_yields_a_strict_partition() {
    for (label, changeset, _) in fixtures() {
        for shape in Shape::ALL {
            for (index, run) in load_runs(&label, &changeset, shape).iter().enumerate() {
                assert_eq!(
                    run.refined.partition.file_count(),
                    changeset.len(),
                    "{label} {} run {index}: every file assigned exactly once",
                    shape.slug()
                );
                let mut seen = std::collections::BTreeSet::new();
                for members in run.refined.partition.groups.values() {
                    for path in members {
                        assert!(
                            seen.insert(path.clone()),
                            "{label} {} run {index}: {path} in two groups",
                            shape.slug()
                        );
                    }
                }
            }
        }
    }
}

/// The verdict in docs/GROUPING_PASSES.md, locked against the committed runs.
/// A single full call is below the bar on this fixture (0.361 mean vs 0.394)
/// and only the consensus of three clears it, so the shape that ships is three
/// calls and a vote, not one call. If this test fails the verdict has moved.
#[test]
fn consensus_of_three_clears_the_bar_where_one_call_does_not() {
    let (label, changeset, expected) = fixtures()
        .into_iter()
        .find(|(label, _, _)| label == ORCA_FIXTURE)
        .expect("the checked-in fixture is always present");
    let runs = load_runs(&label, &changeset, Shape::Full);
    assert!(runs.len() >= 3, "need three recorded full runs to vote");

    let bar = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected)
        .f1;
    for run in &runs {
        let f1 = run.refined.partition.score_against(&expected).f1;
        assert!(
            f1 < bar,
            "a single full call scored {f1:.3}, at or over {bar:.3}"
        );
    }

    let partitions: Vec<_> = runs
        .iter()
        .map(|run| run.refined.partition.clone())
        .collect();
    let voted = refine::consensus(&partitions[..3])
        .expect("three partitions vote")
        .score_against(&expected)
        .f1;
    assert!(
        voted > bar,
        "consensus of three scored {voted:.3}, under {bar:.3}"
    );
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
            "without directory tokens".into(),
            GroupingConfig {
                use_dir_tokens: false,
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

/// True on both fixtures since directory evidence was turned on (`gd-26r.21`).
/// It was true on fixture 1 only while the engine grouped on filename tokens
/// alone: fixture 2 lost to parent-directory, 0.282 against 0.312.
#[test]
fn heuristic_beats_every_baseline_on_both_fixtures() {
    for (label, changeset, expected) in fixtures() {
        let heuristic = passes::group(&changeset, GroupingConfig::default())
            .partition()
            .score_against(&expected);

        for (baseline_label, baseline) in [
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
                "{label}: heuristic F1 {:.3} must beat {baseline_label} F1 {:.3}",
                heuristic.f1,
                score.f1
            );
        }
    }
}

/// Directory evidence is the whole of fixture 2's result, so the ablation is
/// locked rather than left to the report: without directory tokens the engine
/// falls back below parent-directory, which is the state `gd-26r.20` recorded.
#[test]
fn directory_evidence_carries_the_second_fixture() {
    let Some((changeset, expected)) = external_fixture(SECOND_FIXTURE) else {
        skip_notice(SECOND_FIXTURE);
        return;
    };

    let with_dirs = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected);
    let without_dirs = passes::group(
        &changeset,
        GroupingConfig {
            use_dir_tokens: false,
            ..GroupingConfig::default()
        },
    )
    .partition()
    .score_against(&expected);
    let parent_dir = passes::baseline::parent_directory(&changeset).score_against(&expected);

    assert!(
        (0.36..0.40).contains(&with_dirs.f1),
        "second-fixture F1 drifted: {:.3}",
        with_dirs.f1
    );
    assert!(
        without_dirs.f1 < parent_dir.f1 && with_dirs.f1 > parent_dir.f1,
        "directory evidence must be what clears parent-directory here: \
         with {:.3}, without {:.3}, parent-directory {:.3}",
        with_dirs.f1,
        without_dirs.f1,
        parent_dir.f1
    );
}

/// Every fixture available on this machine: the checked-in one always, the
/// external one when its directory is present.
fn fixtures() -> Vec<(String, Changeset, Partition)> {
    let (changeset, expected) = orca();
    let mut fixtures = vec![("orca-971b16754".to_string(), changeset, expected)];
    match external_fixture(SECOND_FIXTURE) {
        Some((changeset, expected)) => {
            fixtures.push((SECOND_FIXTURE.to_string(), changeset, expected))
        }
        None => skip_notice(SECOND_FIXTURE),
    }
    fixtures
}

#[test]
fn scoring_harness_is_calibrated() {
    for (label, _, expected) in fixtures() {
        let self_score = expected.score_against(&expected);
        assert!(
            (self_score.f1 - 1.0).abs() < 1e-9,
            "{label}: a partition must score 1.0 against itself"
        );
    }

    let (_, expected) = orca();
    let self_score = expected.score_against(&expected);
    assert_eq!(self_score.group_count, 13);
    assert!((self_score.largest_share - 38.0 / 161.0).abs() < 1e-9);
}

/// Locks the numbers recorded in docs/GROUPING_PASSES.md so a change to the
/// passes has to move them deliberately.
#[test]
fn recorded_numbers_hold() {
    let (changeset, expected) = orca();
    let score = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected);

    assert!(
        (0.38..0.41).contains(&score.f1),
        "F1 drifted: {:.3}",
        score.f1
    );
    assert!(
        score.precision > 0.45,
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

/// An extension names a file's language or role, never its concern, so one that
/// survives tokenisation becomes a cluster key and groups files by layer —
/// exactly what GROUPING.md rule 1 forbids. `.cs` did this on fixture 2
/// (`handler-cs`, `port-cs`, `program-cs`), so the whole list is under test, not
/// the one entry that was caught.
#[test]
fn extensions_never_survive_as_concern_tokens() {
    let cases = [
        ("src/Api/Program.cs", "Program", vec!["program"]),
        (
            "src/Data/Migrations/20240101_Init.Designer.cs",
            "20240101_Init-Designer",
            vec!["20240101", "init", "designer"],
        ),
        (
            "src/Billing/Northwind.Billing.Api.csproj",
            "Northwind-Billing-Api",
            vec!["northwind", "billing", "api"],
        ),
        (
            "Directory.Build.props",
            "Directory-Build",
            vec!["directory", "build"],
        ),
        ("src/main/java/Claim.java", "Claim", vec!["claim"]),
        (
            "web/src/hooks/use-patient.ts",
            "use-patient",
            vec!["use", "patient"],
        ),
        ("web/src/pages/Worklist.razor", "Worklist", vec!["worklist"]),
        (
            "api/appsettings.Development.json",
            "appsettings-Development",
            vec!["appsetting", "development"],
        ),
        ("infra/main.tf", "main", vec!["main"]),
    ];

    for (path, stem, tokens) in cases {
        let file = ChangedFile {
            path: path.to_string(),
            kind: ChangeKind::Modified,
            rename_from: None,
        };
        assert_eq!(file.stem(), stem, "stem of {path}");
        assert_eq!(file.name_tokens(), tokens, "tokens of {path}");
    }
}

/// The same guarantee where it actually bites: no group the engine produces may
/// be named after a file extension.
#[test]
fn no_group_is_named_after_an_extension() {
    for (label, changeset, _) in fixtures() {
        let grouping = passes::group(&changeset, GroupingConfig::default());
        for assignment in &grouping.assignments {
            for token in assignment.group.split('-') {
                assert!(
                    !changeset::EXTENSIONS.contains(&token),
                    "{label}: group `{}` is named after the extension `{token}`",
                    assignment.group
                );
            }
        }
    }
}

#[test]
fn grouping_is_a_strict_partition() {
    for (label, changeset, _) in fixtures() {
        let grouping = passes::group(&changeset, GroupingConfig::default());
        let partition = grouping.partition();

        assert_eq!(
            partition.file_count(),
            changeset.len(),
            "{label}: every file assigned exactly once"
        );
        let mut seen = std::collections::BTreeSet::new();
        for members in partition.groups.values() {
            for path in members {
                assert!(
                    seen.insert(path.clone()),
                    "{label}: {path} appears in two groups"
                );
            }
        }
    }
}
