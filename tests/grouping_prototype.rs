//! Prototype grouping engine scored against the hand-grouped fixtures.
//!
//! Run the report with:
//!   cargo test --test grouping_prototype -- --ignored --nocapture report
//!
//! Fixture 1 (orca) is checked in, paths-only and provisional
//! (tests/fixtures/grouping/README.md), so its numbers compare passes against
//! each other and catch regressions. They do not declare a pass good.
//!
//! Fixture 2 is hand-grouped but carries file paths from a private work
//! repository, so it is **not** checked in. It is read at runtime from
//! `$TUICR_GROUPING_FIXTURES` (default `~/.local/share/tuicr-fixtures`). Tests
//! that are only about it are `#[ignore]`d, so a run without the directory
//! lists them as ignored; the rest print a notice and score fixture 1 alone.
//! docs/GROUPING_PASSES.md records what it produced.

mod grouping;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use grouping::changeset::{self, ChangeKind, ChangedFile, Changeset};
use grouping::passes::{self, GroupingConfig};
use grouping::refine::{self, RunRecord, Shape};
use grouping::score::{self, Partition, Score};

const ORCA_FILES: &str = include_str!("fixtures/grouping/orca-971b16754.files");
const ORCA_GROUPS: &str = include_str!("fixtures/grouping/orca-971b16754.groups");
/// A synthetic changeset with no ground truth, checked in so the passes that
/// fire on nothing in fixture 1 are exercised without the private fixture.
const FIRE_CHECK_FILES: &str = include_str!("fixtures/grouping/fire-check-synthetic.files");

/// The second, hand-grouped fixture: one .NET-dominant PR, 158 files.
const SECOND_FIXTURE: &str = "meridian-6c22fda02";
/// Paths-only, no expected grouping: exists purely so the two passes that
/// fired on nothing in fixture 1 meet a lockfile and a CI file.
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

/// The same, for a test that has nothing to say without the external fixture
/// and is `#[ignore]`d for it: reaching here means it was asked for explicitly.
fn require_external_fixture(name: &str) -> (Changeset, Partition) {
    external_fixture(name).unwrap_or_else(|| {
        panic!(
            "no fixture {name} in {} (set TUICR_GROUPING_FIXTURES)",
            fixture_dir().display()
        )
    })
}

fn external_changeset(name: &str) -> Option<Changeset> {
    let files = std::fs::read_to_string(fixture_dir().join(format!("{name}.files"))).ok()?;
    Some(Changeset::parse(&files))
}

/// Says which fixture was not on this machine, for the callers that still have
/// fixture 1 to work on. libtest captures this like any other output, so it is
/// visible under `--nocapture` and nowhere else — a test whose *only* subject
/// is the external fixture must be `#[ignore]`d instead, so libtest lists it as
/// ignored rather than passing green having asserted nothing.
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

/// A count alone is not well-formedness: one misspelt path in the `.groups`
/// file and one duplicated path leave the count intact, while the scorer — which
/// matches paths by string — counts the typo's pairs against precision and never
/// for recall, silently deflating every published number. So the grouped paths
/// are checked as a *set* against the changeset, and the header-less bucket
/// `Partition::parse` opens for stray lines is required to be absent.
fn assert_well_formed(label: &str, changeset: &Changeset, expected: &Partition) {
    assert!(
        !expected.groups.contains_key(score::UNGROUPED),
        "{label}: {} lines precede the first [group] header",
        expected.groups.get(score::UNGROUPED).map_or(0, Vec::len)
    );

    let grouped: BTreeSet<&str> = expected
        .groups
        .values()
        .flatten()
        .map(String::as_str)
        .collect();
    let changed: BTreeSet<&str> = changeset.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        grouped.difference(&changed).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "{label}: the expected grouping names paths the changeset does not"
    );
    assert_eq!(
        changed.difference(&grouped).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "{label}: the changeset has paths the expected grouping does not place"
    );
    assert_eq!(
        expected.file_count(),
        grouped.len(),
        "{label}: a path is placed in more than one group"
    );
}

#[test]
fn fixture_is_well_formed() {
    let (changeset, expected) = orca();
    assert_eq!(changeset.len(), 161, "fixture changeset size");
    assert_well_formed(ORCA_FIXTURE, &changeset, &expected);
}

#[test]
#[ignore = "needs $TUICR_GROUPING_FIXTURES"]
fn second_fixture_is_well_formed() {
    let (changeset, expected) = require_external_fixture(SECOND_FIXTURE);
    assert_eq!(changeset.len(), 158, "second fixture changeset size");
    assert_well_formed(SECOND_FIXTURE, &changeset, &expected);
}

/// A printer, not a test: it asserts nothing and is read with `--nocapture`.
/// Ignored so a `cargo test` run is not paying for it.
#[test]
#[ignore = "printer; run with --ignored --nocapture report"]
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

/// The two passes that matched nothing in fixture 1 were kept as unscored
/// bets. These are the changesets that make them fire or expose them as dead
/// code: each carries a lockfile and a CI workflow, and a rename besides, so
/// the deleted rename pass stays deleted. There is no hand grouping for either,
/// so nothing here is scored — only whether a pass fires.
///
/// The synthetic one is checked in precisely so this runs in CI, where the
/// private fixture is absent and neither calibration fixture carries a lockfile
/// or a `.github/` path — without it both passes go unverified on every run.
#[test]
fn unfired_passes_meet_a_changeset_that_should_fire_them() {
    check_unfired_passes_fire("fire-check-synthetic", &Changeset::parse(FIRE_CHECK_FILES));
}

/// The same check against the real changeset the synthetic one stands in for.
#[test]
#[ignore = "needs $TUICR_GROUPING_FIXTURES"]
fn unfired_passes_meet_the_recorded_fire_check_changeset() {
    let changeset = external_changeset(FIRE_CHECK_FIXTURE).unwrap_or_else(|| {
        panic!(
            "no fixture {FIRE_CHECK_FIXTURE} in {} (set TUICR_GROUPING_FIXTURES)",
            fixture_dir().display()
        )
    });
    check_unfired_passes_fire(FIRE_CHECK_FIXTURE, &changeset);
}

fn check_unfired_passes_fire(label: &str, changeset: &Changeset) {
    println!("\n\n########## {label} (fire check, no ground truth) ##########");
    println!("  {} files", changeset.len());

    let renamed: Vec<_> = changeset
        .files
        .iter()
        .filter(|file| file.kind == ChangeKind::Renamed)
        .collect();
    println!("  {} renames in the changeset", renamed.len());

    let grouping = passes::group(changeset, GroupingConfig::default());
    println!("\n=== which passes fired ===");
    for (pass, count) in grouping.pass_hits() {
        println!("  {pass:<20} {count:>3} files");
    }

    let hits = grouping.pass_hits();
    assert!(
        hits.contains_key("mechanical"),
        "{label}: the changeset carries a lockfile, so the mechanical pass must fire"
    );
    assert!(
        hits.contains_key("config-ci"),
        "{label}: the changeset carries a CI workflow, so the config-ci pass must fire"
    );

    assert!(
        !renamed.is_empty(),
        "{label}: the fire-check changeset is supposed to carry a rename"
    );
}

/// GROUPING.md rule 11, as it now reads. Git reports a rename as a single entry
/// keyed by the new path, so a rename's two halves are one file in the diff and
/// there is no pair to reunite — the old `enforce_rename_pairs` looked the old
/// path up in `assigned`, where it could never be a key, and so never executed
/// on any input. What rule 11 asks for is a property of the representation, and
/// this is the test that says so.
#[test]
fn a_rename_is_one_file_in_the_changeset() {
    let changeset = Changeset::parse(
        "M\tsrc/app/scheduler.ts\n\
         R096\tsrc/app/old-catalog.ts\tsrc/app/widget-catalog.ts\n\
         A\tsrc/app/widget-catalog.test.ts\n",
    );
    let renamed = &changeset.files[1];
    assert_eq!(renamed.path, "src/app/widget-catalog.ts");
    assert_eq!(renamed.kind, ChangeKind::Renamed);
    assert_eq!(
        renamed.rename_from.as_deref(),
        Some("src/app/old-catalog.ts")
    );

    assert_eq!(
        changeset.len(),
        3,
        "the rename contributed one file and not two"
    );
    assert!(
        !changeset
            .files
            .iter()
            .any(|file| Some(file.path.as_str()) == renamed.rename_from.as_deref()),
        "the old path is not a file in the changeset"
    );
}

/// The agglomerative baseline answers "could paths alone do better", so it has
/// to report the group count it was asked for. It names groups from the
/// changeset's own vocabulary, and two clusters of the same vocabulary derive
/// the same name; `Partition` buckets by name, so an unsuffixed collision would
/// union them and the baseline would score as a coarser grouping than the one
/// it actually produced.
#[test]
fn the_agglomerative_baseline_keeps_its_group_count_when_two_clusters_share_a_name() {
    let changeset = Changeset::parse(
        "M\talpha/widget-catalog.ts\n\
         M\talpha/widget-catalog.test.ts\n\
         M\tbeta/widget-catalog.ts\n\
         M\tbeta/widget-catalog.test.ts\n",
    );

    let partition = passes::agglomerative_grouping(&changeset, 2);
    assert_eq!(
        partition.groups.len(),
        2,
        "two clusters were asked for and two must be reported: {:?}",
        partition.groups
    );
    assert_eq!(
        partition.file_count(),
        changeset.len(),
        "suffixing a colliding name must not drop or duplicate a file"
    );
}

// --- the passes that only fixture-wide scores covered -----------------------
//
// Rule 4, rule 9, absorption and the close-call annotation were exercised only
// by the two calibration fixtures' aggregate F1, where any of them could stop
// firing and cost a point or two of a number nobody reads as a pass boundary.
// Each is aimed at here with a changeset built to make it decide something, and
// judged on where the files land rather than on how the pass is written.

/// Concern-bearing filler, in dirs and vocabulary the tests below never touch.
/// Cluster keys are dropped once they name more than `ubiquity_threshold` of the
/// changeset, so a pass that only decides between two clusters still needs a
/// changeset big enough for either to be distinctive.
const FILLER: &str = "M\tsrc/theme/color-token.ts\n\
     M\tsrc/theme/color-scale.ts\n\
     M\tsrc/theme/color-mode.ts\n\
     M\tsrc/upload/chunk-reader.ts\n\
     M\tsrc/upload/chunk-writer.ts\n\
     M\tsrc/upload/chunk-queue.ts\n\
     M\tsrc/mailer/digest-daily.ts\n\
     M\tsrc/mailer/digest-weekly.ts\n\
     M\tsrc/mailer/digest-retry.ts\n\
     M\tsrc/mailer/digest-bounce.ts\n\
     M\tsrc/locale/plural-rule.ts\n\
     M\tsrc/locale/plural-format.ts\n\
     M\tsrc/locale/plural-parse.ts\n";

/// Where each path landed, by group name.
fn placement(changeset: &Changeset, config: GroupingConfig) -> BTreeMap<String, String> {
    passes::group(changeset, config)
        .assignments
        .iter()
        .map(|a| (a.path.clone(), a.group.clone()))
        .collect()
}

