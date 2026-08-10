//! The opt-in model refine pass (`gd-26r.11`), as a prototype that can be
//! scored by the same harness as the heuristics.
//!
//! The pass itself is one agent-CLI call: heuristic groups in, a revised
//! grouping out. That call is nondeterministic and costs money, so it is not
//! made from a test. Instead this module splits the pass in two halves that
//! meet at a recorded JSON file:
//!
//! * [`prompt`] builds exactly what is sent, from the changeset and the
//!   heuristic grouping. `emit_refine_prompts` writes those to disk.
//! * [`apply`] turns a recorded response back into a [`Partition`], which the
//!   existing scorer grades against the hand grouping like any other pass.
//!
//! `scripts/grouping-refine-runs.sh` is the middle: it runs the agent CLI over
//! the emitted prompts N times per shape and records each response with its
//! token counts and wall clock. Replaying recorded runs is what makes a
//! nondeterministic pass reviewable — and measuring how far two runs of the
//! same prompt diverge is itself one of the four questions the ticket asks.
//!
//! Shelling out to an agent CLI from Rust is `gd-26r.13`, still open. This
//! prototype deliberately does the shelling in a shell script rather than in
//! Rust so it does not pre-empt that decision; what it contributes is the
//! contract the call has to carry and the cost it has to pay.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::score::{Partition, free_name};
use tuicr::grouping::changeset::{ChangeKind, Changeset};

/// How much freedom the refine call is given. The ticket asks whether a
/// cheaper shape captures most of the benefit, so the shapes are the
/// experiment, not a configuration surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Merge, split and rename: the design doc's `--refine`.
    Full,
    /// [`Shape::Full`] plus a push toward coarser groups. Fixture 1's loss
    /// under `Full` was concentrated in two large groups the model subdivided,
    /// so this asks whether that is a granularity problem the prompt can fix
    /// or a judgement the model will not change. The instruction names no
    /// group count: telling it how many groups to produce when both fixtures
    /// happen to have thirteen or fourteen would be fitting the prompt to the
    /// answers. It does name a floor on group size, four files, which is a
    /// statement about what "coarser" means rather than about either answer.
    FullCoarse,
    /// Whole heuristic groups may be unioned and renamed. No individual file
    /// moves, so the refined partition is always a coarsening of the
    /// heuristic one — it can only trade precision for recall.
    MergeOnly,
    /// Names and order only. The partition is untouched **by construction**,
    /// which the scorer — which ignores names — therefore cannot grade.
    NamingOnly,
    /// No heuristic grouping at all: the flat file list, the rules, and group
    /// it. The other four shapes all hand the model an answer and ask it to
    /// improve on it, which makes every one of them a measurement of the pair
    /// rather than of the model — so nothing recorded so far can say whether
    /// the heuristics are helping the call or anchoring it. This is the control
    /// that separates them.
    ///
    /// The heuristic grouping has not entirely left the picture: `apply`'s
    /// repair path still restores a dropped path to its heuristic group,
    /// because a strict partition has to come out either way. That is a safety
    /// net rather than an input — the model never sees it — so a cold run with
    /// a high repair count is leaning on the heuristics after all, and the
    /// repair column is what says so.
    Cold,
}

impl Shape {
    pub const ALL: [Shape; 5] = [
        Shape::Full,
        Shape::FullCoarse,
        Shape::MergeOnly,
        Shape::NamingOnly,
        Shape::Cold,
    ];

    /// The four shapes that revise a heuristic grouping, which is what the
    /// published `gd-26r.11` corpus is made of. [`Shape::Cold`] is not among
    /// them: it is given no grouping to revise, so a test that expects ten
    /// published runs of every shape does not expect them of it.
    pub const REVISIONS: [Shape; 4] = [
        Shape::Full,
        Shape::FullCoarse,
        Shape::MergeOnly,
        Shape::NamingOnly,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Shape::Full => "full",
            Shape::FullCoarse => "full-coarse",
            Shape::MergeOnly => "merge-only",
            Shape::NamingOnly => "naming-only",
            Shape::Cold => "cold",
        }
    }

    /// The shape a recorded run's file stem names. A run is
    /// `<shape>-<model>-<nn>`, and the model segment carries hyphens of its own
    /// (`claude-sonnet-4-5`), so counting hyphens from the right reads the
    /// wrong segment as soon as a second arm is recorded. Match on the slug
    /// instead; one slug is a prefix of another ("full", "full-coarse"), so the
    /// longest match wins.
    pub fn from_run_stem(stem: &str) -> Option<Shape> {
        Shape::ALL
            .into_iter()
            .filter(|shape| {
                stem.strip_prefix(shape.slug())
                    .is_some_and(|rest| rest.starts_with('-'))
            })
            .max_by_key(|shape| shape.slug().len())
    }
}

