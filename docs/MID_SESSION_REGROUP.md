# Applying a grouping result under the reader

Part of the grouping design set. [`docs/GROUPING_INDEX.md`](GROUPING_INDEX.md)
maps a question to the doc that owns the answer.

Design record for `gd-26r.14` on the wayfinder map `gd-26r` (grouped review of
large changesets in tuicr). **Decision-only**: no code lands with this ticket.
What is described here ships with the grouping engine, because there is no
grouping engine in `src/` yet to attach it to — the passes live in
`tests/grouping/` as a scored prototype.

Assumed, settled elsewhere: grouping is file-level with a **strict partition**
(`gd-26r.5`); groups are collapsible top-level sidebar nodes with `expanded_groups`
keyed on the group id (`gd-26r.7`, revised by `gd-26r.31` and `gd-26r.35`,
`docs/SIDEBAR_MODEL.md`); grouping is
a **persisted session artefact** with a stable opaque `group_id`, reopening never
regroups and never refines, incremental assignment is heuristics-only and never
moves an unchanged file, and the auto backstop is heuristics-only so a threshold
crossing cannot silently spend money (`gd-26r.8`,
`docs/REGROUPING_STATE.md`).

## The measurement this ticket was waiting on

This ticket blocked itself once, on a latency figure. It is now measured
(`gd-26r.24`, `docs/GROUPING_PASSES.md`): the refine pass takes roughly
**30–70s on a ~160-file changeset**, with the shipped default at **36.6s on
fixture 1 and 60.0s on fixture 2**. That is a typical case, not a bound.

Two inherited numbers were wrong and are retired. `gd-26r.4`'s claim that the
refined grouping "lands seconds later" was never true, and the 190s figure the
earlier session designed against is superseded by the cost work. The policy
below is designed against 30–70s.

## The decision

**Refine blocks the whole TUI at startup. Nothing renders until the grouping is
final.** `:regroup` mid-review stays async. That is the whole shape; the rest
follows.

> **Reviewed and upheld by `gd-26r.27`, on narrower grounds.** This ruling was
> made before an order metric existed. `gd-26r.22` built one, found evidence
> against the conclusion, and handed it up rather than reopening this record.
> The review confirmed the premise — heuristic group order really is chance or
> worse, `tau_group` −0.192 and +0.080 — and upheld blocking, while striking one
> of the three supports below. The amendments are marked inline and argued in
> full in the `gd-26r.27` section of `docs/GROUPING_PASSES.md`.

### Why blocking wins at startup

The async model in `gd-26r.4` — open instantly on heuristic groups, swap when
refine lands — was justified by the belief that the swap happened in seconds.
At 30–70s it does not land while you are orienting; it lands while you are deep
inside a file, and it reorders the sidebar under you for a benefit you have
already stopped waiting for.

The decisive argument is not the interruption, though. It is that **the
heuristic arm cannot produce a meaningful group order**, so an instant-open on
heuristics does not merely show worse groups, it shows them in no meaningful
order:

- No group order. `Partition.groups` is a name-keyed `BTreeMap` and the
  heuristic arm's reading order is `gd-26r.12` Decision 3's size-descending
  proxy. Intent-centrality (`docs/GROUPING.md` rule 2) is a judgement about what
  the changeset is *for*, which is exactly what token heuristics are bad at.
  **`gd-26r.27`: measured, and this is confirmed** — `tau_group` −0.192 on
  fixture 1, which is worse than chance, and +0.080 on fixture 2, which is
  chance. This is now the *whole* of what blocking buys in ordering terms, and
  what it buys is +0.042 and +0.335 under a metric biased toward refine.
- ~~No within-group file order.~~ **Struck by `gd-26r.27`.** This described an
  unimplemented rule rather than a property of the heuristic arm: both arms
  built a group from a path-keyed map, so every test preceded the code it
  covered, 0 of 42 pairs right on fixture 1. A deterministic sort over paths the
  engine already holds fixes it on **both** arms with no model call, taking rule
  4 to +1.000 and `tau_within` to the fixture's own ceiling. It is no longer a
  reason to block, and it is no longer a defect of the fallback either.
- Partition quality also favours refine, unevenly: heuristics score **0.394 and
  0.378** F1 on the two fixtures against refine's **0.427 and 0.711**. Fixture 1
  is nearly a tie; fixture 2 is not close. Ordering, not partition quality, was
  the argument here — **but `gd-26r.27` found the order gain weaker than assumed
  and this one unchanged, so it is the partition gain that now carries the
  ruling.** It transfers across both fixtures; the order gain does not.

Opening instantly therefore buys a first screen you cannot trust to start with
the most relevant files — which is the entire point of the collapsed 13-row
overview (`docs/SIDEBAR_MODEL.md`). Waiting once, at the only moment in the
review when you have nothing to lose by waiting, buys the ordering that makes
the overview worth reading.

