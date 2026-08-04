# Grouping fixtures

Test input for the grouping engine: a real changeset, plus the grouping it is
expected to produce, plus how closeness to that grouping is scored.

Guidelines the expected groupings follow: `docs/GROUPING.md`.

## Status: calibration-grade, not a gold standard

This fixture is **provisional**. The expected grouping was derived from
filename tokens and then reviewed by a human who does not know the source
codebase well. It is not authored ground truth.

That matters for how scores are read. A grouping engine tuned against this
fixture is partly being graded against the same kind of reasoning that produced
it, so a high score is weak evidence. Use it to catch gross regressions and to
compare passes against each other — not to declare a pass good.

Real ground truth is meant to arrive later, harvested from corrections a human
makes during actual reviews of repos they know. See the follow-up ticket.

## Contents

| file | what |
| --- | --- |
| `orca-971b16754.files` | the changeset: `<status>\t<path>`, one line per file |
| `orca-971b16754.groups` | expected grouping: `[group]` headers, indented paths |

### Provenance

- Source: `stablyai/orca`, commit `971b16754`
- Title: `fix(github): load PR diffs for Enterprise remotes (#8932)`
- 161 files, 9932 insertions, 1957 deletions
- Regenerate the input (requires access to that repo):
  ```
  git show --name-status --format= 971b16754 | sort -k2 | awk '{printf "%s\t%s\n",$1,$2}'
  ```

### Paths only, no content

The fixture records file paths, change status and nothing else. The source
repository is private, so diff bodies cannot be checked in here.

The consequence is a real limit, not a formality: **any heuristic that reads
diff content — and any model pass that reads hunks — cannot be scored against
this fixture.** Only path shape, filename tokens, change kind, and rename
pairing are testable. A grouping approach that needs content needs a different
fixture.

## Scoring

Two groupings of the same file set are compared by **pairwise co-membership**.
For every unordered pair of files, each grouping either puts them together or
does not:

- true positive — both groupings co-group the pair
- false positive — the computed grouping co-groups a pair the expected one splits
- false negative — the expected grouping co-groups a pair the computed one splits

Then precision, recall and their harmonic mean, F1. Group *names* are ignored,
so a pass that finds the right partition under different names scores full
marks; only the partition is judged.

Pairwise F1 was chosen because it degrades in the two directions that actually
matter and tells them apart: over-merging shows up as low precision,
over-splitting as low recall. A metric that collapsed both into one number
would hide which mistake a pass is making.

### Report the baselines alongside the score

A pairwise F1 in isolation is easy to misread, because degenerate partitions
can score deceptively well on a changeset with one dominant concern. Always
report, on the same fixture:

- **all-one-group** — every file in a single group. Recall 1.0, precision poor.
- **one-file-per-group** — no pair co-grouped. Precision undefined, recall 0.
- **directory** — group by top-level directory. The cheap heuristic any real
  pass must beat to justify itself.

A pass that does not clearly beat all three is not doing useful work, whatever
its absolute F1.

### Report the shape too

Alongside the score, report group count and largest-group share. They catch the
failure a single number hides: a partition can score respectably while putting
most of the changeset in one bucket, which is the problem grouping exists to
solve. On this fixture the expected grouping is 13 groups with a largest share
of 24%.