/// The corpus arm a recorded run belongs to: the middle segment of
/// `<shape>-<arm>-<nn>`. Two runs are only comparable when they were recorded
/// the same way, and the envelope records the model but not the reasoning
/// effort — `gd-26r.24`'s `opus-low` arm answers as `claude-opus-5` exactly as
/// the published arm does, so grouping the report by model alone would average
/// a cheap arm into an expensive one and publish the mean of two experiments.
/// The file name is the only place the whole recipe is written down, so it is
/// what the arms are keyed on.
pub fn arm_from_run_stem(stem: &str) -> Option<&str> {
    let shape = Shape::from_run_stem(stem)?;
    let rest = stem.strip_prefix(shape.slug())?.strip_prefix('-')?;
    let (arm, run) = rest.rsplit_once('-')?;
    // A trailing segment that is not a run number means the stem is not a
    // recorded run at all, and returning the whole tail as an arm would file it
    // as one rather than surfacing the malformed name.
    run.parse::<u32>().ok()?;
    (!arm.is_empty()).then_some(arm)
}

/// The rules the call is held to. A digest of `docs/GROUPING.md` rather than
/// the file itself: the prose there addresses a human deciding what the engine
/// should aim at, and the sections on ambiguity annotation and on what is not
/// decided are noise in a prompt. Every numbered rule survives.
const RULES: &str = "\
1. Groups are concern-shaped, not directory-shaped. A concern routinely spans
   several top-level directories. A directory cut carries no information the
   file tree did not already show.
2. Groups sort by intent-centrality: the changeset's actual subject first, work
   incidental to it next, drive-by fixes last.
3. Drive-by and unrelated changes are collected into a named group and ordered
   last, never scattered through the concern groups.
4. A test file lives in its production file's group and never precedes it.
   Tests never form their own group.
5. When two groups claim a file, the more intent-central group takes it.
6. A residual group is a smell. If a group amounts to \"everything that did not
   sort elsewhere\", either name the concern it represents or split it.
7. A burst of new test files sharing a filename token pins a concern, more
   strongly than directory proximity does.
8. Mechanical and generated changes (lockfiles, generated output, vendored
   updates, formatter sweeps) get their own group, ordered last.
9. A doc belonging to one concern joins that concern's group; a doc covering
   the whole change goes in a docs group of its own.
10. Group names use the changeset's own vocabulary — terms that appear in its
    filenames — in kebab-case. `enterprise-host-routing` is legible; `shared`
    is not.
11. A rename is one file. It arrives as a single entry keyed by the new path.";

fn status(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
    }
}

/// The heuristic grouping as the prompt renders it, largest group first so the
/// residual smells rule 6 targets are the first thing read.
fn render_groups(changeset: &Changeset, heuristic: &Partition) -> String {
    let kinds: BTreeMap<&str, ChangeKind> = changeset
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.kind))
        .collect();
    let mut groups: Vec<_> = heuristic.groups.iter().collect();
    groups.sort_by_key(|(name, members)| (std::cmp::Reverse(members.len()), name.as_str()));

    let mut out = String::new();
    for (name, members) in groups {
        out.push_str(&format!("[{name}] ({} files)\n", members.len()));
        for path in members {
            let kind = kinds
                .get(path.as_str())
                .copied()
                .unwrap_or(ChangeKind::Modified);
            out.push_str(&format!("  {} {path}\n", status(kind)));
        }
    }
    out
}

/// The changeset as a flat list, for [`Shape::Cold`]. Changeset order — what
/// `git diff --name-status` hands over, path-sorted — and deliberately nothing
/// else. Any reordering here (by size, by directory, by heuristic group) would
/// smuggle a grouping back into the prompt, and a control that hints is not a
/// control.
fn render_files(changeset: &Changeset) -> String {
    let mut out = String::new();
    for file in &changeset.files {
        out.push_str(&format!("  {} {}\n", status(file.kind), file.path));
    }
    out
}

