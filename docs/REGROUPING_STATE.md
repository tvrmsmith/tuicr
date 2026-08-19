# Review state when the grouping changes

Design record for `gd-26r.8` on the wayfinder map `gd-26r` (grouped review of
large changesets in tuicr). **Decision-only**: no code lands with this ticket.
The session-model change described here ships with the grouping engine itself,
because there is no grouping engine in `src/` yet to attach it to.

Assumed, settled elsewhere: grouping is file-level with a **strict partition**
(`gd-26r.5`); groups are collapsible top-level sidebar nodes with
`expanded_groups` keyed on the group id (`gd-26r.7`, revised by `gd-26r.31` and
`gd-26r.35`); the engine lives in
tuicr with an
optional async refine that shells out to an agent CLI (`gd-26r.4`); refine's
run-to-run movement is measured in `docs/GROUPING_PASSES.md` (`gd-26r.11`).

## Correction to the ticket's own premise

The ticket said review state is keyed by *path + content_hash*, and concluded
that this settles the corruption question. The mechanism was wrong; the
conclusion survives for a different reason.

`ReviewSession.files: HashMap<PathBuf, FileReview>` (`src/model/review.rs:121`)
is keyed by **path alone**. `content_hash` (`review.rs:27`) is a field on the
*value*, compared in `add_file` (`review.rs:164-177`) purely to clear `reviewed`
when the file changed. `reviewed_hunks` (`review.rs:25`) is genuinely
content-addressed and survives anything.

So under a file-level strict partition, **no grouping change can corrupt review
state**, because review state does not know groups exist. Every decision below
is about experience and cost, not safety. The one real hazard the seam map
identified — two `DiffFile`s sharing a `display_path()` aliasing into one
`FileReview` — was closed by `gd-26r.5` choosing a strict partition.

## The shape of the answer

**The grouping is a persisted artefact of the session, not a derived view.** It
is computed once, kept, and only ever recomputed wholesale when the human asks.
Everything else follows from that.

The measurements force it. Only **3–12% of files keep identical group-mates
across ten refine runs** (`docs/GROUPING_PASSES.md`, *Stability*), so a grouping
recomputed on every open would be a different grouping on every open. Since
`gd-26r.7` puts group identity into UI state as well as review state, that
churn would reset the sidebar every morning. Persisting is not an optimisation
here; it is the only way the feature is usable across more than one sitting.

## What is persisted

Two additions to the session JSON, both `#[serde(default)]` so existing session
files parse unchanged (precedent: `review.rs:24,26`; legacy-parse tests at
`review.rs:508-520,643`).

1. **`group_id` on `FileReview`** — a stable opaque id, not a name.
2. **A group table on `ReviewSession`** — one record per group:

   | field | purpose |
   | --- | --- |
   | `id` | stable identity; what `FileReview.group_id` and `expanded_dirs` key on |
   | `name` | display only, rendered verbatim (`docs/SIDEBAR_MODEL.md`) |
   | `order` | position in intent-centrality order |
   | `source` | `heuristics` \| `refined` \| `incremental` — what produced this group |
   | `new_since_full_pass` | set when opened by incremental assignment |
   | `unbounded` | set when the size cap's split pass could not bound the group (`gd-26r.23`) |

**Why an id and not the name.** Names are not stable and are not even stable in
*kind*: the heuristic arm derives mechanical token joins
(`github-client-work-item-queries`), the refine arm returns readable concern
names, and a refine run may rename a group it otherwise leaves intact. With the
name as the key, every rename is indistinguishable from every file in that group
moving, and all group-keyed UI state dies for nothing. This **amends
`gd-26r.4`**, which said "an optional `group` field on `FileReview`".

**Why `source` is load-bearing.** It is what lets the status indicator
(`gd-26r.15`) say *heuristics only, refine unavailable* per group rather than
globally, and it gives `gd-26r.18` somewhere to record a human correction as a
first-class provenance value rather than an indistinguishable overwrite.

Not persisted: the runner-up group and one-line reason the engine records on
close calls (~14% of files). `gd-26r.7` deliberately does not surface them, and
until a human can act on them they are dead weight in a file that lives forever.
`gd-26r.18` may reverse this.

**Persisted forever, or not at all.** Sessions are never swept — no TTL, no age
pruning — and `discard_session_and_quit` is the normal exit for a comment-free
review. So the grouping must tolerate both. It does: the group table is
bounded by the changeset (13 groups for 161 files in the fixture), and a
discarded session simply loses a grouping that costs heuristic time to rebuild.

## What invalidates it

Nothing invalidates the grouping wholesale except a human asking. Below the
wholesale level there is exactly one mechanism, and it only ever *adds*.