/// GROUPING.md rule 4: a test file follows its production file even when a token
/// cluster has already claimed it for a different concern. Here
/// `export-stock.test.ts` carries both `stock` and `export`, and the
/// `stock` cluster is the stronger of the two, so the cluster pass takes it
/// away from the file it tests — which is what the rule exists to undo. The
/// config switch is the control: same changeset, same clusters, and the only
/// thing that moves is the file the rule is about.
#[test]
fn a_test_file_leaves_its_cluster_for_the_group_of_the_file_it_tests() {
    let changeset = Changeset::parse(&format!(
        "M\tsrc/inventory/stock.ts\n\
         M\tsrc/inventory/stock-void.ts\n\
         A\tsrc/inventory/stock-batch.test.ts\n\
         A\tsrc/inventory/stock-line.test.ts\n\
         A\tsrc/inventory/export-stock.test.ts\n\
         M\tsrc/inventory/export.ts\n\
         M\tsrc/report/export-csv.ts\n\
         M\tsrc/report/export-pdf.ts\n\
         M\tsrc/report/export-html.ts\n\
         {FILLER}"
    ));
    let test = "src/inventory/export-stock.test.ts";
    let production = "src/inventory/export.ts";

    let following = placement(&changeset, GroupingConfig::default());
    let cluster_keeps_it = placement(
        &changeset,
        GroupingConfig {
            tests_follow_production: false,
            ..GroupingConfig::default()
        },
    );

    assert_eq!(
        following[test], following[production],
        "the test did not follow {production}; the whole placement was {following:?}"
    );
    assert_ne!(
        cluster_keeps_it[test], cluster_keeps_it[production],
        "with the rule off the cluster must keep the test, or this changeset does not \
         exercise rule 4 at all: {cluster_keeps_it:?}"
    );
    assert_eq!(
        cluster_keeps_it[test], cluster_keeps_it["src/inventory/stock.ts"],
        "the cluster that loses the test must be the stock one: {cluster_keeps_it:?}"
    );
}

/// GROUPING.md rule 9: a doc covering the whole change is its own group, and a
/// doc sitting beside the code it documents is not. The two halves are one
/// decision — the pass tests scope, not the extension — so both are asserted
/// against one changeset.
#[test]
fn a_whole_change_doc_groups_apart_from_a_doc_beside_its_code() {
    let changeset = Changeset::parse(&format!(
        "M\tREADME.md\n\
         M\tdocs/architecture.md\n\
         A\tdocs/grouping.md\n\
         M\tsrc/upload/README.md\n\
         {FILLER}"
    ));

    let placed = placement(&changeset, GroupingConfig::default());
    let whole_change = &placed["README.md"];
    assert_eq!(
        placed["docs/architecture.md"], *whole_change,
        "the whole-change docs must be one group: {placed:?}"
    );
    assert_eq!(
        placed["docs/grouping.md"], *whole_change,
        "the whole-change docs must be one group: {placed:?}"
    );
    assert_ne!(
        placed["src/upload/README.md"], *whole_change,
        "a doc beside the code it documents belongs to that code, not to the docs group: \
         {placed:?}"
    );
}

/// Absorption is the alternative to a directory bucket for a file no cluster
/// claimed: `stream-reader.ts` carries no key any cluster formed on, and its
/// only evidence is one filename token and a directory shared with the chunk
/// cluster. Switched off it falls through to `dir:`, the outcome GROUPING.md
/// rule 6 calls a smell — so the switch shows both what the pass does and what
/// it is for.
#[test]
fn an_unclaimed_file_is_absorbed_by_its_nearest_group_instead_of_a_directory_bucket() {
    let changeset = Changeset::parse(&format!(
        "M\tsrc/inventory/stock.ts\n\
         M\tsrc/inventory/stock-void.ts\n\
         M\tsrc/inventory/stock-batch.ts\n\
         M\tsrc/inventory/stock-line.ts\n\
         M\tsrc/inventory/stock-note.ts\n\
         M\tsrc/upload/chunk-retry.ts\n\
         M\tsrc/upload/chunk-abort.ts\n\
         M\tsrc/upload/stream-reader.ts\n\
         {FILLER}"
    ));
    let stray = "src/upload/stream-reader.ts";

    let absorbed = placement(&changeset, GroupingConfig::default());
    let dropped = placement(
        &changeset,
        GroupingConfig {
            absorb_leftovers: false,
            ..GroupingConfig::default()
        },
    );

    assert_eq!(
        absorbed[stray], absorbed["src/upload/chunk-reader.ts"],
        "the stray must join the group it shares its vocabulary with: {absorbed:?}"
    );
    assert!(
        dropped[stray].starts_with("dir:"),
        "with absorption off the stray must fall through to a directory bucket, or this \
         changeset does not exercise the pass: {dropped:?}"
    );
}

/// The close-call annotation GROUPING.md asks for, which `gd-26r.8` will show a
/// reviewer as "nearly went here instead". `query-cache.ts` carries two formed
/// cluster keys with close scores: the stronger takes it, and the weaker has to
/// be recorded rather than forgotten. Only a file whose second-best claim is a
/// real group gets one, so the filler files must come back unannotated.
#[test]
fn a_file_two_clusters_nearly_took_records_the_one_that_lost() {
    let changeset = Changeset::parse(&format!(
        "M\tsrc/core/query-parser.ts\n\
         M\tsrc/core/query-plan.ts\n\
         M\tsrc/core/query-filter.ts\n\
         M\tsrc/core/query-index.ts\n\
         M\tsrc/core/query-cache.ts\n\
         M\tsrc/store/cache-warm.ts\n\
         M\tsrc/store/cache-evict.ts\n\
         M\tsrc/store/cache-stat.ts\n\
         {FILLER}"
    ));
    let contested = "src/core/query-cache.ts";

    let grouping = passes::group(&changeset, GroupingConfig::default());
    let close_calls: BTreeMap<&str, &passes::RunnerUp> = grouping
        .close_calls()
        .into_iter()
        .map(|a| (a.path.as_str(), a.runner_up.as_ref().unwrap()))
        .collect();

    let runner_up = close_calls
        .get(contested)
        .unwrap_or_else(|| panic!("{contested} was contested and must say so: {close_calls:?}"));
    assert_eq!(
        runner_up.group, "cache",
        "the group that lost {contested} must be named: {runner_up:?}"
    );
    assert!(
        !close_calls.contains_key("src/core/query-parser.ts"),
        "a file only one cluster ever claimed is not a close call: {close_calls:?}"
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

/// Runs per shape in the committed corpus. Every published figure is a property
/// of this N, so the tests that lock those figures assert it rather than
/// accepting whatever happens to be on disk.
const RECORDED_RUNS: usize = 10;

/// The model docs/GROUPING_PASSES.md reports. A run is only comparable with
/// runs of the same model, so a second arm recorded with `TUICR_REFINE_MODEL`
/// is a second experiment beside this one rather than five more runs of it.
const PUBLISHED_MODEL: &str = "claude-opus-5";

/// The published fixture-1 `full` triple distribution: 50 of 120 clear the bar,
/// the spread runs 0.343 to 0.439, and the upper median is 0.369. Each figure is
/// locked as a band closed on both sides, with a little slack for a re-record.
/// A count that fell to 0 would make the published "coin flip" as wrong as a
/// count that rose to a majority, so an unnoticed improvement has to fail the
/// lock too — `docs/GROUPING_PASSES.md` would have to be rewritten either way.
const TRIPLES_OVER_BAR_FLOOR: usize = 45;
const TRIPLES_OVER_BAR_CEILING: usize = 55;
const PUBLISHED_TRIPLE_MIN: f64 = 0.343;
const PUBLISHED_TRIPLE_MAX: f64 = 0.439;
const PUBLISHED_TRIPLE_UPPER_MEDIAN: f64 = 0.369;
/// Slack around each published triple F1: wide enough to absorb a re-record,
/// narrow enough that a moved distribution fails.
const PUBLISHED_F1_SLACK: f64 = 0.02;

/// The other half of the fixture-1 verdict: `merge-only` mean 0.417 against the
/// 0.394 bar, 9 of 10 single calls over it, 107 of 120 triples. Bracketed on
/// both sides like the `full` figures — a merge-only arm that quietly got worse
/// would leave "the shape that should ship on fixture 1" false, and one that got
/// better would leave the comparison with `full` understated.
const MERGE_ONLY_MEAN_F1: f64 = 0.417;
/// A count out of ten admits no band: the proportional slack the triple bands
/// carry rounds to half a run, and both 8 and 10 make the published "9 of 10"
/// false, so the figure is locked as itself.
const MERGE_ONLY_SINGLES_OVER_BAR: usize = 9;
const MERGE_ONLY_TRIPLES_OVER_BAR_FLOOR: usize = 100;
const MERGE_ONLY_TRIPLES_OVER_BAR_CEILING: usize = 115;

/// `full`'s own fixture-1 row, published as mean 0.353 across a 0.321-0.373
/// spread. The triple distribution was bracketed and the single calls only
/// checked against the bar, so the arm could have moved a long way under the bar
/// — in either direction — with the row left stale and nothing failing.
const FULL_MEAN_F1: f64 = 0.353;
const FULL_WORST_F1: f64 = 0.321;
const FULL_BEST_F1: f64 = 0.373;

/// `full`'s fixture-1 stability row: pairwise agreement 0.827 mean against 0.715
/// worst, 11.8% of files identically placed in all ten runs, 0.790 mean overlap.
/// These are the figures the "opt-in, not on by default" recommendation rests on
/// as much as the accuracy ones.
const FULL_AGREEMENT_MEAN: f64 = 0.827;
const FULL_AGREEMENT_WORST: f64 = 0.715;
const FULL_IDENTICAL_SHARE: f64 = 0.118;
const FULL_MEAN_OVERLAP: f64 = 0.790;
/// Agreement and overlap sit where the F1s do, so they carry the same slack.
const PUBLISHED_STABILITY_SLACK: f64 = 0.02;
/// The identical share does not: 0.118 of 161 files is nineteen of them, and an
/// F1-sized band would let five more or five fewer through while "11.8% of files
/// land in the same group every time" stayed published. A tenth of the figure is
/// under two files, which is as fine as a re-record can be held to.
const PUBLISHED_SHARE_SLACK: f64 = 0.012;

/// The third fixture-1 arm: `full-coarse` mean 0.386 under the 0.394 bar, 4 of
/// 10 single calls over it, 58 of 120 triples. Bracketed like the other two, so
/// the middle shape cannot drift into either of its neighbours unremarked.
const FULL_COARSE_MEAN_F1: f64 = 0.386;
const FULL_COARSE_SINGLES_OVER_BAR: usize = 4;
const FULL_COARSE_TRIPLES_OVER_BAR_FLOOR: usize = 52;
const FULL_COARSE_TRIPLES_OVER_BAR_CEILING: usize = 64;

/// Writes one prompt per fixture per shape for the run script to send. Not an
/// assertion but a side effect on `target/`, so it is ignored by default and
/// invoked deliberately — run it, then run the script. What the prompt must
/// *say* is pinned by `the_emitted_prompt_carries_the_contract` instead.
#[test]
#[ignore = "writes prompts for scripts/grouping-refine-runs.sh; run with --ignored"]
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

/// A markdown ordered list read back as the items it declares: ordinal to text,
/// with each item's continuation lines folded into it and anything unindented or
/// blank closing the item it follows. Both `docs/GROUPING.md` and the digest the
/// prompt carries are written as one of these, which is what makes them
/// comparable at all. A rule's title is bold by convention, but the ordinal is
/// what makes it a rule, so an unbolded one still counts here rather than
/// dropping out of the comparison and taking its digest entry with it.
fn numbered_items(text: &str) -> BTreeMap<u32, String> {
    let mut items: BTreeMap<u32, String> = BTreeMap::new();
    let mut open: Option<u32> = None;

    for line in text.lines() {
        let indented = line.starts_with([' ', '\t']);
        let starts = line
            .split_once(". ")
            .and_then(|(head, rest)| head.parse::<u32>().ok().map(|ordinal| (ordinal, rest)));

        match starts {
            Some((ordinal, rest)) if !indented => {
                items.insert(ordinal, rest.to_string());
                open = Some(ordinal);
            }
            _ if indented && !line.trim().is_empty() => {
                if let Some(ordinal) = open {
                    let item = items.get_mut(&ordinal).expect("an open item has an entry");
                    item.push(' ');
                    item.push_str(line.trim());
                }
            }
            _ => open = None,
        }
    }

    items
}

/// The words a rule is made of, for matching one statement of it against
/// another: lowercased, punctuation and markdown emphasis dropped, and anything
/// under three characters left out as carrying no subject matter.
fn rule_words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|word| word.len() > 2)
        .map(str::to_ascii_lowercase)
        .collect()
}