/// Exactly what is sent to the agent CLI. Paths and change status only: the
/// fixtures carry no diff bodies, so neither does this.
pub fn prompt(changeset: &Changeset, heuristic: &Partition, shape: Shape) -> String {
    let count = changeset.len();
    if shape == Shape::Cold {
        let files = render_files(changeset);
        return format!(
            "You are grouping the files of one changeset for code review, from \
             scratch. There is no starting grouping; produce one.\n\n\
             You have file paths and change status (A added, M modified, D deleted, R renamed) \
             and nothing else. You cannot read the diff. Do not guess at content you cannot \
             see.\n\n\
             The rules a good grouping follows:\n\n{RULES}\n\n\
             The {count} files of this changeset:\n\n\
             {files}\n\
             Group them. Output JSON and nothing else:\n\
             {{\"groups\": [{{\"name\": \"kebab-case-name\", \"files\": [\"path\", ...]}}, ...]}}\n\n\
             List the groups in the order a reviewer should read them (rule 2). Every one \
             of the {count} input paths must appear in exactly one group. Do not invent, \
             omit or duplicate a path.\n"
        );
    }

    let groups = render_groups(changeset, heuristic);
    let group_count = heuristic.groups.len();

    let coarseness = match shape {
        Shape::FullCoarse => {
            "\n\nPrefer fewer, larger groups. When two candidate groups are \
             plausibly the same concern, merge them; split only where the concerns are clearly \
             different. Do not create a concern group of fewer than four files — a group that \
             small belongs inside a larger one unless it is mechanical, generated or \
             documentation."
        }
        _ => "",
    };

    let task = match shape {
        Shape::Full | Shape::FullCoarse => format!(
            "Revise the grouping. You may merge groups, split them, move individual \
             files between them, and rename them.\n\n\
             Output JSON and nothing else:\n\
             {{\"groups\": [{{\"name\": \"kebab-case-name\", \"files\": [\"path\", ...]}}, ...]}}\n\n\
             List the groups in the order a reviewer should read them (rule 2). Every one \
             of the {count} input paths must appear in exactly one group. Do not invent, \
             omit or duplicate a path.{coarseness}"
        ),
        Shape::MergeOnly => format!(
            "Revise the grouping by MERGING ONLY. You may union whole groups together and \
             rename the result. You may NOT move an individual file between groups, and you \
             may NOT split a group.\n\n\
             Output JSON and nothing else:\n\
             {{\"groups\": [{{\"name\": \"kebab-case-name\", \"merge\": [\"input-group-name\", ...]}}, ...]}}\n\n\
             List the groups in reading order (rule 2). Each of the {group_count} input group \
             names must appear exactly once across all `merge` lists — a group that merges \
             with nothing is a `merge` list of one."
        ),
        Shape::NamingOnly => format!(
            "Rename and reorder the groups. Do NOT change which files are in which group.\n\n\
             Output JSON and nothing else:\n\
             {{\"groups\": [{{\"name\": \"kebab-case-name\", \"was\": \"input-group-name\"}}, ...]}}\n\n\
             List the groups in the order a reviewer should read them (rule 2). Each of the \
             {group_count} input group names must appear exactly once as a `was`."
        ),
        // Returned above: it is the one shape whose prompt shows no grouping,
        // so it shares neither the framing nor the group count this format
        // string is built around.
        Shape::Cold => unreachable!("the cold prompt is built before this match"),
    };

    format!(
        "You are grouping the files of one changeset for code review. A heuristic pass has \
         already produced a grouping; your job is to improve it.\n\n\
         You have file paths and change status (A added, M modified, D deleted, R renamed) \
         and nothing else. You cannot read the diff. Do not guess at content you cannot see.\n\n\
         The rules a good grouping follows:\n\n{RULES}\n\n\
         The heuristic grouping of this {count}-file changeset, largest group first:\n\n\
         {groups}\n{task}\n"
    )
}

/// What a recorded run produced, once the shape's constraint has been enforced.
#[derive(Debug, Clone)]
pub struct Refined {
    pub partition: Partition,
    /// Group names in the order the model returned them (rule 2), which the
    /// pairwise scorer cannot see.
    pub order: Vec<String>,
    pub repairs: Vec<String>,
}

