//! The refine arm: one model call over the heuristic grouping, and the reader
//! that turns its answer back into a strict partition.
//!
//! Two halves that never meet in this file. [`prompt`] renders what is sent and
//! [`apply`] reads what comes back; the call between them is
//! [`super::vertex`], and the blocking startup screen that drives it is
//! `crate::app::refine`. Splitting it here is what lets every repair branch be
//! tested against a hand-written body with no network and no key.
//!
//! What the arm is worth, and why it is allowed to block startup, is
//! `docs/GROUPING_PASSES.md`: **F1 0.427 and 0.711 against the heuristics'
//! 0.394 and 0.378** on the two fixtures — the undisputed half of the case —
//! while the group order it also buys is +0.042 on fixture 1, which is chance.
//! The shape is `docs/GROUPS_CONTRACT.md`: one call, no vote, a group order and
//! nothing about file order, read leniently, and every violation repaired
//! against the heuristic partition rather than rejected.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::changeset::{ChangeKind, Changeset};
use super::{Group, GroupId, GroupSource, Grouping, PresentedGroup};

/// The rules the call is held to. A digest of `docs/GROUPING.md` rather than
/// the file itself: the prose there addresses a human deciding what the engine
/// should aim at, and the sections on ambiguity annotation and on what is not
/// decided are noise in a prompt. Every numbered rule of 1–11 survives.
///
/// Rules 12–14 are deliberately absent. They order the files *inside* a group,
/// which the engine does itself on both arms (`gd-26r.27`), so the response
/// owes nothing about it and the prompt must not ask.
///
/// `tests/grouping/refine.rs` sends this same constant, so
/// `every_documented_rule_reaches_the_model` grades the shipped text rather
/// than a copy of it.
pub const RULES: &str = "\
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

/// Where a path lands that neither the response nor the heuristic grouping
/// placed — which only a `changeset` and a `heuristic` built from different
/// changesets can produce. Total coverage is tuicr's invariant, so such a path
/// gets a group rather than a panic.
const UNPLACED: &str = "unplaced";

/// The first of `base`, `base-2`, `base-3`, … that `used` does not already
/// hold.
///
/// Group membership is bucketed by name on the way in, so anything that files
/// two distinct groups under one name unions them silently. Every producer of a
/// group name resolves collisions from here, including the harness's
/// `Partition`, so a name the engine uniquifies and a name the scorer
/// uniquifies are the same name.
pub fn free_name(base: &str, used: &BTreeSet<String>) -> String {
    if !used.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| !used.contains(candidate))
        .expect("a free name exists")
}

pub fn status(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
    }
}

