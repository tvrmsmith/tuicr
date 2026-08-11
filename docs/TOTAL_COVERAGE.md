# Total coverage of the partition

Design record for `gd-26r.12` on the wayfinder map `gd-26r` (grouped review of
large changesets in tuicr). **Mostly decision-only**: the one thing that lands
as code with this ticket is the guard on the commit-message hoist
(`src/app/tests/commit_message_hoist_tests.rs`), because that invariant exists
in `src/` today and was unguarded. Everything else shipped with the grouping
engine, whose passes now live in `src/grouping/passes.rs`; they were a scored
prototype under `tests/grouping/` when this record was written, and
`tests/grouping/` holds the scoring harness alone today.

Assumed, settled elsewhere: grouping is file-level with a **strict partition**
(`gd-26r.5`); groups are collapsible top-level sidebar nodes whose
members are listed directly beneath them, with no directory rows inside a
group (`gd-26r.7`, `gd-26r.31`, `docs/SIDEBAR_MODEL.md`); grouping is a
persisted session artefact keyed by a stable opaque `group_id`, incremental
assignment is heuristics-only and never moves an unchanged file (`gd-26r.8`,
`docs/REGROUPING_STATE.md`); refine blocks the TUI at startup, is cancellable,
and the heuristics-only fallback is a first-class human-causable outcome
(`gd-26r.14`, `docs/MID_SESSION_REGROUP.md`).

## The measurement that reshaped the second question

The ticket asked what happens to "files no heuristic pass claims". Run on both
fixtures, the answer is **none** — the heuristic arm is already total by
construction. `directory_fallback_pass` (`src/grouping/passes.rs:449-457`)
is the sixth and last pass and claims unconditionally, so nothing can survive
it unassigned.

| fixture | files | unclaimed | claimed by `directory-fallback` | `dir:` groups | singleton groups |
| --- | --- | --- | --- | --- | --- |
| `orca-971b16754` (checked in) | 161 | **0** | 7 (4.3%) | 4 | 1 |
| `meridian-6c22fda02` (private) | 158 | **0** | 15 (9.5%) | 11 | 10 |

Note on the ticket's wording: fixture 2 is **not** committed — `gd-26r.20` kept
it out of the repo because its paths come from a private work repository. It is
read at runtime from `$TUICR_GROUPING_FIXTURES`, and was present on this machine,
so both numbers above are measured rather than inferred.

So an **Ungrouped bucket is not a real surface — it would be dead code**, and
the question it was asked in service of is empty. The real question the
measurement exposes is different: on fixture 2 the fallback turns 15 leftover
files into **11 groups, 10 of them one file each**, inflating the collapsed
overview that `gd-26r.7` identified as the whole feature.

### The absorb floor was measured, not assumed

The obvious response — let the heuristics place these files rather than leaving
them to a directory fallback — is what `absorb_leftovers_pass` already tries and
refuses, because their best similarity falls under `absorb_min_similarity = 2.0`.
Dropping that floor was swept:

| fixture 2 (158 files) | F1 | P | R | groups | singletons | fallback files |
| --- | --- | --- | --- | --- | --- | --- |
| floor 0.00 | 0.345 | 0.345 | 0.346 | 18 | 1 | 0 |
| floor 0.50 – 1.25 | 0.346 | 0.354 | 0.339 | 22 | 5 | 4 |
| **floor 2.00 (shipped)** | **0.378** | **0.425** | 0.340 | 29 | 11 | 15 |
| floor 2.00, leftovers merged into one `other` bucket | 0.365 | 0.387 | 0.346 | 19 | 1 | 15 |

Fixture 1 is nearly flat across the whole sweep (0.394 / 0.395 / 0.394, and
0.397 for the merged bucket). **Precision is the tell**: forcing the strays into
the nearest group costs fixture 2 **0.08 precision**, because they land in real
concern groups and dilute them. That is `GROUPING.md` rule 6's smell hidden
inside good groups rather than shown. Absorbing everything is the worst arm on
the only fixture that distinguishes the arms at all.

## Decision 1 — the commit-message pseudo-file sits outside the partition