/// Turns a recorded response into a partition, enforcing the shape rather than
/// trusting it. A model pass that can silently drop a file would break the
/// strict partition `docs/GROUPING.md` requires, so every violation is repaired
/// against the heuristic grouping and counted: the repair count is part of the
/// verdict, not a detail.
pub fn apply(
    body: &str,
    changeset: &Changeset,
    heuristic: &Partition,
    shape: Shape,
) -> Result<Refined, String> {
    let value = extract_json(body)?;
    let heuristic_group = heuristic.group_of();
    let all_paths: BTreeSet<&str> = changeset.files.iter().map(|f| f.path.as_str()).collect();

    let groups = value
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| "response has no `groups` array".to_string())?;

    let mut order = Vec::new();
    let mut assigned: BTreeMap<String, String> = BTreeMap::new();
    let mut repairs = Vec::new();
    // Two returned groups are two groups even when the model gives them the
    // same name, and `Partition` buckets by name, so every name a group is
    // filed under is reserved as it is taken.
    let mut used: BTreeSet<String> = BTreeSet::new();

    match shape {
        // `Cold` answers in the same schema as `Full` — a list of named groups
        // each carrying its own paths — and is read the same way. The two
        // differ in what was asked, not in what comes back.
        Shape::Full | Shape::FullCoarse | Shape::Cold => {
            for (index, group) in groups.iter().enumerate() {
                let name = group_name(group, index, &mut used, &mut repairs);
                order.push(name.clone());
                // A group with no usable `files` is not an empty group: it has
                // already taken a name and an `order` slot, and its intended
                // paths will be swept up by the restore loop below and reported
                // as generic drops. Name the shape violation where it happens.
                let Some(files) = group.get("files").and_then(Value::as_array) else {
                    repairs.push(format!("group `{name}` has no `files`"));
                    continue;
                };
                for path in files.iter().filter_map(Value::as_str) {
                    if !all_paths.contains(path) {
                        repairs.push(format!("invented path dropped: {path}"));
                        continue;
                    }
                    if let Some(previous) = assigned.insert(path.to_string(), name.clone()) {
                        repairs.push(format!(
                            "duplicate path kept in `{previous}`, not `{name}`: {path}"
                        ));
                        assigned.insert(path.to_string(), previous);
                    }
                }
            }
        }
        Shape::MergeOnly | Shape::NamingOnly => {
            let mut claimed: BTreeSet<String> = BTreeSet::new();
            for (index, group) in groups.iter().enumerate() {
                let name = group_name(group, index, &mut used, &mut repairs);
                order.push(name.clone());
                // As in the `files` case above, a group that omits its source
                // key has still taken a name and an order slot, so the omission
                // is named rather than read as "claims nothing".
                let key = if shape == Shape::MergeOnly {
                    "merge"
                } else {
                    "was"
                };
                let sources = if shape == Shape::MergeOnly {
                    group.get(key).and_then(Value::as_array).map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<String>>()
                    })
                } else {
                    group
                        .get(key)
                        .and_then(Value::as_str)
                        .map(|was| vec![was.to_string()])
                };
                let Some(sources) = sources else {
                    repairs.push(format!("group `{name}` has no `{key}`"));
                    continue;
                };
                // Merge-only can only coarsen, and it is this loop that makes
                // that structural rather than a property of the answers: a
                // claimed source group is written whole under one name, a
                // source claimed twice is skipped, and an unclaimed one is kept
                // whole below, so no body can split a heuristic group across two
                // names. `merge_only_cannot_split_a_heuristic_group` aims a body
                // that asks for exactly that at this loop and holds it here.
                for source in sources {
                    let Some(members) = heuristic.groups.get(&source) else {
                        repairs.push(format!("unknown input group ignored: {source}"));
                        continue;
                    };
                    if !claimed.insert(source.clone()) {
                        repairs.push(format!(
                            "input group claimed twice, second ignored: {source}"
                        ));
                        continue;
                    }
                    for path in members {
                        assigned.insert(path.clone(), name.clone());
                    }
                }
            }
            for (source, members) in &heuristic.groups {
                if !claimed.contains(source) {
                    let name = free_name(source, &used);
                    used.insert(name.clone());
                    repairs.push(format!(
                        "input group never claimed, kept as-is under `{name}`: {source}"
                    ));
                    for path in members {
                        assigned.entry(path.clone()).or_insert_with(|| name.clone());
                    }
                }
            }
        }
    }

    // A dropped path goes back under its heuristic group's own name, not a
    // uniquified one: the prompt shows the model those names, so a returned
    // group called `X` *is* heuristic group `X` and the dropped path belongs in
    // it. Reusing the plain name also puts co-dropped paths from one heuristic
    // group back together, which a per-path uniquified name would split.
    //
    // With this loop in place `assigned` is keyed by every distinct changeset
    // path and nothing else, so the strict partition holds for any parseable
    // body by construction. That is why the claim is stated here and not
    // asserted over the recorded runs, where it could not fail.
    for path in &all_paths {
        if !assigned.contains_key(*path) {
            let name = heuristic_group
                .get(path)
                .copied()
                .expect("the heuristic grouping places every path in the changeset");
            repairs.push(format!("dropped path restored to `{name}`: {path}"));
            assigned.insert(path.to_string(), name.to_string());
        }
    }

    Ok(Refined {
        partition: Partition::from_assignments(assigned),
        order,
        repairs,
    })
}

