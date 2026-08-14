# The groups contract

What the grouping engine hands tuicr. Two shapes, one of which is not a
contract at all any more.

## Correction to the ticket's own premise, twice over

`gd-26r.10` was written when the engine might be a separate binary, and its
NOTES were rewritten once when it turned out not to be. Both framings are now
dead:

1. **`gd-26r.4`**: the engine lives *in* tuicr. There is no cross-tool groups
   file, so nothing on that axis to approve.
2. **`gd-26r.13`**: the refine pass does not shell out to an agent CLI either.
   It calls Vertex AI directly over HTTP with a hand-rolled `authorized_user`
   ADC refresh on the `ureq` already in `Cargo.toml` — no subprocess, no agent
   SDK — because `gd-26r.24` measured the CLI at roughly 2x the cost and wall
   clock for F1 inside noise.

So the NOTES' claim that "only the refine request/response crosses a process
boundary, so contract-approval applies to it alone" rests on a boundary that no
longer exists in the form described. What remains is an HTTPS call to a vendor
API this repo does not own — not a tool boundary between two binaries we ship.

## Ruling — contract-approval does NOT apply

Stated plainly because the map's Notes said the opposite and are amended by
this ticket.

The `contract-approval` gate covers contracts **an external consumer depends
on**: service↔service, frontend↔BFF, event schemas, published APIs. Its purpose
is that such a contract is expensive to reverse once someone else has built
against it.

The Vertex boundary inverts every term of that:

- **We are the consumer, not the producer.** Google owns the Vertex contract.
  We cannot version it, approve it, or prevent it drifting.
- **What we author is a prompt and a hoped-for JSON shape**, both of which ship
  inside the same binary as their reader. Editing one edits the other.
- **Nothing downstream binds to it.** No deployable consumes our response
  envelope. It is read once, repaired, and turned into the in-process grouping.
- **Reversal is free.** The repair path below means a shape change degrades to
  heuristics rather than breaking a consumer.

No contract identity is derived, no record is written, no drift check applies.
The response shape is documented here instead, which is the useful half of the
gate without the ceremony.

---

## Shape 1 — the in-process engine interface

The engine's output type, consumed by the ordering and tree seam
(`sort_files_by_directory` + `build_visible_items`, one seam with two call
sites per `gd-26r.6`; hooks at `init.rs:569` and `diff_load.rs:584`).

### The type

```rust
/// What a grouping pass produces: an ordered set of groups, and the placement
/// of every changeset path into exactly one of them.
pub struct Grouping {
    /// Reading order (GROUPING.md rule 2) IS the Vec position. There is no
    /// separate `order` field to fall out of sync with the slice.
    pub groups: Vec<Group>,
    pub assignments: Vec<Assignment>,
}

pub struct Group {
    /// Stable opaque identity (gd-26r.8). Not the name, never the name.
    pub id: GroupId,
    /// Display only, rendered verbatim (SIDEBAR_MODEL.md).
    pub name: String,
    /// What produced THIS group. Per-group, not global (gd-26r.12 Decision 4).
    pub source: GroupSource,
    /// Set when opened by incremental assignment (REGROUPING_STATE.md).
    pub new_since_full_pass: bool,
}

pub enum GroupSource { Heuristics, Refined, Incremental }

pub struct Assignment {
    pub path: PathBuf,
    pub group_id: GroupId,
    /// Which pass claimed it. Debug-only, never persisted.
    pub pass: &'static str,
    /// On close calls only (~14% of files). Debug-only, never persisted.
    pub runner_up: Option<RunnerUp>,
}
```

### Why this and not the prototype's shape

The prototype's `Grouping { assignments: Vec<Assignment> }`, with
`Assignment.group: String`, was the shape the harness started from. That is the honest starting point but wrong at
two points, both settled after it was written:

- **It is name-keyed.** `gd-26r.8` made identity a stable opaque `group_id`
  precisely because names are unstable *in kind* — the heuristic arm derives
  mechanical token joins, the refine arm returns readable concern names, and a
  refine run may rename a group it otherwise leaves intact. Name as key makes
  every rename indistinguishable from every file in the group moving.