/// How much of a digest rule the documented rule accounts for. Containment
/// rather than symmetric overlap: the digest is a compression, so the doc says
/// more than it does and a Jaccard score would punish the longest doc rules for
/// being long.
fn covered_by(digest: &BTreeSet<String>, documented: &BTreeSet<String>) -> f64 {
    if digest.is_empty() {
        return 0.0;
    }
    digest.intersection(documented).count() as f64 / digest.len() as f64
}

/// The prompt carries a hand-written digest of `docs/GROUPING.md`, so an added
/// or removed rule in the doc leaves the digest a rule short with nothing
/// failing — rule 11's wording already drifted once on this branch. The two are
/// not compared as prose: the digest is a compression, and asserting the texts
/// match would fail on the compression itself. What is locked is that each
/// digest rule still restates *its own* documented rule — its vocabulary has to
/// come from that rule more than from any other — so a rule renumbered, swapped
/// with its neighbour, or replaced under an unchanged number fails here, while
/// a rewording that says the same thing passes.
#[test]
fn every_documented_rule_reaches_the_model() {
    const GROUPING_MD: &str = include_str!("../docs/GROUPING.md");
    let documented: BTreeMap<u32, BTreeSet<String>> = numbered_items(GROUPING_MD)
        .into_iter()
        .map(|(ordinal, text)| (ordinal, rule_words(&text)))
        .collect();
    let ordinals: BTreeSet<u32> = documented.keys().copied().collect();
    assert_eq!(
        ordinals,
        (1..=documented.len() as u32).collect::<BTreeSet<u32>>(),
        "docs/GROUPING.md's rules are a contiguous numbered list from 1"
    );

    let changeset = Changeset::parse(FIRE_CHECK_FILES);
    let grouping = passes::group(&changeset, GroupingConfig::default());
    for shape in Shape::ALL {
        let slug = shape.slug();
        let digest = numbered_items(&refine::prompt(&changeset, &grouping, shape));
        assert_eq!(
            digest.keys().copied().collect::<BTreeSet<u32>>(),
            ordinals,
            "{slug}: the prompt's digest and docs/GROUPING.md name different rules"
        );

        for (ordinal, text) in &digest {
            let words = rule_words(text);
            let mut ranked: Vec<(f64, u32)> = documented
                .iter()
                .map(|(number, rule)| (covered_by(&words, rule), *number))
                .collect();
            ranked.sort_by(|left, right| right.0.total_cmp(&left.0));
            let (best, matched) = ranked[0];
            let (runner_up, other) = ranked[1];
            assert!(
                matched == *ordinal && best > runner_up,
                "{slug}: the digest's rule {ordinal} reads as docs/GROUPING.md rule {matched} \
                 ({best:.2} of its words) ahead of rule {other} ({runner_up:.2}), so the two \
                 lists no longer state the same rule under this number"
            );
        }
    }
}

/// The prompt is the one part of this pass that deterministic CI can pin: it is
/// a generated interface handed to an agent, and `emit_refine_prompts` only
/// writes it to disk. What the model then does with it is not testable here.
#[test]
fn the_emitted_prompt_carries_the_contract() {
    let changeset = Changeset::parse(FIRE_CHECK_FILES);
    let grouping = passes::group(&changeset, GroupingConfig::default());
    let partition = grouping.partition();
    let group_count = partition.groups.len();

    for shape in Shape::ALL {
        let prompt = refine::prompt(&changeset, &grouping, shape);
        let slug = shape.slug();

        let blocks = parse_prompt_groups(&prompt);
        let shown: BTreeMap<&str, Vec<&str>> = blocks
            .iter()
            .map(|(name, _, members)| (*name, members.clone()))
            .collect();
        let expected: BTreeMap<&str, Vec<&str>> = partition
            .groups
            .iter()
            .map(|(name, members)| {
                (
                    name.as_str(),
                    members.iter().map(String::as_str).collect::<Vec<&str>>(),
                )
            })
            .collect();
        assert_eq!(
            shown, expected,
            "{slug}: the prompt must show each heuristic group with its own files"
        );
        for (name, declared, members) in &blocks {
            assert_eq!(
                *declared,
                members.len(),
                "{slug}: group `{name}` declares a file count it does not list"
            );
        }
        let order: Vec<(usize, &str)> = blocks
            .iter()
            .map(|(name, _, members)| (members.len(), *name))
            .collect();
        let mut largest_first = order.clone();
        largest_first.sort_by_key(|(size, name)| (std::cmp::Reverse(*size), *name));
        assert_eq!(
            order, largest_first,
            "{slug}: groups are shown largest first, so rule 6's residual smells read first"
        );

        let (schema, permission, forbidden): (&str, &str, &[&str]) = match shape {
            Shape::Full => (
                "\"files\": [\"path\", ...]",
                "You may merge groups, split them, move individual files between them",
                &["Prefer fewer, larger groups"],
            ),
            Shape::FullCoarse => (
                "\"files\": [\"path\", ...]",
                "Prefer fewer, larger groups",
                &[],
            ),
            Shape::MergeOnly => (
                "\"merge\": [\"input-group-name\", ...]",
                "You may NOT move an individual file between groups, and you may NOT split a group",
                &[
                    "You may merge groups, split them",
                    "Prefer fewer, larger groups",
                ],
            ),
            Shape::NamingOnly => (
                "\"was\": \"input-group-name\"",
                "Do NOT change which files are in which group",
                &["\"merge\"", "Prefer fewer, larger groups"],
            ),
        };
        assert!(
            prompt.contains(schema),
            "{slug}: the prompt must state the answer schema `{schema}`"
        );
        assert!(
            prompt.contains(permission),
            "{slug}: the prompt must state what this shape may change: `{permission}`"
        );
        for phrase in forbidden {
            assert!(
                !prompt.contains(phrase),
                "{slug}: the prompt must not carry another shape's instruction: `{phrase}`"
            );
        }

        match shape {
            Shape::Full | Shape::FullCoarse => assert!(
                prompt.contains(&format!("of the {} input paths", changeset.len())),
                "{slug}: the prompt must state the file count it demands back"
            ),
            Shape::MergeOnly | Shape::NamingOnly => assert!(
                prompt.contains(&format!("of the {group_count} input group")),
                "{slug}: the prompt must state the group count it demands back"
            ),
        }
    }
}

/// The group blocks the prompt renders, in the order it renders them: name, the
/// file count it declares, and the paths listed under it.
fn parse_prompt_groups(prompt: &str) -> Vec<(&str, usize, Vec<&str>)> {
    let mut blocks: Vec<(&str, usize, Vec<&str>)> = Vec::new();
    for line in prompt.lines() {
        let header = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("] ("))
            .and_then(|(name, tail)| {
                tail.strip_suffix(" files)")
                    .and_then(|count| count.parse::<usize>().ok())
                    .map(|count| (name, count))
            });
        if let Some((name, count)) = header {
            blocks.push((name, count, Vec::new()));
            continue;
        }
        let listed = line
            .strip_prefix("  ")
            .and_then(|rest| rest.split_once(' '))
            .filter(|(status, _)| matches!(*status, "A" | "M" | "D" | "R"));
        if let (Some((_, path)), Some(block)) = (listed, blocks.last_mut()) {
            block.2.push(path);
        }
    }
    blocks
}

// --- what `apply` does with an answer that breaks the shape ------------------
//
// No recorded run has yet needed a repair, so every branch below is reachable
// only from a hand-written body. That is the point: the repairs are what make a
// nondeterministic pass shippable, and a claim of 0.0 repairs is only
// meaningful if a repair could have been counted.

/// A changeset and a hand-built heuristic grouping over it, so a repair branch
/// can be aimed at a known partition rather than at whatever the passes happen
/// to produce.
fn refine_case(groups: &[(&str, &[&str])]) -> (Changeset, passes::Grouping) {
    let mut text = String::new();
    let mut assignments = Vec::new();
    for (name, members) in groups {
        for path in *members {
            text.push_str(&format!("M\t{path}\n"));
            assignments.push(passes::Assignment {
                path: (*path).to_string(),
                group: (*name).to_string(),
                pass: "fixture",
                runner_up: None,
            });
        }
    }
    (Changeset::parse(&text), passes::Grouping { assignments })
}

fn two_groups() -> (Changeset, passes::Grouping) {
    refine_case(&[
        ("alpha", &["src/a.ts", "src/b.ts"]),
        ("beta", &["src/c.ts", "src/d.ts"]),
    ])
}

/// Group name to sorted members, which is all the scorer sees.
fn buckets(refined: &refine::Refined) -> Vec<(String, Vec<String>)> {
    refined
        .partition
        .groups
        .iter()
        .map(|(name, members)| {
            let mut members = members.clone();
            members.sort();
            (name.clone(), members)
        })
        .collect()
}

fn refine_with(body: &str, shape: Shape) -> refine::Refined {
    let (changeset, grouping) = two_groups();
    refine::apply(body, &changeset, &grouping, shape).expect("a well-formed body applies")
}

#[test]
fn a_dropped_path_is_restored_to_its_heuristic_group() {
    let refined = refine_with(
        r#"{"groups":[{"name":"one","files":["src/a.ts","src/b.ts","src/c.ts"]}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            ("beta".to_string(), vec!["src/d.ts".to_string()]),
            (
                "one".to_string(),
                vec![
                    "src/a.ts".to_string(),
                    "src/b.ts".to_string(),
                    "src/c.ts".to_string()
                ]
            ),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec!["dropped path restored to `beta`: src/d.ts"]
    );
}

#[test]
fn a_duplicated_path_stays_in_the_group_that_claimed_it_first() {
    let refined = refine_with(
        r#"{"groups":[
            {"name":"one","files":["src/a.ts","src/b.ts"]},
            {"name":"two","files":["src/b.ts","src/c.ts","src/d.ts"]}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "one".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
            (
                "two".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec!["duplicate path kept in `one`, not `two`: src/b.ts"]
    );
}

#[test]
fn an_invented_path_is_dropped() {
    let refined = refine_with(
        r#"{"groups":[
            {"name":"one","files":["src/a.ts","src/b.ts","src/nowhere.ts"]},
            {"name":"two","files":["src/c.ts","src/d.ts"]}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "one".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
            (
                "two".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec!["invented path dropped: src/nowhere.ts"]
    );
}

/// A model asked for "JSON and nothing else" mostly complies; a fence and a
/// closing sentence are the common near-miss, and they must score as the answer
/// they wrap rather than as a failed call.
#[test]
fn a_fenced_answer_with_trailing_prose_applies_as_the_bare_one() {
    let bare = r#"{"groups":[
        {"name":"one","files":["src/a.ts","src/b.ts"]},
        {"name":"two","files":["src/c.ts","src/d.ts"]}]}"#;
    let fenced = format!("Here is the regrouping:\n\n```json\n{bare}\n```\n\nHope that helps.\n");
    let (changeset, grouping) = two_groups();
    let from_fenced =
        refine::apply(&fenced, &changeset, &grouping, Shape::Full).expect("a fenced body applies");
    assert_eq!(
        buckets(&from_fenced),
        buckets(&refine_with(bare, Shape::Full))
    );
    assert!(from_fenced.repairs.is_empty());
}