/// The name a returned group is filed under, which is not always the name the
/// model gave it. `Partition` buckets by name, so a reused name would silently
/// union two groups the model returned separately and a missing name would
/// union every group that lacks one — both changing the answer being scored.
/// Each is uniquified and counted as a repair instead.
fn group_name(
    group: &Value,
    index: usize,
    used: &mut BTreeSet<String>,
    repairs: &mut Vec<String>,
) -> String {
    let base = match group.get("name").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => {
            repairs.push(format!(
                "group {index} has no `name`, called `unnamed-{index}`"
            ));
            format!("unnamed-{index}")
        }
    };
    let name = free_name(&base, used);
    if name != base {
        repairs.push(format!("group name `{base}` reused, filed as `{name}`"));
    }
    used.insert(name.clone());
    name
}

/// How far two runs of the same prompt moved, as pairwise co-membership F1 of
/// one run's partition against the other's. 1.000 is byte-identical grouping;
/// the heuristics score 1.000 here by construction.
///
/// This is deliberately the *same* metric the accuracy numbers use, so a
/// stability figure and an accuracy figure are directly comparable: a pass that
/// scores 0.45 against the human and 0.60 against itself is telling you most of
/// its distance from the human is its own noise.
pub fn agreement(runs: &[Partition]) -> Option<Agreement> {
    if runs.len() < 2 {
        return None;
    }
    let mut scores = Vec::new();
    for (i, left) in runs.iter().enumerate() {
        for right in &runs[i + 1..] {
            scores.push(left.score_against(right).f1);
        }
    }
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    let worst = scores.iter().copied().fold(f64::INFINITY, f64::min);
    let best = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some(Agreement { mean, worst, best })
}

#[derive(Debug, Clone, Copy)]
pub struct Agreement {
    pub mean: f64,
    pub worst: f64,
    pub best: f64,
}