### Incremental assignment — the only automatic change

Runs on `:reload` (`:e`; `CommandSpec::new(&["e", "reload"], handler.rs:28)` →
`App::reload_diff_files`, `diff_load.rs:509-631`) and on commit-selection
changes. **Heuristics only — never refine.** Deterministic, instant, free.

- **New files** are assigned: they join an existing group or open a new one.
- **Content-changed files** are reassigned, detected by the already-shipped
  `content_hash` compare in `add_file` (`review.rs:164-177`).
- **Unchanged files never move.** Ever. This is the invariant the whole
  decision rests on: between one full pass and the next, the sidebar you are
  reading does not rearrange itself.

Reassigning content-changed files sounds more disruptive than it is:
`add_file` has **already cleared `reviewed`** on that file before grouping sees
it, because its content moved. So an incremental reassignment never relocates
something the human had approved.

**The size cap does not run incrementally** (`gd-26r.23`). An arrival that
pushes a group past the hard cap for its arm is left where it belongs: a group
row splitting in two under a reader mid-review is worse than a row that is
briefly too big, and refusing the arrival would mint groups by arrival order.
The drift the arrivals add carries the review to the `gd-26r.36` backstop's full
pass, and that pass splits. The cap is therefore an invariant of each full pass,
not of every frame.

**A new group is appended last and marked new.** It is by definition unranked —
intent-centrality order is a property of a full pass — so it sits at the bottom
next to the drive-bys, carrying `new_since_full_pass`. Existing group order is
never reshuffled incrementally. This gives the staleness indicator
(`gd-26r.15`) group granularity instead of only a global percentage.

### Commit-selection changes

The file set becomes a different changeset, served from `commit_diff_cache`
(`src/app/mod.rs:1284`, keyed `(usize, usize)`).

**Groups carry over where paths match**; unmatched files are incrementally
assigned exactly as on `:reload`. One rule covers every non-regroup path.

**And the staleness fraction jumps accordingly.** Switching to a largely
unrelated commit range leaves most files unmatched, drives the incrementally-
assigned fraction up, and so pushes the auto-regroup backstop toward firing.
That is the intended behaviour, not a side effect: a wholesale changeset switch
*should* look maximally stale, and the backstop is what notices.

Narrowing or widening a range — the common case — matches nearly every path and
costs nothing.

### The staleness indicator and the auto backstop

tuicr tracks the fraction of files incrementally assigned since the last full
pass and surfaces it. That indicator is the primary defence against drift.

**The auto full regroup stays** (`gd-26r.4`), at a deliberately high threshold —
but **it is heuristics-only**. This narrows `gd-26r.4`, which left it ambiguous.
The backstop must be free, instant and deterministic: a threshold crossing that
silently spent $1.40–2.00 and three minutes and reshuffled the sidebar mid-review
would be exactly the surprise this whole ticket exists to prevent. Refine costs
money and wall-clock, so refine is something the human asks for.

**Shipped by `gd-26r.36`**, with the threshold at `[grouping].regroup_threshold`
— a drift *percentage*, defaulting to 75, `0` for off. Three things it settled
that this record left open:

- **It fires on `Grouping::drift_percent()`**, the whole-percent form of the one
  drift number the sidebar chip renders (`gd-26r.37`). Not a second fraction of
  its own: a backstop that fired at a value the reader never watched approach is
  a surprise rather than a backstop, and comparing the raw fraction to a
  configured percentage would let it trip at 74.6% under a header still reading
  `75% new`. It inherits that number's group-derived semantics with it, so a
  file that *joined* an established group moves neither the chip nor the
  threshold.
- **It is polled from the main loop**, next to `:regroup`'s own poller, rather
  than called from the dozen loads that drift a grouping. A landing under the
  reader is what that loop already does, and one load forgetting the call would
  be a silent gap.
- **A crossing has to happen under the reader.** The first poll latches whatever
  drift the review opened carrying and fires nothing, so a session persisted
  over the threshold is not regrouped on its first frame — *reopening a session
  never regroups* holds literally, and a full pass would otherwise mint fresh
  ids over exactly the groups the reader came back to. The next arrival, which
  they are present for, fires it.

## How a human forces a regroup

`:regroup`. It is the only path to a full recompute, and the only path to
refine after a session's first grouping.

| trigger | scope | refine? |
| --- | --- | --- |
| first grouping of a session | full | yes, if `[grouping].refine = true` |
| `:reload`, commit switch | incremental, additive only | never |
| auto threshold crossed | full | never — heuristics only |
| `:regroup` | full | yes, if `[grouping].refine = true` |