#[test]
fn a_body_with_no_json_object_is_not_an_answer() {
    let (changeset, grouping) = two_groups();
    let error = refine::apply(
        "I could not group these files.",
        &changeset,
        &grouping,
        Shape::Full,
    )
    .expect_err("prose alone is not an answer");
    assert!(
        error.contains("no JSON object"),
        "the rejection must say what was missing: {error}"
    );
}

/// Two returned groups are two groups. Filing both under the name the model
/// reused would union them and score a partition it never proposed.
#[test]
fn two_groups_sharing_a_name_stay_two_groups() {
    let refined = refine_with(
        r#"{"groups":[
            {"name":"one","files":["src/a.ts","src/b.ts"]},
            {"name":"one","files":["src/c.ts","src/d.ts"]}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "one".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
            (
                "one-2".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
        ]
    );
    assert_eq!(refined.order, vec!["one", "one-2"]);
    assert_eq!(
        refined.repairs,
        vec!["group name `one` reused, filed as `one-2`"]
    );
}

#[test]
fn a_group_without_a_name_is_counted_not_defaulted() {
    let refined = refine_with(
        r#"{"groups":[
            {"files":["src/a.ts","src/b.ts"]},
            {"files":["src/c.ts","src/d.ts"]}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "unnamed-0".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
            (
                "unnamed-1".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec![
            "group 0 has no `name`, called `unnamed-0`",
            "group 1 has no `name`, called `unnamed-1`",
        ]
    );
}

/// A group with no `files` has still taken a name and a reading-order slot, so
/// it is not the same thing as a group the model deliberately left empty. The
/// omission is named where it happens rather than turning up downstream as
/// unexplained dropped paths.
#[test]
fn a_group_that_omits_its_files_is_recorded_not_read_as_empty() {
    let refined = refine_with(
        r#"{"groups":[{"name":"one","files":["src/a.ts"]},{"name":"two"}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            ("alpha".to_string(), vec!["src/b.ts".to_string()]),
            (
                "beta".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
            ("one".to_string(), vec!["src/a.ts".to_string()]),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec![
            "group `two` has no `files`",
            "dropped path restored to `alpha`: src/b.ts",
            "dropped path restored to `beta`: src/c.ts",
            "dropped path restored to `beta`: src/d.ts",
        ]
    );
    assert_eq!(refined.order, vec!["one".to_string(), "two".to_string()]);
}

#[test]
fn merge_only_ignores_an_input_group_that_does_not_exist() {
    let refined = refine_with(
        r#"{"groups":[
            {"name":"m","merge":["alpha","ghost"]},
            {"name":"n","merge":["beta"]}]}"#,
        Shape::MergeOnly,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "m".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
            (
                "n".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
        ]
    );
    assert_eq!(refined.repairs, vec!["unknown input group ignored: ghost"]);
}

#[test]
fn merge_only_gives_a_twice_claimed_input_group_to_its_first_claimant() {
    let refined = refine_with(
        r#"{"groups":[
            {"name":"m","merge":["alpha","beta"]},
            {"name":"n","merge":["beta"]}]}"#,
        Shape::MergeOnly,
    );
    assert_eq!(
        buckets(&refined),
        vec![(
            "m".to_string(),
            vec![
                "src/a.ts".to_string(),
                "src/b.ts".to_string(),
                "src/c.ts".to_string(),
                "src/d.ts".to_string()
            ]
        )]
    );
    assert_eq!(
        refined.repairs,
        vec!["input group claimed twice, second ignored: beta"]
    );
}

/// An unclaimed input group survives, and survives *apart* even when the model
/// gave one of its own groups that group's name.
#[test]
fn merge_only_keeps_an_unclaimed_input_group_under_a_free_name() {
    let refined = refine_with(
        r#"{"groups":[{"name":"beta","merge":["alpha"]}]}"#,
        Shape::MergeOnly,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "beta".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
            (
                "beta-2".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec!["input group never claimed, kept as-is under `beta-2`: beta"]
    );
}

/// The merge-only mirror of `naming_only_records_a_group_that_omits_its_was`: a
/// group with no `merge` has still taken a name and a reading-order slot, so the
/// omission is named where it happens rather than surfacing downstream as an
/// input group nobody claimed.
#[test]
fn merge_only_records_a_group_that_omits_its_merge() {
    let refined = refine_with(
        r#"{"groups":[{"name":"m","merge":["alpha"]},{"name":"n"}]}"#,
        Shape::MergeOnly,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "beta".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
            (
                "m".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec![
            "group `n` has no `merge`",
            "input group never claimed, kept as-is under `beta`: beta",
        ]
    );
    assert_eq!(refined.order, vec!["m".to_string(), "n".to_string()]);
}

/// The shape's whole promise: merge-only can coarsen and nothing else, so a
/// refined group is a union of whole heuristic groups however the answer is
/// written. Here the body asks for `alpha` split down the middle — the one thing
/// `full` could do and this shape may not — and the split is refused rather than
/// half-applied. This is the regression lock on `apply`'s merge-only loop, which
/// is where "coarsen only" is made structural rather than trusted to the answer.
#[test]
fn merge_only_cannot_split_a_heuristic_group() {
    let (changeset, grouping) = two_groups();
    let refined = refine::apply(
        r#"{"groups":[
            {"name":"m","merge":["alpha"],"files":["src/a.ts"]},
            {"name":"n","merge":["src/b.ts","beta"]}]}"#,
        &changeset,
        &grouping,
        Shape::MergeOnly,
    )
    .expect("a parseable body applies");

    let heuristic = grouping.partition();
    let refined_group = refined.partition.group_of();
    for (name, members) in &heuristic.groups {
        let landed: BTreeSet<&str> = members
            .iter()
            .map(|path| refined_group[path.as_str()])
            .collect();
        assert_eq!(
            landed.len(),
            1,
            "heuristic group `{name}` was split across {landed:?}"
        );
    }
    assert_eq!(
        refined.repairs,
        vec!["unknown input group ignored: src/b.ts"]
    );
}

/// Applies an adversarial naming-only body and asserts the two things that
/// shape promises: the partition the scorer sees is byte-for-byte the heuristic
/// one, and every deviation from the contract was named as a repair.
///
/// The replay over the recorded runs cannot establish the first half — `apply`
/// writes whole heuristic groups under new names and restores unclaimed ones
/// verbatim, so co-membership survives *any* parseable body, `{"groups":[]}`
/// included. Only a body that tries to move a file can show it is refused.
fn naming_only_leaves_the_partition_alone(body: &str, repairs: &[&str]) {
    let (changeset, grouping) = two_groups();
    let refined = refine::apply(body, &changeset, &grouping, Shape::NamingOnly)
        .expect("a parseable body applies");
    assert_eq!(
        co_membership(&refined.partition),
        co_membership(&grouping.partition()),
        "naming-only moved a file"
    );
    assert_eq!(refined.repairs, repairs);
}

/// A `files` array is the `full` shape's contract, not this one's. Answering
/// with one — here one that interleaves the two heuristic groups — must move
/// nothing, and must not pass silently: each group is missing its `was`.
#[test]
fn naming_only_refuses_a_files_array_that_moves_a_path() {
    naming_only_leaves_the_partition_alone(
        r#"{"groups":[
            {"name":"one","files":["src/a.ts","src/c.ts"]},
            {"name":"two","files":["src/b.ts","src/d.ts"]}]}"#,
        &[
            "group `one` has no `was`",
            "group `two` has no `was`",
            "input group never claimed, kept as-is under `alpha`: alpha",
            "input group never claimed, kept as-is under `beta`: beta",
        ],
    );
}

/// Two returned groups naming the same input group would split it in half if
/// the second claim were honoured. It is refused, and `beta` — which no group
/// then claims — is kept whole.
#[test]
fn naming_only_refuses_a_second_claim_on_one_input_group() {
    naming_only_leaves_the_partition_alone(
        r#"{"groups":[{"name":"one","was":"alpha"},{"name":"two","was":"alpha"}]}"#,
        &[
            "input group claimed twice, second ignored: alpha",
            "input group never claimed, kept as-is under `beta`: beta",
        ],
    );
}

#[test]
fn naming_only_records_a_group_that_omits_its_was() {
    naming_only_leaves_the_partition_alone(
        r#"{"groups":[{"name":"one","was":"alpha"},{"name":"two"}]}"#,
        &[
            "group `two` has no `was`",
            "input group never claimed, kept as-is under `beta`: beta",
        ],
    );
}

#[test]
fn naming_only_ignores_a_was_naming_no_input_group() {
    naming_only_leaves_the_partition_alone(
        r#"{"groups":[{"name":"one","was":"ghost"},{"name":"two","was":"beta"}]}"#,
        &[
            "unknown input group ignored: ghost",
            "input group never claimed, kept as-is under `alpha`: alpha",
        ],
    );
}

/// Several paths dropped out of one heuristic group go back into it together,
/// not into a group each.
#[test]
fn co_dropped_paths_are_restored_to_one_group() {
    let refined = refine_with(
        r#"{"groups":[{"name":"one","files":["src/a.ts","src/b.ts"]}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![
            (
                "beta".to_string(),
                vec!["src/c.ts".to_string(), "src/d.ts".to_string()]
            ),
            (
                "one".to_string(),
                vec!["src/a.ts".to_string(), "src/b.ts".to_string()]
            ),
        ]
    );
    assert_eq!(
        refined.repairs,
        vec![
            "dropped path restored to `beta`: src/c.ts",
            "dropped path restored to `beta`: src/d.ts",
        ]
    );
}

/// A model group that reuses a heuristic group's name *is* that group — the
/// prompt showed the model that name — so a path it dropped goes back into it,
/// not into a uniquified twin of it.
#[test]
fn a_dropped_path_rejoins_the_model_group_that_reused_its_heuristic_name() {
    let refined = refine_with(
        r#"{"groups":[
            {"name":"beta","files":["src/a.ts","src/b.ts","src/c.ts"]}]}"#,
        Shape::Full,
    );
    assert_eq!(
        buckets(&refined),
        vec![(
            "beta".to_string(),
            vec![
                "src/a.ts".to_string(),
                "src/b.ts".to_string(),
                "src/c.ts".to_string(),
                "src/d.ts".to_string()
            ]
        )]
    );
    assert_eq!(
        refined.repairs,
        vec!["dropped path restored to `beta`: src/d.ts"]
    );
}