/// The heuristic grouping as the prompt renders it, largest group first so the
/// residual smells rule 6 targets are the first thing read.
///
/// Groups are shown **by name**. The model never sees a [`GroupId`]: identity is
/// resolved on the way back by [`apply`]'s rejoin rule, which buys the same
/// thing without adding an invented id to the hallucination surface
/// (`docs/GROUPS_CONTRACT.md`).
///
/// Members are listed by path, *not* in the engine's within-group order, for two
/// reasons that agree. It is the order the recorded runs behind
/// `docs/GROUPING_PASSES.md` were measured on, pinned by
/// `the_shipped_prompt_is_the_prompt_the_numbers_were_measured_on`. And the
/// within-group order is engine-owned on both arms (rules 12–14, absent from
/// [`RULES`]): showing a production file ahead of its test would display an
/// ordering the answer format cannot carry back and the model is not asked
/// about.
fn render_groups(changeset: &Changeset, heuristic: &Grouping) -> String {
    let kinds: BTreeMap<&str, ChangeKind> = changeset
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.kind))
        .collect();

    let mut groups: Vec<(&Group, Vec<String>)> = heuristic
        .groups()
        .iter()
        .map(|group| {
            let mut members = heuristic
                .files_in(&group.id)
                .map(|path| path.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            members.sort();
            (group, members)
        })
        .collect();
    groups.sort_by_key(|(group, members)| (std::cmp::Reverse(members.len()), group.name.clone()));

    let mut out = String::new();
    for (group, members) in groups {
        out.push_str(&format!("[{}] ({} files)\n", group.name, members.len()));
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

/// How coarse the answer should be (`gd-26r.23`), the whole of what this arm is
/// told about group size.
///
/// The size signal and the soft cap, and **no target group count**: a count as
/// `f(file count)` and a cap that scales with the changeset were both rejected,
/// because the model is the thing that can see which concerns the changeset
/// actually has and a formula is not. `gd-26r.11`'s full-coarse arm failed with
/// a bare coarseness instruction and no size signal; the file count is the
/// variable it was missing, and whether size alone is enough is the open risk
/// the first week of real reviews tests.
///
/// Public because the harness's parity test splices it into the `Shape::Full`
/// prompt `docs/GROUPING_PASSES.md`'s refine numbers were measured on: this
/// sentence is the one deliberate difference between the two, and anything else
/// that drifts still fails that test.
pub fn size_guidance() -> String {
    format!(
        " A group should hold at most {} files, and the size of the changeset sets how coarse \
         the grouping should be: the smaller the changeset, the fewer and coarser its groups.",
        super::caps::SOFT_CAP
    )
}

/// Exactly what is sent. Paths and change status only: no diff bodies, because
/// a hunk-reading pass does not fit in one call over a 161-file changeset and
/// was never measured (`docs/GROUPING_PASSES.md`).
pub fn prompt(changeset: &Changeset, heuristic: &Grouping) -> String {
    let count = changeset.len();
    let groups = render_groups(changeset, heuristic);
    let size = size_guidance();

    format!(
        "You are grouping the files of one changeset for code review. A heuristic pass has \
         already produced a grouping; your job is to improve it.\n\n\
         You have file paths and change status (A added, M modified, D deleted, R renamed) \
         and nothing else. You cannot read the diff. Do not guess at content you cannot see.\n\n\
         The rules a good grouping follows:\n\n{RULES}\n\n\
         The heuristic grouping of this {count}-file changeset, largest group first:\n\n\
         {groups}\n\
         Revise the grouping. You may merge groups, split them, move individual \
         files between them, and rename them.\n\n\
         Output JSON and nothing else:\n\
         {{\"groups\": [{{\"name\": \"kebab-case-name\", \"files\": [\"path\", ...]}}, ...]}}\n\n\
         List the groups in the order a reviewer should read them (rule 2). Every one \
         of the {count} input paths must appear in exactly one group. Do not invent, \
         omit or duplicate a path.{size}\n"
    )
}

/// A refined grouping and what had to be fixed to obtain it.
#[derive(Debug, Clone)]
pub struct Refined {
    pub grouping: Grouping,
    /// One line per violation repaired. The count is part of the verdict, not a
    /// detail: a model measured at 0.5 invented paths a call is one config edit
    /// away at all times, so a misbehaving arm has to surface as a number
    /// rather than as a silent fallback nobody can explain.
    pub repairs: Vec<String>,
}

/// Turns a response body into a grouping, enforcing the strict partition rather
/// than trusting it.
///
/// **Any parseable body is applied**: there is no share-of-repairs threshold
/// above which a response is discarded, because the money is spent at dispatch
/// and discarding a paid-for 30–70s result over an omitted path is the wrong
/// trade (`docs/GROUPS_CONTRACT.md`). The only `Err` is a body that yields no
/// JSON or no `groups` array — the one failure the caller retries.
///
/// Total coverage is tuicr's invariant, not the model's promise. The restore
/// loop runs over every changeset path, so the result places every path in
/// exactly one group whatever came back.
pub fn apply(body: &str, changeset: &Changeset, heuristic: &Grouping) -> Result<Refined, String> {
    let value = extract_json(body)?;
    let groups = value
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| "response has no `groups` array".to_string())?;

    let all_paths: BTreeSet<&str> = changeset.files.iter().map(|f| f.path.as_str()).collect();
    let heuristic_of = heuristic_group_names(heuristic);

    let model_names = model_names(groups);

    let mut repairs = Vec::new();
    // Two returned groups are two groups even when the model gives them the
    // same name, and membership is bucketed by name, so every name a group is
    // filed under is reserved as it is taken.
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();
    let mut assigned: BTreeMap<String, String> = BTreeMap::new();

    for (index, group) in groups.iter().enumerate() {
        let name = group_name(group, index, &mut used, &mut repairs);
        order.push(name.clone());
        // A group with no usable `files` is not an empty group: it has already
        // taken a name and an order slot, and its intended paths will be swept
        // up by the restore loop below. Name the shape violation where it
        // happens.
        let Some(files) = group.get("files").and_then(Value::as_array) else {
            repairs.push(format!("group `{name}` has no `files`"));
            continue;
        };
        for entry in files {
            // A `files` entry that is not a string names no path at all, and
            // the path it was meant to be is reported as dropped by the restore
            // loop. Counted here so the shape violation is the model's and not
            // a silent one.
            let Some(path) = entry.as_str() else {
                repairs.push(format!("group `{name}` has a non-string `files` entry"));
                continue;
            };
            if !all_paths.contains(path) {
                repairs.push(format!("invented path dropped: {path}"));
                continue;
            }
            claim(&mut assigned, path, &name, &mut repairs);
        }
    }

    let restored = restore_dropped(
        &all_paths,
        &heuristic_of,
        &model_names,
        &mut used,
        &mut order,
        &mut assigned,
        &mut repairs,
    );

    let mut members: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (path, name) in &assigned {
        members.entry(name.as_str()).or_default().push(path.clone());
    }

    // A group whose every path was claimed elsewhere or invented keeps its name
    // and its order slot with no members (`docs/GROUPS_CONTRACT.md`). The
    // sidebar renders no row for it, and an incremental pass later can put a
    // file back into the slot the model asked for.
    let mut presented: Vec<PresentedGroup> = Vec::with_capacity(order.len());
    let mut inherited: BTreeSet<&str> = BTreeSet::new();
    for name in &order {
        let members = members.remove(name.as_str()).unwrap_or_default();
        // Per-path, not per-name: a group the model returned can end up holding
        // nothing but paths the restore loop put back — every file it listed
        // taken as a duplicate by an earlier group — and a group whose every
        // placement is the heuristics' own must not present as refined.
        let source = if members.iter().all(|path| restored.contains(path)) {
            GroupSource::Heuristics
        } else {
            GroupSource::Refined
        };
        let id = rejoin(heuristic, name, &members, &mut inherited);
        presented.push(PresentedGroup {
            id,
            name: name.clone(),
            source,
            new_since_full_pass: false,
            unbounded: false,
            members,
        });
    }

    // The cap last, over the finished partition, and by each group's own source
    // (`gd-26r.23`). Not a repair: an oversized answer violates nothing the
    // contract asks for, so it is not counted into `repairs` and does not push
    // the arm towards the "this model is misbehaving" warning.
    let presented = super::caps::enforce(presented);

    Ok(Refined {
        grouping: Grouping::restore(changeset, presented),
        repairs,
    })
}

/// Files one returned path under one returned group name, first claim winning.
///
/// A path repeated inside one group changes nothing about the partition, and
/// the count is read by a human as the arm's verdict, so it is not inflated by
/// a repair that moved no file. `tests/grouping/refine.rs` calls this rather
/// than keeping its own copy: the harness measured the published figures, so a
/// second copy of the rule is a second answer to the same question.
pub fn claim(
    assigned: &mut BTreeMap<String, String>,
    path: &str,
    name: &str,
    repairs: &mut Vec<String>,
) {
    use std::collections::btree_map::Entry;

    match assigned.entry(path.to_string()) {
        Entry::Vacant(slot) => {
            slot.insert(name.to_string());
        }
        Entry::Occupied(held) if held.get() != name => {
            let previous = held.get();
            repairs.push(format!(
                "duplicate path kept in `{previous}`, not `{name}`: {path}"
            ));
        }
        Entry::Occupied(_) => {}
    }
}

/// The names the model actually wrote, before any uniquifying.
///
/// [`restore_dropped`] needs to tell "the group called `X` *is* heuristic group
/// `X`" from "`X` is what [`group_name`] renamed something else to". Shared with
/// the measurement harness, which reads the same bodies through this function
/// rather than its own copy of the loop.
pub fn model_names(groups: &[Value]) -> BTreeSet<String> {
    groups
        .iter()
        .filter_map(|group| group.get("name").and_then(Value::as_str))
        .map(clean_name)
        .filter(|name| !name.is_empty())
        .collect()
}

/// Puts every changeset path the response left out back where the heuristics
/// had it, and returns the paths that had to be put back.
///
/// A dropped path goes back under its heuristic group's own name whenever the
/// model itself wrote that name: the prompt showed the model those names, so a
/// returned group the model called `X` *is* heuristic group `X` and the dropped
/// path belongs in it. Reusing the plain name also puts co-dropped paths from
/// one heuristic group back together, which a per-path uniquified name would
/// split, so the decision is made once per heuristic group and memoised.
///
/// A name `used` holds that the model never wrote is a different group wearing
/// it — `group_name` uniquifies a repeated `X` to `X-2`, and a heuristic group
/// genuinely called `X-2` must not be unioned into it. Those take a free name,
/// so the repair line names the group the file actually went to. `UNPLACED` is
/// always resolved this way: it is tuicr's own bucket, not a name the model can
/// have meant.
///
/// A heuristic name the response never mentioned opens a group of its own,
/// appended to `order` after everything the model ordered — which is also where
/// an unplaced concern belongs by rule 3.
pub fn restore_dropped(
    all_paths: &BTreeSet<&str>,
    heuristic_of: &BTreeMap<&str, &str>,
    model_names: &BTreeSet<String>,
    used: &mut BTreeSet<String>,
    order: &mut Vec<String>,
    assigned: &mut BTreeMap<String, String>,
    repairs: &mut Vec<String>,
) -> BTreeSet<String> {
    let mut restored: BTreeSet<String> = BTreeSet::new();
    // One target per heuristic group, so two paths dropped out of the same
    // group land back together rather than in `X-3` and `X-4`.
    let mut targets: BTreeMap<&str, String> = BTreeMap::new();
    for path in all_paths {
        if assigned.contains_key(*path) {
            continue;
        }
        // The heuristic partition is total over its own changeset, but the
        // caller takes the two as independent arguments and cannot enforce that
        // they agree. A path in neither is a caller mistake, and this is the
        // module whose whole premise is that trouble degrades to a grouping
        // rather than to a startup crash, so it is counted like every other
        // violation.
        let (source, dropped) = match heuristic_of.get(*path).copied() {
            Some(name) => (name, true),
            None => (UNPLACED, false),
        };
        let name = match targets.get(source) {
            Some(name) => name.clone(),
            None => {
                let name = if dropped && model_names.contains(source) {
                    source.to_string()
                } else {
                    free_name(source, used)
                };
                if used.insert(name.clone()) {
                    order.push(name.clone());
                }
                targets.insert(source, name.clone());
                name
            }
        };
        if dropped {
            repairs.push(format!("dropped path restored to `{name}`: {path}"));
        } else {
            repairs.push(format!(
                "path in no heuristic group either, filed under `{name}`: {path}"
            ));
        }
        restored.insert(path.to_string());
        assigned.insert(path.to_string(), name);
    }
    restored
}

/// Every changeset path to the name of the heuristic group holding it.
fn heuristic_group_names(heuristic: &Grouping) -> BTreeMap<&str, &str> {
    let names: BTreeMap<&GroupId, &str> = heuristic
        .groups()
        .iter()
        .map(|group| (&group.id, group.name.as_str()))
        .collect();
    heuristic
        .assignments()
        .iter()
        .filter_map(|assignment| {
            let path = assignment.path.to_str()?;
            Some((path, *names.get(&assignment.group_id)?))
        })
        .collect()
}

/// The identity a returned group takes on, per `docs/GROUPS_CONTRACT.md`:
/// identical membership first, then matching name, then a fresh id.
///
/// Membership is checked first because renaming an otherwise intact group is
/// the refine arm's strongest measured move, and under name-only matching that
/// case reads as every file in the group moving — discarding exactly the
/// group-keyed sidebar state a stable id exists to preserve.
///
/// `inherited` stops two returned groups claiming one existing id, which
/// membership-then-name cannot rule out on its own: a group can match one
/// existing group by membership and another by name.
fn rejoin<'a>(
    heuristic: &'a Grouping,
    name: &str,
    members: &[String],
    inherited: &mut BTreeSet<&'a str>,
) -> GroupId {
    let wanted: BTreeSet<&str> = members.iter().map(String::as_str).collect();
    let free = |group: &Group, inherited: &BTreeSet<&str>| !inherited.contains(group.id.as_str());

    let by_membership = heuristic.groups().iter().find(|group| {
        free(group, inherited)
            && heuristic
                .files_in(&group.id)
                .filter_map(|path| path.to_str())
                .collect::<BTreeSet<&str>>()
                == wanted
    });
    let matched = by_membership.or_else(|| {
        heuristic
            .groups()
            .iter()
            .find(|group| free(group, inherited) && group.name == name)
    });

    match matched {
        Some(group) => {
            inherited.insert(group.id.as_str());
            group.id.clone()
        }
        None => GroupId::new(),
    }
}