`gd-26r.27` weighed relaxing this against exactly that sentence, since the sort
now repairs within-group order for free. It does not relax: the collapsed
overview shows **group rows**, so the sort repairs what a reader sees *second*,
on expanding a group, while what they see first is still ordered at −0.192.
Substituting the cheaper `naming-only` shape for full refine at startup was also
priced and rejected — it reaches `tau_group` +0.302 on fixture 2 for \$0.12–0.22
and 20–40s, but cannot move the partition at all, so it saves 10–30 seconds of a
once-per-review wait by giving back the one advantage that transfers.

**The cost is bounded by `gd-26r.8`.** Reopening a session never refines, so the
wait is once per *review*, not once per open and never per `:reload`. A 30–70s
block on a review that will run for an hour or more is a fair trade; a 30–70s
block every morning would not have been.

### What blocking looks like

The TUI does not render. A pre-TUI progress screen shows that refine is running,
the file count, and elapsed time, and it names the cancel key. It must run a
minimal event read to do that — a blocking startup with no input loop cannot
honour the cancel below, and that is an implementation requirement of this
decision, not an optional nicety.

### The escape: cancel key and timeout

30–70s is a typical case, not a bound; a slow, rate-limited or hung agent CLI
can sit far longer. Two escapes, both landing in the same place:

- **Cancel key** during the wait abandons refine and opens immediately on the
  heuristic grouping.
- **A configured timeout** does the same automatically.

Both produce `gd-26r.15`'s *refine unavailable* state, with the groups marked
`source = heuristics`. **`gd-26r.27` improved what that state looks like**: the
heuristic grouping now carries the within-group sort, so a cancelled or timed-out
startup opens on groups that read production before test, mechanical files at
the tail, broad tests after unit tests. What the escape gives up is the group
order and the partition gain, and nothing else. The timeout default should be
generous — well beyond the
measured 70s — because the money is spent at dispatch, so a premature timeout
throws away a paid-for result and gets the unordered grouping anyway.

### The configuration surface

No new mode key. `[grouping].refine` is already the knob:

| setting | startup behaviour |
| --- | --- |
| `refine = true` | block on the refined grouping, cancellable, with a timeout |
| `refine = false` | open instantly on heuristics, no wait, no *group* order — but with the `gd-26r.27` within-group sort, which is free and unconditional |
| `--no-grouping` | no grouping at all (`gd-26r.4`) |

Plus one new key for the timeout. Adding it means adding it to `KNOWN_KEYS`
(`src/config/mod.rs:169-198`) or startup emits an unknown-key warning.

Blocking is acceptable **because** it is configurable: anyone who wants the
instant open sets `refine = false` and accepts the unordered grouping, and
anyone who wants neither has `--no-grouping`.

### `:regroup` stays async

The symmetry argument fails. Startup blocks because you have nothing else to do;
mid-review you demonstrably do — you are reading a file, and that reading does
not depend on the sidebar being re-sorted. Freezing the TUI for 30–70s in the
middle of a review would make `:regroup` a command you avoid.

So one async landing path survives, and it is a **requested** one. The
heuristics-only auto backstop is the other landing, and it is instant and free.
Both are governed by the two rules below.

## What a landing does

Two landings can move the sidebar under the reader: an async `:regroup` result,
and the auto-threshold backstop. They behave identically.

### Cursor and selection — `gd-26r.8` is unamended

**All groups collapse. The diff pane does not move. Sidebar selection lands on
the collapsed group row containing the current file.**

The exception considered and rejected was to expand the current file's group so
its row stays visible. It was rejected because the collapsed 13-row overview
*is* the thing you just asked for: after a regroup, the first useful screen is
the new shape of the changeset, not the row you were on. You have lost nothing
— the diff pane still holds your file at your cursor line, and one keypress
opens the group back to it.

This resolves, rather than dodges, the hazard the earlier session recorded:
`jump_to_file` (`src/app/navigation.rs`) inserted only *path* ancestors
into `expanded_dirs` when this was written, so it did not expand a group node
— `gd-26r.28` has since routed it through `reveal_file`, which inserts the
group key while grouping is on — and the current file
would otherwise have **no visible sidebar row at all** after a landing. The
answer is not to teach `jump_to_file` about groups for this path. It is that
selection after a landing targets the **group row**, not the file row — which
`docs/SIDEBAR_MODEL.md` already requires of `ensure_valid_tree_selection`
(`src/app/tree.rs`), which must fall back to the group node rather than
`select(0)`.

Cursor preservation here therefore means *the diff pane is undisturbed*, not
*your file stays visible in the sidebar*. The `:reload` precedent
(`diff_load.rs:588-603`, the only `reset_position = false` sort) still applies
for re-finding the file by `display_path()`; what differs is that it must not
then call `expand_all_dirs`.