/// The envelope of a failed call still carries a `result` — the error text.
/// Scoring that as an answer, or dropping it silently, both shrink the run
/// count the published numbers are a property of.
#[test]
fn an_errored_envelope_is_not_a_run() {
    let envelope = r#"{"type":"result","subtype":"error_max_turns","is_error":true,
        "result":"Reached max turns","duration_ms":1000,"total_cost_usd":0.01}"#;
    let error = RunRecord::parse(envelope).expect_err("an errored envelope is rejected");
    assert!(
        error.contains("error_max_turns"),
        "the rejection must name the subtype: {error}"
    );

    let ok = r#"{"type":"result","subtype":"success","is_error":false,
        "result":"{\"groups\":[]}","duration_ms":1000,"total_cost_usd":0.01,
        "modelUsage":{
            "claude-haiku-4-5":{"inputTokens":5,"outputTokens":7,
                "cacheReadInputTokens":100,"cacheCreationInputTokens":0},
            "claude-opus-5":{"inputTokens":10,"outputTokens":20,
                "cacheReadInputTokens":0,"cacheCreationInputTokens":30}}}"#;
    let record = RunRecord::parse(ok).expect("a successful envelope parses");
    assert_eq!(record.body, "{\"groups\":[]}");
    assert_eq!(record.model, "claude-opus-5");
    assert_eq!(record.input_tokens, 15.0);
    assert_eq!(record.output_tokens, 27.0);
    assert_eq!(record.cached_tokens, 130.0);
    assert_eq!(record.cost_usd, 0.01);
    assert_eq!(record.wall_clock_ms, 1000.0);
}

/// The cost table is read straight out of these fields, so an envelope that
/// stopped carrying one of them must fail loudly rather than report a run that
/// cost nothing, took no time and came from `unknown`.
#[test]
fn an_envelope_missing_a_cost_field_is_not_a_run() {
    for (missing, envelope) in [
        (
            "`modelUsage`",
            r#"{"subtype":"success","is_error":false,"result":"{}",
                "duration_ms":1000,"total_cost_usd":0.01}"#,
        ),
        (
            "empty `modelUsage`",
            r#"{"subtype":"success","is_error":false,"result":"{}",
                "duration_ms":1000,"total_cost_usd":0.01,"modelUsage":{}}"#,
        ),
        (
            "`modelUsage.claude-opus-5` has no `inputTokens`",
            r#"{"subtype":"success","is_error":false,"result":"{}",
                "duration_ms":1000,"total_cost_usd":0.01,
                "modelUsage":{"claude-opus-5":{"outputTokens":20,
                    "cacheReadInputTokens":0,"cacheCreationInputTokens":30}}}"#,
        ),
        (
            "`modelUsage.claude-opus-5` has no `outputTokens`",
            r#"{"subtype":"success","is_error":false,"result":"{}",
                "duration_ms":1000,"total_cost_usd":0.01,
                "modelUsage":{"claude-opus-5":{"inputTokens":10,
                    "cacheReadInputTokens":0,"cacheCreationInputTokens":30}}}"#,
        ),
        (
            "`modelUsage.claude-opus-5` has no `cacheCreationInputTokens`",
            r#"{"subtype":"success","is_error":false,"result":"{}",
                "duration_ms":1000,"total_cost_usd":0.01,
                "modelUsage":{"claude-opus-5":{"inputTokens":10,"outputTokens":20,
                    "cacheReadInputTokens":0}}}"#,
        ),
        (
            "`modelUsage.claude-opus-5` has no `cacheReadInputTokens`",
            r#"{"subtype":"success","is_error":false,"result":"{}",
                "duration_ms":1000,"total_cost_usd":0.01,
                "modelUsage":{"claude-opus-5":{"inputTokens":10,"outputTokens":20,
                    "cacheCreationInputTokens":30}}}"#,
        ),
        (
            "`total_cost_usd`",
            r#"{"subtype":"success","is_error":false,"result":"{}","duration_ms":1000,
                "modelUsage":{"claude-opus-5":{"inputTokens":10,"outputTokens":20,
                    "cacheReadInputTokens":0,"cacheCreationInputTokens":30}}}"#,
        ),
        (
            "`duration_ms`",
            r#"{"subtype":"success","is_error":false,"result":"{}","total_cost_usd":0.01,
                "modelUsage":{"claude-opus-5":{"inputTokens":10,"outputTokens":20,
                    "cacheReadInputTokens":0,"cacheCreationInputTokens":30}}}"#,
        ),
    ] {
        let error = RunRecord::parse(envelope)
            .expect_err(&format!("an envelope missing {missing} is rejected"));
        assert!(
            error.contains(missing),
            "the rejection must name what was missing ({missing}): {error}"
        );
    }
}

/// A model segment carries hyphens of its own, so the shape a recorded run
/// belongs to cannot be found by counting hyphens from the right — that reads
/// `full-claude-sonnet-4-5-01` as shape `full-claude-sonnet-4` and quietly
/// leaves a whole arm out of every report.
#[test]
fn a_recorded_runs_shape_survives_a_hyphenated_model_name() {
    for (stem, shape) in [
        ("full-opus-01", Some(Shape::Full)),
        ("full-coarse-opus-01", Some(Shape::FullCoarse)),
        ("merge-only-opus-05", Some(Shape::MergeOnly)),
        ("naming-only-opus-05", Some(Shape::NamingOnly)),
        ("full-claude-sonnet-4-5-01", Some(Shape::Full)),
        ("full-coarse-claude-sonnet-4-5-01", Some(Shape::FullCoarse)),
        ("fullish-opus-01", None),
        ("notes", None),
    ] {
        assert_eq!(Shape::from_run_stem(stem), shape, "shape of `{stem}`");
    }
}

/// A partition written the way a run's answer reads: group name, then members.
fn partition_of(groups: &[(&str, &[&str])]) -> Partition {
    Partition::from_assignments(groups.iter().flat_map(|(name, members)| {
        members
            .iter()
            .map(move |path| (path.to_string(), name.to_string()))
    }))
}

/// The co-membership the scorer sees: for each path, its group-mates including
/// itself. Names are not part of the answer `consensus` gives, so a test that
/// asserted them would be asserting `format!("consensus-{root}")`. This is
/// `Partition::companions`, which `refine::file_stability` measures stability
/// with — one definition of "the same grouping" for both.
fn co_membership(partition: &Partition) -> BTreeMap<String, BTreeSet<String>> {
    partition.companions()
}

/// One expected set of group-mates, spelled the way a test reads it.
fn mates(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| (*path).to_string()).collect()
}

/// The threshold the whole "three calls and a vote" recommendation rests on: a
/// pair travels together only when a strict majority of runs put it together.
#[test]
fn consensus_keeps_a_pair_two_of_three_runs_agree_on_and_drops_a_lone_vote() {
    let together = partition_of(&[("a", &["one", "two"]), ("b", &["three"])]);
    let apart = partition_of(&[("a", &["one"]), ("b", &["two", "three"])]);
    let voted = refine::consensus(&[together.clone(), together, apart])
        .expect("three runs are enough to vote");

    let voted_mates = co_membership(&voted);
    assert_eq!(
        voted_mates["one"],
        mates(&["one", "two"]),
        "two of three runs co-grouped one and two"
    );
    assert_eq!(
        voted_mates["three"],
        mates(&["three"]),
        "only one of three runs co-grouped two and three"
    );
}

/// Groups are the connected components of the surviving pairs, so a majority on
/// `a-b` and a majority on `b-c` puts `a`, `b` and `c` together even though no
/// majority ever voted for `a-c` directly — here only one run of the three
/// proposed that group. Two majorities of the same runs always overlap, and the
/// overlapping run's own partition is transitive, so some run always does
/// propose the closure; what the vote adds is that a minority proposal survives
/// when the chain of majorities reaches it.
#[test]
fn consensus_closes_a_chain_of_majorities_transitively() {
    let ab = partition_of(&[("x", &["a", "b"]), ("y", &["c"])]);
    let bc = partition_of(&[("x", &["a"]), ("y", &["b", "c"])]);
    let all = partition_of(&[("x", &["a", "b", "c"])]);
    let voted = refine::consensus(&[ab, bc, all]).expect("three runs are enough to vote");

    assert_eq!(
        co_membership(&voted)["a"],
        mates(&["a", "b", "c"]),
        "a-b and b-c both carry two of three votes, so the component is all three"
    );
}

/// A vote of one is its own majority, so consensus of a single run must be that
/// run's partition. This is the identity the "consensus of N" column degenerates
/// to and the sanity check on the majority arithmetic at N = 1.
#[test]
fn consensus_of_one_run_is_that_run() {
    let run = partition_of(&[("a", &["one", "two"]), ("b", &["three", "four"])]);
    let voted = refine::consensus(std::slice::from_ref(&run)).expect("one run still votes");

    assert_eq!(
        co_membership(&voted),
        co_membership(&run),
        "a single run is unanimous with itself"
    );
}

/// No runs is no consensus, not an empty partition: an empty partition would
/// score against the hand grouping like any other answer and publish a figure
/// derived from nothing.
#[test]
fn consensus_of_no_runs_is_none() {
    assert!(refine::consensus(&[]).is_none());
}

/// A strict majority, not half. The published "consensus of 10" column is an
/// even-N vote, where a 5-5 split must lose; at the odd N the other tests use,
/// a tie-permitting threshold would score identically and the column would move
/// with nothing failing.
#[test]
fn consensus_needs_more_than_half_at_an_even_number_of_runs() {
    let both = partition_of(&[("a", &["one", "two", "three"])]);
    let split = partition_of(&[("a", &["one", "two"]), ("b", &["three"])]);
    let neither = partition_of(&[("a", &["one"]), ("b", &["two"]), ("c", &["three"])]);
    let voted = refine::consensus(&[both.clone(), both, split, neither])
        .expect("four runs are enough to vote");

    let voted_mates = co_membership(&voted);
    assert_eq!(
        voted_mates["one"],
        mates(&["one", "two"]),
        "one and two carry three of four votes"
    );
    assert_eq!(
        voted_mates["three"],
        mates(&["three"]),
        "two of four is not a majority, so three travels alone"
    );
}

/// A path only some runs carry must survive the vote. Taking the path set from
/// one run would delete it from the voted partition outright, depressing recall
/// with no signal — the failure this test exists to catch.
#[test]
fn consensus_carries_every_path_any_run_grouped() {
    let short = partition_of(&[("a", &["one", "two"])]);
    let long = partition_of(&[("a", &["one", "two"]), ("b", &["three"])]);
    let voted = refine::consensus(&[short.clone(), long.clone(), long])
        .expect("three runs are enough to vote");

    assert_eq!(
        co_membership(&voted),
        BTreeMap::from([
            ("one".to_string(), mates(&["one", "two"])),
            ("two".to_string(), mates(&["one", "two"])),
            ("three".to_string(), mates(&["three"])),
        ]),
        "the voted partition carries the union of the runs' paths, not run 0's, \
         and the path only later runs grouped is a group of its own"
    );
}

/// `agreement` and `file_stability` produce the whole stability table in
/// docs/GROUPING_PASSES.md, and both are only reachable from a printer, so
/// these hold them to hand-computed values.
#[test]
fn two_identical_runs_agree_completely_and_move_no_file() {
    let run = partition_of(&[("g", &["a", "b", "c"])]);
    let agreement = refine::agreement(&[run.clone(), run.clone()]).expect("two runs agree");
    assert_eq!(
        (agreement.mean, agreement.worst, agreement.best),
        (1.0, 1.0, 1.0)
    );

    let stability = refine::file_stability(&[run.clone(), run]).expect("two runs are comparable");
    assert_eq!(
        (stability.identical_share, stability.mean_overlap),
        (1.0, 1.0)
    );
}