- **It carries no order.** `gd-26r.12` Decision 3 made the heuristic arm emit
  one (size-descending, `dir:` groups pinned last), so `order` is a field both
  arms populate. Encoding it as `Vec` position rather than an integer field
  removes the class of bug where the two disagree.

### What the seam is owed

The seam consumes `groups` in order and, for each group, the files assigned to
it. That is exactly the contiguity `SIDEBAR_MODEL.md` requires: `diff_files`
contiguous **by group**. Handing the seam a flat per-file map would leave it to
rediscover the grouping by bucketing, with no order to bucket into — which is
how the prototype's shape fails here.

That requirement used to be two-level — by group, and within a group by
directory, with `seen_dirs` reset at every group boundary. `gd-26r.31` dropped
the second level with the in-group directory rows that needed it.

The engine does **not** produce `diff_files` order itself; it produces the
grouping, and the seam flattens it. Keeping the flatten on the seam side is
what lets the ungrouped tree modes (`nested` / `compact` / `flat`) vary without
the engine knowing.

### Provenance is two-tiered

- **`Group.source` is contract and is persisted.** `gd-26r.15` needs it to say
  *heuristics only, refine unavailable* per group rather than globally, and
  `gd-26r.18` needs somewhere to record a human correction as a first-class
  provenance value rather than an indistinguishable overwrite.
- **`Assignment.pass` and `Assignment.runner_up` are debug-only.** In-memory
  derived state, recomputed every regroup, exposed only through a debug surface
  and the fixture harness's `pass_hits`. Never persisted, never in the sidebar.
  This is `REGROUPING_STATE.md`'s existing ruling ("Not persisted: the runner-up
  group and one-line reason") and `SIDEBAR_MODEL.md`'s, restated rather than
  reopened. `gd-26r.18` may reverse the `runner_up` half if a correction UI
  gives a human something to act on.

---

## Shape 2 — the refine request and response envelope

Not a gated contract (above), but the shape still has to be written down
because tuicr's reader is the only thing enforcing it.

### The request

The prompt (`src/grouping/refine.rs::prompt`, which the harness in
`tests/grouping/refine.rs` sends verbatim) is the shipped shape: the rules,
the changeset as paths plus change status (`A`/`M`/`D`/`R`), and the heuristic
grouping rendered largest-group-first as `[name] (n files)` followed by its
paths. No diff bodies — the fixtures carry none and neither does this.

**Existing group assignments are shown as names, and ids are mapped outside the
prompt.** The model never sees a `GroupId`. tuicr keeps a `name → GroupId` map
alongside the request and resolves identity on the way back. Exposing ids to
the model would buy explicit rejoin at the cost of prompt noise and a fresh
hallucination surface — an invented id — for a problem the rejoin rule below
solves without it.

The exchange is **single-shot, bounded and cancellable** (`gd-26r.14`): one
request, one response, a timeout, and a way to abandon a call in flight. Not
streaming, not incremental, not conversational.

### The response

```json
{"groups": [{"name": "kebab-case-name", "files": ["path", ...]}, ...]}
```

- **Array position is reading order.** Required, not optional (`gd-26r.14`
  point 2): blocking startup exists precisely to obtain the order the heuristic
  arm could not produce. **Within-group file order is not carried at all**
  (`gd-26r.27`): the engine sorts each group's files itself, deterministically
  and identically on both arms, so `groups[].files` order is read as membership
  and discarded as order.
- **Only `groups[].name` and `groups[].files` are read.**
- The `merge` / `was` variants of the prototype's other shapes
  (`Shape::MergeOnly`, `Shape::NamingOnly`) are harness arms, not the shipped
  envelope. If one ships later it is an addition here, read the same way.

### Rejoining an existing group

A returned group inherits an existing `GroupId` by this rule, in order:

