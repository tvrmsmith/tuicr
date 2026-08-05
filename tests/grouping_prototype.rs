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

use std::path::PathBuf;

use grouping::changeset::{self, ChangeKind, ChangedFile, Changeset};
use grouping::passes::{self, GroupingConfig};
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
/// grouping for it, so nothing here is scored — only whether a pass fires and
/// whether the rename pair lands in one group.
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

    let partition = grouping.partition();
    for file in renamed {
        let Some(from) = file.rename_from.as_deref() else {
            continue;
        };
        let group_of = |path: &str| {
            partition
                .groups
                .iter()
                .find(|(_, members)| members.iter().any(|member| member == path))
                .map(|(name, _)| name.clone())
        };
        println!(
            "  rename {from}\n      -> {} : group {:?}",
            file.path,
            group_of(&file.path)
        );
        assert!(
            group_of(&file.path).is_some(),
            "the surviving half of a rename must be grouped"
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

/// True on fixture 1 only. Fixture 2 is the counter-example and has its own
/// test below: the pass set does not transfer.
#[test]
fn heuristic_beats_every_baseline_on_the_first_fixture() {
    let (changeset, expected) = orca();
    {
        let label = "orca-971b16754";
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

/// The transfer result, locked so it cannot quietly change: on a .NET-shaped
/// changeset the heuristics lose to grouping by parent directory. Recorded as a
/// finding, not an aspiration — see docs/GROUPING_PASSES.md.
#[test]
fn second_fixture_loses_to_parent_directory() {
    let Some((changeset, expected)) = external_fixture(SECOND_FIXTURE) else {
        skip_notice(SECOND_FIXTURE);
        return;
    };

    let heuristic = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected);
    let parent_dir = passes::baseline::parent_directory(&changeset).score_against(&expected);

    assert!(
        heuristic.f1 < parent_dir.f1,
        "the recorded inversion is gone: heuristic F1 {:.3} now beats parent-directory {:.3}. \
         Good news, but docs/GROUPING_PASSES.md needs rewriting.",
        heuristic.f1,
        parent_dir.f1
    );
    assert!(
        (0.27..0.30).contains(&heuristic.f1),
        "second-fixture F1 drifted: {:.3}",
        heuristic.f1
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