/// One file moved between two four-file runs, worked out by hand: the runs hold
/// 2 and 3 co-membership pairs and share 1, so precision 1/2, recall 1/3,
/// F1 0.4; no file keeps its exact group-mates, and the four Jaccard overlaps
/// are 2/3, 2/3, 1/4 and 1/2.
#[test]
fn one_moved_file_scores_its_hand_computed_agreement_and_overlap() {
    let left = partition_of(&[("x", &["a", "b"]), ("y", &["c", "d"])]);
    let right = partition_of(&[("x", &["a", "b", "c"]), ("y", &["d"])]);

    let agreement = refine::agreement(&[left.clone(), right.clone()]).expect("two runs agree");
    assert!(
        (agreement.mean - 0.4).abs() < 1e-9,
        "agreement mean {:.6}",
        agreement.mean
    );

    let stability = refine::file_stability(&[left, right]).expect("two runs are comparable");
    assert_eq!(stability.identical_share, 0.0);
    let expected_overlap = (2.0 / 3.0 + 2.0 / 3.0 + 0.25 + 0.5) / 4.0;
    assert!(
        (stability.mean_overlap - expected_overlap).abs() < 1e-9,
        "mean overlap {:.6}, expected {expected_overlap:.6}",
        stability.mean_overlap
    );
}

/// A path only some runs carry is the least stable path there is, so it has to
/// count as unstable whichever run happens to be first. Anchoring the reference
/// set on run 0 scored this pair 1.000 / 1.000 one way round and 2/3 the other.
#[test]
fn a_path_missing_from_one_run_counts_against_stability_either_way_round() {
    let short = partition_of(&[("g", &["a", "b"])]);
    let long = partition_of(&[("g", &["a", "b"]), ("h", &["c"])]);

    for runs in [[short.clone(), long.clone()], [long.clone(), short.clone()]] {
        let stability = refine::file_stability(&runs).expect("two runs are comparable");
        assert!(
            (stability.identical_share - 2.0 / 3.0).abs() < 1e-9,
            "identical share {:.6}",
            stability.identical_share
        );
        assert!(
            (stability.mean_overlap - 2.0 / 3.0).abs() < 1e-9,
            "mean overlap {:.6}",
            stability.mean_overlap
        );
    }
}

/// `worst` and `best` are published beside `mean` in the stability table, and
/// two runs make all three the same number — so a fold that returned the wrong
/// end of the distribution would pass every two-run test here. Three runs with
/// three distinct pairwise F1s: {ab|cd} against {abc|d} is 0.400, against
/// {abcd} is 0.500, and {abc|d} against {abcd} is 2/3.
#[test]
fn three_runs_report_the_ends_of_the_agreement_spread_not_just_its_middle() {
    let pair = partition_of(&[("x", &["a", "b"]), ("y", &["c", "d"])]);
    let three = partition_of(&[("x", &["a", "b", "c"]), ("y", &["d"])]);
    let all = partition_of(&[("x", &["a", "b", "c", "d"])]);

    let agreement = refine::agreement(&[pair, three, all]).expect("three runs agree");
    let expected = (0.4 + 0.5 + 2.0 / 3.0) / 3.0;
    for (label, actual, want) in [
        ("mean", agreement.mean, expected),
        ("worst", agreement.worst, 0.4),
        ("best", agreement.best, 2.0 / 3.0),
    ] {
        assert!(
            (actual - want).abs() < 1e-9,
            "agreement {label} {actual:.6}, expected {want:.6}"
        );
    }
}

/// The per-group slice is how docs/GROUPING_PASSES.md says where a pass wins and
/// loses, worked out by hand: of the three pairs in a three-file expected group,
/// a partition that keeps `a` with `b` and puts `c` elsewhere keeps one.
#[test]
fn recall_per_expected_group_counts_the_pairs_a_partition_keeps() {
    let expected = partition_of(&[("A", &["a", "b", "c"])]);
    let split = partition_of(&[("x", &["a", "b"]), ("y", &["c"])]);

    let recall = refine::recall_per_expected_group(&split, &expected);
    assert_eq!(recall.len(), 1);
    let (name, size, kept) = &recall[0];
    assert_eq!((name.as_str(), *size), ("A", 3));
    assert!((kept - 1.0 / 3.0).abs() < 1e-9, "recall {kept:.6}");
}

/// A path the partition does not place is not co-grouped with anything —
/// including another path the partition does not place. Comparing the two
/// lookups directly scored an absent pair as kept, so a partition that lost half
/// an expected group outright read as *better* than one that merely split it.
#[test]
fn recall_per_expected_group_does_not_credit_pairs_of_paths_it_never_placed() {
    let expected = partition_of(&[("A", &["a", "b", "c", "d"])]);
    let half_missing = partition_of(&[("x", &["a", "b"])]);

    let recall = refine::recall_per_expected_group(&half_missing, &expected);
    let (_, _, kept) = &recall[0];
    assert!(
        (kept - 1.0 / 6.0).abs() < 1e-9,
        "recall {kept:.6}: only a-b of the six pairs survived, and c-d is not a pair \
         the partition kept together"
    );
}

#[test]
fn stability_of_a_single_run_is_not_a_number() {
    let run = partition_of(&[("g", &["a", "b"])]);
    assert!(refine::agreement(std::slice::from_ref(&run)).is_none());
    assert!(refine::file_stability(std::slice::from_ref(&run)).is_none());
}

struct Run {
    record: RunRecord,
    refined: refine::Refined,
}

/// Every unordered triple of `n` run indices. A three-call vote picks one of
/// these at random in production, so a claim about "consensus of three" is a
/// claim about all of them.
fn triples(n: usize) -> impl Iterator<Item = (usize, usize, usize)> {
    (0..n).flat_map(move |i| ((i + 1)..n).flat_map(move |j| ((j + 1)..n).map(move |k| (i, j, k))))
}

/// Every three-call vote the recorded runs admit, scored against the hand
/// grouping and sorted ascending. The printer and the test that locks the
/// verdict both publish numbers off this distribution, so they read it from one
/// place: two copies could drift while both still compiled, and the printer is
/// `#[ignore]`d, so the drift would not surface.
fn triple_f1s(partitions: &[Partition], expected: &Partition) -> Vec<f64> {
    let mut voted: Vec<f64> = triples(partitions.len())
        .filter_map(|(i, j, k)| {
            let ballot = [
                partitions[i].clone(),
                partitions[j].clone(),
                partitions[k].clone(),
            ];
            refine::consensus(&ballot).map(|c| c.score_against(expected).f1)
        })
        .collect();
    voted.sort_by(|a, b| a.partial_cmp(b).expect("F1 is never NaN"));
    voted
}

/// The upper-middle element of an ascending distribution. Named rather than
/// spelled `voted[len / 2]` in two places, because the test that locks the
/// verdict and the printer that publishes it must name the same statistic — at
/// an even 120 triples the two middles differ in the third decimal.
fn upper_median(sorted: &[f64]) -> f64 {
    sorted[sorted.len() / 2]
}

/// Every recorded run on disk, or a panic naming the one that could not be
/// read. Every claim on this branch is a property of *N* runs, so a run that
/// dropped out quietly would shrink N with no signal — "no runs recorded" and
/// "a corrupt envelope" must not look alike. Only a genuinely absent directory
/// is a legitimate empty answer.
fn load_runs(fixture: &str, changeset: &Changeset, shape: Shape) -> Vec<Run> {
    let dir = run_dir(fixture);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => panic!("cannot read recorded runs in {}: {error}", dir.display()),
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| entry.expect("read a recorded run directory entry").path())
        .filter(|path| {
            let Some(stem) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
            else {
                return false;
            };
            let recorded = Shape::from_run_stem(stem).unwrap_or_else(|| {
                panic!(
                    "recorded run {} names no known shape; it would be scored under none",
                    path.display()
                )
            });
            recorded == shape
        })
        .collect();
    paths.sort();

    let grouping = passes::group(changeset, GroupingConfig::default());
    paths
        .iter()
        .map(|path| {
            let envelope = std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            let record = RunRecord::parse(&envelope)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let refined = refine::apply(&record.body, changeset, &grouping, shape)
                .unwrap_or_else(|error| panic!("{}: unusable answer: {error}", path.display()));
            Run { record, refined }
        })
        .collect()
}

/// The published arm alone. `load_runs` returns every arm on disk and
/// `refine_report` reports them apart, so a test that locks an N-run figure
/// filters to the model that figure is about — otherwise recording the second
/// arm the run script advertises fails the lock with a message blaming the
/// corpus size.
fn published_arm(fixture: &str, changeset: &Changeset, shape: Shape) -> Vec<Run> {
    load_runs(fixture, changeset, shape)
        .into_iter()
        .filter(|run| run.record.model == PUBLISHED_MODEL)
        .collect()
}

/// The committed corpus is ten published-model runs per shape, and every claim
/// docs/GROUPING_PASSES.md pins on a replay test is a property of all ten — a
/// replay test that skipped a short run list would stay green after half the
/// envelopes were deleted, publishing figures derived from the remainder. Every
/// test that locks a fixture-1 figure comes through here rather than counting
/// for itself, so there is one place the corpus size is asserted and one
/// message when it is wrong. The external fixture may legitimately be absent.
fn expect_full_corpus(label: &str, runs: &[Run], shape: Shape) {
    if label != ORCA_FIXTURE {
        return;
    }
    let published = runs
        .iter()
        .filter(|run| run.record.model == PUBLISHED_MODEL)
        .count();
    assert_eq!(
        published,
        RECORDED_RUNS,
        "{label} {}: every published figure is a property of {RECORDED_RUNS} \
         {PUBLISHED_MODEL} runs",
        shape.slug()
    );
}

#[test]
#[ignore = "printer; run with --ignored --nocapture refine_report"]
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
                // "Consensus of 3" above is one arbitrary triple, and a shipped
                // three-call vote draws a different one every time. What decides
                // whether voting is a fix is therefore the whole distribution, so
                // it is printed rather than left to be reasoned about from one
                // draw.
                let voted = triple_f1s(&partitions, &expected);
                if !voted.is_empty() {
                    let over = voted.iter().filter(|f1| **f1 > bar).count();
                    println!(
                        "  every triple: {over}/{} clear the bar  min {:.3} upper median {:.3} \
                         max {:.3} mean {:.3}",
                        voted.len(),
                        voted[0],
                        upper_median(&voted),
                        voted[voted.len() - 1],
                        voted.iter().sum::<f64>() / voted.len() as f64,
                    );
                    // Where the *first* triple — the one a five-run corpus and
                    // an eye on run order would have reported — sits in the
                    // whole distribution. Published as a rank rather than
                    // reasoned about, so no claim about that draw rests on a
                    // statistic this printer does not emit.
                    let first = refine::consensus(&partitions[..3])
                        .expect("a non-empty triple distribution means three runs")
                        .score_against(&expected)
                        .f1;
                    let above = voted.iter().filter(|f1| **f1 > first).count();
                    println!(
                        "  first triple {first:.3}: {above} of {} score above it, so it ranks \
                         {} from the top",
                        voted.len(),
                        above + 1
                    );
                    let singles = f1s.iter().filter(|f1| **f1 > bar).count();
                    println!("  single calls clearing the bar: {singles}/{}", f1s.len());
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
                        "  where it wins and loses, per expected group \
                         (heuristic -> run 0 / mean of {} runs, then every run):",
                        partitions.len()
                    );
                    let heuristic_recall = refine::recall_per_expected_group(
                        &passes::group(&changeset, GroupingConfig::default()).partition(),
                        &expected,
                    );
                    // Run 0 alone invited the question "is that a typical run?"
                    // often enough that the mean and the whole spread are printed
                    // beside it, rather than recomputed by hand each time.
                    let per_run: Vec<Vec<f64>> = partitions
                        .iter()
                        .map(|partition| {
                            refine::recall_per_expected_group(partition, &expected)
                                .into_iter()
                                .map(|(_, _, recall)| recall)
                                .collect()
                        })
                        .collect();
                    let mut rows: Vec<_> = heuristic_recall
                        .iter()
                        .enumerate()
                        .map(|(group, (name, size, before))| {
                            let spread: Vec<f64> = per_run.iter().map(|run| run[group]).collect();
                            let mean = spread.iter().sum::<f64>() / spread.len() as f64;
                            (name.clone(), *size, *before, spread, mean)
                        })
                        .collect();
                    rows.sort_by_key(|row| std::cmp::Reverse(row.1));
                    for (name, size, before, spread, mean) in rows {
                        let every = spread
                            .iter()
                            .map(|recall| format!("{recall:.2}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        println!(
                            "    {name:<28} {size:>3} files   {before:.2} -> {:.2} / {mean:.2}  \
                             {:+.2}   [{every}]",
                            spread[0],
                            mean - before
                        );
                    }
                }
            }
        }
    }
}