**`Commit Message (<sha>)` is not a member of any group. It is pinned above
every group node as a depth-0 row, and keeps `diff_files` index 0.**

The partition is total over **real files**. The groups contract never sees a
pathless entry, and the engine is never asked to reason about one.

This is the option that costs one sidebar row:

```
┌ Files · 0/161 ───────────────┐
│  ▢   Commit Message (971b167) │
│▶ enterprise-host-routing  0/18│
│▶ work-items                0/8│
│▶ github-client-plumbing   0/38│
└───────────────────────────────┘
```

Rejected alternatives:

- **Its own singleton group, ordered first.** Uniform — every `diff_files` entry
  would carry a `group_id` and `build_visible_items` would need no special case.
  Rejected because it costs two rows instead of one in the 13-row overview, the
  group is collapsible so the message can be hidden by accident, and the engine
  still has to be told never to touch it. The uniformity is bought back at the
  price of the thing the overview is for.
- **Joining the first or most intent-central group.** Rejected because it gives
  up the hoist: the message would sit inside a collapsible group, invisible on
  the first screen, and would *relocate between runs* — which group is "first"
  is refine's answer, and only 3–12% of files keep their group-mates across runs
  (`docs/GROUPING_PASSES.md`).

### The hoist is now guarded

`gd-26r.12` was warned that nothing guarded the index-0 hoist, and that was
correct: `rg is_commit_message src/app/tests/` found only struct-literal
`false`s. Two independent places put the pseudo-file at index 0 —
`insert_commit_message_if_single` (`src/app/diff_load.rs:34-97`) inserts it, and
`sort_files_by_directory` (`src/app/tree.rs:144-150,165`) hoists it back ahead of
every real file on **every** reorder, of which the seam map counts 17 non-test
call sites.

`src/app/tests/commit_message_hoist_tests.rs` covers both, and stands entirely
on the hoist itself, touching no part of the grouping engine:

1. `insert_commit_message_if_single` puts it at index 0, flagged, exactly once.
2. `sort_files_by_directory(true)` keeps it at index 0. The discriminating case
   is a repo-root file: `README.md` is filed under `"."`, which sorts before
   every real directory in the `BTreeMap`, so without the hoist it takes index 0.
3. The hoist survives `sort_files_by_directory(false)`, the position-preserving
   branch `:reload` takes.
4. Two commits selected means no pseudo-file at all — so the tests above cannot
   pass against an implementation that hoists something unconditionally.

Mutation-checked: commenting out the hoist fails tests 2 and 3.

## Decision 2 — leftovers stay as per-directory `dir:` groups

**No Ungrouped bucket, no Other bucket. The absorb floor stays at 2.0, and the
files it refuses become real groups named for their parent directory.**

They are ordinary groups in the partition — the strict partition is unqualified,
and a leftover file is in exactly one group like every other file.

This is the best-measured arm on fixture 2 (F1 0.378, precision 0.425) and it
preserves where each stray came from rather than pooling them into a bucket
whose only content is "we could not tell". The two rejected arms both trade
measured accuracy for overview compactness:

- **One `other` bucket, ordered last.** 29 rows to 19 and 10 singletons to 1 on
  fixture 2, and a small *gain* on fixture 1 (0.394 to 0.397). Rejected: it
  costs 0.013 F1 and 0.04 precision on fixture 2, and pooling discards the one
  thing the fallback actually knows.
- **Dropping the absorb floor.** Rejected on the measurement above.

**The known cost, recorded rather than papered over.** On a fixture-2-shaped
changeset the collapsed overview is 29 rows, not 13, and 10 of those rows are
one-file groups. `GROUPING.md` rule 1 says groups are concern-shaped, not
directory-shaped, and a `dir:` group is by construction the latter. That is the
accepted price of not polluting concern groups, and it is visible: a screen of
`dir:` rows reads as "the heuristics gave up here", which is exactly the signal
rule 6 wants a residual group to send. If `gd-26r.18`'s human corrections or a
third fixture change the balance, this is the decision to revisit — the merged
bucket is a one-line change to the fallback pass.

