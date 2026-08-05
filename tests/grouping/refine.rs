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

use super::changeset::{ChangeKind, Changeset};
use super::passes::Grouping;
use super::score::Partition;

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
    /// or a judgement the model will not change. The instruction is
    /// deliberately non-numeric: telling it how many groups to produce when
    /// both fixtures happen to have thirteen or fourteen would be fitting the
    /// prompt to the answers.
    FullCoarse,
    /// Whole heuristic groups may be unioned and renamed. No individual file
    /// moves, so the refined partition is always a coarsening of the
    /// heuristic one — it can only trade precision for recall.
    MergeOnly,
    /// Names and order only. The partition is untouched **by construction**,
    /// which the scorer — which ignores names — therefore cannot grade.
    NamingOnly,
}

impl Shape {
    pub const ALL: [Shape; 4] = [
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
        }
    }
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
fn render_groups(changeset: &Changeset, grouping: &Grouping) -> String {
    let kinds: BTreeMap<&str, ChangeKind> = changeset
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.kind))
        .collect();
    let partition = grouping.partition();
    let mut groups: Vec<_> = partition.groups.iter().collect();
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

/// Exactly what is sent to the agent CLI. Paths and change status only: the
/// fixtures carry no diff bodies, so neither does this.
pub fn prompt(changeset: &Changeset, grouping: &Grouping, shape: Shape) -> String {
    let groups = render_groups(changeset, grouping);
    let count = changeset.len();
    let group_count = grouping.partition().groups.len();

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
    grouping: &Grouping,
    shape: Shape,
) -> Result<Refined, String> {
    let value = extract_json(body)?;
    let heuristic = grouping.partition();
    let heuristic_group: BTreeMap<&str, &str> = heuristic
        .groups
        .iter()
        .flat_map(|(name, members)| members.iter().map(move |p| (p.as_str(), name.as_str())))
        .collect();
    let all_paths: BTreeSet<&str> = changeset.files.iter().map(|f| f.path.as_str()).collect();

    let groups = value
        .get("groups")
        .and_then(Json::as_array)
        .ok_or_else(|| "response has no `groups` array".to_string())?;

    let mut order = Vec::new();
    let mut assigned: BTreeMap<String, String> = BTreeMap::new();
    let mut repairs = Vec::new();

    match shape {
        Shape::Full | Shape::FullCoarse => {
            for group in groups {
                let name = group
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("unnamed");
                order.push(name.to_string());
                let files = group.get("files").and_then(Json::as_array);
                for path in files.into_iter().flatten().filter_map(Json::as_str) {
                    if !all_paths.contains(path) {
                        repairs.push(format!("invented path dropped: {path}"));
                        continue;
                    }
                    if let Some(previous) = assigned.insert(path.to_string(), name.to_string()) {
                        repairs.push(format!(
                            "duplicate path kept in `{previous}`, not `{name}`: {path}"
                        ));
                        assigned.insert(path.to_string(), previous);
                    }
                }
            }
        }
        Shape::MergeOnly | Shape::NamingOnly => {
            let key = if shape == Shape::MergeOnly {
                "merge"
            } else {
                "was"
            };
            let mut claimed: BTreeSet<String> = BTreeSet::new();
            for group in groups {
                let name = group
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("unnamed");
                order.push(name.to_string());
                let sources: Vec<String> = match shape {
                    Shape::MergeOnly => group
                        .get(key)
                        .and_then(Json::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Json::as_str)
                        .map(str::to_string)
                        .collect(),
                    _ => group
                        .get(key)
                        .and_then(Json::as_str)
                        .map(str::to_string)
                        .into_iter()
                        .collect(),
                };
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
                        assigned.insert(path.clone(), name.to_string());
                    }
                }
            }
            for (source, members) in &heuristic.groups {
                if !claimed.contains(source) {
                    repairs.push(format!("input group never claimed, kept as-is: {source}"));
                    for path in members {
                        assigned
                            .entry(path.clone())
                            .or_insert_with(|| source.clone());
                    }
                }
            }
        }
    }

    for path in &all_paths {
        if !assigned.contains_key(*path) {
            let fallback = heuristic_group.get(path).copied().unwrap_or("unassigned");
            repairs.push(format!("dropped path restored to `{fallback}`: {path}"));
            assigned.insert(path.to_string(), fallback.to_string());
        }
    }

    Ok(Refined {
        partition: Partition::from_assignments(assigned),
        order,
        repairs,
    })
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
/// Paired with [`FileStability::churn`], the softer reading of the same thing:
/// the mean overlap of a file's group-mates between two runs. The strict share
/// answers "would a reviewer notice"; the overlap answers "by how much".
pub fn file_stability(runs: &[Partition]) -> Option<FileStability> {
    if runs.len() < 2 {
        return None;
    }
    let companions = |partition: &Partition| -> BTreeMap<String, BTreeSet<String>> {
        partition
            .groups
            .values()
            .flat_map(|members| {
                let set: BTreeSet<String> = members.iter().cloned().collect();
                members.iter().map(move |path| (path.clone(), set.clone()))
            })
            .collect()
    };
    let all: Vec<_> = runs.iter().map(companions).collect();
    let (reference, others) = all.split_first()?;

    let identical = reference
        .iter()
        .filter(|(path, set)| others.iter().all(|other| other.get(*path) == Some(set)))
        .count();

    let mut overlaps = Vec::new();
    for (index, left) in all.iter().enumerate() {
        for right in &all[index + 1..] {
            for (path, mine) in left {
                let Some(theirs) = right.get(path) else {
                    continue;
                };
                let union = mine.union(theirs).count();
                overlaps.push(if union == 0 {
                    1.0
                } else {
                    mine.intersection(theirs).count() as f64 / union as f64
                });
            }
        }
    }

    Some(FileStability {
        identical_share: identical as f64 / reference.len().max(1) as f64,
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
    let group_of: BTreeMap<&str, &str> = partition
        .groups
        .iter()
        .flat_map(|(name, members)| members.iter().map(move |p| (p.as_str(), name.as_str())))
        .collect();

    expected
        .groups
        .iter()
        .map(|(name, members)| {
            let mut kept = 0u64;
            let mut total = 0u64;
            for (index, left) in members.iter().enumerate() {
                for right in &members[index + 1..] {
                    total += 1;
                    if group_of.get(left.as_str()) == group_of.get(right.as_str()) {
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
    let first = runs.first()?;
    let paths: Vec<String> = first.groups.values().flatten().cloned().collect();
    let index: BTreeMap<&str, usize> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| (path.as_str(), i))
        .collect();

    let mut votes = vec![vec![0usize; paths.len()]; paths.len()];
    for run in runs {
        for members in run.groups.values() {
            let ids: Vec<usize> = members
                .iter()
                .filter_map(|path| index.get(path.as_str()).copied())
                .collect();
            for (n, &a) in ids.iter().enumerate() {
                for &b in &ids[n + 1..] {
                    votes[a][b] += 1;
                    votes[b][a] += 1;
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
    /// Cache creation plus cache read. On a `claude -p` call this is dominated
    /// by the CLI's own system prompt, which the task text does not control —
    /// see the cost section of docs/GROUPING_PASSES.md.
    pub cached_tokens: f64,
    pub cost_usd: f64,
    pub wall_clock_ms: f64,
}

impl RunRecord {
    pub fn parse(envelope: &str) -> Result<RunRecord, String> {
        let value = Json::parse(envelope)?;
        let body = value
            .get("result")
            .and_then(Json::as_str)
            .ok_or_else(|| "CLI envelope has no `result`".to_string())?
            .to_string();

        // A `claude -p` call bills against more than one model: the answering
        // model plus whatever small model the CLI uses for its own bookkeeping.
        // Every one of them is part of what the call costs, so they are summed,
        // and the model that wrote the answer — the one with the most output —
        // is the one the run is labelled with.
        let usage = value.get("modelUsage").and_then(Json::as_object);
        let field = |stats: &Json, key: &str| stats.get(key).and_then(Json::as_f64).unwrap_or(0.0);
        let (mut input, mut output, mut cached) = (0.0, 0.0, 0.0);
        let mut model = "unknown".to_string();
        let mut most_output = f64::NEG_INFINITY;
        for (name, stats) in usage.into_iter().flatten() {
            let own_output = field(stats, "outputTokens");
            input += field(stats, "inputTokens");
            output += own_output;
            cached +=
                field(stats, "cacheCreationInputTokens") + field(stats, "cacheReadInputTokens");
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
                .and_then(Json::as_f64)
                .unwrap_or(0.0),
            wall_clock_ms: value
                .get("duration_ms")
                .and_then(Json::as_f64)
                .unwrap_or(0.0),
        })
    }
}

// --- a JSON reader, because the harness has no serde dependency ------------
//
// The prototype needs to read agent-CLI responses and their usage counters, and
// tuicr does not depend on serde. Pulling a dependency into the crate for a
// throwaway prototype would be the wrong trade, so this is a minimal reader:
// enough for the two shapes of document involved, and no more.

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn parse(text: &str) -> Result<Json, String> {
        let chars: Vec<char> = text.chars().collect();
        let mut cursor = 0;
        let value = parse_value(&chars, &mut cursor)?;
        Ok(value)
    }
}

/// A model asked for "JSON and nothing else" mostly complies, and a prototype
/// that fell over on a stray fence would be measuring the fence. The first
/// balanced `{...}` in the body is taken.
fn extract_json(body: &str) -> Result<Json, String> {
    let start = body
        .find('{')
        .ok_or_else(|| "no JSON object in response".to_string())?;
    Json::parse(&body[start..])
}

fn skip_whitespace(chars: &[char], cursor: &mut usize) {
    while *cursor < chars.len() && chars[*cursor].is_whitespace() {
        *cursor += 1;
    }
}

fn parse_value(chars: &[char], cursor: &mut usize) -> Result<Json, String> {
    skip_whitespace(chars, cursor);
    match chars.get(*cursor) {
        None => Err("unexpected end of JSON".into()),
        Some('{') => parse_object(chars, cursor),
        Some('[') => parse_array(chars, cursor),
        Some('"') => parse_string(chars, cursor).map(Json::String),
        Some('t') => parse_literal(chars, cursor, "true", Json::Bool(true)),
        Some('f') => parse_literal(chars, cursor, "false", Json::Bool(false)),
        Some('n') => parse_literal(chars, cursor, "null", Json::Null),
        Some(_) => parse_number(chars, cursor),
    }
}

fn parse_literal(
    chars: &[char],
    cursor: &mut usize,
    text: &str,
    value: Json,
) -> Result<Json, String> {
    if chars[*cursor..].starts_with(&text.chars().collect::<Vec<_>>()[..]) {
        *cursor += text.len();
        Ok(value)
    } else {
        Err(format!("expected `{text}`"))
    }
}

fn parse_number(chars: &[char], cursor: &mut usize) -> Result<Json, String> {
    let start = *cursor;
    while *cursor < chars.len() && "+-0123456789.eE".contains(chars[*cursor]) {
        *cursor += 1;
    }
    let text: String = chars[start..*cursor].iter().collect();
    text.parse()
        .map(Json::Number)
        .map_err(|_| format!("bad number `{text}`"))
}

fn parse_string(chars: &[char], cursor: &mut usize) -> Result<String, String> {
    *cursor += 1; // opening quote
    let mut out = String::new();
    while *cursor < chars.len() {
        match chars[*cursor] {
            '"' => {
                *cursor += 1;
                return Ok(out);
            }
            '\\' => {
                *cursor += 1;
                let escape = *chars.get(*cursor).ok_or("unterminated escape")?;
                out.push(match escape {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'u' => {
                        let hex: String = chars
                            .get(*cursor + 1..*cursor + 5)
                            .ok_or("truncated \\u escape")?
                            .iter()
                            .collect();
                        *cursor += 4;
                        char::from_u32(u32::from_str_radix(&hex, 16).map_err(|_| "bad \\u escape")?)
                            .unwrap_or('\u{fffd}')
                    }
                    other => other,
                });
                *cursor += 1;
            }
            other => {
                out.push(other);
                *cursor += 1;
            }
        }
    }
    Err("unterminated string".into())
}

fn parse_array(chars: &[char], cursor: &mut usize) -> Result<Json, String> {
    *cursor += 1;
    let mut items = Vec::new();
    loop {
        skip_whitespace(chars, cursor);
        match chars.get(*cursor) {
            Some(']') => {
                *cursor += 1;
                return Ok(Json::Array(items));
            }
            Some(',') => *cursor += 1,
            None => return Err("unterminated array".into()),
            _ => items.push(parse_value(chars, cursor)?),
        }
    }
}

fn parse_object(chars: &[char], cursor: &mut usize) -> Result<Json, String> {
    *cursor += 1;
    let mut map = BTreeMap::new();
    loop {
        skip_whitespace(chars, cursor);
        match chars.get(*cursor) {
            Some('}') => {
                *cursor += 1;
                return Ok(Json::Object(map));
            }
            Some(',') => *cursor += 1,
            Some('"') => {
                let key = parse_string(chars, cursor)?;
                skip_whitespace(chars, cursor);
                if chars.get(*cursor) != Some(&':') {
                    return Err(format!("expected `:` after key `{key}`"));
                }
                *cursor += 1;
                map.insert(key, parse_value(chars, cursor)?);
            }
            None => return Err("unterminated object".into()),
            Some(other) => return Err(format!("unexpected `{other}` in object")),
        }
    }
}
