# Heuristic grouping passes: findings

What the grouping engine runs, in what order, and what happens when two passes
claim the same file. The rules being aimed at are `docs/GROUPING.md`; the
fixture and scoring method are `tests/fixtures/grouping/README.md`.

Everything down to [The second fixture](#the-second-fixture-does-the-pass-set-transfer)
is measured against **one** changeset (orca `971b16754`, 161 files) by
`tests/grouping_prototype.rs`:

```
cargo test --test grouping_prototype -- --nocapture report
```

That fixture is provisional and paths-only, so these numbers rank passes
against each other. They do not certify any pass as good. The second-fixture
section then re-runs the same harness on a changeset from a deliberately
different repo, and is where the transfer verdicts live — read it before
trusting anything above it.

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

**This read was not acted on until a second fixture existed.** It rests on one
changeset from one repo, and three passes firing on nothing beside a ubiquity
threshold that behaves like a cliff are exactly the symptoms of a single-fixture
result. That second fixture is below.

---

# The second fixture: does the pass set transfer?

`gd-26r.20`. Fixture 1 is a TypeScript/Electron app: one language, one package,
shallow paths, filenames doing all the naming. Fixture 2 is deliberately the
opposite shape — a single PR from a **private, .NET-dominant enterprise
monorepo**: 158 files, 91 C# and 45 TS/TSX, deep product/service/layer paths,
one lockfile, no renames, no CI file. It carries a resident-lifecycle feature
plus a revenue-cycle pipeline, hand-grouped by the human into 14 groups
(largest 13.3%).

**The fixture is not in this repo.** Its paths are confidential, so
`meridian-6c22fda02.files` / `.groups` live outside the tree, loaded at runtime
from `$TUICR_GROUPING_FIXTURES` (default `~/.local/share/tuicr-fixtures`). Every
test that needs it skips with a notice when the directory is absent, so the
public checkout stays green. No path from it appears in this document.

## Headline numbers, same baselines

| grouping | F1 | precision | recall | groups | largest |
| --- | --- | --- | --- | --- | --- |
| all-one-group | 0.153 | 0.083 | 1.000 | 1 | 100.0% |
| one-file-per-group | 0.000 | 0.000 | 0.000 | 158 | 0.6% |
| top-level-directory | 0.155 | 0.084 | 0.977 | 3 | 98.1% |
| **parent-directory** | **0.312** | **0.678** | **0.203** | **63** | **9.5%** |
| heuristic passes | 0.282 | 0.299 | 0.267 | 31 | 17.1% |
| expected grouping (self-score) | 1.000 | 1.000 | 1.000 | 14 | 13.3% |

**Verdict: the pass set does not transfer.** On fixture 1 the heuristics beat
every baseline by ~0.14 F1; on fixture 2 they *lose to parent-directory*, 0.282
against 0.312, and lose badly on precision — 0.30 against 0.68. Locked as a
regression test (`second_fixture_loses_to_parent_directory`) so it cannot
quietly change without someone rewriting this section.

The inversion is not noise, it is structural. Fixture 1 refuted directory
grouping (parent-directory 0.219, *below* all-one-group) because a TS app's
directories are layers — `main/`, `renderer/`, `ipc/` — while its concerns live
in filenames. A .NET monorepo inverts that: **the directory tree already is the
concern tree.** Project folders, `Controllers/`, `Events/`, `Persistence/`,
per-feature page folders — the path carries the concern and the filename carries
the layer suffix. Parent-directory reaches 63 groups against 14 expected and
still gets 0.68 precision: it fragments the truth rather than mixing it.

The corollary is that "grouping by directory is refuted" was a fixture-1
statement dressed up as a general one. The honest general statement is that
**whether path or filename carries the concern is a property of the repo, not of
grouping**, and the engine currently hardcodes one answer.

Consistent with that, directory tokens as evidence now help on *both* fixtures
(0.294 vs 0.282 here; 0.394 vs 0.391 there) — the one ablation that improved
twice.

## Ablations, both fixtures

| variant | fixture 1 F1 | fixture 2 F1 |
| --- | --- | --- |
| default | **0.391** | 0.282 |
| no ubiquity stoplist | 0.255 | 0.232 |
| tie-break: cluster keeps the test | 0.388 | 0.282 |
| directory tokens as evidence | 0.394 | **0.294** |
| no test-burst boost (rule 7) | 0.391 | 0.282 |
| no leftover absorption | 0.393 | 0.273 |
| absorb everything | — | 0.278 |
| absorb only on strong match | — | 0.284 |
| agglomerative, 16 groups | 0.310 | 0.207 |

Three results replicate cleanly:

- **The ubiquity stoplist is load-bearing on both.** Removing it costs 0.136 on
  fixture 1 and 0.050 on fixture 2, and on fixture 2 it collapses the largest
  group from 17% to 58% — the failure mode is the same shape even where the F1
  cost is smaller.
- **The rule 4 tie-break is free on both** (0.388/0.391, 0.282/0.282). The spec
  decides; the metric still abstains.
- **The test-burst boost is inert on both** — identical to three decimals twice.
  Two fixtures now say it measures nothing. It stays only because the rule came
  from real reviews.
- **Agglomerative clustering loses on both**, and worse here. Not the algorithm.

## The ubiquity threshold does not transfer

The threshold was flagged as the most likely place the engine was overfit. It
was.

| threshold | fixture 1 F1 | fixture 2 F1 |
| --- | --- | --- |
| 0.05 | 0.242 | **0.328** |
| 0.10 | 0.250 | 0.324 |
| 0.15 | 0.250 | 0.282 |
| 0.20 | **0.391** | 0.282 |
| 0.25 | **0.391** | 0.282 |
| 0.30 | 0.255 | 0.282 |
| 0.35 | 0.255 | 0.282 |
| 0.50 | 0.255 | 0.282 |
| 1.00 (stoplist off) | 0.255 | 0.232 |

The two optima are **disjoint**: fixture 1 peaks only inside `[0.20, 0.25]` and
falls off a cliff either side; fixture 2 peaks at `≤ 0.10` and is flat from 0.15
to 0.50. There is no value that is best for both, and each fixture's optimum is
in the other's dead zone. The shipped 0.25 costs fixture 2 0.046 F1 against its
own best.

The cliff shape is itself fixture-specific. Fixture 1's cliff is one token
(`github`, on 26% of files) crossing the line; fixture 2 has no such token and
so has a plateau instead. **A cliff at 0.25 was one file's worth of coincidence.**

**Verdict: the threshold must be derived per changeset, not configured.** What
replicates is the *mechanism* — some stoplist beats no stoplist on both, and off
is the worst setting on both. What does not replicate is the number. The
defensible next move is to derive the cut from the token-frequency distribution
of the changeset itself (a gap or knee in the sorted frequencies) rather than
from a fixed share, since the share that means "repo vocabulary" evidently
depends on how many files a repo's vocabulary reaches.

Min-cluster shows the same rot in miniature — fixture 1: 3 → 0.391, 5 → 0.396;
fixture 2: 3 → 0.282, 5 → 0.298. Both drift upward with a larger minimum, which
at least agrees in direction, but nothing here justifies a tuned constant.

## Verdict on the three unfired passes

Fixture 2 has a lockfile but no rename and no CI file, so a second, **fire-check
only** fixture was captured from the same repo — a 148-file span carrying a
lockfile, a merge-queue workflow and one rename. It has no hand grouping and is
never scored; it exists purely to make the three passes meet their inputs
(`unfired_passes_meet_a_changeset_that_should_fire_them`).

**1. mechanical — fires, and is under-inclusive.** It caught the lockfile on
both changesets. It caught *only* the lockfile: the human named EF migration
`*.Designer.cs` and `*DbContextModelSnapshot.cs` as mechanical under rule 8, and
the suffix list has no entry for either. Verdict: **vindicated as a pass,
refuted as a pattern list.** The list is TS/Go/Python-shaped; generated-code
markers are per-ecosystem and the list needs to grow per ecosystem, which is an
argument against pattern lists as the mechanism.

**2. config-ci — fires, but only by luck of location.** Zero hits on fixture 2,
two on the fire-check (a workflow file and a root Makefile). The `.github/`
directory rule works. The `CONFIG_NAMES` rule is guarded by
`!file.path.contains('/')` — root only — so in a monorepo, where every
`package.json` is nested, it can never match. Verdict: **half the pass is dead
by construction in exactly the repo shape that has the most config files.**

A real finding fell out of hunting for a CI file: **no single PR in the last
1200 commits of that repo touches both 100–300 files and a `.github/` file.** CI
changes ship as tiny PRs. A pass tuned for CI-inside-a-big-changeset may be
aimed at a case that mostly does not occur.

**3. rename-pair — dead code, structurally.** The fire-check changeset contains
a rename; the rename pass records **zero** hits on it. Reading
`enforce_rename_pairs` explains why and the fixture only confirms it: the pass
looks the *old* path up in `assigned`, but `Changeset::parse` follows git and
represents a rename as a **single** entry whose `path` is the new path. The old
path is never a key in `assigned`, so the branch cannot be taken on any input.
Verdict: **not "unscoreable" — unreachable.** Rule 11 is currently unimplemented
and the pass must be rewritten against the single-entry representation (or rule
11 recast as "a rename's two halves are one file", which is what the diff
actually says). Reassuringly, the one rename's surviving half landed in a
sensible group anyway, via the token pass.

So of the three bets: one is real but under-specified, one is half-dead, one is
entirely dead. Keeping unfired passes as bets was the wrong call — a pass that
has never fired is a pass that has never been tested, and two of three were
broken.

## Ceiling: what filename tokens can reach here

| expected group | files | best key | F1 |
| --- | --- | --- | --- |
| rcm-pipeline-realtime | 9 | `pipeline` | 1.00 |
| rcm-payer | 7 | `payer` | 0.92 |
| rcm-rules | 14 | `sc` | 0.88 |
| dependencies | 2 | `pnpm-lock` | 0.67 |
| platform-billing-reuse | 5 | `worklist-item` | 0.57 |
| resident-registration | 12 | `resident` | 0.56 |
| rcm-service-scaffold | 18 | `csproj` | 0.54 |
| rcm-pipeline-stages | 19 | `event` | 0.54 |
| admission-census | 14 | `patient` | 0.52 |
| rcm-authorization | 6 | `role` | 0.50 |
| rcm-claims-api | 21 | `claim` | 0.48 |
| rcm-web-worklist | 15 | `page` | 0.45 |
| web-shell | 8 | `tab` | 0.40 |
| service-host-wiring | 8 | `development` | 0.36 |

Size-weighted mean best-key F1: **0.58 here against 0.64 on fixture 1.** The
"filename tokens get roughly two thirds" read survives as an order of magnitude,
but the *shape* is different and worse. Fixture 1 had seven groups essentially
findable (F1 ≥ 0.9) and three big unreachable ones. Fixture 2 has **three**
above 0.85 and no cliff after: ten of fourteen groups sit in a 0.36–0.57 mush.
The signal is not missing from a few large cross-cutting groups, it is thin
almost everywhere.

Two mechanical causes, both visible in the derived names (`use`, `handler-cs`,
`port-cs`, `program-cs`, `sc`, `in-memory`):

- **`.cs` is not in the tokeniser's extension list**, so it survives as a token
  and clusters every C# file's layer suffix. `handler-cs` and `port-cs` are
  groups of "files that are handlers" across unrelated concerns — layer
  grouping, precisely what rule 1 forbids.
- **React hook and .NET type conventions dominate the filename.** `use-*`
  clusters every hook regardless of concern; `*Controller`, `*Service`,
  `*Repository` do the same. Both are strong, consistent, and orthogonal to the
  concern.

That is the same failure the ubiquity stoplist exists to prevent, but the
stoplist only catches tokens on ≥25% of the changeset, and `use` at 11 files of
158 is well under it while being pure noise. **The stoplist is measuring
ubiquity when the thing that matters is whether a token names a layer or a
concern** — and a token's frequency is a poor proxy for that.

## What this changes

- **The pass set is not portable as tuned.** It is a TypeScript-app grouper. It
  should either derive its constants per changeset or be honest that it needs a
  per-ecosystem profile (extension list, layer-suffix stoplist, generated-file
  markers). Deriving is the better bet; profiles are the pattern-list mistake
  again.
- **Directory evidence should be on**, not off. It is the only ablation that
  helped both fixtures, and on repos where the tree is the concern tree it is
  the strongest signal available.
- **Fix rename-pair or delete it.** It has never executed.
- **`gd-26r.11`'s scoping changes.** Fixture 1 suggested an LLM pass aimed at
  three big cross-cutting groups the heuristics cannot see. Fixture 2 says the
  heuristics can be beaten by `parent-directory` outright, so the model pass is
  not a top-up on a good grouping — on some repo shapes it is the grouping. The
  ceiling it must clear is *the better of the heuristics and the baselines*, per
  changeset, not the heuristics alone.
- **Group sizing is a live question the metric cannot see.** The human's read
  while hand-grouping was that groupings should get **coarser as the changeset
  gets smaller** — a lone CI file deserves its own group in a 20-file PR and
  does not in a 200-file one — and that a **configurable soft/hard cap on group
  size** (~20 soft, 25 hard) is wanted, with no minimum, since a single generated
  or doc file is a legitimate group of one. Neither is implemented and pairwise
  F1 would barely notice either.

Two fixtures is still two. What they establish is not the right constants but
that **constants are the wrong shape of answer**, and that a result from one
changeset should be assumed local until a second one says otherwise. That is now
cheap to check: the harness is fixture-parameterised, so a third fixture is a
directory drop, not a code change.
