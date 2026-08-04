# Heuristic grouping passes: findings

What the grouping engine runs, in what order, and what happens when two passes
claim the same file. The rules being aimed at are `docs/GROUPING.md`; the
fixture and scoring method are `tests/fixtures/grouping/README.md`.

Everything below is measured against **one** changeset (orca `971b16754`, 161
files) by `tests/grouping_prototype.rs`:

```
cargo test --test grouping_prototype -- --nocapture report
```

That fixture is provisional and paths-only, so these numbers rank passes
against each other. They do not certify any pass as good.

## Headline numbers

| grouping | F1 | precision | recall | groups | largest |
| --- | --- | --- | --- | --- | --- |
| all-one-group | 0.241 | 0.137 | 1.000 | 1 | 100.0% |
| one-file-per-group | 0.000 | 0.000 | 0.000 | 161 | 0.6% |
| top-level-directory | 0.255 | 0.150 | 0.829 | 2 | 85.7% |
| parent-directory | 0.219 | 0.285 | 0.178 | 26 | 23.0% |
| **heuristic passes** | **0.391** | **0.552** | **0.303** | **25** | **18.0%** |
| expected grouping (self-score) | 1.000 | 1.000 | 1.000 | 13 | 23.6% |

The passes beat every baseline by roughly 0.14 F1, and they beat them in the
direction that matters: precision 0.55 against the best baseline's 0.29, with a
largest-group share of 18% rather than 86%.

The do-nothing worry from the first hand partition is gone. Against the current
fixture, all-one-group scores 0.241, not the 0.83 it scored when 133 of 161
files sat in one group. Re-cutting the fixture into 13 groups is what made the
metric able to tell passes apart at all.

### Ablations

| variant | F1 | precision | recall | groups |
| --- | --- | --- | --- | --- |
| default | 0.391 | 0.552 | 0.303 | 25 |
| no ubiquity stoplist | 0.255 | 0.262 | 0.248 | 21 |
| tie-break: cluster keeps the test | 0.388 | 0.524 | 0.308 | 25 |
| directory tokens as evidence | 0.394 | 0.479 | 0.334 | 19 |
| no test-burst boost (rule 7) | 0.391 | 0.552 | 0.303 | 25 |
| no leftover absorption | 0.393 | 0.601 | 0.292 | 32 |
| agglomerative clustering, 16 groups | 0.310 | 0.277 | 0.350 | 16 |

## What the passes are

