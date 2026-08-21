# The grouping feedback log

**Decision record and schema.** `gd-26r.43`, building on `gd-26r.18`.

The reader marks a wrong group or file as they read (`gd-26r.41`) and answers
one question when they leave (`gd-26r.42`). This document owns what that
answer becomes on disk: where the file is, what one line means field by field,
and what a reader of the file may and may not conclude from it.

The partition itself is assumed here. `GROUPING.md` says what a good group is,
`GROUPS_CONTRACT.md` says what the engine hands the sidebar, and
`GROUPING_INDEX.md` routes any other question.

## Where it lives

One file, at `<data dir>/grouping-feedback.jsonl`:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/tuicr/grouping-feedback.jsonl` |
| Linux | `~/.local/share/tuicr/grouping-feedback.jsonl` |

A sibling of the `reviews/` directory sessions live in, and never a file in a
repo working tree: a file appearing in your tree because someone reviewed your
PR is a surprise. `src/persistence/feedback_log.rs` owns the path and is the
only writer.

The log outlives everything around it. `:q!` on a review that left no comments
deletes the session file on the way out, and the vote cast a keystroke earlier
is still in the log.

## Three rulings that shape it

From `gd-26r.18`, and none of them this document's to revisit.

**tuicr never reads this file back.** The loop it feeds is offline and
human-driven: someone reads the log, concludes something about the prompt or
the passes, and changes the source. Feeding accumulated votes into a refine
call would invalidate `gd-26r.32`'s byte-for-byte prompt parity bar, which is
the only thing keeping two runs of the same changeset comparable.

**No CLI surface.** `review grouping-feedback` was offered and declined. An
agent doing the offline pass opens the path above.

**A skip writes nothing.** Not a line, not an empty file. An unanswered prompt
is not data, and the file's absence is how you tell a build where nobody voted
from one where somebody voted `useless`.

## Append-only, and what that costs a reader

Every submitted vote appends one line. Nothing is deduplicated, rewritten or
compacted. Voting, then reopening the review a week later and voting again,
leaves two lines about one review, and if a `:regroup` happened between them
they describe genuinely different partitions. Join on
`review.session_id`, order by `recorded_at`, and decide which one you want.

**Skip a line you cannot parse.** A torn line is not supposed to be reachable
(the append takes a lockfile on a sibling and `fsync`s), but a reader that
aborts the whole pass on one bad line turns a corrupted byte into a lost year
of votes. Parse per line, count the failures, carry on.

## The known bias: there is no denominator

The log records votes, and only votes. It does not record the reviews where
the prompt fired and the reader pressed Esc, nor the reviews the reader quit
out of before the prompt fired at all. So the file supports "of the people who
answered, this many said `useless`" and supports nothing of the form "n% of
reviews were badly grouped". Anyone who divides by the number of lines in this
file is dividing by the number of people willing to answer a prompt, which
correlates with having something to complain about.

`review.landing_count` is the nearest thing to a within-entry control: a
non-zero value says the partition in this entry is not the only one the reader
saw before voting.

## The schema

One JSON object per line, compact. `schema_version` is `1`.

```json
{"schema_version":1,"tuicr_version":"0.22.0","recorded_at":"2026-08-20T18:04:11.229Z","review":{...},"verdict":"mixed","tags":["junk-drawer group"],"note":"the auth group is a dumping ground","marks":{...},"arm":{...},"grouping_enabled":true,"has_commit_message":false,"groups":[...],"files":[...],"unpartitioned":[]}
```

### Top level

| Field | Meaning |
| --- | --- |
| `schema_version` | The shape below. Bumped when a reader has to know. |
| `tuicr_version` | The build that wrote the line, and the most useful field here: it pins the refine prompt, the pass names, the tag list and the size caps, none of which are versioned in themselves. |
| `recorded_at` | RFC 3339, UTC, at submit. |
| `review` | How to find the review again. See below. |
| `verdict` | `useful`, `mixed` or `useless`. Three points on purpose: a binary turns "mostly fine, one junk-drawer group" into noise, and five invites false precision on a two-second judgement. |
| `tags` | A subset of the fixed seven, in the list's constant order rather than click order. The list is a source constant (`GROUPING_FEEDBACK_TAGS`), never a config key, so votes stay aggregatable across a year. |
| `note` | The reader's free text, trimmed, or `null`. |
| `marks` | What the reader flagged while reading. See below. |
| `arm` | Which arm produced this partition, and under what config. See below. |
| `grouping_enabled` | Whether `<leader>g` was on when the vote was cast. `false` means the reader was looking at a directory listing and voting on a partition they were not reading. |
| `has_commit_message` | Whether the review carried the commit-message pseudo-file, which sits outside the partition (`TOTAL_COVERAGE.md` Decision 1) and so appears in neither `files` nor `unpartitioned`. Without this the file count disagrees with the row count the reader saw. |
| `groups` | The partition's groups, in reading order. |
| `files` | Every placed file, in reading order. |
| `unpartitioned` | Changeset files the partition does not place. |

### `review`

| Field | Meaning |
| --- | --- |
| `session_id` | The session's own id. Two entries sharing it are two votes on one review. |
| `session_version` | The session file's schema version. |
| `slug` | The slug sessions are keyed by, or `null`. Deriving a local slug reads the repo's `origin` remote, and a vote must never fail because that did not work. |
| `repo_path` | Canonicalized where the filesystem allowed it. |
| `branch`, `base_commit`, `commit_range`, `diff_source` | The changeset's coordinates, straight off the session. |
| `pr` | The PR identity, for a review where `repo_path` is only a local checkout and says nothing. `null` for a local review. |
| `landing_count` | Groupings landed since the review was opened: `:regroup`, a refine answer, the `gd-26r.36` backstop. A fresh review whose startup refine landed reads `0`, because that wait is not a landing. |

### `marks`

`marks.groups[]` carries `id` and `name`; `marks.files[]` is a path array. Both
are snapshots taken when the prompt opened.

The block stays separate from `groups` and `files` rather than folding a
`marked` flag into them, because a mark is the human's input and outlives what
it points at. A group mark's `name` is `null` when a landing dropped the group
between the mark and the vote, and a marked path need not still be in the
changeset at all.

### `arm`

| Field | Meaning |
| --- | --- |
| `config_now.refine_enabled`, `.model`, `.timeout_ms` | `[grouping]`'s refine settings **as they were at vote time**, which is not necessarily what produced the partition. Reopening a refined review with `refine = false` reads `refine_enabled: false` beside a `groups` array full of `refined`. That is not a contradiction. |
| `attempt` | The refine call whose result is this partition, or `null`. |
| `refine_in_flight` | A `:regroup` refine call was still out when the vote was cast, so the partition below is about to be replaced by one this entry does not describe. |

`attempt` is `null` for two reasons, and `groups[].source` tells them apart.
Beside `heuristics` groups it means refine never ran. Beside `refined` groups
it means the grouping was restored from an earlier sitting: the attempt is
in-memory state the session file deliberately never keeps, so reopening a
review loses it.

A refine call that was made and did not land is **not** `null` here. It is a
present `attempt` with a non-`landed` outcome, sitting beside a `groups` array
of `heuristics`.

When `attempt` is present:

| Field | Meaning |
| --- | --- |
| `model` | The model the call asked for, resolved environment then config then default. What it asked for, not necessarily what answered. |
| `effort` | Reasoning effort. Pinned in source with no config knob (`gd-26r.24`). |
| `prompt_template_fingerprint` | FNV-1a over the prompt's invariant text alone: the instructions, the rules and the size guidance, with the changeset's rendering excluded. **This is the field to group by** when you are asking which prompt was in force, which is the thing the offline loop revises. |
| `prompt_fingerprint` | FNV-1a over the exact bytes sent. Every changeset path is in there, so this identifies one call and never a prompt. |
| `prompt_bytes` | Size of those bytes. |
| `attempts` | `1`, or `2` when the first answer was unparseable and the identical prompt was resent. A call that needed the retry is a materially worse call. |
| `outcome` | `landed` (with `repairs`, the count of contract violations repaired into the model's partition), `cancelled`, `timed_out`, or `failed` (with `reason`). |

Every outcome but `landed` means the partition being voted on is the heuristic
fallback. Drawing that distinction is most of why the log exists: one session
mixes `heuristics` and `refined` groups (`gd-26r.32`), so "the grouping was
bad" cannot otherwise say whether the complaint is about the model or about the
fallback.

The outcome is recorded when the grouping it produced is **installed**, not
when the call returns. A parked answer can be dropped before adoption, and an
outcome recorded at return time would claim a partition the reader never saw.

### `groups`

| Field | Meaning |
| --- | --- |
| `id` | Joins to `files[].group` and `marks.groups[].id`. Minted fresh on every landing, so it is meaningless across entries. |
| `name` | What the sidebar showed. |
| `order` | Position in the reading order, which is also this array's order. Recorded explicitly so a tool that re-sorts the array cannot lose it. |
| `source` | `heuristics`, `refined` or `incremental`. |
| `unbounded` | Over the size cap and deliberately left whole (`gd-26r.38`). The cap's value is not in the payload; `tuicr_version` pins it. |
| `new_since_full_pass` | Minted by incremental assignment since the last full pass. |

**This array is authoritative for group existence and reading order.** A group
holding no file in this changeset is legal, since a narrowed commit range keeps
the group its files return to (`ReviewSession::grouping_for`), so
reconstructing the partition by grouping `files` alone silently drops it and
renumbers everything after it.

### `files`

In reading order: the groups in turn, and inside each group the within-group
sort (`GROUPING.md` § Within-group file order). Membership is this array
grouped by `group`, which cannot contradict `groups` because both come from one
`Grouping`.

| Field | Meaning |
| --- | --- |
| `path` | The diff's display path: the new path of a rename, the old path of a deletion. A deletion's path therefore names a file that no longer exists. |
| `status` | `added`, `modified`, `deleted`, `renamed`, `copied`, or `null` when the grouping placed a path the changeset no longer carries. |
| `renamed_from` | The source of a rename or a copy. The engine sees a copy as a rename that left its source behind, so both fill this. |
| `group` | Joins to `groups[].id`. |
| `pass` | The heuristic pass that claimed the file, or `restored` for one the heuristics never placed: a refined assignment, or a rehydrated session. This is derived debug state the session never keeps, free at vote time because the grouping is in memory, and it is what says **why** a file landed where it did. |
| `runner_up` | The group that nearly claimed it instead, and the one-line reason (`GROUPING.md` § Ambiguous assignments). The `group` here is a *name*, not an id: it is what the engine recorded when it made the call, and the losing group need not survive into the final partition. |

### `unpartitioned`

`path` and `status`, for changeset files the partition does not place. Normally
empty, because the partition is total by construction (`TOTAL_COVERAGE.md`). A
non-empty array means the grouping had drifted behind the changeset when the
vote was cast, and it is the same drift a `null` `status` in `files` reports
from the other side.

## What this document does not own

Where the marks come from and what a landing does to them
(`REGROUPING_STATE.md`), what the prompt looks like and which keys it takes
(`SIDEBAR_MODEL.md` § Revision (`gd-26r.42`)), when it fires and how to turn it
off (`CONFIG.md` § Grouping), and what the partition being voted on means at
all (`GROUPING.md`, `GROUPS_CONTRACT.md`).