1. **Identical membership** — its file set exactly equals an existing group's
   file set. Inherits that id regardless of name.
2. **Matching name** — its name equals an existing group's name. Inherits that
   id.
3. **Otherwise a new id.**

Membership is checked first because the rename-intact case is the refine arm's
strongest measured move: `gd-26r.11` found naming-only refine cheap
($0.12–0.22), fast (20–40s) and perfectly stable, and plausibly the largest
user-visible benefit. Under name-only matching that exact case reads as every
file in the group moving to a new group, discarding the group-keyed
`expanded_groups` state `group_id` was created to preserve.

Dominant-membership matching (inherit from whichever group contributes the most
files, above a threshold) was rejected: it needs an uncalibrated constant, and
it lets a merged group silently inherit one parent's identity.

### Forward compatibility

**Read leniently, no version field.** Unknown keys — on a group or at the top
level — are ignored. There is no version number in the request or the response.

Both sides ship in the same binary and are edited together, so there is no skew
to detect; and a version number echoed by a language model is unenforceable,
which just makes it another repair case. The persisted session already has its
own versioning story via `#[serde(default)]` (`REGROUPING_STATE.md`).

---

## Repair and failure behaviour

The contract must define this rather than assume a well-behaved model:
`gd-26r.24` measured **gemini-3-flash inventing 0.2 to 0.5 paths per call**
against **zero for every Claude arm**. The shipped default is `claude-opus-5`
at low effort, but the model is a config knob, so the badly-behaved case is one
config edit away at all times.

### Repair — the prototype's behaviour, shipped as-is

`src/grouping/refine.rs::apply` is the contract, and the prototype harness in
`tests/grouping/refine.rs` calls the shipped rules rather than copying them, so
the recorded figures describe the pass the binary runs. Every violation is
repaired against the heuristic partition and **counted**; the repair count is
part of the verdict, not a detail.

| Violation | Repair |
| --- | --- |
| Path not in the changeset | Dropped, counted as `invented path dropped` |
| Path in two *different* groups | First group wins, second occurrence counted. The same path twice in one group is not a violation and is not counted |
| Path omitted entirely | Restored to its **existing heuristic group**, counted |
| Path in no heuristic group either | Filed under `unplaced`, counted. Unreachable while the heuristic partition is total over the same changeset; it is repaired rather than asserted because losing a file is the worse failure |
| Group with no `files` | Keeps its name and order slot; its paths fall to the restore loop |
| `files` entry that is not a string | Skipped, counted as a non-string entry; the path it was meant to be falls to the restore loop |
| Group with no/blank `name` | Filed as `unnamed-<index>`, counted |
| Name reused across two groups | Second uniquified via `free_name`, counted |

Two properties fall out and are worth stating:

- **The strict partition of `gd-26r.5` holds for any parseable body by
  construction.** The restore loop runs over every changeset path, so after
  `apply` the assignment map is keyed by every distinct path and nothing else.
  Total coverage is **tuicr's invariant, not the agent's promise** — the engine
  proposes, tuicr closes (`gd-26r.12` Decision 3).
- **A dropped path returns under its heuristic group's own name, not a
  uniquified one.** The prompt showed the model those names, so a returned group
  called `X` *is* heuristic group `X`; reusing the plain name also puts
  co-dropped paths back together, which per-path uniquification would split.

**The pseudo-file needs no special clause.** The commit-message row sits outside
the partition entirely (`gd-26r.12` Decision 1), so the request carries real
changeset paths only and a response naming `Commit Message (<sha>)` is just
another unknown path, dropped by the row above.

### No rejection threshold

**Any parseable body is applied.** There is no share-of-repairs threshold above
which a response is discarded.

`gd-26r.12` already ruled that discarding a paid-for 30–70s result over an
omitted path is the wrong trade, and the money is spent at dispatch. A threshold
would add a constant nobody has calibrated to guard against a failure mode
measured at 0.5 invented paths per call on the worst arm. Repairs are recorded,
so a misbehaving model surfaces as a startup warning naming the count rather
than as a silent fallback the human cannot explain.