/// The name a returned group is filed under, which is not always the name the
/// model gave it. Membership is bucketed by name, so a reused name would
/// silently union two groups the model returned separately and a missing name
/// would union every group that lacks one — both changing the answer. Each is
/// uniquified and counted instead.
pub fn group_name(
    group: &Value,
    index: usize,
    used: &mut BTreeSet<String>,
    repairs: &mut Vec<String>,
) -> String {
    let base = match group.get("name").and_then(Value::as_str).map(clean_name) {
        Some(name) if !name.is_empty() => name,
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

/// A group name is rendered verbatim in the sidebar and persisted into the
/// session (`docs/SIDEBAR_MODEL.md`), so a trailing newline or an escape
/// sequence in one is a corrupted row for the rest of the review. Surrounding
/// whitespace goes, and so does anything that steers a terminal without
/// occupying a cell; nothing else is prettified.
///
/// That is control characters *and* the Unicode format category: a right-to-
/// left override (U+202E) reverses the rest of the row and a zero-width space
/// (U+200B) makes two different group names look like one. Both survive
/// `is_control`, which is why it is not the whole test. The trim comes after
/// the filter, since a name that is only zero-width characters trims to
/// nothing and must be caught as nameless rather than filed as blank.
fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control() && !is_format(*c))
        .collect::<String>()
        .trim()
        .to_string()
}