## Decision 3 — the heuristic arm emits an order

`gd-26r.14` found the heuristic arm produces no reading order at any level,
because `Partition.groups` is a name-keyed `BTreeMap`. That is not merely an
absence: it actively places leftovers wrong, since `dir:…` sorts under "d",
interleaved among the concern groups. "Sorting leftovers last" is unreachable
without an explicit order.

**The heuristic arm populates the `order` field `gd-26r.8` already put on the
group record: concern groups by member count descending, `dir:` fallback groups
pinned last as a class and ordered among themselves by size descending.**

This is not intent-centrality — `gd-26r.14` was right that token heuristics
cannot produce that, and whether they ever can is `gd-26r.22`'s question. It is
a defensible proxy that costs nothing (`report_fixture` already sorts by size)
and it matters *because* `gd-26r.14` made the heuristics-only outcome routine: a
cancelled refine must not open on a screen led by a one-file `dir:` group.

Rejected: alphabetical with leftovers pinned last (stable, but the first screen
is sorted by an accident of naming), and no order at all (leaves the routine
cancelled-refine screen led by whatever sorts first).

## Decision 4 — what this settles for `gd-26r.10` (groups contract shape)

Stated here rather than absorbed. Answering the ticket's own framing directly:

1. **The contract does NOT require total coverage. A refine response may name a
   subset of the changeset's paths, and tuicr completes it.** A response that
   omits three paths is applied, not rejected. `gd-26r.14` made the fallback
   routine and the money is spent at dispatch, so discarding a paid-for 30–70s
   result over an omitted path is the wrong trade.

2. **Completion means: an omitted path keeps its existing heuristic group.** The
   heuristic partition is computed first and is always total, so every path
   already has a group before refine is asked. Completion is therefore "leave it
   where the heuristics put it" — nothing is re-derived, and there is no bucket
   to invent.

   The consequence, which `gd-26r.8` already supports: the applied partition can
   mix refined groups with carried-over heuristic ones, so **`source` is
   per-group and load-bearing, not a global flag**. `gd-26r.15` should read it
   per group.

3. **Total coverage is tuicr's invariant, not the agent's promise.** The strict
   partition of `gd-26r.5` holds unconditionally at the point of application.
   What changes is who guarantees it: tuicr does, by construction, before the
   partition is ever applied. The engine may propose; tuicr closes.

4. **The pseudo-file is out of the contract's domain entirely.** The request
   carries real changeset paths only; a response naming `Commit Message (<sha>)`
   is naming a path that is not in the changeset, and is treated the same as any
   other unknown path. Whatever `gd-26r.10` decides about unknown paths in a
   response covers this case with no special clause.

5. **Reading order is required of the response** (unchanged from `gd-26r.14`),
   and the heuristic arm now also produces one (Decision 3) — so `order` is a
   field both arms populate, not a refine-only concession.

## What this constrains elsewhere

- **`build_visible_items` gains one branch, not a new node type.** The pinned
  commit-message row is a `FileTreeItem::File { file_idx: 0, depth: 0 }` emitted
  before the group loop. (`gd-26r.7` also required `seen_dirs` to reset at each
  group boundary; `gd-26r.31` removed in-group directory rows altogether, so
  there is no `seen_dirs` in the grouped branch to reset.) The pseudo-file is not in any group, so it does not
  disturb the re-scoped contiguity invariant — but the invariant's
  `debug_assert` must be written to start *after* it rather than at index 0.
- **`expand_all_dirs`, `jump_to_file` and `ensure_valid_tree_selection`** need no
  new handling for the pseudo-file: it has no path ancestors today and gains
  none, and it is always visible, so there is nothing to expand to reach it.
- **`gd-26r.15`** reads `source` per group, because a completed partial response
  mixes refined and heuristic groups in one partition.
- **`gd-26r.22`** inherits Decision 3 as the floor to beat: size-descending with
  leftovers last is what an intent-centrality order has to improve on, on the
  heuristic side.
- **`gd-26r.18`** owns the revisit on Decision 2: human corrections are the first
  evidence that will say whether a screen of `dir:` rows is legible or noise.