### Parse failure — one retry, then fall back

A body that yields no JSON, or JSON with no `groups` array, gets **exactly one
retry**: the **identical prompt, resent**. No conversation, no error fed back —
two independent one-shot calls, so `gd-26r.14`'s single-shot ruling holds
literally. This recovers the common nondeterministic failures (a prose prefix, a
truncated body) without introducing a turn.

**The retry gets its own full timeout.** Accepted cost, stated rather than
buried: worst-case blocking startup doubles, and it doubles on the one path
where the human is watching a pre-TUI progress screen with nothing else to look
at. The alternative — sharing one budget — was rejected in favour of the retry
actually having a chance to complete.

`gd-26r.15` owns the consequence: the pre-TUI progress surface must make a
retry legible, or a doubled wait reads as a hang.

**Retry applies to parse failure only.** Everything else is terminal on the
first attempt.

### Every other failure is one outcome

Per `gd-26r.14` point 4, all of these collapse to the same result:

- HTTP error, auth/ADC failure, timeout, cancellation, retry-also-unparseable

The result: **keep the heuristic partition, `source` stays `Heuristics`, apply
nothing partial, surface the reason.** Application is atomic (`gd-26r.14`
point 3) — a full replacement partition, sidebar rebuilt in one shot — so there
is no half-applied state to unwind.

Falling back is a **first-class outcome, not an error path**, and cancellation
makes it one a human deliberately causes.

### Nothing is owed about positional stability

Gap state is discarded on every reorder (`gd-26r.14` point 5, and commit
`c0979af` which made `sort_files_by_directory` clear expanded gaps), so no part
of the response needs to preserve, describe or remap file positions.

---

## What this settles for `gd-26r.18` (human corrections)

Stated here, not absorbed — the correction UX is that ticket's to design.

1. **A correction has a place to live in the type, and it is `GroupSource`.**
   The enum ships as `Heuristics | Refined | Incremental`; `gd-26r.18` adds a
   human variant so a corrected group is distinguishable from a generated one
   rather than an indistinguishable overwrite. `source` is already per-group and
   already persisted, so this costs no new field.
2. **The rejoin rule is where a correction must survive a regroup.**
   Membership-then-name means a refine run that renames an intact group keeps
   its id — but a run that *moves files* into or out of a corrected group does
   not, and will take the correction with it. Deciding whether a corrected
   group is pinned against that is `gd-26r.18`'s, and this contract deliberately
   leaves the hook rather than choosing.
3. **`runner_up` is available but unpersisted.** The engine records the rejected
   alternative and a one-line reason on close calls (~14% of files) as
   in-memory debug state. If the correction UI wants to offer "did you mean
   X?", `gd-26r.18` is the ticket that flips it to persisted — that reversal is
   explicitly sanctioned here and by `REGROUPING_STATE.md`.
4. **Corrections are the only input that cannot be regenerated**, and
   `discard_session_and_quit` is the normal exit for a comment-free review
   (`GROUPING.md`). Nothing in this contract stores them yet; `gd-26r.18` must
   not put them anywhere that path destroys.
5. **The engine proposes, tuicr closes.** Every repair here writes into the same
   place a human correction will: the applied partition, after the response is
   read. A correction is not a different mechanism, it is the same one with a
   different source.

**Not settled here, and deliberately not absorbed:** the accepted cost
`gd-26r.12` recorded — a fixture-2-shaped changeset showing 29 rows with 10
one-file `dir:` groups. `gd-26r.18` owns that revisit.

## Deliberately not decided here

- ~~Within-group file order~~ — settled by `gd-26r.27`: the engine's sort owns
  it, the contract carries nothing about it.
- How the pre-TUI progress screen renders a retry (`gd-26r.15`).
- Group sizing and any soft/hard cap (`gd-26r.23`).
- Vertex credential types beyond `authorized_user` ADC (map fog).
- How a human reassigns a file (`gd-26r.18`).