/// The Unicode `Cf` block, spelled out because `char` does not carry general
/// categories and pulling a table in for three ranges is not worth it. Covers
/// the bidi controls, the zero-width joiners and marks, and the interlinear
/// annotation and tag blocks.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{00ad}'
        | '\u{0600}'..='\u{0605}'
        | '\u{061c}'
        | '\u{06dd}'
        | '\u{070f}'
        | '\u{180e}'
        | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206f}'
        | '\u{feff}'
        | '\u{fff9}'..='\u{fffb}'
        | '\u{110bd}'
        | '\u{1d173}'..='\u{1d17a}'
        | '\u{e0001}'
        | '\u{e0020}'..='\u{e007f}'
    )
}

/// A model asked for "JSON and nothing else" mostly complies, and a reader that
/// fell over on a stray fence would spend a retry on the fence. Reading from the
/// first `{` with a streaming deserialiser takes the first balanced object and
/// ignores whatever follows it.
pub fn extract_json(body: &str) -> Result<Value, String> {
    let start = body
        .find('{')
        .ok_or_else(|| "no JSON object in response".to_string())?;
    first_value(&body[start..])
}

/// `serde_json::from_str` rejects trailing content, which a fenced answer has.
/// The streaming deserialiser stops at the end of the first value instead.
pub fn first_value(text: &str) -> Result<Value, String> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .next()
        .ok_or_else(|| "empty JSON document".to_string())?
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grouping::passes::GroupingConfig;
    use std::path::Path;

    fn changeset() -> Changeset {
        Changeset::parse(
            "M\tsrc/pricing/rules.ts\n\
             M\tsrc/pricing/rules.test.ts\n\
             M\tsrc/pricing/table.ts\n\
             M\tdocs/README.md\n\
             M\tpackage-lock.json\n",
        )
    }

    fn heuristic() -> Grouping {
        crate::grouping::group_changeset(&changeset(), GroupingConfig::default())
    }

    fn names(grouping: &Grouping) -> Vec<&str> {
        grouping
            .groups()
            .iter()
            .map(|group| group.name.as_str())
            .collect()
    }

    /// The heuristic group with the most files. The rejoin tests need a group
    /// that can lose a member and still be a group, which the one-file groups
    /// this fixture also produces cannot do.
    fn biggest(grouping: &Grouping) -> Group {
        grouping
            .groups()
            .iter()
            .max_by_key(|group| grouping.files_in(&group.id).count())
            .expect("the fixture groups into something")
            .clone()
    }

    fn paths(grouping: &Grouping, name: &str) -> Vec<String> {
        let group = grouping
            .groups()
            .iter()
            .find(|group| group.name == name)
            .expect("group is present");
        grouping
            .files_in(&group.id)
            .map(|path| path.to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn the_prompt_carries_the_rules_the_heuristic_groups_and_no_within_group_order() {
        let changeset = changeset();
        let heuristic = heuristic();
        let text = prompt(&changeset, &heuristic);

        assert!(text.contains(RULES), "the rules digest is sent verbatim");
        assert!(
            text.contains(&format!(
                "The heuristic grouping of this {}-file changeset",
                changeset.len()
            )),
            "{text}"
        );

        // Every group is headed by its own name and member count, every member
        // is listed once beneath it with its change status, and the headers run
        // largest first.
        let mut header_at: Vec<(usize, usize)> = Vec::new();
        let mut listed = 0;
        for group in heuristic.groups() {
            let members: Vec<String> = heuristic
                .files_in(&group.id)
                .map(|path| path.to_string_lossy().to_string())
                .collect();
            let header = format!("[{}] ({} files)\n", group.name, members.len());
            let at = text
                .find(&header)
                .unwrap_or_else(|| panic!("`{header}` is missing from:\n{text}"));
            header_at.push((at, members.len()));
            for path in &members {
                assert_eq!(
                    text.matches(&format!("  M {path}\n")).count(),
                    1,
                    "`{path}` is listed once, with its change status:\n{text}"
                );
                listed += 1;
            }
        }
        assert_eq!(listed, changeset.len(), "every file is shown");
        header_at.sort_by_key(|(at, _)| *at);
        assert!(
            header_at.windows(2).all(|pair| pair[0].1 >= pair[1].1),
            "the groups are rendered largest first: {header_at:?}"
        );

        // Rules 12-14 order files inside a group and the engine owns them on
        // both arms, so the digest the model is sent stops at rule 11: asking
        // would invite an answer the reader discards.
        let numbered: Vec<u32> = RULES
            .lines()
            .filter_map(|line| line.split_once('.'))
            .filter_map(|(head, _)| head.parse().ok())
            .collect();
        assert_eq!(numbered, (1..=11).collect::<Vec<u32>>(), "{RULES}");
    }

    #[test]
    fn a_clean_response_replaces_the_partition_and_carries_its_order() {
        let changeset = changeset();
        let refined = apply(
            r#"{"groups": [
                {"name": "pricing", "files": ["src/pricing/rules.ts", "src/pricing/rules.test.ts", "src/pricing/table.ts"]},
                {"name": "docs", "files": ["docs/README.md"]},
                {"name": "mechanical", "files": ["package-lock.json"]}
            ]}"#,
            &changeset,
            &heuristic(),
        )
        .expect("a well-formed body applies");

        assert!(refined.repairs.is_empty(), "{:?}", refined.repairs);
        assert_eq!(names(&refined.grouping), ["pricing", "docs", "mechanical"]);
        assert!(
            refined
                .grouping
                .groups()
                .iter()
                .all(|group| group.source == GroupSource::Refined)
        );
        // The engine's within-group sort applies to a refined grouping for
        // free: production before its test (rule 4), and the seam never
        // re-sorts.
        assert_eq!(
            paths(&refined.grouping, "pricing"),
            [
                "src/pricing/rules.ts",
                "src/pricing/rules.test.ts",
                "src/pricing/table.ts"
            ]
        );
    }

    #[test]
    fn every_repair_is_made_and_counted_and_the_partition_stays_total() {
        let changeset = changeset();
        let heuristic = heuristic();
        let refined = apply(
            r#"{"groups": [
                {"name": "pricing", "files": ["src/pricing/rules.ts", "src/invented.ts"]},
                {"name": "pricing", "files": ["src/pricing/rules.ts", "src/pricing/table.ts"]},
                {"files": ["docs/README.md"]},
                {"name": "empty"}
            ]}"#,
            &changeset,
            &heuristic,
        )
        .expect("a repairable body still applies");

        // Every repair, in the order they are made, and nothing else: a count
        // the human reads as the arm's verdict cannot be asserted loosely.
        assert_eq!(
            refined.repairs,
            [
                "invented path dropped: src/invented.ts",
                "group name `pricing` reused, filed as `pricing-2`",
                "duplicate path kept in `pricing`, not `pricing-2`: src/pricing/rules.ts",
                "group 2 has no `name`, called `unnamed-2`",
                "group `empty` has no `files`",
                "dropped path restored to `mechanical`: package-lock.json",
                "dropped path restored to `dir:src/pricing`: src/pricing/rules.test.ts",
            ]
        );

        // The repair lines are a report of where the files went, so where they
        // went is asserted too: the duplicate stayed in the first group that
        // claimed it, the reused name is a second group rather than a union,
        // and the nameless group kept its own file.
        assert_eq!(
            names(&refined.grouping),
            [
                "pricing",
                "pricing-2",
                "unnamed-2",
                "empty",
                "mechanical",
                "dir:src/pricing"
            ]
        );
        assert_eq!(
            paths(&refined.grouping, "pricing"),
            ["src/pricing/rules.ts"]
        );
        assert_eq!(
            paths(&refined.grouping, "pricing-2"),
            ["src/pricing/table.ts"]
        );
        assert_eq!(paths(&refined.grouping, "unnamed-2"), ["docs/README.md"]);
        assert_eq!(
            paths(&refined.grouping, "mechanical"),
            ["package-lock.json"]
        );
        assert_eq!(
            paths(&refined.grouping, "dir:src/pricing"),
            ["src/pricing/rules.test.ts"]
        );

        // Total by construction: every real path lands in exactly one group,
        // whatever the model did.
        let placed: Vec<String> = refined
            .grouping
            .assignments()
            .iter()
            .map(|assignment| assignment.path.to_string_lossy().to_string())
            .collect();
        assert_eq!(placed.len(), changeset.len());
        assert_eq!(
            placed.iter().collect::<BTreeSet<_>>().len(),
            changeset.len()
        );
        // The group that took a name and an order slot but kept no file keeps
        // both: the slot survives for an incremental pass to fill, and the
        // sidebar draws no row for a group with nothing under it.
        assert!(paths(&refined.grouping, "empty").is_empty());
    }

    /// A `files` entry that is not a string is a shape violation like any
    /// other: it names no path, the path it was meant to be comes back through
    /// the restore loop, and the count says the model got something wrong
    /// rather than blaming the restore.
    #[test]
    fn a_non_string_files_entry_is_repaired_rather_than_silently_skipped() {
        let changeset = changeset();
        let refined = apply(
            r#"{"groups": [
                {"name": "pricing", "files": ["src/pricing/rules.ts", 7]}
            ]}"#,
            &changeset,
            &heuristic(),
        )
        .expect("a repairable body still applies");

        assert_eq!(
            refined.repairs.first().map(String::as_str),
            Some("group `pricing` has a non-string `files` entry")
        );
        assert_eq!(
            paths(&refined.grouping, "pricing"),
            ["src/pricing/rules.ts"]
        );
        assert_eq!(refined.grouping.assignments().len(), changeset.len());
    }

    #[test]
    fn a_dropped_path_returns_under_its_heuristic_group_and_that_group_stays_heuristic() {
        let changeset = changeset();
        let heuristic = heuristic();
        let orphan = Path::new("package-lock.json");
        let heuristic_name = heuristic
            .group(heuristic.group_of(orphan).expect("placed"))
            .expect("named")
            .name
            .clone();

        let refined = apply(
            r#"{"groups": [
                {"name": "pricing", "files": ["src/pricing/rules.ts", "src/pricing/rules.test.ts", "src/pricing/table.ts", "docs/README.md"]}
            ]}"#,
            &changeset,
            &heuristic,
        )
        .expect("applies");

        // One session mixes sources: what the model returned is `Refined`, what
        // the restore loop had to put back is still the heuristics' answer and
        // says so.
        let restored = refined
            .grouping
            .groups()
            .iter()
            .find(|group| group.name == heuristic_name)
            .expect("the heuristic group reopened for the dropped path");
        assert_eq!(restored.source, GroupSource::Heuristics);
        assert_eq!(
            paths(&refined.grouping, &heuristic_name),
            ["package-lock.json"]
        );
        assert_eq!(
            names(&refined.grouping).first().copied(),
            Some("pricing"),
            "the model's order leads; the restored group is appended"
        );
    }

    /// The count is the arm's verdict to the human, so a violation that moved
    /// no file does not raise it.
    #[test]
    fn a_path_listed_twice_in_one_group_is_not_counted_as_a_repair() {
        let refined = apply(
            r#"{"groups": [
                {"name": "pricing", "files": [
                    "src/pricing/rules.ts", "src/pricing/rules.ts",
                    "src/pricing/rules.test.ts", "src/pricing/table.ts"]},
                {"name": "rest", "files": ["docs/README.md", "package-lock.json"]}
            ]}"#,
            &changeset(),
            &heuristic(),
        )
        .expect("applies");

        assert!(refined.repairs.is_empty(), "{:?}", refined.repairs);
        assert_eq!(
            paths(&refined.grouping, "pricing"),
            [
                "src/pricing/rules.ts",
                "src/pricing/rules.test.ts",
                "src/pricing/table.ts"
            ],
            "the repeat is one file, placed once"
        );
    }

    /// A returned group can end up holding only paths the restore loop put
    /// back — every file it listed taken by an earlier group — and then it is
    /// the heuristics' answer under the model's name, which is what `source`
    /// exists to say (`docs/GROUPS_CONTRACT.md`).
    #[test]
    fn a_group_left_holding_only_restored_paths_is_labelled_heuristics() {
        let changeset = changeset();
        let refined = apply(
            r#"{"groups": [
                {"name": "pricing", "files": ["src/pricing/rules.ts", "src/pricing/table.ts"]},
                {"name": "dir:src/pricing", "files": ["src/pricing/rules.ts"]}
            ]}"#,
            &changeset,
            &heuristic(),
        )
        .expect("applies");

        let refilled = refined
            .grouping
            .groups()
            .iter()
            .find(|group| group.name == "dir:src/pricing")
            .expect("the group the restore loop refilled");
        assert_eq!(
            paths(&refined.grouping, "dir:src/pricing"),
            ["src/pricing/rules.test.ts"],
            "every file the model gave it went to the group that claimed it first"
        );
        assert_eq!(
            refilled.source,
            GroupSource::Heuristics,
            "nothing in it is the model's placement"
        );
        assert_eq!(
            refined
                .grouping
                .groups()
                .iter()
                .find(|group| group.name == "pricing")
                .expect("present")
                .source,
            GroupSource::Refined
        );
    }

    /// `apply` takes the changeset and the heuristic grouping separately and
    /// cannot make them agree. A mismatch is a caller mistake in the one module
    /// whose premise is that trouble degrades to a grouping, so it is counted
    /// rather than panicked.
    #[test]
    fn a_path_the_heuristic_grouping_never_saw_is_filed_rather_than_fatal() {
        let changeset = changeset();
        let stale = crate::grouping::group_changeset(
            &Changeset::parse("M\tsrc/pricing/rules.ts\n"),
            GroupingConfig::default(),
        );

        let refined = apply(
            r#"{"groups": [{"name": "pricing", "files": ["src/pricing/rules.ts"]}]}"#,
            &changeset,
            &stale,
        )
        .expect("a stale heuristic grouping is not fatal");

        assert_eq!(
            refined.grouping.assignments().len(),
            changeset.len(),
            "the partition is still total"
        );
        // The literal, not the constant: what the group is called is part of
        // what a human sees in the sidebar, and a test written against the
        // constant would follow a rename that nobody meant to make.
        assert_eq!(
            paths(&refined.grouping, "unplaced"),
            [
                "docs/README.md",
                "src/pricing/rules.test.ts",
                "src/pricing/table.ts",
                "package-lock.json"
            ]
        );
        assert_eq!(
            refined
                .repairs
                .iter()
                .filter(|repair| repair.contains("no heuristic group"))
                .count(),
            4
        );
    }

    /// `source` is per group and load-bearing, and a group is only the
    /// heuristics' answer when *every* placement in it is. One path the model
    /// put there makes it the model's group, whatever the restore loop added.
    #[test]
    fn a_group_holding_a_model_placement_and_a_restored_path_is_refined() {
        let refined = apply(
            r#"{"groups": [
                {"name": "dir:src/pricing", "files": ["src/pricing/rules.ts", "docs/README.md"]}
            ]}"#,
            &changeset(),
            &heuristic(),
        )
        .expect("applies");

        let mixed = refined
            .grouping
            .groups()
            .iter()
            .find(|group| group.name == "dir:src/pricing")
            .expect("present");
        let members = paths(&refined.grouping, "dir:src/pricing");
        assert!(
            members.contains(&"src/pricing/rules.ts".to_string())
                && members.contains(&"src/pricing/rules.test.ts".to_string()),
            "the group holds one of each: {members:?}"
        );
        assert_eq!(
            mixed.source,
            GroupSource::Refined,
            "a group is only the heuristics' answer when every placement in it is"
        );
    }

    /// A name is rendered verbatim in the sidebar and persisted, so what the
    /// model wrapped in whitespace or padded with control characters is not what
    /// the group is called.
    #[test]
    fn a_group_name_is_filed_trimmed_and_free_of_control_characters() {
        let refined = apply(
            "{\"groups\": [\
                {\"name\": \" pricing\\n\", \"files\": [\"src/pricing/rules.ts\", \
                 \"src/pricing/rules.test.ts\", \"src/pricing/table.ts\"]},\
                {\"name\": \"do\\u001b[31mcs\", \"files\": [\"docs/README.md\"]},\
                {\"name\": \"  \", \"files\": [\"package-lock.json\"]}\
            ]}",
            &changeset(),
            &heuristic(),
        )
        .expect("applies");

        assert_eq!(
            names(&refined.grouping),
            ["pricing", "do[31mcs", "unnamed-2"]
        );
        assert_eq!(paths(&refined.grouping, "unnamed-2"), ["package-lock.json"]);
    }

    /// A right-to-left override reverses the rest of the sidebar row and a
    /// zero-width space makes two group names look like one. Neither is a
    /// control character, so `is_control` alone lets both through.
    #[test]
    fn a_name_that_steers_the_terminal_without_taking_a_cell_is_stripped_too() {
        let refined = apply(
            "{\"groups\": [\
                {\"name\": \"pri\\u202ecing\", \"files\": [\"src/pricing/rules.ts\", \
                 \"src/pricing/rules.test.ts\", \"src/pricing/table.ts\"]},\
                {\"name\": \"do\\u200bcs\", \"files\": [\"docs/README.md\"]},\
                {\"name\": \"\\u200b\\ufeff\", \"files\": [\"package-lock.json\"]}\
            ]}",
            &changeset(),
            &heuristic(),
        )
        .expect("applies");

        assert_eq!(names(&refined.grouping), ["pricing", "docs", "unnamed-2"]);
        assert!(
            refined
                .repairs
                .iter()
                .any(|repair| repair.contains("group 2 has no `name`")),
            "a name that is only zero-width characters is nameless, not blank: {:?}",
            refined.repairs
        );
    }

    #[test]
    fn a_renamed_but_intact_group_keeps_its_id() {
        let changeset = changeset();
        let heuristic = heuristic();
        let intact = biggest(&heuristic);
        let members: Vec<String> = heuristic
            .files_in(&intact.id)
            .map(|path| format!("{:?}", path.to_string_lossy()))
            .collect();
        let rest: Vec<String> = changeset
            .files
            .iter()
            .map(|file| format!("{:?}", file.path))
            .filter(|path| !members.contains(path))
            .collect();

        let body = format!(
            r#"{{"groups": [
                {{"name": "a-readable-concern-name", "files": [{}]}},
                {{"name": "everything-else", "files": [{}]}}
            ]}}"#,
            members.join(","),
            rest.join(",")
        );
        let refined = apply(&body, &changeset, &heuristic).expect("applies");

        let renamed = refined
            .grouping
            .groups()
            .iter()
            .find(|group| group.name == "a-readable-concern-name")
            .expect("present");
        assert_eq!(
            renamed.id, intact.id,
            "identical membership inherits the id regardless of the name"
        );
    }

    #[test]
    fn an_unchanged_group_that_keeps_its_name_keeps_its_id() {
        let changeset = changeset();
        let heuristic = heuristic();
        let kept = biggest(&heuristic);
        // Same name, one file short — so membership cannot match and only the
        // name rule can carry the identity across.
        let mut members: Vec<String> = heuristic
            .files_in(&kept.id)
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        let moved = members.pop().expect("a non-empty group");
        assert!(
            !members.is_empty(),
            "the group has to survive losing a file for this to be a name match"
        );
        let listed = |paths: &[String]| {
            paths
                .iter()
                .map(|path| format!("{path:?}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let others: Vec<String> = changeset
            .files
            .iter()
            .map(|file| file.path.clone())
            .filter(|path| !members.contains(path))
            .collect();

        let body = format!(
            r#"{{"groups": [{{"name": {:?}, "files": [{}]}}, {{"name": "elsewhere", "files": [{}]}}]}}"#,
            kept.name,
            listed(&members),
            listed(&others)
        );
        let refined = apply(&body, &changeset, &heuristic).expect("applies");
        let same = refined
            .grouping
            .groups()
            .iter()
            .find(|group| group.name == kept.name)
            .expect("present");
        assert_eq!(same.id, kept.id, "matching name inherits the id");
        assert!(
            paths(&refined.grouping, "elsewhere").contains(&moved),
            "the file that left is where the model put it"
        );
    }

    #[test]
    fn a_body_with_no_json_or_no_groups_array_is_the_only_error() {
        let changeset = changeset();
        let heuristic = heuristic();
        assert!(apply("I am sorry, I cannot help.", &changeset, &heuristic).is_err());
        assert!(apply(r#"{"answer": "none"}"#, &changeset, &heuristic).is_err());
        // A fenced body is not a parse failure; the reader steps over the fence.
        assert!(
            apply(
                "```json\n{\"groups\": [{\"name\": \"all\", \"files\": [\"docs/README.md\"]}]}\n```",
                &changeset,
                &heuristic,
            )
            .is_ok()
        );
    }

    #[test]
    fn unknown_keys_are_ignored_rather_than_rejected() {
        let changeset = changeset();
        let refined = apply(
            r#"{"version": 7, "notes": "hi", "groups": [
                {"name": "all", "confidence": 0.9,
                 "files": ["src/pricing/rules.ts", "src/pricing/rules.test.ts",
                           "src/pricing/table.ts", "docs/README.md", "package-lock.json"]}
            ]}"#,
            &changeset,
            &heuristic(),
        )
        .expect("read leniently, no version field");
        assert_eq!(names(&refined.grouping), ["all"]);
        assert!(refined.repairs.is_empty());
    }
}
