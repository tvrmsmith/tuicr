# Grouping design docs index

Seven documents describe how tuicr cuts a changeset into groups. This file is
the map from a question to the one that answers it.

**One fact, one owner.** Every fact about grouping belongs to exactly one of
these documents. When you learn something new, write it in the owner and point
at it from anywhere else that needs it.

**This file is a map, never a summary.** It carries questions, filenames and
section names. It restates nothing from the documents it points at, so it can
never contradict them. Adding a design doc means adding a row here; it does not
mean copying the doc's content here.

## Which doc answers this

| Question | Owner |
| --- | --- |
| What makes a group a good group, and what decides which group comes first? | [`GROUPING.md`](GROUPING.md) § The rules |
| Where does a test file go, and what order do files take inside one group? | [`GROUPING.md`](GROUPING.md) § Within-group file order |
| Two groups could both claim this file. What does the engine record? | [`GROUPING.md`](GROUPING.md) § Ambiguous assignments |
| Why is every file in exactly one group, and can a file be in none? | [`GROUPING.md`](GROUPING.md) preamble, and [`TOTAL_COVERAGE.md`](TOTAL_COVERAGE.md) |
| What types does the engine hand the sidebar? | [`GROUPS_CONTRACT.md`](GROUPS_CONTRACT.md) § Shape 1 |
| What does a refine response owe, and what happens when it lies? | [`GROUPS_CONTRACT.md`](GROUPS_CONTRACT.md) § Shape 2, § Repair and failure behaviour |
| Does a group that comes back from refine keep its id? | [`GROUPS_CONTRACT.md`](GROUPS_CONTRACT.md) § Rejoining an existing group |
| The model returned something unparseable. What happens next? | [`GROUPS_CONTRACT.md`](GROUPS_CONTRACT.md) § Parse failure |
| Does `contract-approval` apply to the Vertex call? | [`GROUPS_CONTRACT.md`](GROUPS_CONTRACT.md) § Ruling |
| Where does the commit-message row sit relative to the groups? | [`TOTAL_COVERAGE.md`](TOTAL_COVERAGE.md) Decision 1 |
| Why is my sidebar full of one-file `dir:` groups? | [`TOTAL_COVERAGE.md`](TOTAL_COVERAGE.md) Decision 2 |
| What order do heuristic groups come out in, when refine never ran? | [`TOTAL_COVERAGE.md`](TOTAL_COVERAGE.md) Decision 3 |
| Why does the sidebar have no directory rows inside a group? | [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § Revision (`gd-26r.31`) |
| Does the `nested`/`compact`/`flat` tree mode do anything while grouping is on? | [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § The tree mode is its own feature |
| How many rows does each layout cost on the fixture? | [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § Row cost |
| What can the sidebar header chip say, and which one wins? | [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § The header suffix |
| What do `~` and `!` on a group row mean? | [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § The marker slot on group rows |
| What exactly is the `n% new` number counting? | [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § The drift number |
| What invariant must `build_visible_items` hold under grouping? | [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § What this demands of `build_visible_items` |
| Which parts of the grouping survive into the session file? | [`REGROUPING_STATE.md`](REGROUPING_STATE.md) § What is persisted |
| What changes the grouping without the reader asking? | [`REGROUPING_STATE.md`](REGROUPING_STATE.md) § Incremental assignment |
| Which actions regroup, and which of those also refine? | [`REGROUPING_STATE.md`](REGROUPING_STATE.md) § How a human forces a regroup |
| When does tuicr regroup on its own? | [`REGROUPING_STATE.md`](REGROUPING_STATE.md) § The staleness indicator and the auto backstop |
| What does `:regroup` do to my review state and my expanded rows? | [`REGROUPING_STATE.md`](REGROUPING_STATE.md) § What happens to review state across a regroup, § What happens to UI state across a regroup |
| Why does startup freeze while refine runs? | [`MID_SESSION_REGROUP.md`](MID_SESSION_REGROUP.md) § Why blocking wins at startup |
| How do I get out of the wait, and what do I get instead? | [`MID_SESSION_REGROUP.md`](MID_SESSION_REGROUP.md) § The escape: cancel key and timeout |
| Why is `:regroup` async when startup blocks? | [`MID_SESSION_REGROUP.md`](MID_SESSION_REGROUP.md) § `:regroup` stays async |
| A regroup landed while I was reading. Where did my cursor and my expanded gaps go? | [`MID_SESSION_REGROUP.md`](MID_SESSION_REGROUP.md) § What a landing does |
| What did any of this measure: F1, Kendall tau, cost, latency, model comparisons? | [`GROUPING_PASSES.md`](GROUPING_PASSES.md), the chapter for the bead that measured it |
| Which heuristic pass claims a file, and what happens when two claim it? | [`GROUPING_PASSES.md`](GROUPING_PASSES.md) § What the passes are, § Tie-break, and § The pass set now for the set as it stands |
| What does one refine call cost, and how long does it take? | [`GROUPING_PASSES.md`](GROUPING_PASSES.md) § Cutting the price (`gd-26r.24`) |
| Is the heuristic group order any better than chance? | [`GROUPING_PASSES.md`](GROUPING_PASSES.md) § Scoring order (`gd-26r.22`) |
| Which config keys control grouping, and what are the defaults? | [`CONFIG.md`](CONFIG.md) § Grouping |
| Which module holds what, and where does the refine call sit in the startup path? | `AGENTS.md` § Project Structure, § Key Types, § Data Flow |
| What is in a fixture, and how do I run the scoring harness? | `tests/fixtures/grouping/README.md` |
| What key or command does the reader press? | `README.md` and `src/ui/help_popup.rs` |

## The docs, one entry each

### [`GROUPING.md`](GROUPING.md)

**Rules.** No single bead; rules 12 to 14 were added by `gd-26r.22`.

Owns the 14 numbered rules a grouping aims at, the within-group file order, the
testing-order rule, and the ambiguous-assignment annotation (chosen group,
runner-up, one-line reason).

Does not own data shapes, anything the sidebar renders, group sizing, or how
any of it is measured.

### [`GROUPS_CONTRACT.md`](GROUPS_CONTRACT.md)

**Contract.** `gd-26r.10`.

Owns what the engine hands tuicr: the `Grouping`, `Group`, `Assignment` and
`GroupSource` types, the refine request and response envelope, the rejoin rule,
the repair table, the one-retry-then-fall-back path, and the ruling that
`contract-approval` does not apply to the Vertex boundary.

Does not own why a file belongs in a group, how a group is drawn, when refine
runs, or the numbers behind any of it.

### [`TOTAL_COVERAGE.md`](TOTAL_COVERAGE.md)

**Decision record.** `gd-26r.12`.

Owns the partition being total over real files: the commit-message pseudo-file
outside the partition at `diff_files` index 0, leftovers staying as
per-directory `dir:` groups, and the heuristic arm emitting a reading order.

Does not own the strict partition itself (assumed from `gd-26r.5`), the
response's obligations, or what a `dir:` group looks like on screen.

### [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md)

**Decision record.** `gd-26r.7`, revised by `gd-26r.31`, `gd-26r.35`,
`gd-26r.15` and `gd-26r.23`.

Owns everything the reader sees: group rows, no directory rows inside a group,
tree modes as an ungrouped-only feature, row costs, the header chip and its
ranking, the marker slot, the definition of the drift number, and what
`build_visible_items` must hold.

Does not own what triggers a regroup, what the backstop does with the drift
number, or the caps' own constants and enforcement.

### [`REGROUPING_STATE.md`](REGROUPING_STATE.md)

**Decision record.** `gd-26r.8`, extended by `gd-26r.36`.

Owns review state when the grouping changes: what is persisted, incremental
assignment as the only automatic change, the trigger table for what regroups
and what refines, review and UI state across a regroup, and the auto-regroup
backstop.

Does not own the decision to block startup, the definition of the drift number
the backstop reads, or the sidebar shape a landing resets to.

### [`MID_SESSION_REGROUP.md`](MID_SESSION_REGROUP.md)

**Decision record.** `gd-26r.14`, upheld on narrower grounds by `gd-26r.27`.

Owns applying a result under the reader: refine blocking the whole TUI at
startup, `:regroup` staying async, the cancel key and timeout, and what a
landing does to the cursor and to expanded hunk gaps.

Does not own the config keys that switch any of this on, the trigger table, or
the code path the startup wait runs through.

### [`GROUPING_PASSES.md`](GROUPING_PASSES.md)

**Findings log.** One `#`-level chapter per bead.

Owns every measured number in the feature: F1, precision, recall, Kendall tau,
cost, latency, row counts as measured, and model comparisons. Its own
navigation note at the top of the file says which chapters are superseded by
later ones; read that before trusting a chapter above the one you found.

Does not own any decision. A decision that a measurement supports lives in the
design doc for its bead, and this log is what that doc cites.

Ten chapters, in file order. Nothing below the note at the top of the file lists
them, so find your bead here:

| Chapter | Bead |
| --- | --- |
| Heuristic grouping passes: findings, fixture 1 | none |
| The second fixture: does the pass set transfer? | `gd-26r.20` |
| Fixing the defects | `gd-26r.21` |
| The model refine pass | `gd-26r.11` |
| Cutting the price | `gd-26r.24` |
| Scoring order | `gd-26r.22` |
| The within-group sort, and the blocking ruling | `gd-26r.27` |
| Guarding the order numbers | `gd-26r.29` |
| Tokenising the group name | `gd-o7s` |
| The size caps sit after every number above | `gd-26r.23`, built in `gd-26r.38` |

### Facts that live outside these seven

- `AGENTS.md` is the code-level view for someone editing `src/`: modules, key
  types, and the startup data flow.
- [`CONFIG.md`](CONFIG.md) § Grouping owns every `[grouping]` config key, its
  default, and the user-facing behaviour it buys.
- `tests/fixtures/grouping/README.md` owns the fixtures and the scoring
  harness.
- `README.md` and `src/ui/help_popup.rs` own keybindings and commands.

## Facts with more than one narrator

These are told in several places on purpose, for different audiences. One place
is the record; the rest restate it and must be corrected from it.

**Refine gating and blocking startup**, split by facet:

| Facet | Owner |
| --- | --- |
| The decision to block at startup, and why `:regroup` does not | [`MID_SESSION_REGROUP.md`](MID_SESSION_REGROUP.md) |
| Which events trigger a refine, and which only regroup | [`REGROUPING_STATE.md`](REGROUPING_STATE.md) § How a human forces a regroup |
| The config keys, their defaults, and what a reader sees | [`CONFIG.md`](CONFIG.md) § Grouping |
| Where the call sits in the startup path, and which functions run it | `AGENTS.md` § Data Flow |

**The size caps** (`gd-26r.23`, built in `gd-26r.38`). The evidence is
[`GROUPING_PASSES.md`](GROUPING_PASSES.md)'s last chapter, and the constants as
shipped are `AGENTS.md`'s `grouping::caps` entry, which is what a reader
changing the code should trust. `GROUPS_CONTRACT.md` restates them to say a
large group is not a contract violation, `SIDEBAR_MODEL.md` to explain the `!`
marker, and `REGROUPING_STATE.md` to say the cap does not run incrementally.

**The header chip and the row markers.** The sharpest overlap in the set:
changing a chip string or a glyph means editing three files.
[`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § The header suffix and § The marker slot
on group rows own the strings, the glyphs and the ranking. `AGENTS.md` restates
the ranking in the variant names `App::grouping_status()` returns, and
`README.md` restates the strings and both glyphs in user prose. Change the owner
first, then both restatements.

**The drift number.** [`SIDEBAR_MODEL.md`](SIDEBAR_MODEL.md) § The drift number
defines it. [`REGROUPING_STATE.md`](REGROUPING_STATE.md) owns the backstop that
reads it.

**What a landing does to the sidebar** is one rule argued twice, and both
arguments stand. [`MID_SESSION_REGROUP.md`](MID_SESSION_REGROUP.md) § What a
landing does reaches it from the hazard, that the current file would otherwise
have no visible row; [`REGROUPING_STATE.md`](REGROUPING_STATE.md) § What happens
to UI state across a regroup reaches it from the collapsed overview being what
the reader asked for. Neither is a restatement of the other, so a change to the
behaviour has to answer both.

**The strict partition** (`gd-26r.5`). Stated in
[`GROUPING.md`](GROUPING.md)'s preamble and worked out in
[`TOTAL_COVERAGE.md`](TOTAL_COVERAGE.md). Every other doc assumes it and says
so in its own preamble.

## Where the docs lag the record

Nothing below has been rewritten in the docs it describes. Where a passage
contradicts a closed bead, both sides stand and the bead is the record.

**Human corrections were rejected, and the docs still anticipate them.**
`gd-26r.18` closed on 2026-08-19 rejecting its own premise: nothing in the
reader's hands edits the partition, so there is no move-to-group, no merge, no
split, and no accept or reject on a flagged ambiguous call. What ships instead
is a grouping feedback mechanism, under the open build tickets `gd-26r.41`,
`gd-26r.42` and `gd-26r.43`. Five of the seven docs were written before that
close and still read as though a corrections UI is coming:

- `GROUPING.md:135-139`, `:146`
- `GROUPS_CONTRACT.md:134-137`, `:143-144`, `:328-355`, `:357-359`, `:370`
- `SIDEBAR_MODEL.md:396-407`, `:622-623`, `:652`
- `REGROUPING_STATE.md:78-79`, `:84`, `:273-275`
- `TOTAL_COVERAGE.md:152-154`, `:231-232`

Read `bd show gd-26r.18` for the close reason.

**An over-cap group grown by joins is never re-split.** `gd-26r.39` closed on
2026-08-19 accepting this as a known limitation. It corrects one clause of
`gd-26r.23`, which said drift accumulates until the backstop's full pass
re-splits the group. That holds for an arrival that mints a new group and fails
for one that joins an established group, because a join moves neither the drift
number nor any marker. `REGROUPING_STATE.md:115-121` still carries the
uncorrected clause. `gd-26r.40` is the open ticket that will add the missing
paragraph there and in `SIDEBAR_MODEL.md`'s marker-slot section.

**Refine no longer shells out to an agent CLI.** `gd-26r.13` moved it to a
direct Vertex AI call over HTTPS, recorded in `GROUPS_CONTRACT.md`'s opening
correction. `REGROUPING_STATE.md:14-16` and `MID_SESSION_REGROUP.md:113`,
`:257` still describe an agent CLI, and `MID_SESSION_REGROUP.md:235-238` still
says contract approval covers the refine exchange, which
`GROUPS_CONTRACT.md`'s ruling reverses. Trust the contract.

**Open build work.** `gd-26r.40`, `gd-26r.41`, `gd-26r.42` and `gd-26r.43` are
open. When they ship, `gd-26r.40`'s facts land in `REGROUPING_STATE.md` and
`SIDEBAR_MODEL.md`, and the feedback mechanism's land in `SIDEBAR_MODEL.md` for
anything on screen, `CONFIG.md` for anything configurable, and
`REGROUPING_STATE.md` for anything persisted. Nothing about their behaviour is
described anywhere in these docs yet, and nothing should be until it ships.