### Expanded gaps — always cleared

**Every manually expanded hunk gap is discarded on any reorder, on both the
requested `:regroup` path and the unrequested backstop.**

`expanded_top` / `expanded_bottom` are keyed by `GapId { file_idx, hunk_idx }`
(`mod.rs:131-136,1235-1237`) — a *position* into `diff_files`, not a file
identity — so a reorder silently reattaches expanded context to the wrong file.
This already ships as the fix: `gd-26r.26`, commit `c0979af`, folded
`clear_expanded_gaps` into `sort_files_by_directory` with a regression test, so
clearing is free and no future reordering caller can forget it.

Both preserving alternatives were rejected as machinery bought for a rare
moment. Re-keying every `GapId` onto path would make the identity model
permanently more complex; re-keying only the current file's gaps would need the
same re-key path for a single file. Neither is worth it when the diff pane
itself is undisturbed and re-expanding is one keypress.

## Items already settled, carried forward

**Item 1 — cache invalidation — is closed, not open.** Every persistent `App`
field was swept for positional keying rather than trusting seam map §2, and
`expanded_top` / `expanded_bottom` are the only persistent caches keyed by a
position into `diff_files` that survive a sort. `file_line_count_cache` was
already cleared; `line_annotations` and `diff_row_to_annotation` are rebuilt
every time; `commit_diff_cache` and `saved_inline_selection` are keyed by
*commit* indices; `current_file_idx` and `file_list_state` are re-found or
rebuilt; search is order-free. The seam map's list was complete. Shipped as
`gd-26r.26`, commit `c0979af` on `groupdiff`.

Re-keying `GapId` on file identity was considered there and rejected as dead
weight: `gd-26r.5` chose a file-level strict partition, so a file slot does not
change meaning and nothing would exercise the re-key. This decision reaffirms
that rejection from the experience side as well.

**Item 5 — `stage_reviewed` — is already correct and needs only a note.**
`reviewed.rs:36-42` hard-resets `diff_files` / `diff_state` /
`file_list_state` and already calls `clear_expanded_gaps` explicitly on
`NoChanges`. The note for whoever builds the engine: **any future grouping cache
must be cleared there too.** It is a wholesale reset of the file list, so
anything derived from file positions or group membership belongs in it.

## What this settles for `gd-26r.10` (groups contract shape)

Stated here rather than absorbed. Only the refine request/response crosses a
process boundary, so only it is in scope for contract approval.

1. **The exchange is single-shot, bounded and cancellable.** Blocking startup
   means the contract needs a request, one response, a timeout, and a way to
   abandon a call in flight — not a streaming or incremental protocol. A
   progress signal is optional and belongs to `gd-26r.15`, not to the data
   contract.
2. **Reading order is a required field of the response, not an optional one.**
   Blocking startup exists precisely to obtain the order that the heuristic arm
   cannot produce. A refine response without a group order does not satisfy the
   contract; the group order the response carries is
   load-bearing. **Within-group file order is *not* carried, settled by
   `gd-26r.27`**: the engine sorts each group deterministically on both arms, so
   the response owes nothing about it and the prompt does not mention it. This
   narrows the contract rather than widening it.
3. **Application is atomic — a full replacement partition.** The sidebar is
   rebuilt in one shot, groups collapsed. There is no partial or progressive
   application, so the response is never consumed in pieces.
4. **Falling back is a first-class outcome, and it is reachable by the human.**
   Timed out, cancelled, unparseable, CLI absent: all produce the same result —
   keep the heuristic partition, mark `source = heuristics`, apply nothing
   partial. Cancellation makes this an outcome a human *causes*, not only an
   environment fact.
5. **The contract owes nothing about positional stability.** Gap state is
   discarded on every reorder, so no part of the response needs to preserve,
   describe or remap file positions.
6. **Unchanged from `gd-26r.8`**: membership is by path under a strict
   partition, group identity is a stable opaque `group_id` rather than a name,
   and the request must expose existing group assignments so the engine can join
   an existing group rather than only proposing fresh ones.

## What this constrains in `gd-26r.15` (status indicator)

Stated here, not absorbed — the indicator is that ticket's to design.

- **"Refining in flight" has two surfaces, not one.** At startup it is a
  *pre-TUI* progress screen with elapsed time and a cancel hint, because no TUI
  is rendered yet. Mid-review, for async `:regroup`, it is an in-TUI indicator.
  The ticket's assumption of one slot for three states does not survive
  blocking startup.
- **"Refine unavailable" is now human-causable.** Cancelling the startup wait
  produces the same state as a missing CLI, and the indicator should not imply
  the environment failed when the human simply chose not to wait.
- **Staleness is unaffected** by anything decided here.