**Reopening a session never regroups and never refines.** Yesterday's review
shows yesterday's groups. **Nor does the `<leader>g` toggle** (`gd-26r.35`):
turning the grouped sidebar off keeps `App::grouping` populated, so turning it
back on re-sorts from the grouping already in hand — no pass, no call. The one
exception is a review that never had a grouping at all (`--no-grouping`), where
the first toggle on computes one from the heuristics alone. This is the largest cost lever in the whole grouping
feature: refine is now once per *review*, not once per *open* and never per
`:reload`.

## What happens to review state across a regroup

**A reviewed file that lands in a different group stays reviewed.** Content
unchanged means approval unchanged: the human approved the diff, not the
grouping — and they asked for the regroup, so the movement is expected rather
than sprung on them. No signal, no marker, no "arrived from elsewhere" count.

This costs nothing to implement. Path-keyed `ReviewSession.files` already
behaves this way, and `reviewed_hunks` is content-addressed, so both survive a
regroup untouched with no code at all.

The rejected alternative — clearing `reviewed` on group change, on the theory
that the group is part of the context in which the file was approved — is
unusable on the measured numbers. At 3–12% group-mate stability a single
`:regroup` would clear substantially the entire review.

## What happens to UI state across a regroup

**Reset: all groups collapsed.** Not "everything expanded" as `expand_all_dirs`
(`init.rs:570`) does today, and not a best-effort carry-over.

The 13-row collapsed overview is the feature (`gd-26r.7`) — and it is precisely
what a human wants to re-read immediately after asking for a new grouping. It is
also deterministic, needs no remapping logic, and leaves no stale group keys in
`expanded_groups` (keys in `expanded_dirs` before `gd-26r.35` split the two
sets; `(group, directory)` keys before `gd-26r.31` removed in-group directory
rows). Best-effort carry-over is the most
machinery for the least benefit: under 3–12% stability it would mostly fail
anyway.

Note this diverges from startup, where `expand_all_dirs` runs. That is
deliberate. Startup has no prior state to discard; a regroup does.

## Answers to the four questions as asked

1. **Does a reviewed file that moves group stay reviewed?** Yes, silently. And
   the premise is narrower than it looked: a file only ever moves group on an
   explicit `:regroup`, or as an incremental reassignment of a file whose
   content changed — which had already lost its `reviewed` flag.
2. **Persisted or recomputed fresh?** Persisted, as an artefact of the session:
   `group_id` on `FileReview` plus a group table carrying name, order, source.
3. **What invalidates it, and how does a human force a regroup?** Nothing
   invalidates it wholesale automatically except the high auto backstop, which
   is heuristics-only. Below that, incremental assignment adds new files and
   reassigns content-changed ones, never moving anything else. `:regroup` is the
   human's lever and the only route to refine after the first grouping.
4. **Is grouping instability across runs tolerable, and does it put a stability
   requirement on the heuristics?** The question dissolves. Instability was only
   ever a problem for a grouping recomputed per run, and grouping is no longer
   recomputed per run. The heuristics are deterministic already, so they carry
   **no new stability requirement**. Refine's 3–12% is not a defect to fix but a
   property to schedule around: it is confined to the two moments the human
   explicitly asks for a new grouping, when a reshuffle is what they wanted.

## What this constrains in sibling tickets

Stated here rather than absorbed.

- **`gd-26r.4` is amended in two places.** `FileReview` carries a stable
  `group_id` plus a session-level group table, not an `Option<String>` name; and
  the auto regroup at `regroup_threshold` is **heuristics-only**, never refine.
- **`gd-26r.14` (mid-session regroup)** inherits the reset rule: whenever a full
  regroup lands under the user — including the async refine result arriving
  after the TUI opened — the sidebar resets to all groups collapsed. It also
  still owns the `GapId { file_idx, hunk_idx }` hazard (seam map §2):
  `sort_files_by_directory` clears only `file_line_count_cache`,
  not the gap maps.
- **`gd-26r.15` (status indicator)** gains a group-granular input:
  `new_since_full_pass` on the group record, alongside the global
  incrementally-assigned fraction. And the fraction must account for
  commit-selection carry-over misses, which can spike it in one step.
- **`gd-26r.18` (human corrections)** gets `source` on the group record as the
  natural place to mark a correction as human-made, and inherits the decision
  *not* to persist runner-up/reason — which it may reverse.
- **`gd-26r.11`'s consensus-of-three is worth re-examining.** Voting buys
  determinism given fixed inputs, and determinism mattered most under the
  assumption that grouping was recomputed. With grouping computed once and kept,
  the human sees one refine result and keeps it, so the practical value of
  paying 3× for reproducibility drops. Tracked with the cost work.