Run in this order. Order **is** priority: an earlier pass keeps a file a later
one also wants, with two deliberate exceptions noted under [Tie-break](#tie-break).

1. **mechanical** — lockfiles, vendored trees, generated output (rule 8).
2. **config-ci** — `.github/`, root build config (rule 8's neighbour).
3. **whole-change docs** — a `docs/` or root `.md` file (rule 9). A doc beside
   its code is left to the cluster pass, which is the rest of rule 9.
4. **token-cluster** — the load-bearing pass. Described below.
5. **test-pairing** — a test joins its production file's group (rule 4).
6. **absorb-leftovers** — a file no cluster claimed joins its most similar
   placed neighbour, rather than fragmenting into a directory bucket.
7. **directory-fallback** — last resort (rule 6 calls this a smell, so the
   report counts what lands here: 16 of 161 files).
8. **rename-pair** — reunites both halves of a rename, overriding everything
   (rule 11).

On this fixture the split of work is: token-cluster 122 files, absorb 14,
test-pairing 9, directory-fallback 16.

### The token-cluster pass

Filenames are split on separators and camelCase, lowercased and crudely
singularised. Each unigram and adjacent-token bigram is a candidate group key.
Keys are ranked by how many files carry them, bigrams weighted 1.6× as more
specific, then assigned greedily; a key claiming fewer than 3 unclaimed files is
dropped as noise.

The one mechanism that must not be removed is the **ubiquity stoplist**: a
single token carried by ≥25% of the changeset names the repository, not a
concern, and is dropped as a key. On this fixture that token is `github`.
Without the stoplist F1 collapses from 0.391 to 0.255 — baseline level. This is
also the guard against a heuristic that cheats on repo-specific vocabulary, and
it is derived from the changeset itself rather than hardcoded, so it transfers.

Bigrams are exempt from the stoplist: `github` names the repo, but
`github-project` still names a concern inside it.

The threshold is a cliff, not a plateau — 0.20 and 0.25 both give 0.391, while
0.15 gives 0.250 and 0.35 gives 0.255. One fixture cannot calibrate that; a
second changeset should before the value is trusted.

## Which design-doc passes survive

The design doc proposed six. Contact with a real changeset sorts them into three
piles.

**Carrying the work.** Path/module clustering — but as *filename token*
clustering, not path clustering. Grouping by directory is refuted outright:
parent-directory scores 0.219, *below* all-one-group. Directory tokens as
supporting evidence are marginal (0.394 vs 0.391, trading precision for recall)
and not worth the extra rope. Test-source pairing survives, and so does the
directory fallback, though only as a smell counter.

**Fired on nothing, kept anyway.** Rename detection, generated/lockfile, and
config/CI matched zero files here: this changeset has no renames, no lockfiles,
no generated output and no CI files. They are unscoreable on this fixture rather
than disproven — they are cheap, high-precision, and rules 8 and 11 demand them.

**Decision: keep all three, explicitly as bets rather than as evidence.** A
second changeset carrying a lockfile, a rename and a CI file either vindicates
them or exposes the pattern lists as dead code; until then nothing here supports
them but the rules they implement.

**Pruned before writing code.** Any pass reading diff content cannot be scored
here at all, so none was built. That includes the LLM refine pass (`gd-26r.11`).

**Added, not in the design doc.** Two. The ubiquity stoplist, which is the
difference between working and not working. And leftover absorption, which
trades precision for recall about evenly (0.393 → 0.391) but cuts the group
count from 32 to 25 and empties most of the directory-fallback bucket — a
legibility win the F1 does not show.

The test-burst boost (rule 7) changes nothing measurable on this fixture:
identical F1 to three decimal places. Retained at low weight because the rule was
elicited from real reviews, but it is currently unevidenced.

## Tie-break

**Strict pass priority, first match wins**, with exactly two overrides, both
straight from `docs/GROUPING.md`:

- **rule 11 (rename pairs)** overrides any pass.
- **rule 4 (a test follows its production file)** overrides the token-cluster
  pass, but nothing above it.

The fixture does not decide the rule 4 override on its own: letting the cluster
keep the test scores 0.388 against 0.391 for letting the production file win —
inside the noise of a provisional fixture. That is the useful result. Obeying
the human's stated rule is **free**, so the spec decides and the metric abstains.
Both settings hold the strict partition; no file is ever in two groups.

## Close-call annotation

23 of 161 files (14%) carry a runner-up and a one-line reason — enough to be
informative, not so many as to be noise. A file is flagged when its
second-choice group scored at least 75% of its chosen group's score. Sample:

```
src/main/github/client-work-items.test.ts
  client <- work-item: follows production file src/main/github/client.ts
                       (rule 4) over cluster `work-item` (score 16.4)
src/main/git/gh-rate-limit-breaker.ts
  rate-limit <- gh:    filename also carries `gh`; `rate-limit` scored 8.0
                       against 8.0
```

This is derived state, recomputed on every regroup, and it is an annotation on
an assignment — never a bucket of unassigned files.

## Group naming

Names are **derived mechanically**, from the cluster key or, for clusters formed
by other passes, the most frequent shared filename token. Rule 10 asks for the
changeset's own vocabulary, and a token lifted from the filenames is exactly
that.

The finding is that naming is not a separate problem from grouping. Where the
heuristics found a real concern, the derived name is already legible: `work-item`,
`rate-limit`, `check`, `auth`, `issue`, `task`. Where they did not, the derived
name is a generic noun — `client`, `source`, `repo`, `host`, `gh` — or a
`dir:` bucket.

So **an ungrammatical or generic derived name is a signal that the group is
weak**, not a naming problem to fix downstream. Authoring names by hand would
paper over exactly the groups that need attention. This gives the LLM refine
pass in `gd-26r.11` a concrete, cheap trigger: refine the groups whose names did
not derive well, and leave the rest alone.

## The ceiling, and what it means for the next pass

Per-expected-group, the best single filename token that could ever reproduce it:

| expected group | files | best key | F1 |
| --- | --- | --- | --- |
| issues | 4 | `issue` | 1.00 |
| rate-limiting | 5 | `rate-limit` | 1.00 |
| settings-repo-icon | 5 | `icon` | 1.00 |
| tasks | 4 | `task` | 1.00 |
| work-items | 8 | `work-item` | 0.94 |
| pr-checks | 6 | `check` | 0.92 |
| gh-auth | 5 | `auth` | 0.91 |
| pr-actions | 29 | `pr` | 0.74 |
| project-view | 28 | `project` | 0.74 |
| terminal-strays | 2 | `watcher` | 0.67 |
| enterprise-host-routing | 18 | `host` | 0.57 |
| repo-slug-resolution | 9 | `smart` | 0.50 |
| github-client-plumbing | 38 | `github` | 0.26 |

Seven of thirteen groups are essentially findable from a filename token. The
three that are not — `github-client-plumbing` (38 files), `pr-actions` (29),
`project-view` (28) — are 59% of the changeset and dominate the pairwise metric,
which is why recall sits at 0.30 while precision sits at 0.55.

Those three are concern-shaped in the way rule 1 describes: `github-client-plumbing`
spans `git/runner`, `github/client`, `ipc`, `rpc/methods`, `preload`,
`source-control` and renderer store slices with no shared filename token, and
its best key is `github` — the token the stoplist must drop. It is also the
group the fixture README flags as a rule 6 residual smell.

Agglomerative clustering over the same token similarity was tried as a check
that the greedy pass was not simply weak. It scores worse at every group count
(best 0.310 at 16 groups). The limit is the signal, not the algorithm.

**The honest read: filename tokens get roughly the first two thirds of the
grouping, and the remaining third needs something that reads code.** F1 near
0.4 with precision 0.55 is what path-only heuristics are worth on this
changeset.

That points somewhere useful for `gd-26r.11` — an LLM pass would not be
competing with the heuristics for the same groups, it would be aimed at the
three big cross-cutting ones the heuristics provably cannot see, with the
derived-name quality telling it which those are.

**This read is not acted on yet.** It rests on one changeset from one repo, and
three passes firing on nothing beside a ubiquity threshold that behaves like a
cliff are exactly the symptoms of a single-fixture result. A second calibration
fixture from a differently-shaped repo comes first (`gd-26r.20`), and
`gd-26r.11` is gated behind it. Scoping the model pass to a ceiling measured
once would be the same mistake as arguing the pass set on paper.