/// Every committed naming-only run applied cleanly to the heuristic partition.
///
/// Named for that and no more. The *cannot* half of "naming-only cannot move
/// the metric" is structural — `apply` writes whole heuristic groups and
/// restores unclaimed ones verbatim, so this equality holds for any parseable
/// body — and is locked by the adversarial bodies in
/// `naming_only_leaves_the_partition_alone`'s callers, not here. What this adds
/// is that no recorded answer needed that machinery to save it.
#[test]
fn every_recorded_naming_only_run_applied_to_the_heuristic_partition() {
    for (label, changeset, _) in fixtures() {
        let runs = load_runs(&label, &changeset, Shape::NamingOnly);
        expect_full_corpus(&label, &runs, Shape::NamingOnly);
        if runs.is_empty() {
            println!("no naming-only runs for {label}");
            continue;
        }
        let heuristic =
            co_membership(&passes::group(&changeset, GroupingConfig::default()).partition());
        for (index, run) in runs.iter().enumerate() {
            // The equality above holds for any parseable body, `{"groups":[]}`
            // included, because unclaimed groups are restored whole. What says
            // the answer was applied rather than rebuilt from the input is that
            // it needed no repair to get there.
            assert!(
                run.refined.repairs.is_empty(),
                "{label} run {index} needed repairs: {:?}",
                run.refined.repairs
            );
            assert_eq!(
                co_membership(&run.refined.partition),
                heuristic,
                "{label} run {index}: naming-only changed the partition, not only the names"
            );
        }
    }
}

/// "Repairs were 0.0 in every recorded run" is a load-bearing claim in
/// docs/GROUPING_PASSES.md: it is why the strict partition costs nothing. The
/// report that prints it is a printer, so the claim is locked here instead —
/// against every committed run of every shape.
#[test]
fn no_committed_run_needed_a_repair() {
    let (changeset, _) = orca();
    for shape in Shape::ALL {
        let runs = published_arm(ORCA_FIXTURE, &changeset, shape);
        expect_full_corpus(ORCA_FIXTURE, &runs, shape);
        for (index, run) in runs.iter().enumerate() {
            assert!(
                run.refined.repairs.is_empty(),
                "{} run {index} needed repairs: {:?}",
                shape.slug(),
                run.refined.repairs
            );
        }
    }
}

/// The negative half of the verdict in docs/GROUPING_PASSES.md, locked against
/// the committed runs: on fixture 1 `full` loses to the heuristics, and voting
/// does not repair it.
///
/// This replaces a test that asserted the *first* triple clears the bar. It did
/// — 0.411 — but at ten runs 41 of the 120 triples score above that draw, so it
/// ranks 42nd from the top, and the upper median is below the bar. Locking one
/// favourable draw made a coin flip read as a result, so what is locked now is
/// the distribution: no single call clears, a minority of triples do, and the
/// worst, best and upper median triples sit where the document publishes them.
/// A change that genuinely fixed fixture
/// 1 would fail this test, which is the point — the verdict would have moved
/// and the document would have to say so. `refine_report` prints that rank
/// beside the distribution, so the claim above is re-derivable offline like
/// every other published number. `full`'s own row — mean, worst, best, and the
/// agreement and file-stability figures the nondeterminism half of the verdict
/// rests on — is bracketed here too, since all of it can move while every single
/// call stays under the bar.
#[test]
fn voting_does_not_rescue_fixture_one() {
    let (changeset, expected) = orca();
    let runs = published_arm(ORCA_FIXTURE, &changeset, Shape::Full);
    // Every headline this test locks — "0 of 10", "50 of 120", the upper median
    // — is a property of ten runs of one model. At three, `triples` yields a
    // single triple and "the middle of the distribution" degenerates back into
    // the one-draw lock this test exists to replace.
    expect_full_corpus(ORCA_FIXTURE, &runs, Shape::Full);

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

    // Under the bar is not one number: the document publishes the whole row, and
    // a `full` arm that collapsed or crept up to the bar would still be under it
    // and leave the row false.
    let f1s: Vec<f64> = partitions
        .iter()
        .map(|partition| partition.score_against(&expected).f1)
        .collect();
    let mean = f1s.iter().sum::<f64>() / f1s.len() as f64;
    let worst = f1s.iter().copied().fold(f64::INFINITY, f64::min);
    let best = f1s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for (label, published, actual) in [
        ("mean", FULL_MEAN_F1, mean),
        ("worst", FULL_WORST_F1, worst),
        ("best", FULL_BEST_F1, best),
    ] {
        assert!(
            (actual - published).abs() <= PUBLISHED_F1_SLACK,
            "full's {label} single call scored {actual:.3}, off the published {published:.3} by \
             more than {PUBLISHED_F1_SLACK:.3}"
        );
    }

    // The nondeterminism half of the verdict, from the same ten runs.
    let agreement = refine::agreement(&partitions).expect("ten runs agree about something");
    let stability = refine::file_stability(&partitions).expect("ten runs place files somewhere");
    for (label, published, actual, slack) in [
        (
            "mean agreement",
            FULL_AGREEMENT_MEAN,
            agreement.mean,
            PUBLISHED_STABILITY_SLACK,
        ),
        (
            "worst agreement",
            FULL_AGREEMENT_WORST,
            agreement.worst,
            PUBLISHED_STABILITY_SLACK,
        ),
        (
            "identical share",
            FULL_IDENTICAL_SHARE,
            stability.identical_share,
            PUBLISHED_SHARE_SLACK,
        ),
        (
            "mean overlap",
            FULL_MEAN_OVERLAP,
            stability.mean_overlap,
            PUBLISHED_STABILITY_SLACK,
        ),
    ] {
        assert!(
            (actual - published).abs() <= slack,
            "full's {label} is {actual:.3}, off the published {published:.3} by more than \
             {slack:.3}"
        );
    }

    let voted = triple_f1s(&partitions, &expected);

    // The document publishes 50 of 120 clearing, a 0.343-0.439 spread and a
    // 0.369 upper median, and reads a coin flip off them. Each is bracketed, so
    // a distribution that moved in either direction fails here rather than
    // leaving the published figures quietly false.
    let over = voted.iter().filter(|f1| **f1 > bar).count();
    assert!(
        (TRIPLES_OVER_BAR_FLOOR..=TRIPLES_OVER_BAR_CEILING).contains(&over),
        "{over} of {} triples clear the bar, outside the published \
         {TRIPLES_OVER_BAR_FLOOR}-{TRIPLES_OVER_BAR_CEILING}, so the verdict has moved",
        voted.len()
    );
    let median = upper_median(&voted);
    assert!(
        median < bar,
        "the upper median triple scored {median:.3}, at or over {bar:.3}"
    );
    for (label, published, actual) in [
        ("worst", PUBLISHED_TRIPLE_MIN, voted[0]),
        ("best", PUBLISHED_TRIPLE_MAX, voted[voted.len() - 1]),
        ("upper median", PUBLISHED_TRIPLE_UPPER_MEDIAN, median),
    ] {
        assert!(
            (actual - published).abs() <= PUBLISHED_F1_SLACK,
            "the {label} triple scored {actual:.3}, off the published {published:.3} by more \
             than {PUBLISHED_F1_SLACK:.3}"
        );
    }
}

/// The positive half of the fixture-1 verdict, locked against the same corpus:
/// `merge-only` beats the heuristics where `full` loses to them. Only the
/// negative half was locked before, so a regression that flattened the shapes
/// into each other would have failed nothing — and "merge-only is the shape that
/// should ship on fixture 1" is the recommendation the document actually makes.
#[test]
fn merge_only_clears_the_bar_on_fixture_one() {
    let (changeset, expected) = orca();
    let runs = published_arm(ORCA_FIXTURE, &changeset, Shape::MergeOnly);
    expect_full_corpus(ORCA_FIXTURE, &runs, Shape::MergeOnly);

    let bar = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected)
        .f1;
    let partitions: Vec<_> = runs
        .iter()
        .map(|run| run.refined.partition.clone())
        .collect();
    let f1s: Vec<f64> = partitions
        .iter()
        .map(|partition| partition.score_against(&expected).f1)
        .collect();

    let mean = f1s.iter().sum::<f64>() / f1s.len() as f64;
    assert!(
        mean > bar,
        "merge-only averaged {mean:.3}, at or under the {bar:.3} bar it is published as clearing"
    );
    assert!(
        (mean - MERGE_ONLY_MEAN_F1).abs() <= PUBLISHED_F1_SLACK,
        "merge-only averaged {mean:.3}, off the published {MERGE_ONLY_MEAN_F1:.3} by more \
         than {PUBLISHED_F1_SLACK:.3}"
    );

    let singles = f1s.iter().filter(|f1| **f1 > bar).count();
    assert_eq!(
        singles,
        MERGE_ONLY_SINGLES_OVER_BAR,
        "{singles} of {} single merge-only calls clear the bar, not the published \
         {MERGE_ONLY_SINGLES_OVER_BAR}, so the published row has moved",
        f1s.len()
    );

    let voted = triple_f1s(&partitions, &expected);
    let over = voted.iter().filter(|f1| **f1 > bar).count();
    assert!(
        (MERGE_ONLY_TRIPLES_OVER_BAR_FLOOR..=MERGE_ONLY_TRIPLES_OVER_BAR_CEILING).contains(&over),
        "{over} of {} merge-only triples clear the bar, outside the published \
         {MERGE_ONLY_TRIPLES_OVER_BAR_FLOOR}-{MERGE_ONLY_TRIPLES_OVER_BAR_CEILING}, so the \
         verdict has moved",
        voted.len()
    );
    assert!(
        upper_median(&voted) > bar,
        "the upper median merge-only triple scored {:.3}, at or under {bar:.3}",
        upper_median(&voted)
    );
}