/// The share of files that landed in the same group in every run, judged by
/// group *membership* rather than group name: a file is stable when the set of
/// files it shares a group with is identical across runs.
///
/// This is what `gd-26r.8` (regrouping state) actually needs to know — not how
/// much the partition moved on average, but how many files a reviewer would
/// find somewhere else after a re-run.
/// Paired with [`FileStability::mean_overlap`], the softer reading of the same thing:
/// the mean overlap of a file's group-mates between two runs. The strict share
/// answers "would a reviewer notice"; the overlap answers "by how much".
pub fn file_stability(runs: &[Partition]) -> Option<FileStability> {
    if runs.len() < 2 {
        return None;
    }
    let all: Vec<_> = runs.iter().map(Partition::companions).collect();
    // The union of every run's paths, not run 0's: a path only some runs carry
    // is exactly the least stable kind of path there is, and anchoring on run 0
    // would drop it from both figures and so overstate stability precisely when
    // the runs disagree most.
    let paths: BTreeSet<&String> = all.iter().flat_map(BTreeMap::keys).collect();

    let identical = paths
        .iter()
        .filter(|path| {
            let mut sets = all.iter().map(|run| run.get(**path));
            let first = sets.next().flatten();
            first.is_some() && sets.all(|set| set == first)
        })
        .count();

    let mut overlaps = Vec::new();
    for (index, left) in all.iter().enumerate() {
        for right in &all[index + 1..] {
            for path in &paths {
                // A path one run does not carry shares no group-mates with the
                // run that does: overlap 0, not skipped. `mine` always holds
                // the path itself, so a union of zero means both sides are
                // absent, which is not a disagreement.
                let overlap = match (left.get(*path), right.get(*path)) {
                    (None, None) => continue,
                    (Some(mine), Some(theirs)) => {
                        mine.intersection(theirs).count() as f64 / mine.union(theirs).count() as f64
                    }
                    (Some(_), None) | (None, Some(_)) => 0.0,
                };
                overlaps.push(overlap);
            }
        }
    }

    Some(FileStability {
        identical_share: identical as f64 / paths.len().max(1) as f64,
        mean_overlap: overlaps.iter().sum::<f64>() / overlaps.len().max(1) as f64,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct FileStability {
    /// Share of files whose set of group-mates is byte-identical in every run.
    /// What `gd-26r.8` needs: how many files a reviewer would find somewhere
    /// else after a re-run.
    pub identical_share: f64,
    /// Mean Jaccard overlap of a file's group-mates between two runs.
    pub mean_overlap: f64,
}

/// Per expected group, the share of its internal file pairs a grouping keeps
/// together. Pairwise recall, sliced by the human's groups rather than summed
/// over the changeset.
///
/// A single F1 cannot tell "spread this group's files everywhere" apart from
/// "split this group in two", and the two are very different failures when the
/// human's own group is one the fixture flags as a residual smell. This is the
/// slice that tells them apart.
pub fn recall_per_expected_group(
    partition: &Partition,
    expected: &Partition,
) -> Vec<(String, usize, f64)> {
    let group_of = partition.group_of();

    expected
        .groups
        .iter()
        .map(|(name, members)| {
            let mut kept = 0u64;
            let mut total = 0u64;
            for (index, left) in members.iter().enumerate() {
                for right in &members[index + 1..] {
                    total += 1;
                    // A path the grouping does not place is not co-grouped with
                    // anything. Comparing the two lookups directly made a pair
                    // of absent paths compare `None == None` and score as kept,
                    // which reads a group the pass lost entirely as recall 1.00.
                    let together = match (group_of.get(left.as_str()), group_of.get(right.as_str()))
                    {
                        (Some(left), Some(right)) => left == right,
                        _ => false,
                    };
                    if together {
                        kept += 1;
                    }
                }
            }
            let recall = if total == 0 {
                1.0
            } else {
                kept as f64 / total as f64
            };
            (name.clone(), members.len(), recall)
        })
        .collect()
}

/// The partition several runs agree on: two files are co-grouped when a strict
/// majority of runs co-grouped them, and the groups are the connected
/// components of what survives.
///
/// This exists because "run it N times and take the consensus" is the obvious
/// answer to a nondeterministic pass, and it costs N times as much — so
/// whether it actually buys accuracy or stability has to be measured rather
/// than assumed.
pub fn consensus(runs: &[Partition]) -> Option<Partition> {
    if runs.is_empty() {
        return None;
    }
    // The union across runs, not run 0's paths: a run that carries a path run 0
    // does not would otherwise have that path silently deleted from the voted
    // partition, depressing recall with no signal.
    let paths: Vec<String> = runs
        .iter()
        .flat_map(|run| run.groups.values().flatten().cloned())
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    let index: BTreeMap<&str, usize> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| (path.as_str(), i))
        .collect();

    let mut votes = vec![vec![0usize; paths.len()]; paths.len()];
    for run in runs {
        for members in run.groups.values() {
            let ids: Vec<usize> = members.iter().map(|path| index[path.as_str()]).collect();
            // Upper triangle only, which is the half the tally below reads.
            for (n, &a) in ids.iter().enumerate() {
                for &b in &ids[n + 1..] {
                    votes[a.min(b)][a.max(b)] += 1;
                }
            }
        }
    }

    let majority = runs.len() / 2 + 1;
    let mut component: Vec<usize> = (0..paths.len()).collect();
    fn root(component: &mut [usize], mut node: usize) -> usize {
        while component[node] != node {
            component[node] = component[component[node]];
            node = component[node];
        }
        node
    }
    for (a, row) in votes.iter().enumerate() {
        for (b, &count) in row.iter().enumerate().skip(a + 1) {
            if count >= majority {
                let (left, right) = (root(&mut component, a), root(&mut component, b));
                if left != right {
                    component[left] = right;
                }
            }
        }
    }

    let mut assignments = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let group = root(&mut component, i);
        assignments.push((path.clone(), format!("consensus-{group}")));
    }
    Some(Partition::from_assignments(assignments))
}

/// One recorded agent-CLI call: what it answered, what it cost, how long it
/// took. Read straight from the CLI's own `--output-format json` envelope, so
/// the cost numbers are the CLI's accounting and not this prototype's guess.
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub model: String,
    pub body: String,
    pub input_tokens: f64,
    pub output_tokens: f64,
    /// Cache creation plus cache read. Every one of the forty committed
    /// fixture-1 envelopes wrote cache and read none, and the write sits a
    /// near-constant ~2,690 tokens above that run's `input_tokens` on all four
    /// shapes, so this column tracks the task text with a fixed CLI preamble on
    /// top — see the cost section of docs/GROUPING_PASSES.md.
    pub cached_tokens: f64,
    pub cost_usd: f64,
    pub wall_clock_ms: f64,
}

impl RunRecord {
    pub fn parse(envelope: &str) -> Result<RunRecord, String> {
        let value = first_value(envelope)?;

        // Two recorders write into this corpus: the CLI one, whose envelope is
        // the CLI's own `--output-format json` output, and the Vertex one,
        // which wraps a raw provider response. They are told apart by an
        // explicit marker rather than by sniffing for a field, so a CLI
        // envelope that changed shape fails as a CLI envelope instead of being
        // silently read as the other thing.
        if value.get("transport").and_then(Value::as_str) == Some("vertex") {
            return RunRecord::parse_vertex(&value);
        }

        // The recorder writes a full envelope for any zero-exit CLI call, and
        // an errored one still carries a `result` — the error text. Scoring
        // that as an answer, or dropping it quietly, both shrink N without
        // saying so, so a failed call is rejected by name.
        let subtype = value
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        let is_error = value
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if is_error || subtype != "success" {
            return Err(format!(
                "CLI envelope is not a successful call: subtype `{subtype}`, is_error {is_error}"
            ));
        }

        let body = value
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "CLI envelope has no `result`".to_string())?
            .to_string();

        // `modelUsage` is keyed by model, and every committed envelope carries
        // exactly one entry. Nothing in the envelope shape promises that, so if
        // a call ever bills against more than one model every entry is part of
        // what it cost and they are summed, with the run labelled by the model
        // that wrote the answer — the one with the most output.
        let usage = value
            .get("modelUsage")
            .and_then(Value::as_object)
            .ok_or_else(|| "CLI envelope has no `modelUsage`".to_string())?;
        if usage.is_empty() {
            return Err("CLI envelope has an empty `modelUsage`".to_string());
        }
        // Every model entry of every recorded envelope carries all four token
        // counts, and a call that hit no cache writes an explicit zero rather
        // than omitting the key, so all four are required to be present. Zero
        // is a legitimate count; a missing key is a changed envelope shape, and
        // defaulting it would understate a published column with no signal.
        let required = |name: &str, stats: &Value, key: &str| {
            stats
                .get(key)
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("`modelUsage.{name}` has no `{key}`"))
        };
        let (mut input, mut output, mut cached) = (0.0, 0.0, 0.0);
        let mut model = "unknown".to_string();
        let mut most_output = f64::NEG_INFINITY;
        for (name, stats) in usage {
            let own_output = required(name, stats, "outputTokens")?;
            input += required(name, stats, "inputTokens")?;
            output += own_output;
            cached += required(name, stats, "cacheCreationInputTokens")?
                + required(name, stats, "cacheReadInputTokens")?;
            if own_output > most_output {
                most_output = own_output;
                model = name.clone();
            }
        }

        Ok(RunRecord {
            model,
            body,
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cached,
            cost_usd: value
                .get("total_cost_usd")
                .and_then(Value::as_f64)
                .ok_or_else(|| "CLI envelope has no `total_cost_usd`".to_string())?,
            wall_clock_ms: value
                .get("duration_ms")
                .and_then(Value::as_f64)
                .ok_or_else(|| "CLI envelope has no `duration_ms`".to_string())?,
        })
    }

    /// A run recorded straight against Vertex by `grouping-refine-runs-vertex.sh`.
    ///
    /// Two things differ from the CLI envelope and both are load-bearing for the
    /// cost column. Vertex reports what it billed in tokens but never in money,
    /// so the price is carried in the envelope, written at record time, and the
    /// cost is derived here rather than read; and the two publishers Vertex
    /// fronts agree on nothing below the URL, so each one's usage block is read
    /// on its own terms.
    fn parse_vertex(value: &Value) -> Result<RunRecord, String> {
        let field = |name: &str| {
            value
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("vertex envelope has no `{name}`"))
        };
        let model = field("model")?.to_string();
        let publisher = field("publisher")?;
        let response = value
            .get("response")
            .ok_or_else(|| "vertex envelope has no `response`".to_string())?;

        let price = |name: &str| {
            value
                .get("price_usd_per_mtok")
                .and_then(|prices| prices.get(name))
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("vertex envelope has no `price_usd_per_mtok.{name}`"))
        };
        let count = |usage: &Value, key: &str| {
            usage
                .get(key)
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("vertex usage block has no `{key}`"))
        };

        let (body, input, output, cached) = match publisher {
            "google" => {
                let usage = response
                    .get("usageMetadata")
                    .ok_or_else(|| "vertex response has no `usageMetadata`".to_string())?;
                // A cached prompt token is counted in `promptTokenCount` as
                // well, so charging both would bill the cached part twice — at
                // the uncached rate, which is the direction that flatters
                // nothing and inflates the column.
                let cached = usage
                    .get("cachedContentTokenCount")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                // Gemini bills thinking at the output rate and reports it in a
                // field of its own, which is the split `gd-26r.24` had to
                // measure by replay on the CLI transport. Both are output.
                let thoughts = usage
                    .get("thoughtsTokenCount")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let parts = response
                    .pointer("/candidates/0/content/parts")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "vertex response has no candidate parts".to_string())?;
                // A thought part carries `thought: true` and is the model's
                // reasoning, not its answer. Concatenating it into the body
                // would put prose in front of the JSON.
                let body: String = parts
                    .iter()
                    .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect();
                (
                    body,
                    count(usage, "promptTokenCount")? - cached,
                    count(usage, "candidatesTokenCount")? + thoughts,
                    cached,
                )
            }
            "anthropic" => {
                let usage = response
                    .get("usage")
                    .ok_or_else(|| "vertex response has no `usage`".to_string())?;
                let optional = |key: &str| usage.get(key).and_then(Value::as_f64).unwrap_or(0.0);
                let content = response
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "vertex response has no `content`".to_string())?;
                // Same rule as the Gemini branch: the thinking blocks are
                // billed as output but are not the answer. Here they are told
                // apart by block type rather than by a flag.
                let body: String = content
                    .iter()
                    .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect();
                (
                    body,
                    count(usage, "input_tokens")?,
                    count(usage, "output_tokens")?,
                    optional("cache_creation_input_tokens") + optional("cache_read_input_tokens"),
                )
            }
            other => return Err(format!("unknown vertex publisher `{other}`")),
        };

        // An empty body parses as "no JSON object in response" later, which
        // blames the model for a recorder that filtered every part away.
        if body.trim().is_empty() {
            return Err(format!(
                "vertex {publisher} response carries no answer text"
            ));
        }

        Ok(RunRecord {
            model,
            body,
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cached,
            cost_usd: (input * price("input")?
                + output * price("output")?
                + cached * price("cached")?)
                / 1_000_000.0,
            wall_clock_ms: value
                .get("duration_ms")
                .and_then(Value::as_f64)
                .ok_or_else(|| "vertex envelope has no `duration_ms`".to_string())?,
        })
    }
}

/// A model asked for "JSON and nothing else" mostly complies, and a prototype
/// that fell over on a stray fence would be measuring the fence. Reading from
/// the first `{` with a streaming deserialiser takes the first balanced object
/// and ignores whatever follows it.
fn extract_json(body: &str) -> Result<Value, String> {
    let start = body
        .find('{')
        .ok_or_else(|| "no JSON object in response".to_string())?;
    first_value(&body[start..])
}

/// `serde_json::from_str` rejects trailing content, which a fenced answer has.
/// The streaming deserialiser stops at the end of the first value instead.
fn first_value(text: &str) -> Result<Value, String> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .next()
        .ok_or_else(|| "empty JSON document".to_string())?
        .map_err(|error| error.to_string())
}