/// The middle of the three model shapes on fixture 1, locked the same way as the
/// two either side of it. `full-coarse` is the shape the document uses to argue
/// that `full`'s losses are over-splitting rather than misreading — it asks for
/// the same freedom over fewer groups and lands between `full` and `merge-only`
/// — and that argument is a claim about all three rows, so leaving this one
/// unlocked left the comparison free to close up with nothing failing.
#[test]
fn full_coarse_lands_between_the_other_shapes_on_fixture_one() {
    let (changeset, expected) = orca();
    let runs = published_arm(ORCA_FIXTURE, &changeset, Shape::FullCoarse);
    expect_full_corpus(ORCA_FIXTURE, &runs, Shape::FullCoarse);

    let bar = passes::group(&changeset, GroupingConfig::default())
        .partition()
        .score_against(&expected)
        .f1;
    let partitions: Vec<_> = runs
        .iter()
        .map(|run| run.refined.partition.clone())
        .collect();
    let f1s: Vec<f64> = partitions
        .iter()
        .map(|partition| partition.score_against(&expected).f1)
        .collect();

    let mean = f1s.iter().sum::<f64>() / f1s.len() as f64;
    assert!(
        mean < bar,
        "full-coarse averaged {mean:.3}, at or over the {bar:.3} bar it is published as missing"
    );
    assert!(
        (mean - FULL_COARSE_MEAN_F1).abs() <= PUBLISHED_F1_SLACK,
        "full-coarse averaged {mean:.3}, off the published {FULL_COARSE_MEAN_F1:.3} by more \
         than {PUBLISHED_F1_SLACK:.3}"
    );

    let singles = f1s.iter().filter(|f1| **f1 > bar).count();
    assert_eq!(
        singles,
        FULL_COARSE_SINGLES_OVER_BAR,
        "{singles} of {} single full-coarse calls clear the bar, not the published \
         {FULL_COARSE_SINGLES_OVER_BAR}, so the published row has moved",
        f1s.len()
    );

    // The betweenness itself, off one corpus in one place. Each arm's own row is
    // bracketed above, but three bands that happen to be ordered are not the
    // same claim as an ordering: a re-record that landed all three inside their
    // bands with `full-coarse` on the wrong side of a neighbour would leave the
    // argument the document makes from this row unsupported and nothing failing.
    let full_mean = shape_mean_f1(&changeset, &expected, Shape::Full);
    let merge_only_mean = shape_mean_f1(&changeset, &expected, Shape::MergeOnly);
    assert!(
        full_mean < mean && mean < merge_only_mean,
        "the shapes are published as full {full_mean:.3} < full-coarse {mean:.3} < merge-only \
         {merge_only_mean:.3}; that ordering no longer holds, so over-splitting is not what the \
         corpus shows"
    );

    let voted = triple_f1s(&partitions, &expected);
    let over = voted.iter().filter(|f1| **f1 > bar).count();
    assert!(
        (FULL_COARSE_TRIPLES_OVER_BAR_FLOOR..=FULL_COARSE_TRIPLES_OVER_BAR_CEILING).contains(&over),
        "{over} of {} full-coarse triples clear the bar, outside the published \
         {FULL_COARSE_TRIPLES_OVER_BAR_FLOOR}-{FULL_COARSE_TRIPLES_OVER_BAR_CEILING}, so the \
         verdict has moved",
        voted.len()
    );
}

/// One shape's mean single-call F1 over the whole published fixture-1 corpus,
/// for the comparisons that are claims about more than one arm.
fn shape_mean_f1(changeset: &Changeset, expected: &Partition, shape: Shape) -> f64 {
    let runs = published_arm(ORCA_FIXTURE, changeset, shape);
    expect_full_corpus(ORCA_FIXTURE, &runs, shape);
    let total: f64 = runs
        .iter()
        .map(|run| run.refined.partition.score_against(expected).f1)
        .sum();
    total / runs.len() as f64
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

fn assert_heuristic_beats_every_baseline(label: &str, changeset: &Changeset, expected: &Partition) {
    let heuristic = passes::group(changeset, GroupingConfig::default())
        .partition()
        .score_against(expected);

    for (baseline_label, baseline) in [
        ("all-one-group", passes::baseline::all_one_group(changeset)),
        (
            "one-file-per-group",
            passes::baseline::one_file_per_group(changeset),
        ),
        (
            "top-level-directory",
            passes::baseline::top_level_directory(changeset),
        ),
        (
            "parent-directory",
            passes::baseline::parent_directory(changeset),
        ),
    ] {
        let score = baseline.score_against(expected);
        assert!(
            heuristic.f1 > score.f1,
            "{label}: heuristic F1 {:.3} must beat {baseline_label} F1 {:.3}",
            heuristic.f1,
            score.f1
        );
    }
}

/// True on both fixtures since directory evidence was turned on (`gd-26r.21`).
/// It was true on fixture 1 only while the engine grouped on filename tokens
/// alone: fixture 2 lost to parent-directory, 0.282 against 0.312.
///
/// Named for what it checks rather than for the claim: `fixtures()` yields only
/// the checked-in fixture on a machine without the private one — CI — so the
/// fixture-2 half is `heuristic_beats_every_baseline_on_the_second_fixture`,
/// `#[ignore]`d so libtest lists it as unrun rather than passing green.
#[test]
fn heuristic_beats_every_baseline_on_every_available_fixture() {
    for (label, changeset, expected) in fixtures() {
        assert_heuristic_beats_every_baseline(&label, &changeset, &expected);
    }
}

#[test]
#[ignore = "needs $TUICR_GROUPING_FIXTURES"]
fn heuristic_beats_every_baseline_on_the_second_fixture() {
    let (changeset, expected) = require_external_fixture(SECOND_FIXTURE);
    assert_heuristic_beats_every_baseline(SECOND_FIXTURE, &changeset, &expected);
}

/// Directory evidence is the whole of fixture 2's result, so the ablation is
/// locked rather than left to the report: without directory tokens the engine
/// falls back below parent-directory, which is the state `gd-26r.20` recorded.
#[test]
#[ignore = "needs $TUICR_GROUPING_FIXTURES"]
fn directory_evidence_carries_the_second_fixture() {
    let (changeset, expected) = require_external_fixture(SECOND_FIXTURE);

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
    let mut fixtures = vec![(ORCA_FIXTURE.to_string(), changeset, expected)];
    match external_fixture(SECOND_FIXTURE) {
        Some((changeset, expected)) => {
            fixtures.push((SECOND_FIXTURE.to_string(), changeset, expected))
        }
        None => skip_notice(SECOND_FIXTURE),
    }
    fixtures
}

/// Every published number flows through `score_against`, and scoring a
/// partition against itself is a degenerate check — it holds for any symmetric
/// pair-counting bug. So one case is worked out by hand instead.
///
/// Expected `{A:[a,b,c], B:[d]}` holds 3 + 0 co-membership pairs; computed
/// `{X:[a,b], Y:[c,d]}` holds 1 + 1. The pair they share is (a,b): `c` and `d`
/// are together in the computed grouping but apart in the expected one. So
/// precision 1/2, recall 1/3, F1 0.4, over 2 groups whose largest holds half
/// the changeset.
#[test]
fn the_scorer_matches_a_hand_computed_case() {
    let expected = partition_of(&[("A", &["a", "b", "c"]), ("B", &["d"])]);
    let computed = partition_of(&[("X", &["a", "b"]), ("Y", &["c", "d"])]);
    let score = computed.score_against(&expected);

    assert!(
        (score.precision - 0.5).abs() < 1e-9,
        "P {:.6}",
        score.precision
    );
    assert!(
        (score.recall - 1.0 / 3.0).abs() < 1e-9,
        "R {:.6}",
        score.recall
    );
    assert!((score.f1 - 0.4).abs() < 1e-9, "F1 {:.6}", score.f1);
    assert_eq!(score.group_count, 2);
    assert!((score.largest_share - 0.5).abs() < 1e-9);

    // The asymmetric half: swapping the arguments swaps precision and recall
    // and leaves F1 where it was, which a mis-scoped overlap map would not.
    let flipped = expected.score_against(&computed);
    assert!((flipped.precision - 1.0 / 3.0).abs() < 1e-9);
    assert!((flipped.recall - 0.5).abs() < 1e-9);
    assert!((flipped.f1 - 0.4).abs() < 1e-9);
}

/// A partition that co-groups nothing has no true positives to be precise
/// about, and F1 is zero rather than undefined there.
#[test]
fn a_partition_that_co_groups_nothing_scores_zero_not_nan() {
    let expected = partition_of(&[("A", &["a", "b"])]);
    let singletons = partition_of(&[("x", &["a"]), ("y", &["b"])]);
    let score = singletons.score_against(&expected);
    assert_eq!((score.precision, score.recall, score.f1), (0.0, 0.0, 0.0));
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

/// The heuristic row docs/GROUPING_PASSES.md publishes for fixture 1: F1 0.394
/// out of precision 0.479 and recall 0.334, over 19 groups whose largest holds
/// 22.4% of the changeset. The whole row is locked, and every figure on both
/// sides — a one-sided check passes a pass that got better as readily as one
/// that regressed, and both leave the published row false. Deterministic
/// figures off a checked-in fixture, so the band is a rounding, not a re-record
/// allowance.
const RECORDED_PRECISION: f64 = 0.479;
const RECORDED_RECALL: f64 = 0.334;
const RECORDED_LARGEST_SHARE: f64 = 0.224;
const RECORDED_GROUP_COUNT: usize = 19;
const RECORDED_SLACK: f64 = 0.005;

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
    for (label, published, actual) in [
        ("precision", RECORDED_PRECISION, score.precision),
        ("recall", RECORDED_RECALL, score.recall),
        (
            "largest group share",
            RECORDED_LARGEST_SHARE,
            score.largest_share,
        ),
    ] {
        assert!(
            (actual - published).abs() <= RECORDED_SLACK,
            "{label} drifted: {actual:.3}, off the published {published:.3} by more than \
             {RECORDED_SLACK:.3}"
        );
    }
    assert_eq!(
        score.group_count, RECORDED_GROUP_COUNT,
        "the heuristics produced {} groups, not the published {RECORDED_GROUP_COUNT}",
        score.group_count
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
/// (`handler-cs`, `port-cs`, `program-cs`), so every entry on the list is under
/// test, not the one entry that was caught. What no test here can catch is an
/// extension *missing* from the list: to this code it is simply not an
/// extension, which is why the list is audited per ecosystem.
#[test]
fn extensions_never_survive_as_concern_tokens() {
    for ext in changeset::EXTENSIONS {
        let file = ChangedFile {
            path: format!("src/x/widget.{ext}"),
            kind: ChangeKind::Modified,
            rename_from: None,
        };
        assert_eq!(file.stem(), "widget", "stem of widget.{ext}");
        assert_eq!(file.name_tokens(), ["widget"], "tokens of widget.{ext}");
    }

    let cases = [
        ("src/Api/Program.cs", "Program", vec!["program"]),
        (
            "src/Data/Migrations/20240101_Init.Designer.cs",
            "20240101_Init-Designer",
            vec!["20240101", "init", "designer"],
        ),
        (
            "src/Catalog/Contoso.Catalog.Api.csproj",
            "Contoso-Catalog-Api",
            vec!["contoso", "catalog", "api"],
        ),
        (
            "Directory.Build.props",
            "Directory-Build",
            vec!["directory", "build"],
        ),
        ("src/main/java/Widget.java", "Widget", vec!["widget"]),
        (
            "web/src/hooks/use-tenant.ts",
            "use-tenant",
            vec!["use", "tenant"],
        ),
        ("web/src/pages/Report.razor", "Report", vec!["report"]),
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
/// be named after a file extension. It shares the tokeniser's oracle, so like
/// the test above it holds the audited list and not an entry missing from it.
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
    }
}
