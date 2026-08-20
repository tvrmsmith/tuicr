# Heuristic grouping passes: findings

Part of the grouping design set. [`docs/GROUPING_INDEX.md`](GROUPING_INDEX.md)
maps a question to the doc that owns the answer.

What the grouping engine runs, in what order, and what happens when two passes
claim the same file. The rules being aimed at are `docs/GROUPING.md`; the
fixture and scoring method are `tests/fixtures/grouping/README.md`.

Everything down to [The second fixture](#the-second-fixture-does-the-pass-set-transfer)
is measured against **one** changeset (orca `971b16754`, 161 files) by
`tests/grouping_prototype.rs`:

```
cargo test --test grouping_prototype -- --ignored --nocapture report
```

That fixture is provisional and paths-only, so these numbers rank passes
against each other. They do not certify any pass as good. The second-fixture
section then re-runs the same harness on a changeset from a deliberately
different repo, and is where the transfer verdicts live — read it before
trusting anything above it.

> **The current heuristic numbers are in
> [Fixing the defects](#fixing-the-defects-gd-26r21)**, and the model refine
> pass is measured after it in
> [The model refine pass](#the-model-refine-pass-gd-26r11). The two sections
> above *Fixing the defects* are kept as the record of what was
> measured when, and several of their findings are superseded there: four engine
> defects turned out to be *causing* results the earlier sections read as facts
> about grouping.

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
2. **config-ci** — `.github/`, `.circleci/`, `.husky/` (rule 8's neighbour).
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
here at all, so none was built. That includes a *hunk-reading* LLM refine pass;
`gd-26r.11` was later prototyped in the paths-only shape these fixtures can
score — see [The model refine pass](#the-model-refine-pass-gd-26r11).

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
one lockfile, no renames, no CI file. It carries two unrelated product features
— the domain is private and does not matter to any result here — hand-grouped by
the human into 14 groups (largest 13.3%).

**The fixture is not in this repo.** Its paths are confidential, so
`meridian-6c22fda02.files` / `.groups` live outside the tree, loaded at runtime
from `$TUICR_GROUPING_FIXTURES` (default `~/.local/share/tuicr-fixtures`). Every
test that needs it skips with a notice when the directory is absent, so the
public checkout stays green. No path from it appears in this document.

Nor does any of its vocabulary: expected group names, derived group names and
best-key tokens are all fragments of private paths, so **the per-group fixture-2
tables are not published either.** They live beside the fixture, in
`$TUICR_GROUPING_FIXTURES/meridian-6c22fda02.notes.md`. Every fixture-2 result
that does not carry a private identifier — F1 means, worst and best, the bar,
consensus figures, group counts, largest-group share, token and cost and
wall-clock numbers, stability, and every verdict drawn from them — stays here,
and the analysis is written to stand without the tables. Where a particular
fixture-2 group has to be named below, it is given a pseudonymous label
(`concern-a`, `concern-b`, …) that carries no meaning outside the sentence
using it.

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

That fixture is private, so the same test also runs a **committed synthetic
changeset**, `tests/fixtures/grouping/fire-check-synthetic.files`: twelve
invented paths carrying a lockfile, two `.github/workflows/` files, a
`.husky/pre-commit` and one rename. It is likewise unscored. Without it these
passes go unverified on any machine that does not have the private fixture,
which includes CI.

**1. mechanical — fires, and is under-inclusive.** It caught the lockfile on
both changesets. It caught *only* the lockfile: the human named EF migration
`*.Designer.cs` and `*DbContextModelSnapshot.cs` as mechanical under rule 8, and
the suffix list has no entry for either. Verdict: **vindicated as a pass,
refuted as a pattern list.** The list is TS/Go/Python-shaped; generated-code
markers are per-ecosystem and the list needs to grow per ecosystem, which is an
argument against pattern lists as the mechanism.

**2. config-ci — fires, but only by luck of location.** Zero hits on fixture 2,
two on the fire-check (a workflow file and a root Makefile) — the Makefile via
the filename list that [fix 3](#fix-3-config-ci--narrowed) later deleted, so the
pass claims one file there now, and three on the synthetic changeset. The `.github/`
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

The per-group table — each expected group, its size, the best single filename
token that could ever reproduce it, and that token's F1 — is unpublishable here:
the group names and the tokens are both fragments of private paths. It is in
`$TUICR_GROUPING_FIXTURES/meridian-6c22fda02.notes.md`. Its distribution is the
result, and that is public.

Size-weighted mean best-key F1: **0.58 here against 0.64 on fixture 1.** The
"filename tokens get roughly two thirds" read survives as an order of magnitude,
but the *shape* is different and worse. Fixture 1 had seven groups essentially
findable (F1 ≥ 0.9) and three big unreachable ones. Fixture 2 has **three**
above 0.85 and no cliff after: ten of fourteen groups sit in a 0.36–0.57 mush.
The signal is not missing from a few large cross-cutting groups, it is thin
almost everywhere.

Two mechanical causes, both visible in the derived names — the publishable ones
are the language-convention names (`use`, `handler-cs`, `port-cs`,
`program-cs`), the rest being repo vocabulary:

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

---

# Fixing the defects (`gd-26r.21`)

`gd-26r.20` closed with four defects and no fixes. They are fixed here, each in
its own commit, with both fixtures rescored after each one so no change is
bundled. That mattered more than usual: `gd-26r.11` measures a model pass
against the heuristic number, and a baseline depressed by known bugs would
flatter it.

## Attribution, per fix

Heuristic F1 after each commit, both fixtures:

| # | fix | fixture 1 | fixture 2 |
| --- | --- | --- | --- |
| — | before (`gd-26r.20` state) | 0.391 | 0.282 |
| 1 | tokeniser extension list audited | 0.391 | 0.296 (+0.014) |
| 2 | directory evidence on, under the stoplist | 0.394 (+0.003) | **0.378** (+0.082) |
| 3 | config-ci narrowed to CI directories | 0.394 | 0.378 |
| 4 | rename pass deleted | 0.394 | 0.378 |

Fixes 3 and 4 move nothing by construction: neither hand-grouped fixture carries
a CI file, and the rename pass had never executed on any input. They are
correctness and honesty, not score.

Nearly all of the movement is fix 2, and only because fix 1 came first — see
below.

## The headline numbers now

| grouping | fixture 1 F1 | fixture 2 F1 |
| --- | --- | --- |
| all-one-group | 0.241 | 0.153 |
| one-file-per-group | 0.000 | 0.000 |
| top-level-directory | 0.255 | 0.155 |
| parent-directory | 0.219 | 0.312 |
| **heuristic passes** | **0.394** | **0.378** |
| best agglomerative | 0.310 | 0.315 |
| expected (self-score) | 1.000 | 1.000 |

Full rows: fixture 1 P 0.479 / R 0.334 / 19 groups / largest 22.4%; fixture 2
P 0.425 / R 0.340 / 29 groups / largest 14.6%.

**The recorded inversion is gone.** The heuristics now beat every baseline on
both fixtures, where before they lost to parent-directory on fixture 2 (0.282
against 0.312). `second_fixture_loses_to_parent_directory` is replaced by
`directory_evidence_carries_the_second_fixture`, which locks the ablation rather
than the inversion, and `heuristic_beats_every_baseline_on_the_first_fixture`
becomes `..._on_both_fixtures`.

The directory-fallback bucket — rule 6's smell counter — shrinks from 16 to 7
files on fixture 1 and 19 to 15 on fixture 2.

## Fix 1: the tokeniser extension list

`.cs` was missing, so it survived tokenisation and clustered C# files by layer
suffix: `handler-cs`, `port-cs`, `program-cs` — groups of "files that are
handlers" across unrelated concerns, which is precisely what rule 1 forbids. The
brief was to audit the list rather than add one entry, and the list was
web-stack-shaped: no .NET, no JVM, no C family, no shell, no SQL, no Terraform,
no project or build files. It now covers those.

Worth 0.014 F1 on fixture 2 and nothing on fixture 1 — which is the point, since
fixture 1 is the repo shape the list was written against. Two tests hold it:
`extensions_never_survive_as_concern_tokens` over a table of filenames from
several ecosystems, and `no_group_is_named_after_an_extension` over both
fixtures.

**A finding fell out of it.** With `.cs` stripped, the ubiquity stoplist became
*completely inert* on fixture 2 — identical F1 at every threshold from 0.05 to
off. The "stoplist is load-bearing on both fixtures" result from `gd-26r.20` was,
on fixture 2, the stoplist catching `cs` on 58% of the files: it was cleaning up
after the tokeniser bug, not finding repo vocabulary. The stoplist only became
load-bearing there again after fix 2 gave it directory tokens to judge.

## Fix 2: directory evidence, and why it looked marginal

`gd-26r.20` recommended turning directory evidence on because it was the one
ablation that helped both fixtures — but only just: 0.394 vs 0.391 and 0.294 vs
0.282. After fix 1 the fixture-2 gain shrank to 0.001, which nearly killed the
recommendation.

The reason it looked marginal is a second defect, found while measuring it:
**directory tokens bypassed the ubiquity stoplist entirely.** `ubiquitous_tokens`
computed document frequency over `name_tokens()` only, while `candidate_keys`
admitted `dir_tokens()` as cluster keys. So every directory token — `src`, `app`,
`apps`, the repo's own product directory — was a free key exempt from the one
mechanism that stops layer and repo vocabulary being used as a concern.

Subjecting directory tokens to the same stoplist changes the value of directory
evidence on fixture 2 from **+0.001 to +0.082**:

| | fixture 1 | fixture 2 |
| --- | --- | --- |
| directory tokens off | 0.391 | 0.296 |
| on, leaking past the stoplist | 0.394 | 0.297 |
| **on, under the stoplist** | **0.394** | **0.378** |

**Verdict: directory evidence helps both fixtures and is on by default. It does
not need to be derived per changeset.** The honest form of `gd-26r.20`'s
"whether path or filename carries the concern is a property of the repo" is that
the engine does not have to choose: with both admitted as evidence and both
subject to the stoplist, the changeset's own token distribution decides which
one carries, per repo, for free. On fixture 1 (concerns in filenames) directory
tokens cost 0.073 precision and buy 0.031 recall, netting +0.003; on fixture 2
(the tree *is* the concern tree) they are the difference between losing to
parent-directory and beating it.

Consistent with that, the derived group names on fixture 2 stop being layer
nouns — they become the changeset's own domain nouns, where the old run produced
`handler-cs`, `port-cs`, `program-cs`, `use`. Both name lists are domain
vocabulary from private paths, so only the layer-suffix half is quotable here;
the pair is in `$TUICR_GROUPING_FIXTURES/meridian-6c22fda02.notes.md`.

**The ubiquity threshold verdict softens.** `gd-26r.20` found the two fixtures'
optima disjoint, each in the other's dead zone, and concluded the threshold must
be derived per changeset. With both defects fixed they now overlap:

| threshold | fixture 1 | fixture 2 |
| --- | --- | --- |
| 0.05 | 0.250 | 0.295 |
| 0.10 | 0.258 | 0.302 |
| 0.15 | 0.258 | **0.378** |
| 0.20 | 0.315 | **0.378** |
| 0.25 | **0.394** | **0.378** |
| 0.30 | **0.394** | 0.320 |
| 0.50 | **0.394** | 0.297 |
| off | 0.214 | 0.297 |

The shipped 0.25 is now optimal for both — but it is the *only* value that is,
sitting on fixture 1's rising edge and fixture 2's falling one. That is a
coincidence to be honest about, not a calibration, so deriving the cut from the
changeset's own frequency distribution remains the right direction; it is just
no longer forced by a contradiction. Min-cluster still refuses to agree
(fixture 1 prefers 2, fixture 2 prefers 4), and both fixtures now prefer a
stricter absorption threshold than the shipped 2.0 (0.399 and 0.382 at 3.0) —
tuning, deliberately not done here.

## Fix 3: config-ci — narrowed

**Verdict: narrowed, not deleted.** The `.github/` `.circleci/` `.husky/`
directory rule is kept; the `CONFIG_NAMES` filename list is deleted outright.

The list was guarded by `!path.contains('/')`, so it could only ever match at the
repo root — dead in exactly the monorepo shape that has the most build config.
Unguarding it was measured before deciding: it claims **one** extra file on
fixture 2 and moves F1 by nothing. Growing it to cover .NET would make things
worse, not better — fixture 2's 18 `.csproj` files are one hand-grouped
*concern* (`concern-a`, pseudonymous), and a config bucket would shred it. Nested
build config is better served by the token and directory evidence that fix 2
turned on.

The directory half survives because it is cheap and precise and rule 8 asks for
it, and because the 1200-commit finding — no PR in that repo touches both
100–300 files and a `.github/` file — argues the pass will rarely fire, not that
it is wrong when it does. A pass that fires rarely and correctly costs nothing.
The fire-check changeset still fires it.

## Fix 4: rename-pair — deleted, rule 11 recast

**Verdict: deleted.** `enforce_rename_pairs` looked the *old* path up in
`assigned`, but git reports a rename as a single entry keyed by the new path, so
the old path was never a key and the branch could not be taken on any input.

The fix is not to rewrite it. There is no pair to reunite: a rename's two halves
are **one file** in the diff, so rule 11 is satisfied by construction. Rule 11 is
recast in `docs/GROUPING.md` as a constraint on the representation — any
representation that splits a rename into a delete and an add is wrong, rather
than a case to reconcile afterwards — and `a_rename_is_one_file_in_the_changeset`
replaces the pass.

Using the old path as *evidence* (a renamed file also carrying its former name's
tokens) was considered and rejected: neither hand-grouped fixture contains a
rename, so it would be another unscoreable bet, and `gd-26r.20`'s clearest
lesson is that unfired passes are untested passes.

## The pass set now

1. **mechanical** — lockfiles, vendored trees, generated output (rule 8).
2. **config-ci** — CI directories only.
3. **whole-change docs** — a `docs/` or root `.md` file (rule 9).
4. **token-cluster** — filename *and* directory tokens, both under the ubiquity
   stoplist.
5. **test-pairing** — a test joins its production file's group (rule 4).
6. **absorb-leftovers**.
7. **directory-fallback** — last resort and smell counter.

Seven passes, all of which fire on at least one real changeset. That was not
true before.

## What this leaves

- **The engine is no longer a TypeScript-app grouper.** It beats every baseline
  on both fixtures, and the gap on the .NET fixture (0.378 against 0.312) is
  similar in size to the gap on the TS one (0.394 against 0.255). Two of the
  three things `gd-26r.20` read as "the pass set does not transfer" were bugs.
- **`gd-26r.11`'s bar is the honest one now**: 0.394 and 0.378, both of which
  already clear the best baseline per changeset, rather than a number depressed
  by known defects.
- **What is genuinely unsolved is the ceiling.** Best-key F1 per expected group
  is unchanged in character: fixture 2 still has ten of fourteen groups in a
  0.36–0.57 mush, and no filename or directory token reaches them. That is the
  case for a pass that reads code, and it is exactly where `gd-26r.11` should
  aim. (`gd-26r.11` then reached that mush from paths alone — see [Where fixture
  2 goes right](#where-fixture-2-goes-right); the mush was evidence against
  *token matching*, not for code-reading.)
- **Still not addressed, deliberately.** Group sizing (soft/hard caps, coarser
  as the changeset shrinks) — separate fog. The ubiquity stoplist measuring
  frequency when the question is layer-versus-concern — a design question, not a
  defect, and fix 2 blunts it rather than answering it: directory tokens give
  the stoplist a second population to judge, which is why `use` and the layer
  suffixes stopped dominating, but nothing here makes frequency the right proxy.

# The model refine pass (`gd-26r.11`)

`gd-26r.4` already settled that refine is an *optional, async agent-CLI call that
degrades to heuristics-only*. This ticket asks the four questions that decide
whether to build it at all: how much closer to the gold standard it gets, what a
run costs, how much it moves between runs, and whether a cheaper shape captures
most of the benefit.

## How it was measured

A nondeterministic paid call cannot live inside `cargo test`, so the prototype is
cut in three:

1. `emit_refine_prompts` writes one prompt per fixture per shape into
   `target/grouping-refine/prompts/`. Input is the changeset, the heuristic
   groups, and a digest of the eleven rules from `docs/GROUPING.md`.
2. `scripts/grouping-refine-runs.sh` sends each prompt N times and records the
   CLI's own `--output-format json` envelope — answer, token counts, cost, wall
   clock. Runs from a `mktemp -d` with `--setting-sources '' --tools ''`, so the
   model cannot read the repo it is grouping.
3. `refine_report` replays the recorded envelopes deterministically and scores
   them with the same harness that scores the heuristics.

So the pass is nondeterministic but its *evidence* is not: the forty orca
envelopes are committed under `tests/fixtures/grouping/refine/`, so **every
fixture-1 number below can be re-derived offline with no API key**. The forty
meridian envelopes carry private paths in both prompt and answer, so they stay
beside the fixture under `$TUICR_GROUPING_FIXTURES/refine/` and are never
committed — **no fixture-2 number below is reproducible without that private
directory on the machine**. Where a table mixes the two, the fixture-2 rows are
taken on trust by anyone who does not have it.

Ten runs per shape per fixture, `claude-opus-5`, eighty calls in total. The
corpus was extended from five to ten deliberately, because five did not support
the verdict the first pass at this document drew from it — see *Which three,
though*.

**Overlap with `gd-26r.13`.** That ticket — shelling out to an agent CLI from
Rust — is still open and unclaimed. This prototype needs the mechanism, so it
does the shelling in a shell script *on purpose*, to use it without deciding it.
Nothing here constrains `gd-26r.13`'s answer.

## The four shapes

| shape | what the model may do |
| --- | --- |
| **full** | regroup freely: merge, split, move single files, rename |
| **full-coarse** | as full, plus "prefer fewer, larger groups; no concern group under four files" |
| **merge-only** | merge whole heuristic groups; never split one |
| **naming-only** | rename groups and order them; the partition is fixed |

All four are *revisions*: each is handed the heuristic grouping and asked to
improve on it. `gd-26r.24` later added a fifth, **cold** — no starting grouping
at all — which is not a shape to choose between but the control that says what
the other four owe to their seed. It is measured in [The heuristics are
load-bearing](#the-heuristics-are-load-bearing-the-cold-control) and is not part
of the eighty runs below.

`apply` enforces the strict partition `docs/GROUPING.md` assumes before its
numbered rules — every file in exactly one group — on the way back in, and
counts repairs. **Repairs were 0.0 in every one of the eighty recorded runs**,
but the four shapes do not all put the same claim at risk, so the result reads
in two parts:

- In the forty **full** and **full-coarse** runs the answer names paths, and no
  answer dropped, duplicated or invented one.
- In the forty **merge-only** and **naming-only** runs the answer names input
  *groups*, never paths, so a path-level violation is structurally impossible
  there rather than absent. What those runs did establish is that no answer
  named an unknown input group, claimed one twice, or left one unclaimed.

`apply` also enforces what neither result covers: two returned groups sharing a
name, or returning none, would silently union in the partition, so each is
uniquified and counted. Eighty runs of one model on two changesets is not
evidence that any of this cannot happen, so the enforcement stays — that is what
makes an unreliable pass shippable. The repair branches are unit-tested against
hand-written bodies (`a_dropped_path_is_restored_to_its_heuristic_group` and the
tests beside it), because a count of zero repairs means nothing unless a repair
could have been counted. The zero itself is locked for the forty committed
fixture-1 runs by `no_committed_run_needed_a_repair`, which runs on every
`cargo test` rather than only when someone reads the report.

## Accuracy

Bar per fixture is the better of heuristics and baselines, per `gd-26r.21`:
0.394 on fixture 1 (orca, 161 files, TypeScript) and 0.378 on fixture 2
(meridian, 158 files, .NET).

Ten runs per shape. The last two columns are the ones that decide anything: a
shipped pass draws *some* run, or *some* triple, not the first one recorded.

**Fixture 1 — bar 0.394:**

| shape | F1 mean | worst | best | single calls over bar | consensus of 3 | consensus of 10 | triples over bar |
| --- | --- | --- | --- | --- | --- | --- | --- |
| full | 0.353 | 0.321 | 0.373 | **0 of 10** | 0.411 | 0.408 | **50 of 120** |
| full-coarse | 0.386 | 0.344 | 0.475 | 4 of 10 | 0.420 | 0.416 | 58 of 120 |
| merge-only | **0.417** | 0.385 | 0.438 | **9 of 10** | 0.438 | **0.448** | **107 of 120** |
| naming-only | 0.394 | 0.394 | 0.394 | 0 of 10 | 0.394 | 0.394 | 0 of 120 |

**Fixture 2 — bar 0.378:**

| shape | F1 mean | worst | best | single calls over bar | consensus of 3 | consensus of 10 | triples over bar |
| --- | --- | --- | --- | --- | --- | --- | --- |
| full | **0.763** | 0.673 | 0.844 | **10 of 10** | **0.847** | 0.845 | **120 of 120** |
| full-coarse | 0.754 | 0.671 | 0.816 | 10 of 10 | 0.683 | 0.787 | 120 of 120 |
| merge-only | 0.398 | 0.339 | 0.443 | 8 of 10 | 0.388 | 0.388 | 117 of 120 |
| naming-only | 0.378 | 0.378 | 0.378 | 0 of 10 | 0.378 | 0.378 | 0 of 120 |

Consensus is a majority co-membership vote across runs, resolved into groups by
union-find: two files are grouped if most runs grouped them. **"Consensus of 3"
is the first three recorded runs, one of the 120 triples ten runs admit**; the
`triples over bar` column is all 120, and both are printed by `refine_report`.
Naming-only scores *exactly* the bar on both fixtures, so it clears it zero times
by construction rather than by failing — see *The cheaper shapes*.

**The headline is the split, and it is sharper at ten runs than it was at five.**
On fixture 2 full refine is transformational: 0.763 against a bar of 0.378, every
one of ten calls over it, every one of 120 triples over it, 0.847 voted. On
fixture 1 **no single full call cleared the bar in ten attempts**, and voting is a
coin flip rather than a fix — 50 of 120 triples clear, and the upper median
triple scores 0.369 against the 0.394 bar. Refine is not uniformly better than the
heuristics: it is enormously better on one changeset and slightly *worse* on the
other.

### Where fixture 1 goes wrong

Per-expected-group recall under `full`, run 0 and the mean of all ten runs.
`refine_report` prints both, plus every run, for every expected group:

| expected group | files | heuristic | refine run 0 | refine mean of 10 |
| --- | --- | --- | --- | --- |
| github-client-plumbing | 38 | 0.05 | 0.14 | 0.15 |
| pr-actions | 29 | 0.45 | 0.29 | **0.28** |
| project-view | 28 | 0.75 | 0.35 | **0.33** |
| enterprise-host-routing | 18 | 0.20 | 0.25 | 0.26 |
| gh-auth | 5 | 0.60 | 1.00 | 0.96 |
| settings-repo-icon | 5 | 0.20 | 1.00 | 1.00 |

The whole loss is two groups — `pr-actions` and `project-view`, 57 of 161 files
— and both are groups the token heuristics *already find*. The model splits them
into finer, individually-coherent concerns; the fixture keeps them whole. Ten
runs make this the *stable* failure rather than one bad draw: `project-view`
recall never once reached the heuristic 0.75 under `full`, spanning 0.27 to 0.37
across the ten, and `pr-actions` spans 0.26 to 0.32 against 0.45.

Two things follow, and they pull in opposite directions. Fixture 1's expected
grouping is itself provisional and was drawn with the token evidence in view
(`tests/fixtures/grouping/README.md`), so on this fixture the heuristics are
partly being graded against their own reasoning. But that is an explanation, not
an excuse: the metric is the metric, and by it a single call loses here.

`full-coarse` exists to test whether the loss is *granularity* — it instructs the
model toward fewer, larger groups without naming a group count, to avoid fitting
the prompt to the answers. It does name one number — no concern group under four
files — which is a floor on group size, not a target for how many. It lifts fixture 1 to 0.386 — still under the bar — and
drops fixture 2 to 0.754, and on fixture 1 it does not reliably repair the two
groups the model splits. Across the ten `full-coarse` runs, `project-view` recall
is 0.34, 0.34, 0.75, 0.34, 0.69, 0.37, 0.34, 0.29, 0.34, 0.34 (mean 0.41, against
0.75 for the heuristics alone) and `pr-actions` is 0.29, 0.27, 0.48, 0.45, 0.43,
0.28, 0.41, 0.28, 0.27, 0.41 (mean 0.36, against 0.45). Two of the ten runs put
`project-view` back most of the way and the other eight leave it near a third. So
the disagreement is judgement about where one concern ends, not group size, and
the coarseness instruction just trades one fixture for the other — unreliably,
run to run. Those per-run figures were computed ad hoc when the corpus was five
runs; now that it is fixed at ten, `refine_report` prints them.

### Where fixture 2 goes right

Per-expected-group recall was tabulated on the same slice as fixture 1's — run 0
and the ten-run mean under `full`, with a `full-coarse` column beside it, because
this is the fixture where coarsening costs rather than helps and the two arms
disagree on individual groups even where their means are close. Fixture 2's
expected group names cannot be published, so that table is in
`$TUICR_GROUPING_FIXTURES/meridian-6c22fda02.notes.md`; the labels below are
pseudonymous and mean nothing beyond the sentences using them.

**All fourteen expected groups improve under `full` on the ten-run mean**, and
the improvement is large: twelve of the fourteen gain more than +0.30 recall over
the heuristics. Averaging is the honest view here and it is less flattering than
run 0 was. Run 0 showed eight groups at a perfect 1.00; across ten runs only
three hold 1.00 *every* time, and the 2-file dependency-manifest group — 0.00 in
run 0 — turns out to be found in two runs of ten, for a mean of 0.20. A per-group
1.00 is a property of a run, not of the pass.

Run 0 also understated `full-coarse` here. On the ten-run mean it is the better
arm on **nine** of the fourteen groups — including `concern-b`, `concern-c` and
`concern-e` — and worse on four, of which the largest is `concern-a`, the 18
`.csproj` files (0.70 against 0.75). It still loses the fixture on overall F1,
0.754 against 0.763, and the reason is that this table is *recall* per expected
group. Coarsening buys recall by merging and pays for it in precision, which
per-group recall cannot see: on run 0 `full-coarse` scores P 0.725 / R 0.852
against `full`'s P 0.856 / R 0.823. Winning nine of fourteen recall columns is
therefore not a case for coarsening; it is the signature of the trade.

This is the direct answer to the ceiling `gd-26r.21` left open. Ten of fourteen
groups sat in a 0.36–0.57 best-key mush no filename or directory token reaches —
**and the model reached them from paths alone.** The mush was never evidence that
code-reading is required; it was evidence that *token matching* is the wrong
reader of paths. A path carries domain meaning that a tokeniser cannot see and a
model can.

## Cost

Per call, averaged over the ten runs, from the CLI's own accounting:

| fixture | shape | input | cached | output | cost | wall clock |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | full | 5,373 | 8,068 | 15,793 | $0.472 | 141.8s |
| 1 | full-coarse | 5,466 | 8,159 | 15,725 | $0.471 | 143.5s |
| 1 | merge-only | 5,404 | 8,096 | 2,373 | $0.137 | 27.9s |
| 1 | naming-only | 5,353 | 8,046 | 1,927 | $0.125 | 20.2s |
| 2 | full | 9,497 | 12,189 | 21,331 | $0.657 | 190.6s |
| 2 | full-coarse | 9,590 | 12,284 | 19,597 | $0.615 | 172.4s |
| 2 | merge-only | 9,528 | 12,221 | 4,313 | $0.232 | 56.9s |
| 2 | naming-only | 9,477 | 12,170 | 3,726 | $0.217 | 39.6s |

Cost is dominated by *output*, not input: a full regrouping restates every path,
so output runs 2.94× and 2.88× input on fixture 1 (`full`, `full-coarse`) and
2.25× and 2.04× on fixture 2, against 0.44× and 0.36× for fixture 1's
`merge-only` and `naming-only`, which restate names alone. What output tracks is
how much of the grouping the shape rewrites, not file count: fixture 2 has three
fewer files than fixture 1 and emits a third more output on `full`. Input does
not track file count either — it rises 5,373 to 9,497, +77%, on the fixture with
three *fewer* files — but it is small enough beside output that the shape, not
the input, is what the bill follows. What either of them does past ~160 files is
not measured here; see *What this does not answer*.

The cached column is cache *writes*: `cacheReadInputTokens` is zero in all forty
committed fixture-1 envelopes, so no call here was served from a warm cache, and
every row's cached figure sits a near-constant ~2,690 tokens above its own input
— the CLI's fixed preamble on top of the task text, not a term that grows on its
own.

`full` and `full-coarse` run from just under two and a half minutes to a little
over three per call, and the cheaper shapes from 20.2s to 56.9s. On a review the
user is waiting to start, that is the empirical confirmation of `gd-26r.4`: this
cannot be synchronous.
The heuristics must render immediately and refine must arrive later or not at
all.

## Stability

Treated as first class, because run-to-run movement is exactly what `gd-26r.8`
has to hold state across.

| fixture | shape | agreement mean | worst | files with identical group-mates every run | mean group-mate overlap |
| --- | --- | --- | --- | --- | --- |
| 1 | full | 0.827 | 0.715 | 11.8% | 0.790 |
| 1 | full-coarse | 0.815 | 0.691 | 3.1% | 0.763 |
| 1 | merge-only | 0.802 | 0.676 | 1.2% | 0.723 |
| 2 | full | 0.795 | 0.632 | 3.2% | 0.740 |
| 2 | full-coarse | 0.830 | 0.692 | 3.2% | 0.776 |
| 2 | merge-only | 0.801 | 0.643 | 1.3% | 0.730 |
| both | naming-only | 1.000 | 1.000 | 100% | 1.000 |

Agreement is pairwise F1 of one run scored against another, so it is the same
metric as accuracy with a run standing in for the gold standard.

**Refine agrees with itself about as much as it agrees with the truth.** On
fixture 2, accuracy 0.763 against self-agreement 0.795: the pass has essentially
reached its own noise floor, and nondeterminism — not capability — is now the
binding constraint. On fixture 1 accuracy is well *below* self-agreement, so
there the limit is judgement.

Two consequences for `gd-26r.8`:

- **Almost no file lands with exactly the same group-mates in every run** — 3.2%
  to 11.8% under `full`, and this is the number that moved most between five runs
  and ten. At five runs it read 21.5% and 27.3%; every extra run can only take
  files out of that set, and it kept taking them, which means the five-run figure
  was an artefact of a short corpus rather than a property of the pass. Not that
  every file moves — mean group-mate overlap is 0.72–0.79 — but essentially every
  file sees *some* churn at its group's edges across ten runs. Any state keyed on
  "the group a file is in" will be invalidated wholesale by a re-run.
- **Merge-only is by far the least stable per file**: 1.2% identical, despite
  respectable whole-partition agreement. Merging is all-or-nothing at group
  granularity, so one different merge decision relocates a whole group at once.
  Whole-partition agreement hides this; the per-file number is the one that
  matters for held state.

Voting helps, and it is deterministic given its inputs, but it is not the lever
the five-run corpus made it look like. Consensus of three lifts fixture 1 from
0.353 to 0.411 and fixture 2 from 0.763 to 0.847. Three votes is enough —
consensus of *ten* is no better on either fixture (0.408 and 0.845), so seven
more calls buy nothing. That is the one voting claim ten runs strengthened.

**Which three, though.** That 0.411 is one particular triple: the first three
recorded `full` runs. Ten runs admit 120 distinct triples, and `refine_report`
now scores all of them. **On fixture 1, 50 of the 120 clear the 0.394 bar and 70
do not.** The distribution runs 0.343 to 0.439 with an upper median of 0.369 —
the statistic `refine_report` prints under that name, the 61st of the 120 sorted
ascending — that is, **the middle three-call vote lands below the bar.** A shipped "three calls and a
vote" draws an arbitrary triple, so on fixture 1 consensus is a coin flip that
loses slightly more often than it wins, not a fix.

This is a correction, and it is the reason the corpus was doubled. The five-run
version of this document reported that seven of ten triples cleared the bar and
called the result provisional pending ten runs. At ten runs the honest figure is
42%, and the earlier 0.411 headline was a mildly favourable draw rather than a
typical one: 41 of the 120 triples score above it, so it ranks 42nd from the top,
above the 0.369 upper median and below the 0.439 best. `refine_report` prints
that rank beside the distribution, so it re-derives offline like every other
number here.

Fixture 2 is nowhere near its bar under any triple — 120 of 120 clear, minimum
0.640 against a bar of 0.378 — so nothing here qualifies that side of the split.
And on fixture 1 the shape whose triples do reliably clear is not `full` at all
but `merge-only`, at 107 of 120; see *The cheaper shapes*.

## The cheaper shapes

**Naming-only cannot move the metric.** Not "did not" — *cannot*. The scorer
ignores group names by design, so a shape that only renames scores exactly the
heuristic number on every run, on both fixtures, forever.
Five tests lock this between them. The *cannot* is structural, and adversarial
hand-written bodies show it: a naming-only answer that supplies a `files` array
interleaving two heuristic groups, one that claims an input group twice, one that
omits its `was` and one that names a group that does not exist all leave the
partition byte-identical and are counted as repairs
(`naming_only_refuses_a_files_array_that_moves_a_path` and its three siblings).
`every_recorded_naming_only_run_applied_to_the_heuristic_partition` then adds
that no committed run needed that machinery. It is also the one shape that is
perfectly stable and costs $0.12–0.22 for 20–40s.

This is a real limit on the answer, not a result: **naming-only is the shape this
harness is structurally blind to.** The heuristic group names are mechanical
token joins (`github-client-work-item-queries`), and the model's are readable
concern names. That may well be worth $0.15 on its own — the point of grouping is
a review order a human can follow — but nothing in these fixtures can say so. It
needs a different kind of evaluation.

**Merge-only clears the bar on both fixtures, and does it far more reliably than
`full` does on fixture 1**: mean 0.417 against 0.394 and 0.398 against 0.378, with
9 of 10 single calls and 107 of 120 triples over the bar on fixture 1, against
`full`'s 0 of 10 and 50 of 120. At $0.137 and 28s it is under a third of full's
cost. Its consensus is stable too — 0.438 at three votes, 0.448 at ten — which
retires the "erratic consensus" claim the five-run corpus produced (0.351 at five
votes was a short-corpus artefact).

**On fixture 1 alone, merge-only is the shape that should ship.** But it captures
almost none of the fixture-2 benefit — 0.398 against full's 0.763 — because the
win there comes from *re-cutting* groups the heuristics drew wrong, and merge-only
is constrained to coarsen — `apply` writes each claimed heuristic group whole
under one name, so the shape can only trade precision for recall, whatever a run
answers. +0.020 over the bar is not a result worth an API call and a
nondeterministic dependency.

So the cheap shapes do not capture most of the benefit. The benefit *is* the
splitting — where splitting helps at all.

## What the paths-only fixtures could not measure

Stated plainly, because it is a large hole:

- **Nothing here tested a pass that reads hunks.** Both fixtures are paths and
  status only, since the source repos are private. Every number above is for a
  model reading file paths, change kinds, and the heuristic grouping. Whether
  reading the diff would add to 0.763, and what that would cost, is unmeasured
  and cannot be measured on these fixtures at all.
- **The cost of a hunk-reading pass is out of reach anyway, on this evidence.**
  Fixture 1's diff is roughly 11.9k changed lines, on the order of 120–200k
  tokens. The top of that range reaches `claude-opus-5`'s context window before
  any output, and the bottom leaves little of it for anything else. A
  code-reading refine could not be relied on to fit in one call over a 161-file
  changeset; it would need chunking or summarisation, which is a different pass
  with a different design.
- **Naming quality is invisible here**, as above. It may be the largest
  user-visible benefit and it scores zero on this harness.
- **Reading order is invisible here, and unrepresented on the heuristic side.**
  The scorer is order-blind — it compares partitions, and a partition has no
  order — so no number in this document says whether either arm's order is good.
  The two arms are not even symmetrical about it: refine returns groups in a
  proposed reading order (`Refined.order`) and the report prints it, while the
  heuristic arm carries no order at all — `Grouping` holds a bag of assignments,
  `Partition.groups` is a name-keyed `BTreeMap`, and `report_fixture` prints by
  group size. So `docs/GROUPING.md` rules 2, 3 and 8 — intent-centrality
  ordering, drive-bys last, mechanical changes last — are scored on neither arm,
  and the heuristic baseline the refine numbers are compared against does not
  attempt them. This is a known gap tracked as `gd-26r.22`, not an oversight:
  order-aware scoring means an order on the heuristic side, an order metric, and
  ordered expectations in both fixtures, which is its own piece of work and is
  deliberately out of scope for `gd-26r.11`.
- **Two fixtures, one model, ten runs.** Both are single feature-branch PRs of
  ~160 files. Nothing here speaks to a 400-file changeset, a merge commit, a
  refactor sweep, or a cheaper model — `sonnet` resolves on this deployment and
  was not run.
- **Fixture 1's expected grouping is provisional** and was drawn with token
  evidence in view. It is the harsher of the two bars for a reason that is at
  least partly an artefact.

## Verdict

**Ship it, opt-in, as full refine with a consensus of three calls — on the
strength of one fixture, not two.**

- **Full, not the cheap shapes.** Merge-only is the better shape on fixture 1 and
  it still is not worth shipping for: +0.020 on fixture 2 against full's +0.385.
  Naming-only cannot move the metric by construction. The value is in re-cutting
  groups, which only full can do, and where re-cutting helps it is worth more than
  everything else combined.
- **Not `full-coarse`.** It does not clear fixture 1's bar either (mean 0.386, 58
  of 120 triples) and it sells fixture 2 to get there. It does not fix the actual
  disagreement.
- **Three calls, voted** — but for determinism, not for accuracy on fixture 1.
  Voting makes the result reproducible given its inputs and lifts fixture 2 to
  0.847, and ten votes add nothing over three. What it does *not* do is rescue
  fixture 1: no single call cleared that bar in ten attempts and only 50 of 120
  triples do, with the upper median triple at 0.369 under a 0.394 bar. Cost is roughly
  $1.40–2.00 and, run in parallel, about the wall clock of one `full` call —
  141.8s on fixture 1 and 190.6s on fixture 2.
  `voting_does_not_rescue_fixture_one` locks the negative half against the
  committed runs, so a future change that quietly fixes fixture 1 will fail a test
  and force this section to be rewritten.
- **Opt-in and async, degrading to heuristics-only**, exactly as `gd-26r.4`
  settled. The wall-clock numbers are the empirical case for it, not a
  re-litigation.
- **Honest summary of the accuracy claim:** on fixture 1 refine is slightly worse
  than doing nothing; on fixture 2 it doubles the score. Ten runs turned that from
  "roughly a wash on fixture 1" into a measured small loss, and it is the reason
  this must be offered rather than defaulted. Two changesets cannot say which of
  the two is typical, and until a third does, the pass is a bet the user takes
  knowingly.

`gd-26r.8` should assume group membership is not stable across re-runs: 3–12% of
files keep identical group-mates across ten runs, and held state keyed on group
identity will not survive a refresh.

# Cutting the price (`gd-26r.24`)

`gd-26r.11` shipped `full` refine at $0.47–0.66 and 141.8–190.6s per call, and
three voted calls at roughly $1.40–2.00. This ticket asks what that price is
actually buying and which levers move it. Every arm below is `full`, ten runs
per fixture, scored by the same harness against the same bars — 0.394 on fixture
1, 0.378 on fixture 2 — so the numbers sit beside `gd-26r.11`'s unchanged.

Arms are now reported and locked by the **file-name slug**, not by the model.
Two of the arms here answer as `claude-opus-5` exactly as the published one does
and differ only in reasoning effort, which no envelope field records, so the
model alone can no longer tell two experiments apart (`refine::arm_from_run_stem`,
and `a_recorded_runs_arm_is_everything_between_the_shape_and_the_run_number`).
`scripts/grouping-refine-runs.sh` grew `TUICR_REFINE_EFFORT` and
`TUICR_REFINE_SHAPES` to record them, and refuses an effort arm filed under the
published `opus` slug.

## First: the output is mostly reasoning, not answer

This was the gate on everything else, because the delta protocol — return only
the files that moved — is worth building only if the 15,793 output tokens are
mostly answer.

The CLI reports one `outputTokens` figure covering both. The answer is in the
envelope's `result`; the thinking blocks come back **encrypted** — an instrumented
`--output-format stream-json` run records a `thinking` block whose `thinking`
text is empty beside a 39,128-character `signature` — so the split cannot be read
off a stream. It was measured instead by sending each recorded answer back as
*input* behind a fixed carrier and taking the token delta, which counts it under
the model's own tokeniser. The delta appears in both `inputTokens` and
`cacheCreationInputTokens` and the two agree to within three tokens, which is
what makes the method trustworthy rather than a chars-per-token guess. Measured
directly for six answers, then applied as a per-fixture ratio (2.28 characters
per token on fixture 1, 1.99 on fixture 2) to all forty committed runs:

| fixture | shape | output | answer | reasoning | reasoning share |
| --- | --- | --- | --- | --- | --- |
| 1 | full | 15,793 | 4,476 | 11,317 | **71.7%** |
| 1 | full-coarse | 15,725 | 4,342 | 11,382 | 72.4% |
| 1 | merge-only | 2,373 | 339 | 2,034 | 85.7% |
| 1 | naming-only | 1,927 | 525 | 1,402 | 72.7% |
| 2 | full | 21,331 | 8,205 | 13,126 | **61.5%** |
| 2 | full-coarse | 19,597 | 8,239 | 11,358 | 58.0% |
| 2 | merge-only | 4,313 | 724 | 3,589 | 83.2% |
| 2 | naming-only | 3,726 | 1,127 | 2,599 | 69.8% |

**The bill is not the answer, it is the thinking about the answer.** The
supporting arithmetic on `gd-26r.24` read `full` as spending ~98 output tokens
per file against a path costing 15–25. The measured figure is **27.8 answer
tokens per file** on fixture 1 — a path, its quotes, its comma and its share of
the group scaffolding. The response format is already close to the floor a
paths-naming answer can reach.

### So the delta protocol is not worth building

Its ceiling is the whole answer: 4,476 tokens of 15,793 on fixture 1 (28.3%) and
8,205 of 21,331 on fixture 2 (38.5%). That is a delta protocol that returns
*nothing at all*, which is not a protocol. A realistic one — most files stay put,
so emit the movers — might halve the answer, for **~14% of output on fixture 1**.
Against that: it changes the agent-CLI response contract across a process
boundary and so needs `contract-approval` and `gd-26r.10`; it gives the model a
lossier thing to reason over, which `gd-26r.21` showed this engine is sensitive
to; and there is no reason to expect the *reasoning* — the other 72% — to shrink
at all, since the model still has to decide the whole partition before it can
say which parts of it moved.

**Verdict: measured and dropped.** No contract approval was sought, because
nothing here proposes to change the contract. The lever the split points at
instead costs one flag.

## The lever: reasoning effort

If reasoning is 72% of the bill, the knob that sets how much of it happens is the
lever. `MAX_THINKING_TOKENS` does nothing on this CLI — one run at `1024` and one
at `0` returned 16,013 and 18,264 output tokens, both inside the published
arm's ordinary spread. `claude --effort low` is the knob that works.

Recorded as the `opus-low` arm, ten runs per fixture, same prompt, same model:

| | fixture 1 default | fixture 1 **low** | fixture 2 default | fixture 2 **low** |
| --- | --- | --- | --- | --- |
| F1 mean | 0.353 | **0.450** | 0.763 | 0.705 |
| worst / best | 0.321 / 0.373 | 0.409 / 0.486 | 0.673 / 0.844 | 0.574 / 0.805 |
| single calls over bar | 0 of 10 | **10 of 10** | 10 of 10 | 10 of 10 |
| triples over bar | 50 of 120 | **93 of 120** | 120 of 120 | 120 of 120 |
| output tokens | 15,793 | 7,360 | 21,331 | 12,089 |
| of which reasoning | 11,317 (71.7%) | **3,400 (46.2%)** | 13,126 (61.5%) | 4,157 (34.4%) |
| cost | $0.472 | **$0.261** | $0.657 | **$0.426** |
| wall clock | 141.8s | **65.8s** | 190.6s | **102.7s** |

Reasoning falls 70% on fixture 1 and 68% on fixture 2 while the answer barely
moves (4,476 → 3,960 tokens; 8,205 → 7,932). The lever pulls exactly the term the
split identified, which is the strongest evidence that the split was measured
right.

**And on fixture 1 it does not cost accuracy — it buys it.** `gd-26r.11`'s
sharpest negative finding was that no single `full` call cleared fixture 1's bar
in ten attempts. At low effort **all ten clear it**, mean 0.450 against a 0.394
bar, worst run 0.409. The per-group table says why, and it is the same two groups
that were the whole loss:

| expected group | files | heuristic | default, mean of 10 | low, mean of 10 |
| --- | --- | --- | --- | --- |
| project-view | 28 | 0.75 | 0.33 | **0.66** |
| pr-actions | 29 | 0.45 | 0.28 | **0.44** |
| enterprise-host-routing | 18 | 0.20 | 0.26 | **0.49** |
| work-items | 8 | 0.39 | 0.43 | **0.75** |
| settings-repo-icon | 5 | 0.20 | 1.00 | 0.96 |
| github-client-plumbing | 38 | 0.05 | 0.15 | 0.18 |

Default effort returns 15–20 groups against 13 expected; low effort returns
10–14. **The fixture-1 loss was over-splitting, and thinking longer is what
caused it** — more reasoning means more second-guessing of a concern boundary the
heuristics and the human both drew coarser. That is a fixture-1 statement, and
fixture 2 pays 0.058 for the same coarsening; but fixture 2 sits at 1.9× its bar
either way, so the trade is one-sided.

Effort `medium` was probed once rather than recorded as an arm, because no
decision turns on it: on fixture 1 it returned 10,107 output tokens in 91.6s for
$0.330, between the two arms on every axis. `low` is the floor the CLI offers.

## The cheaper model: refuted, at both efforts

`gd-26r.11` left this untested — "sonnet resolves on this deployment and was not
run". It was run, twice, ten runs per fixture per arm.

| arm | fx1 F1 | fx1 cost | fx1 wall | fx2 F1 | fx2 cost | fx2 wall | output tokens (fx1/fx2) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| opus, default | 0.353 | $0.472 | 141.8s | 0.763 | $0.657 | 190.6s | 15,793 / 21,331 |
| **opus, low** | **0.450** | **$0.261** | **65.8s** | **0.705** | **$0.426** | **102.7s** | 7,360 / 12,089 |
| sonnet, default | 0.419 | $0.525 | 241.9s | 0.577 | $0.699 | 331.5s | 30,544 / 40,729 |
| sonnet, low | 0.431 | $0.205 | 66.9s | 0.528 | $0.393 | 138.5s | 9,227 / 19,911 |

**Sonnet at default effort is dearer and slower than opus at default effort**, on
both fixtures — $0.525 against $0.472 and 241.9s against 141.8s on fixture 1,
$0.699 against $0.657 and 331.5s against 190.6s on fixture 2 — while scoring
0.577 against 0.763 on fixture 2. It is cheaper per token and spends 1.9× as many
of them: 40,729 output tokens on fixture 2. The ~5× saving the ticket hoped for
is not merely absent, the sign is wrong. Its tail is worse than its mean, too:
one fixture-2 call was abandoned after 27 minutes and re-recorded, and the arm's
wall-clock spread is 239.6–551.0s against opus-low's 85.9–131.1s.

**Sonnet at low effort is dominated by opus at low effort.** It saves $0.056 and
$0.033 a call and gives up 0.019 F1 on fixture 1 and 0.177 on fixture 2, and on
fixture 2 it is also 36s *slower*. There is no axis on which it is the better buy.

**Verdict: the cheaper model is not a lever here; the effort knob is.** What the
two sonnet arms actually establish is the general form of the finding — on a
reasoning-dominated call, the price is set by how many tokens the model thinks
for, and picking a cheaper per-token model that thinks for more of them loses.

### Haiku loses the same way, harder

Two single probes on fixture 2 rather than arms of ten — enough to close the
question, not enough to publish an F1 against. `claude-haiku-4-5` costs a fifth
of opus per token and is the cheapest Claude on offer, so it is the strongest
form of the cheaper-model argument.

| probe (n=1) | F1 | cost | wall | output tokens | repairs |
| --- | --- | --- | --- | --- | --- |
| haiku, default | 0.494 | $0.339 | 355.0s | 61,927 | 5 |
| haiku, low | 0.378 | $0.170 | 176.2s | 29,196 | **315** |
| opus, low (arm of 10) | **0.705** | **$0.426** | **102.7s** | 12,089 | 0.0 |

At default effort haiku spends **61,927 output tokens** — 2.9× opus's 21,331 —
to arrive at 0.494, and takes 355s doing it. One probe against a ten-run mean is
not a like-for-like comparison, but the gap is not close and neither is the
direction: the cheapest model per token is 3.5× slower and costs 80% as much,
for two thirds the score. Low effort halves both and drops it **exactly onto the
bar**, 0.378, which is the same as failing.

The repair column is the part that would have decided it even had the F1 held.
The harness repairs an answer that misspells or invents a path rather than
scoring it as a parse failure; opus and sonnet need this ~0 times a call, and
low-effort haiku needed it **315 times in one call** — it stopped reproducing
input paths faithfully and started approximating them. An answer that has to be
repaired 315 times is not a grouping of this changeset.

This is the third model to lose the same way, which is what makes it a finding
rather than three results: **on a reasoning-dominated call, per-token price is
nearly irrelevant, because a weaker model closes the capability gap by thinking
longer and pays back the discount with interest** — in wall clock first, then in
money. Ranked by fixture-2 output tokens: opus 21,331, sonnet 40,729, haiku
61,927. That ordering is the whole result.

## Another vendor: Gemini 3 over Vertex AI

Every arm above answers through the Claude Code CLI, so every arm above shares
one model family and one transport. Two Gemini 3 models were recorded through
Vertex AI directly — ten runs per fixture, `full` only, same prompts, same
scoring — which is the first evidence here that separates *this pass is hard*
from *this vendor is expensive*.

The mechanism is `scripts/grouping-refine-runs-vertex.sh`, which posts the same
emitted prompt to `publishers/google/models/<m>:generateContent` and writes an
envelope carrying the raw response, the measured wall clock, and the prices the
cost column is derived from. Vertex reports tokens but not money, so the price
table is in that script with its source and the date it was read; a published
cost column is only as good as its source. `refine::RunRecord::parse` reads both
envelope shapes, so these arms score beside the CLI ones with no other change.

Both were run at the `low` thinking level — the floor Gemini 3 offers, matching
the `--effort low` the Claude arms use.

| arm | fx1 F1 | fx1 over bar | fx1 cost | fx1 wall | fx2 F1 | fx2 over bar | fx2 cost | fx2 wall |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| opus, low (CLI) | 0.450 | 10/10 | $0.261 | 65.8s | **0.705** | 10/10 | $0.426 | 102.7s |
| gemini-3.1-pro, low | **0.492** | 10/10 | $0.070 | 30.6s | 0.550 | 8/10 | $0.095 | 43.9s |
| gemini-3-flash, low | 0.452 | 9/10 | **$0.019** | **30.6s** | 0.585 | 10/10 | **$0.022** | **38.2s** |

Output tokens are where the price comes from, and it is the same story the
sonnet and haiku arms tell, running the other way: 5,172 and 5,553 on fixture 1
against opus-low's 7,360, and 7,310 and 6,797 on fixture 2 against 12,089.
**Gemini at its lowest thinking level is not a cheaper model spending more
tokens; it is a cheaper model spending fewer.** That is what makes flash 13.7×
cheaper on fixture 1 and 19.4× cheaper on fixture 2, and both models roughly 2×
faster on wall clock.

What they buy and what they cost:

- **Fixture 1 they win outright.** `gemini-3.1-pro` posts 0.492, the best mean
  of any arm recorded on this fixture, above opus-low's 0.450 and 1.25× the bar,
  with 10 of 10 single calls clearing and no repairs. Flash matches opus-low's
  mean at a fifteenth of the price.
- **Fixture 2 they give up real accuracy.** 0.550 and 0.585 against opus-low's
  0.705 — a loss of 0.12–0.16 F1, ~20% of the score. Both still clear a bar of
  0.378 comfortably; flash clears it on all ten calls, pro on eight.
- **Pro's tail is the worry, not its mean.** Its fixture-2 spread is 0.353–0.716
  and two calls land under the bar; the two failures are the same failure, a
  collapse into a handful of huge groups (precision 0.223, largest group 51.9%
  of the changeset). Flash's worst call on that fixture is 0.512, comfortably
  clear. **On this evidence flash is the more dependable of the two despite
  being the weaker and cheaper model**, and pro's higher fixture-1 mean is not
  worth its fixture-2 tail.
- **Flash invents paths occasionally**: 0.5 repairs a call on fixture 1, 0.2 on
  fixture 2 — a handful of hallucinated filenames the harness drops. Two orders
  of magnitude below low-effort haiku's 315, and low enough not to bear on the
  verdict, but not zero the way every Claude arm is.

**Vertex caches implicitly, with no cache-control markers to set.** The
fixture-2 Gemini runs report 3,259 and 2,852 `cachedContentTokenCount` against
~6,200 total input — roughly half the prompt read from cache, at a tenth the
input rate, without the harness asking. Fixture 1's prompt is smaller and cached
nothing. This is a difference in kind from the CLI transport, where caching is
prefix-based and the harness defeated it (next section).

**Verdict: `gemini-3-flash` is the recommended shape**, on the human's ruling
that speed is critical and some grouping inaccuracy acceptable. It is the fastest
arm recorded on both fixtures and the only lever that moves cost by an order of
magnitude rather than a factor of two. It is not free — 0.126 F1 on fixture 2,
one fixture-1 call in ten under the bar, and a repair rate no Claude arm has —
and those costs are set out where the recommendation is made below, together with
the runner-up to fall back to if they show up in use.

`gemini-3.1-pro` is not recommended despite the better fixture-1 mean: it is
slower than flash on fixture 2, 4× the price, and its two sub-bar calls are the
collapse failure above. On this evidence the weaker, cheaper model is the more
dependable one.

## Prompt caching: real, and defeated by the harness, not by the CLI

`cacheReadInputTokens` is zero in all forty committed fixture-1 envelopes, and
zero again in all ten `opus-low` fixture-2 runs, which send a byte-identical
prompt one after another. That reads as caching being off.

It is not. Three identical calls issued from **one stable working directory**:

| call | input | cache write | cache read | cost |
| --- | --- | --- | --- | --- |
| 1 | 5,408 | 8,099 | 0 | $0.0853 |
| 2 | 5,408 | 0 | **8,099** | $0.0387 |
| 3 | 5,408 | 0 | **8,099** | $0.0396 |

The recorder runs every call from a fresh `mktemp -d` — deliberately, so the
agent cannot read the repo it is grouping — and the working directory is part of
the CLI's system preamble, so every call presents a different prefix and writes
the cache afresh. **The zero is an artefact of the measurement harness, not a
property of the shipped pass**, which will run from a stable directory.

The gain is small and awkwardly shaped. Priced from the table above, a warm
prefix turns 8,069 cache-write tokens into cache reads and saves **$0.046 a
call** — 17.8% of `opus-low`'s $0.261 on fixture 1, and nothing at all on the
first call, which is the only call a single-call refine makes. It pays only
across repeats of the same prefix, and for a voted triple it pays only if the
second and third calls are issued *after* the first has written — which is
exactly the serialisation that makes a triple cost three calls' wall clock
instead of one.

**Verdict: real, free, and worth taking where it falls out; not a lever.** The
one actionable part is that the shipped pass should call from a stable directory
rather than reproduce the harness's scratch-directory isolation.

## The transport itself: the agent CLI is half the bill

Every Claude arm above was recorded through the Claude Code CLI, and the Gemini
arms through Vertex, so vendor and transport moved together and neither result
could be attributed. `claude-opus-5` answers on both, so recording the same
model, the same prompt and the same effort level both ways separates them. Ten
runs per fixture per arm, `full` only.

| | fx1 F1 | fx1 out tok | fx1 cost | fx1 wall | fx2 F1 | fx2 out tok | fx2 cost | fx2 wall |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| CLI, default effort | 0.353 | 15,793 | $0.472 | 141.8s | 0.763 | 21,331 | $0.657 | 190.6s |
| Vertex, default effort | 0.421 | 11,994 | $0.324 | 107.0s | **0.785** | 16,948 | $0.469 | 147.2s |
| CLI, low effort | **0.450** | 7,360 | $0.261 | 65.8s | 0.705 | 12,089 | $0.426 | 102.7s |
| **Vertex, low effort** | 0.427 | **4,271** | **$0.131** | **36.6s** | 0.711 | **7,697** | **$0.237** | **60.0s** |

**Same model, same effort, same prompt: the CLI costs 2.0× and 1.8× as much and
takes 1.8× and 1.7× as long, and buys nothing that shows above run-to-run
noise.** Across all four pairings the
transport is worth roughly a factor of two on both money and wall clock — a
larger, more reliable saving than the effort lever, and the only lever here whose
accuracy cost is smaller than the arm's own run-to-run spread.

Two things drive it, and only one is the obvious one.

**Input.** The CLI sends 5,408 input plus 8,099 cache-write tokens on fixture 1;
Vertex sends 4,864 and nothing else. The ~8.6K difference is the CLI's system
preamble — tool descriptions, environment, harness instructions — which `--tools
''` and `--setting-sources ''` reduce but do not remove, because the preamble is
the CLI's, not the project's.

**Output, which is the surprise.** The same model at the same effort emits
**4,271 output tokens on Vertex against the CLI's 7,360** — 42% fewer — and
7,697 against 12,089 on fixture 2. Output is reasoning-dominated (71.7%, first
section), so this is the model *thinking substantially less for the same
question*. The prompt is byte-identical; the only difference is the preamble
wrapped around it. Carrying an agent harness's framing into a pure
text-transformation call apparently invites the model to deliberate like an
agent. That is a hypothesis, not a measurement — what is measured is the token
count, and it is not close.

The accuracy column is the part that makes this a free lever rather than a
trade. At default effort Vertex is **better on both fixtures** (0.421 against
0.353, 0.785 against 0.763) — and 0.785 is the highest fixture-2 mean of any arm
recorded. At low effort it is a wash: 0.427 against 0.450 on fixture 1, 0.711
against 0.705 on fixture 2, with 10 of 10 single calls over the bar on both
fixtures either way. The fixture-1 gap of 0.023 runs the CLI's way and is real
but small — a fifth of what a single CLI call varies by across its own ten runs
(0.409–0.486), and reversed in sign on fixture 2. Nothing here says the CLI's
extra tokens buy anything that survives the noise.

**Verdict: on this evidence the shipped pass should call the provider directly,
not shell out to an agent CLI.** The recommended shape is a Google model, so it
requires this anyway; what this section adds is that the transport pays for
itself even if the vendor decision is reversed. That is `gd-26r.13`'s question,
not this
ticket's — `gd-26r.13` is open and unclaimed, and it is where "shelling out to
an agent CLI from Rust" gets decided. This section is the input it was missing:
the transport is not a free implementation detail, it is a 2× tax on both
figures this ticket exists to cut. It also carries costs this harness cannot
price — auth (the CLI carries a subscription, Vertex needs a GCP project and
credentials), and the loss of whatever the CLI provides for free. Those belong
to `gd-26r.13`.

## Consensus of three: the evidence, not the decision

`gd-26r.11` bought three calls for determinism rather than accuracy. `gd-26r.8`
then made grouping a persisted artefact, so the human sees one refine result and
keeps it, which is the assumption that argument rested on. This document does not
settle whether that is still worth 3×; it records what voting is now worth.

On the recommended `opus-low` arm, voting does not help accuracy on either
fixture, and on fixture 1 it **hurts**:

| | fixture 1 | fixture 2 |
| --- | --- | --- |
| single call, mean of 10 | **0.450** | **0.705** |
| every triple, mean of 120 | 0.417 | 0.708 |
| every triple, upper median | 0.422 | 0.722 |
| first triple (`consensus of 3`) | 0.411 | 0.604 |
| single calls over bar | 10 of 10 | 10 of 10 |
| triples over bar | 93 of 120 | 120 of 120 |

The mechanism is the mirror of the effort finding: consensus keeps only pairs a
majority of runs agree on, so it coarsens where runs disagree and re-introduces
some of the fragmentation low effort removed. On fixture 1 the average triple
scores 0.033 *below* the average single call, and 27 of 120 triples fall under a
bar every single call clears.

So the price of determinism is now three calls, $0.78 against $0.26, and a small
accuracy loss — where under `gd-26r.11`'s default-effort arm it was three calls
for a real fixture-2 gain (0.763 → 0.847).

It is not an artefact of the CLI transport either. The same model at the same
effort over Vertex says the same thing: fixture-1 triples average 0.422 against
0.427 for single calls, with 111 of 120 clearing a bar all ten singles clear;
fixture-2 triples average 0.717 against 0.711, a gain of 0.006 for 3× the money.

**Decided: no vote, one call.** The human ruled speed first, and a little
accuracy a fine price for it. Voting loses on both counts. A parallel triple does
not cost one call's wall clock, it costs the *slowest* of three, so on a fixture
spanning 85.9-131.1s it lands near the top of that range every time instead of
the middle — a latency tax even when concurrent, and a worse one if the calls are
serialised to catch the prompt cache. And on this arm the 3x buys no accuracy to
weigh against that. What is left is determinism, which `gd-26r.8` already
supplies by persisting the grouping: the human sees one result and keeps it.
`gd-26r.11`'s consensus of three is superseded — it was a default-effort finding,
and at low effort it holds on neither fixture.

## The heuristics are load-bearing: the cold control

Every shape above hands the model the heuristic grouping and asks it to improve
on it, so every number above measures the *pair*. None of them can say whether
the heuristics are helping the call or merely anchoring it — a model that would
have done as well from a bare file list is being credited for work it did not
need. `cold` is the control that separates them: same rules, same paths, same
change statuses, no starting grouping, "produce one".

Ten runs, both fixtures, on the recommended arm and on its runner-up. `full` is
the same arm with the heuristic grouping restored to the prompt.

| | fixture 1 `cold` | fixture 1 `full` | fixture 2 `cold` | fixture 2 `full` |
| --- | --- | --- | --- | --- |
| bar | 0.394 | 0.394 | 0.378 | 0.378 |
| `gemini-3-flash` F1 mean | 0.371 | **0.452** | 0.420 | **0.585** |
| `gemini-3-flash` over bar | **1 of 10** | 9 of 10 | 7 of 10 | 10 of 10 |
| `opus-low` F1 mean | 0.445 | 0.427 | 0.428 | **0.711** |
| `opus-low` over bar | 8 of 10 | 10 of 10 | 10 of 10 | 10 of 10 |

**The seed is worth more than the model.** Three of the four pairings lose
without it, and the fixture-2 losses are not close: flash drops 0.165 and
opus-low drops 0.283, which is larger than any gap this document has measured
between two models, two efforts or two transports. Flash on fixture 1 falls from
nine calls over the bar to one — the recommended arm, run cold, is a call not
worth making.

The one cell that improves is `opus-low` on fixture 1, 0.427 to 0.445, and it
buys that mean by widening: the cold spread is 0.314-0.550 against a warm arm
where every call cleared the bar, so two of ten cold calls now land under it. A
higher mean that clears the bar less often is the trade this document has
refused everywhere else, and refuses here.

**The failure mode is the one already on file.** On fixture 2 both cold arms
collapse into lumping — precision 0.16-0.31 against recall 0.80-0.91, largest
group 41-64% of the changeset, against 13-20% for the same arms warm. That is
exactly the attractor `gemini-3.1-pro` fell into on two of its ten *warm* runs:
grouping by directory subtree instead of by concern, rule 1's named failure.
Cold, both models fall into it on nearly every run. The heuristic grouping's
real contribution is not the answer it supplies but the shape it forbids — shown
thirteen concern-sized groups, a model revises them; shown a bare list, it
reaches for the file tree.

`opus-low` cold on fixture 2 is the clearest picture of it: run-to-run agreement
0.956, F1 pinned in a 0.420-0.443 band, the same lumped answer ten times. It is
the most *stable* arm in this document and one of the least accurate. Stability
here is a model converging on the wrong structure, which is worth remembering
before reading any agreement figure as quality.

There is no compensating saving. The cold prompt is ~5% shorter, but the answer
is longer — nothing is inherited, so every group is written out from nothing —
and the two roughly cancel: flash fixture 1 is $0.021 cold against $0.019 warm,
`opus-low` fixture 2 $0.230 against $0.237, wall clock within a few seconds
either way on all four. The seed is free.

**So the heuristics stay in the prompt.** The refine pass is a revision pass by
evidence and not just by design, and `gd-26r.11`'s decision to seed it was doing
real work that nothing recorded before this control could see.

Two caveats on the control itself. `apply` still restores a dropped path to its
heuristic group, so the heuristics are a safety net even here — the repair
counts say how much that mattered, and at 0.0-0.3 per call it did not. And the
prompt was written once, not tuned; a cold prompt built to be a cold prompt
might close some of the gap. It would have to close a lot of it.

## Recommended shape

**`full`, `gemini-3-flash` at low thinking, called at Vertex directly, one call,
no vote.**

| | fixture 1 | fixture 2 |
| --- | --- | --- |
| bar | 0.394 | 0.378 |
| F1 mean of 10 | 0.452 | 0.585 |
| F1 worst of 10 | 0.345 | 0.512 |
| single calls over bar | 9 of 10 | 10 of 10 |
| cost per call | **$0.019** | **$0.022** |
| wall clock | **30.6s** | **38.2s** |
| repairs per call | 0.5 | 0.2 |

Four changes from what `gd-26r.11` shipped, each measured separately above: drop
the vote, drop the effort to the floor, drop the agent CLI, and change vendor.
Against three default-effort CLI calls at ~$1.42 and ~$1.97 a run, this is **75×
and 90× cheaper** and **4.6× and 5.0× faster** than the single call a voted
triple's wall clock is bounded by.

**This shape was chosen on a stated priority, not on the numbers alone.** The
human ruled speed critical and some grouping inaccuracy acceptable, twice, with
the fixture-2 cost on the table. What that buys over the runner-up is 6.0s on
fixture 1 and 21.8s on fixture 2 — a third off the slower fixture — plus a
tenfold cost drop that is not what the ruling was about.

What it gives up, stated where the recommendation is made rather than in a
footnote:

- **0.126 F1 on fixture 2** against the runner-up, ~18% of the score. It clears
  the 0.378 bar on all ten calls and beats every heuristic-only baseline, so the
  pass is still worth running; it is further from the ceiling than it needs to be.
- **One call in ten under the bar on fixture 1** (0.345 against 0.394). No other
  recommended-tier arm does that. The failure is over-splitting — 20 groups
  against 13 expected — so the bad case is a reader seeing a changeset cut too
  finely, not a scrambled one.
- **Invented paths.** 0.5 repairs a call on fixture 1, 0.2 on fixture 2: flash
  occasionally emits a filename that is not in the changeset and the harness
  drops it. Every Claude arm is flat zero. Two orders of magnitude below
  low-effort haiku's 315, and nowhere near the "this is not a grouping of this
  changeset" threshold, but the shipped pass needs the same repair step this
  harness has, and a file dropped by repair is a file that lands in no group.
- **A second vendor, on a preview model id.** `gemini-3-flash-preview` carries no
  stability promise; the price was read on one day; nothing here measures quota
  or rate limits.

**The runner-up, and when to take it instead.** `claude-opus-5` at low effort on
the same transport: F1 **0.427 and 0.711**, $0.131 and $0.237, **36.6s and
60.0s**, 10 of 10 over both bars, zero repairs, and the tightest spread in the
document (31.0–44.6s and 58.4–69.1s). It is the accuracy-per-second pick and it
is barely slower. **If the fixture-2 gap or the repair behaviour shows up in real
use, switch to it and lose ~20s** — nothing else about the shape changes, since
both run through the same recorder and the same envelope. Keeping the agent CLI
instead is a further step back again: `full`, `--effort low`, one call, at
$0.261/65.8s and $0.426/102.7s for F1 0.450 and 0.705.

### What actually ships: the runner-up, and no effort knob

The recommendation above stands as written — it is what the numbers say under the
stated priority. **The shipped default is the runner-up**: `claude-opus-5` at low
effort over Vertex. That is a human call made after the measurements, weighing the
0.126 fixture-2 gap, the invented paths and the preview model id against 20
seconds, and it is recorded here rather than folded into the recommendation so the
evidence and the ruling stay separable.

**The model is configurable; reasoning effort is not.** The model is a config knob
defaulting to `claude-opus-5`, so switching to flash — or to whatever is cheapest
in six months — is a config change and not a code change. Effort is pinned at low
with no knob. Low is not universally better, and the exposed-knob case would rest
on that: default effort buys **+0.074 on fixture 2** (0.785 against 0.711). It
loses fixture 1 outright — **0.421 against 0.427 with 8 of 10 over the bar against
10 of 10** — at **2.5× the cost and 2.9× the wall clock**. A knob whose other
setting is slower, dearer, and better on one fixture of two is a knob nobody can
be told how to set, so there is one setting and the document says why.

### The wall-clock number, and how firm it is

**31s on fixture 1 and 38s on fixture 2**, and the figure `gd-26r.14` should plan
against is **roughly 30–70 seconds for a ~160-file changeset**. That range covers
the recommended arm and the runner-up together, deliberately: it does not move if
the vendor decision is revisited. **If `gd-26r.13` keeps the agent CLI it becomes
60–130s**, which is the conservative number to design against while that is open.

Firm parts: ten serial calls per fixture, standard tier, `global` location, taken
the same way and on the same machine as every other number here.

The recommended arm is *not* the tightest arm, and that matters more than its
mean here. Flash spans **21.5–41.8s on fixture 1 and 31.5–65.8s on fixture 2** —
a slowest call roughly 2× its mean — where the runner-up spans 31.0–69.1s across
both fixtures with no outlier. Buying the mean-case 20s costs some of the
predictability, and 30–70s holds for the recommendation only as a typical case,
not a bound.

Soft parts, stated plainly. This is two changesets of ~160 files each, one model,
one week, one network, one region. Wall clock tracks output volume at a
near-constant generation rate, so a changeset needing a bigger answer takes
proportionally longer, and nothing here measures what 400 files does. The sonnet
arm is the warning: a 27-minute call happened, on the same harness, inside a
corpus whose mean was 5 minutes. Treat 30–70s as the shape of the interruption to
design for and not as a bound.

## What this does not answer

- **Whether low effort transfers.** Two fixtures again. Low effort wins fixture 1
  by removing over-splitting and loses 0.058 on fixture 2 for the same reason; a
  third changeset whose true grouping is *finer* than the heuristics' would be
  where this trade goes the other way, and neither fixture is that.
- **Whether the effort knob is stable to depend on.** `--effort` is a CLI flag,
  not part of any contract this repo owns, and its levels are not specified
  anywhere the harness can pin. A future CLI that reinterprets `low` moves every
  number in this section, and nothing here would fail. The direct-transport arms
  use the same knob under another name — `output_config.effort` alongside
  `thinking: {type: adaptive}`, which is what `claude-opus-5` accepts in place of
  a token budget — so switching transport does not escape this.
- **Why the CLI makes the model think 1.7× longer.** Measured on four arms and
  consistent across both fixtures and both effort levels, but the *cause* is
  inferred from one difference (the system preamble) between two otherwise
  identical calls. The number is solid; the story about agent framing inviting
  agent-shaped deliberation is a guess, and a CLI release that trims its preamble
  would change the number without warning.
- **What the recommended vendor costs outside the F1 column.** The Gemini arms
  are priced from a table read on one day, run in one location, on preview model
  ids (`gemini-3-flash-preview`, `gemini-3.1-pro-preview`) that carry no
  stability promise at all. Nothing here measures rate limits, quota, or what
  happens when a preview id is withdrawn — and the recommendation now rests on
  one of those ids. The runner-up exists partly for that reason.
- **Whether flash's fixture-2 gap is the kind a reader notices.** 0.585 against
  0.711 is 0.126 of pairwise co-membership F1, and no number here converts that
  into "how much worse the groups feel". The recommendation was made on a stated
  preference for speed over exactly this, so it is the assumption most worth
  revisiting once someone has used the pass in anger.
- **Naming quality, reading order, and hunk-reading** remain exactly as
  `gd-26r.11` left them — invisible to this harness. Low effort produces fewer,
  coarser groups, which a reader may like more or less than the default arm's
  finer ones, and no number here can say which.
- **Whether a cheaper shape is now faster still.** `merge-only` ran 27.9s and
  56.9s through the CLI at default effort, and has never been run at low effort
  or over the direct transport or on a Google model — each of which roughly
  halves wall clock, so the recommended `full` arm has now overtaken it on both
  fixtures. It stays untested because `gd-26r.24` was told not to retreat to it,
  and `gd-26r.11` rejected it on accuracy — 0.398 against `full`'s
  0.763 on fixture 2 — and that rejection stands on default-effort evidence.
  Low effort moved `full` in a direction nobody predicted, so the low-effort
  merge-only number is not knowable from here. Twenty calls would settle it, and
  it is worth spending them only if wall clock turns out to bind.
- **`gd-26r.11`'s ship verdict is untouched.** This section prices the pass; it
  does not re-open whether to offer it. What it does change is the fixture-1
  half of the accuracy claim, which the human should read before that verdict is
  next relied on: "on fixture 1 refine is slightly worse than doing nothing" is a
  default-effort statement, and at low effort refine beats the heuristics on
  fixture 1 too.

# Scoring order (`gd-26r.22`)

Every number above this line is **order-blind**. Pairwise F1 asks "are these two
files in the same group?" and nothing else, so a grouping that partitions
perfectly and presents the groups back-to-front scores 1.000. This section adds
a second metric that scores order, and reports both fixtures on it.

**Nothing above this line moved.** The order metric is additive: `Partition` and
`score_against` are untouched, and `Partition::parse` now delegates to a new
`parse_with_order` that returns the same partition plus the header order, so
every published F1 is computed by the same code as before. The one fixture edit
— fixture 2's lockfile group moved to last, below — is an order-only change to a
`.groups` file and does not move a single co-membership pair.
`recorded_numbers_hold` still passes.

## The bead's premise, corrected first

`gd-26r.22` was written when the heuristic arm carried no order at all, and says
so. As a statement about the *design* that is dead: `gd-26r.12` Decision 3 gave
the heuristic arm a reading order, and `gd-26r.10` made reading order the `Vec`
position in `Grouping` rather than a parallel field, so there is nothing left to
desync (`docs/GROUPS_CONTRACT.md`, `docs/TOTAL_COVERAGE.md`).

It was still true of the *harness*, which keyed a partition by group name in a
`BTreeMap` and threw the order away on both arms. That is ~30 lines of debt, now
paid. So the comparability complaint survives only as work, not as a finding: the
arms are comparable on order by design, and the real question is the better one —
**does the heuristic order hold up when scored?**

## The metric: Kendall's tau over the induced file sequence

A grouping induces one sequence of files: groups in reading order, files within
each group in listed order. Total coverage guarantees both arms induce a
permutation of the *same* set, so no matching step is needed and two sequences
can be compared directly.

**Kendall's tau** is the fraction of file pairs ordered the same way in both
sequences, rescaled to `[-1, 1]`: `(concordant - discordant) / pairs`. 1.000 is
the ceiling, 0.000 is chance, −1.000 is exactly reversed. The raw range is kept
rather than squashed to `[0, 1]` precisely so that "no better than chance" and
"backwards" read differently.

It is reported as **two** numbers over disjoint pair universes:

- **`tau_group`** — pairs of files the *expected* grouping puts in different
  groups. This scores group order.
- **`tau_within`** — pairs inside one expected group, and only those a rule in
  `docs/GROUPING.md` actually constrains. This scores file order.

Those two universes are exactly the split pairwise F1 already makes. F1 asks
whether a pair is together; tau asks whether a pair is in the right order. **The
two measure different failures and tau does not replace F1** — a partition can be
perfect and reversed, or well-ordered and wrong.

Why tau and not a displacement score (sum of "how far did each group move"):
displacement is dominated by group size and needs a normaliser nobody can defend,
and it cannot distinguish a swapped adjacent pair from a rotation. Tau degrades
in the way the brief asked for, and the report prints the checks:

| perturbation of the expected order | fixture 1 | fixture 2 |
| --- | --- | --- |
| none (self-score, the ceiling) | 1.000 | 1.000 |
| last group moved to first | 0.943 | 0.945 |
| all groups reversed | −1.000 | −1.000 |

One group moved is a small dent; reversed is the floor. That is the property the
metric was chosen for.

**Only the rules score.** `tau_within` counts a pair only where a numbered rule
relates the two files — a test and the production file it covers, a mechanical
file and a non-mechanical one, the group's central file and the rest. Pairs no
rule speaks to carry no intent and are excluded rather than scored against
whatever the fixture happened to type. Per-rule tau is printed beside the pooled
number so a rule the fixtures cannot exhibit is visible as such.

## The bias, stated once here and at every number below

**`tau_group` flatters refine by construction.** The refine prompt asks in words
for a reading order and quotes rule 2 at the model; the heuristic arm's order is
`gd-26r.12` Decision 3's admitted proxy — concern groups by size descending,
`dir:` groups pinned last as a class — which was never claimed to be
intent-centrality. An order metric compares something optimised for against
something approximated. Every `tau_group` figure below carries that caveat, and
the harness prints it above every order block rather than relying on this
paragraph being read.

**`tau_within` is not biased that way.** Neither arm is told anything about
within-group file order: the prompt does not mention it, and rules 12–14
deliberately do not reach the model (`every_documented_rule_reaches_the_model`
says why). Both arms get it from the same place — a path-keyed map — so
`tau_within` compares two things that were equally uninstructed.

## Is the fixtures' expected order meaningful?

Asked of the human rather than assumed, because the answer decides whether any of
the numbers mean anything.

**Fixture 1: yes, on both axes.** Its `.groups` file carries a header comment
saying the groups are in reading order, the order it lists is not
size-descending or alphabetical, and its mechanical-ish group is last. Within
groups, all 40 production/test pairs are adjacent with the test second. It was
authored to the rule.

**Fixture 2: no, as authored.** No header claimed an order, the lockfile group
was listed *first*, and the within-group file order is provably a plain path sort
in 14 of 14 groups. **The human ruled fixture 1's convention normative for both
fixtures**, so the fixture was corrected: `[dependencies]` moved to last per rule
8, and a header comment added stating the order is meaningful. The partition is
untouched, so no F1 above moves. Its within-group order is *not* corrected and
is not claimed to be truth — which is why fixture 2's `tau_within` ceiling is
below 1.000 and is reported as such.

## Fixture 1 (orca, 161 files, 11,118 cross-group pairs)

*`tau_group` flatters refine — see the bias above.*

| arm | `tau_group` | `tau_within` |
| --- | --- | --- |
| **heuristic passes (Decision 3 order)** | **−0.192** | **−0.960** |
| top-level-directory baseline | +0.145 | −0.960 |
| heuristics + a within-group sort | −0.192 | **+0.800** (the ceiling) |
| expected (self-score) | 1.000 | +0.800 |

Refine arms, `tau_group` mean of 10 runs, heuristic arm at −0.192:

| arm | mean | worst | best |
| --- | --- | --- | --- |
| cold · vertex-opus-low | **+0.223** | +0.162 | +0.341 |
| full · opus (default effort, CLI) | +0.217 | +0.108 | +0.377 |
| full-coarse · opus | +0.174 | +0.122 | +0.228 |
| full · vertex-opus-default | +0.159 | +0.076 | +0.249 |
| full · opus-low | +0.136 | +0.021 | +0.284 |
| cold · gemini-3-flash-low | +0.112 | −0.003 | +0.199 |
| full · gemini-3-1-pro-low | +0.086 | −0.021 | +0.169 |
| **full · vertex-opus-low (what ships)** | **+0.044** | −0.047 | +0.140 |
| merge-only · opus | +0.008 | −0.070 | +0.057 |
| naming-only · opus | +0.001 | −0.032 | +0.041 |
| full · gemini-3-flash-low | −0.011 | −0.079 | +0.058 |
| full · sonnet | −0.080 | −0.187 | +0.071 |
| full · sonnet-low | −0.089 | −0.183 | −0.021 |

## Fixture 2 (meridian, 158 files, 11,377 cross-group pairs)

Present and scored: read from `$TUICR_GROUPING_FIXTURES` (default
`~/.local/share/tuicr-fixtures`), which was populated on this machine. Had it
been absent this section would say so rather than quietly report one fixture.

*`tau_group` flatters refine — see the bias above.*

| arm | `tau_group` | `tau_within` |
| --- | --- | --- |
| **heuristic passes (Decision 3 order)** | **+0.080** | −0.667 |
| top-level-directory baseline | −0.451 | −0.333 |
| heuristics + a within-group sort | +0.077 | −0.667 |
| expected (self-score) | 1.000 | −0.333 (**not** 1.000 — see below) |

Refine arms, `tau_group` mean of 10 runs (haiku rows are 1 run), heuristic arm at
+0.080:

| arm | mean | worst | best |
| --- | --- | --- | --- |
| full · gemini-3-1-pro-low | **+0.443** | +0.250 | +0.559 |
| cold · vertex-opus-low | +0.437 | +0.424 | +0.478 |
| full · vertex-opus-default | +0.408 | +0.129 | +0.556 |
| full · opus (default effort, CLI) | +0.389 | +0.222 | +0.569 |
| cold · gemini-3-flash-low | +0.352 | +0.112 | +0.529 |
| full · opus-low | +0.336 | +0.160 | +0.517 |
| **full · vertex-opus-low (what ships)** | **+0.335** | +0.198 | +0.498 |
| naming-only · opus | +0.303 | +0.262 | +0.382 |
| full-coarse · opus | +0.280 | +0.132 | +0.437 |
| full · sonnet-low | +0.236 | −0.136 | +0.401 |
| full · gemini-3-flash-low | +0.238 | +0.064 | +0.457 |
| full · haiku (1 run) | +0.212 | — | — |
| merge-only · opus | +0.205 | +0.122 | +0.261 |
| full · sonnet | +0.184 | +0.054 | +0.297 |
| full · haiku-low (1 run) | −0.146 | — | — |

The within-group sort moves fixture 2's `tau_group` by −0.003. That is not a bug:
`tau_group`'s universe is pairs in different *expected* groups, and two such
files can share a *computed* group, so reordering inside a computed group does
move a few cross-group pairs.

## What the numbers say

**1. The heuristic group order is not adequate.** −0.192 on fixture 1 is *worse
than chance* — a reader given those groups back-to-front would do better — and
+0.080 on fixture 2 is chance with a rounding error. Size-descending is not
intent-centrality, and now there is a number saying so rather than an admission
in a decision record.

**2. The size proxy is beaten by `dirname` on one fixture and beats it on the
other**, which is the same non-transfer the partition work kept finding:
top-level-directory scores +0.145 against the heuristics' −0.192 on fixture 1,
and −0.451 against +0.080 on fixture 2. Neither ordering heuristic is a heuristic
about intent; each is accidentally aligned with one repo's shape.

**3. Both arms score −1.000 on rule 4 on fixture 1 — a perfect inversion.** Of
42 constrained production/test pairs, **zero** are ordered correctly by either
arm, because both build a group from a path-keyed `BTreeMap` and `x.test.ts`
sorts before `x.ts`. Every test in the changeset preceded the code it covered.
The rule has been in `docs/GROUPING.md` since the first draft and nothing
implemented it, because nothing measured it.

**4. A deterministic sort fixes that for free.** Ordering each group by
`(mechanical last, directory, filename stem, production before test, unit test
before broad test, path)` takes rule 4 from −1.000 to **+1.000**, 42 of 42, and
`tau_within` from −0.960 to +0.800 — the fixture's own ceiling. No model call, no
prompt change, no cost. This is the largest single order improvement in the
document and the cheapest.

**5. Refine's order gain is real, modest, and one-fixture.** Under the bias
stated above, the shipped arm gains +0.236 on fixture 1 (−0.192 to +0.044, which
is still indistinguishable from chance) and +0.255 on fixture 2 (+0.080 to
+0.335). The best arm anywhere reaches +0.223 and +0.443. So a model does order
groups better than size-descending, on both fixtures, in the direction the metric
is biased toward — and on fixture 1 the result of paying for it is *chance*.

**6. Order is the one thing the cheap shapes are good at.** `naming-only`
returns the bar exactly on F1 by construction — it cannot move the partition —
and yet scores `tau_group` +0.303 on fixture 2 against the heuristics' +0.080,
close to `full · opus-low`'s +0.336 at a third of the cost and a quarter of the
wall clock. On fixture 1 it manages +0.001, so this does not transfer either. It
is nonetheless the only arm in this document whose entire contribution is order,
and `gd-26r.11` rejected it on an order-blind metric.

**7. Two rules cannot be scored, and the report says so rather than hiding it.**
`central-first` scores −0.250 on fixture 1 and −0.294 on fixture 2 against the
fixtures' *own* expected file order, and `mechanical-last` scores −1.000 on
fixture 2's single constrained pair. The ground truth contradicts the rule, so no
arm is graded on either. Fixture 1 contributes no `mechanical-last` pair (nothing
mechanical) and fixture 2 no `test-follows-production` pair (its .NET test
naming, `FooTests.cs` under a `*.Tests/` project, is not what `is_test` matches —
left alone deliberately, since `is_test` feeds the heuristic passes and changing
it would move published F1).

## Does this challenge `gd-26r.14`?

`gd-26r.14` ruled that refine blocks the whole TUI at startup, and the
load-bearing reason was ordering: the heuristic arm produced no intent-centrality,
so an instant open would show groups in no meaningful order.

**Its premise is confirmed, and its conclusion is challenged anyway.** Stated
plainly rather than softened:

- The heuristic order really is no better than chance — −0.192 and +0.080. That
  half of `gd-26r.14` was right, and is now measured instead of asserted.
- But the thing blocking buys is **+0.044 on fixture 1** — chance, after a
  30–70s wait — and +0.335 on fixture 2. Blocking the TUI for a gain that is
  reliably present on one of two changesets is a different trade than the one
  `gd-26r.14` was ruled on.
- And the sharpest ordering defect in the product is not group order at all:
  **every test file preceded its production file**, on both arms, and a free
  deterministic sort fixes it completely. `gd-26r.14` blocks startup for a
  model call that does not address the failure a reader would notice first.

Not reopened here. A bead is filed against `gd-26r.14` carrying these numbers;
the ruling is the human's to revisit.

## What this does not answer

- **Two fixtures, again.** Every transfer failure in this document repeats in the
  order numbers: the heuristics' order beats `dirname` on one fixture and loses
  on the other, `naming-only` is strong on one and inert on the other.
- **Whether refine can order files within a group.** Not measured, and currently
  not measurable: `apply` rebuilds the partition from a path-keyed map, so any
  within-group order the model returned is discarded before scoring. That is why
  rules 12–14 are kept out of the prompt and why `tau_within` is unbiased. Making
  the answer format carry file order is a change to the refine round-trip, not to
  this harness.
- **Whether tau matches what a reader feels.** It scores pairs, and a reader
  reads a list. A group misplaced by one position and a group misplaced by six
  differ by a lot of pairs and possibly by very little annoyance.
- **`central-first` and `mechanical-last`.** Written down, normative, unscored
  because neither fixture exhibits them. A third fixture authored to the rule
  would settle both; nothing here should be read as evidence for or against
  either rule.

# The within-group sort, and the blocking ruling (`gd-26r.27`)

`gd-26r.22` measured order, found evidence against `gd-26r.14`'s blocking
startup ruling, and was not allowed to reopen it. This section is the review
that was handed up instead. It ships the sort `gd-26r.22` costed, re-scores
every arm on both fixtures with the sort applied, and rules on blocking.

Fixture 2 is present and scored. `$TUICR_GROUPING_FIXTURES` was unset, so it was
read from the default `~/.local/share/tuicr-fixtures`; had neither been
populated this section would say so rather than quietly report one fixture.

## What shipped: the sort is the arm, not a variant of it

`gd-26r.22` left the sort as a row in a report. It is now how a computed
grouping is produced: `Ordered::heuristic` and `Ordered::refined` both apply it,
`Ordered::new` is documented as ground-truth-and-controls only, and
`a_presented_grouping_is_sorted_by_construction` pins that there is no
constructor for a computed grouping that skips it. A caller cannot forget the
step, because there is no step to forget.

It implements `docs/GROUPING.md` rules 4, 12, 13 and 14 against the written
specification rather than an invented one. Key, outermost first: **central file
first** (rule 13), **mechanical stragglers last** (rule 14), **broad tests after
everything narrower** (rule 12), then directory, then stem — which is what puts
a production file next to its test, since `stem` strips the test marker — then
production before test (rule 4), then path for a total and stable order.

Rule 12 is a **band across the group**, not a nudge within one stem: the rule
says integration and end-to-end tests follow the unit tests, not their own unit
test. Neither fixture exhibits it, so
`broad_tests_sort_after_every_unit_test_in_the_group` is the only place it is
checked.

Rule 13 was **priced, not assumed**, because it is the one within-group rule
both fixtures' own file order contradicts. The price is zero: with it and
without it, fixture 1 scores `tau_within` +0.800 and `central-first` −0.250, and
fixture 2 is identical on every number. It ships **on** anyway, by ruling: the
rule is normative in `docs/GROUPING.md`, and a sort implementing only the
measurable subset of a written spec is one nobody can read the spec to check.
Its `central-first` score remains **unpublished as an arm's grade** — the metric
derives "central" from the same function, so scoring it would be largely
self-grading, and both fixtures contradict the rule regardless.

## What the sort bought, on both fixtures

*`tau_group` flatters refine — see the bias in `gd-26r.22` above. `tau_within`
is unbiased: neither arm is prompted for within-group file order, and since this
ticket neither arm produces it — the engine does, identically for both.*

**Fixture 1 (orca, 161 files):**

| arm | `tau_group` | `tau_within` | rule 4 |
| --- | --- | --- | --- |
| heuristics, no within-group sort (`gd-26r.22`) | −0.192 | −0.960 | **−1.000** (0/42) |
| **heuristics as shipped** (rules 4, 12, 13, 14) | −0.191 | **+0.800** | **+1.000** (42/42) |
| the same without rule 13 (what rule 13 is worth) | −0.192 | +0.800 | +1.000 |
| expected (self-score, the ceiling) | 1.000 | +0.800 | +1.000 |

**Fixture 2 (meridian, 158 files):**

| arm | `tau_group` | `tau_within` | rule 4 |
| --- | --- | --- | --- |
| heuristics, no within-group sort (`gd-26r.22`) | +0.080 | −0.667 | — |
| **heuristics as shipped** | +0.077 | −0.667 | — |
| the same without rule 13 | +0.077 | −0.667 | — |
| expected (self-score) | 1.000 | −0.333 | — |

So the sort reaches fixture 1's own within-group ceiling with no model call, and
**does nothing measurable on fixture 2** — which is not a failure of the sort but
of the fixture: fixture 2 contributes **zero** rule-4 pairs, because its .NET
test naming (`FooTests.cs` under a `*.Tests/` project) is not what `is_test`
matches, left alone deliberately since `is_test` feeds the heuristic passes and
changing it would move published F1. Fixture 2's only constrained within-group
pairs are 17 `central-first` and 1 `mechanical-last`, both unscored rules. **The
largest order improvement in this document is therefore also a one-fixture
result**, and the honest reading is that it is free rather than that it is
proven twice.

## Every published number still holds

The sort moves `tau_group` slightly, because two files the *expected* grouping
separates can share a *computed* group. All 28 recorded arm/fixture pairs were
re-scored with it. The largest move anywhere is **0.009** (`cold ·
vertex-opus-low` on fixture 2, +0.437 → +0.428); the median is 0.001. The
shipped arm is **+0.044 → +0.042** on fixture 1 and **+0.335 → +0.335** on
fixture 2. (**Amended by `gd-o7s`, below**: tokenising the group name the way a
filename is tokenised moves the shipped arm's fixture-1 as-shipped figure to
**+0.043**. Everything else in this paragraph stands.) No bar moves, no verdict above changes, and `recorded_numbers_hold`
passes untouched. The report prints both columns — `published` and `as shipped`
— so no number here is a quiet restatement.

## The ruling on blocking: it stands, on narrower grounds

**`gd-26r.14`'s conclusion survives. One of its three supports does not.**

The premise is **confirmed by measurement**, which is the half `gd-26r.22`
already conceded: heuristic group order is −0.192 on fixture 1 and +0.080 on
fixture 2 — chance or worse — so an instant open really does show groups in no
meaningful order. That was an assertion when `gd-26r.14` was ruled and is now a
number.

**The free sort does not change the answer, and the argument that it might is
answered rather than waved off.** `gd-26r.22` called the rule-4 inversion "the
failure a reader would notice first". It is not: the first screen is the
collapsed 13-row overview (`docs/SIDEBAR_MODEL.md`), which shows **group rows**.
Within-group file order is invisible until a group is expanded. So the sort
repairs what a reader sees *second*, and what they see first is still −0.192.
Fixing the second defect for free is a reason to fix it, not a reason to stop
paying for the first.

**The cost side is weaker than `gd-26r.14` knew, and this is recorded rather
than buried.** Under the pro-refine bias, the shipped arm (full ·
vertex-opus-low) buys **+0.042 on fixture 1 — +0.043 since `gd-o7s`,
indistinguishable from chance either way —
after a 30–70s block**, and **+0.335 on fixture 2**. A reader on a
fixture-1-shaped changeset waits a minute for nothing measurable, and that is
one changeset in the two measured. What carries the ruling despite that is the
part that was never disputed: partition quality favours refine on **both**
fixtures, 0.427 and 0.711 against the heuristics' 0.394 and 0.378. The order
argument was `gd-26r.14`'s stated reason, and it has weakened; the partition
argument was always true and is not relitigated here.

**`naming-only` was priced, not dismissed.** It scores `tau_group` +0.302 on
fixture 2 against the heuristics' +0.080 — close to the shipped arm's +0.335 —
at roughly \$0.12–0.22 and 20–40s, and −0.001 on fixture 1. But it **cannot move
the partition at all**, by construction, so adopting it at startup returns F1 to
0.394 and 0.378 and forfeits the undisputed half of the case to save 10–30
seconds of a once-per-review wait. Rejected as a startup arm on that trade, not
on its order number, which is good.

### What is amended in `gd-26r.14`

**Struck.** Its third bullet — "No within-group file order… Tests come first, by
accident" — is no longer a reason to block. It described an unimplemented rule,
not a property of the heuristic arm, and it is fixed for free on both arms.

**Narrowed.** "The heuristic arm cannot produce a reading order at all" is now
"the heuristic arm cannot produce a meaningful **group** order", measured at
−0.192 and +0.080. That is what blocking buys and all it buys.

**Improved, in `gd-26r.14`'s favour.** Its escape hatches — cancel key, timeout,
`refine = false` — now land on a grouping that at least reads production before
test, at fixture 1's within-group ceiling. The fallback is strictly better than
the one it was ruled against, which makes blocking easier to live with, not
harder to justify.

**Unamended.** The landing policy for a grouping result arriving under a reader,
the collapse-all-on-full-regroup rule, `:regroup` staying async, and the six
settlements for `gd-26r.10` all stand as decided, on their own merits.

### The instant open, and the async landing path

The bead's third item was conditional on blocking being relaxed, and asked what
an instant open shows and whether `gd-26r.14`'s async landing path covers it
unchanged. **Blocking is not relaxed, so no async startup path is designed, and
`gd-26r.14`'s landing path is not touched.**

An instant open remains reachable three ways, all of them already specified and
all of them landing in the same place: the cancel key, the timeout, and
`refine = false` (`docs/MID_SESSION_REGROUP.md`, "The escape" and "The
configuration surface"). What it shows is now better than when that section was
written — the heuristic grouping with the within-group sort applied — and what
it gives up is the group order and the partition gain, nothing else. Because
those three paths all produce `gd-26r.15`'s *refine unavailable* state with
`source = heuristics` and apply nothing partial, no landing occurs under a
reader at startup at all: there is no result still in flight to land.

The async landing path therefore continues to govern exactly the two cases it
was decided for — a requested `:regroup` and the heuristics-only auto backstop —
under the two rules stated there: all groups collapse with selection on the
group row containing the current file, and every expanded hunk gap is discarded
(`docs/MID_SESSION_REGROUP.md`, "What a landing does"). Unchanged, and
deliberately not redesigned.

### What this settles for `gd-26r.28`'s first slice

- **Startup blocks.** The first vertical slice builds the pre-TUI progress
  screen with its minimal event read, cancel key and timeout, exactly as
  `docs/MID_SESSION_REGROUP.md` specifies. No async startup path is needed.
- **The within-group sort is engine code, not refine code, and lands in the
  same slice.** It runs on every grouping the engine produces, before any model
  call and on the fallback path, so a cancelled or unavailable refine still
  yields a correctly ordered group. It is pure, deterministic, needs no
  configuration and has no failure mode.
- **The refine response still owes a group order** (`gd-26r.10` settlement 2,
  unamended) and still owes **nothing** about within-group file order. The
  engine's sort is authoritative there on both arms, which is why rules 12–14
  stay out of the prompt.
- **No new configuration.** `[grouping].refine` plus the timeout key is the whole
  surface; the sort adds nothing to `KNOWN_KEYS`.

## What this does not answer

- **Whether the block is worth it on a third changeset.** The order gain is
  reliably present on one fixture of two, and every transfer failure in this
  document repeats here. The ruling rests on the partition gain, which does
  transfer, plus a confirmed premise about the alternative.
- **Whether the sort helps a repo whose tests `is_test` does not match.** On
  fixture 2 it demonstrably does not, and fixing `is_test` for .NET naming would
  move published F1, so it is deliberately not attempted here. (**Corrected by
  `gd-26r.29`, below**: measured, that fix moves no published F1 cell on either
  fixture — and it still buys zero rule-4 pairs, because `is_test` is one of
  three gates and not the binding one.)
- **Whether rule 13 is right.** It ships because the spec says so, and both
  fixtures contradict it. A third fixture authored to the rule would settle it;
  nothing here is evidence either way.

# Guarding the order numbers (`gd-26r.29`)

Every tau figure above this line was **measured, published, and unguarded**.
`recorded_numbers_hold` locked F1, precision, recall, largest share and group
count; nothing asserted on order at all. So the numbers in the two sections
above lived in prose, and a change that regressed rule 4 from +1.000 back
toward −1.000 on fixture 1 — the exact defect `gd-26r.22` found and `gd-26r.27`
fixed — would have kept the whole suite green. `gd-26r.27` re-scored all 28
arm/fixture pairs by hand for that reason; this section makes the part that can
be automatic automatic.

Fixture 2 is present and scored. `$TUICR_GROUPING_FIXTURES` was unset, so it was
read from the default `~/.local/share/tuicr-fixtures`; had neither been
populated this section would say so rather than quietly report one fixture.

## What is barred, and on what axis

| bar | fixture | what it locks | kind |
| --- | --- | --- | --- |
| `recorded_order_numbers_hold` | 1 | rule 4 at +1.000 over **42** pairs, `tau_within` at **+0.800** | quality, unbiased |
| `the_published_heuristic_group_order_still_reproduces` | 1 and 2 | heuristic `tau_group` at **−0.191** / **+0.077** | reproducibility only |
| `the_second_fixture_still_constrains_no_rule_four_pair` | 2 | fixture 2 constrains **zero** rule-4 pairs | premise of every one-fixture caveat here |

**One fixture, one axis, on the only bar that is a quality claim.** The
`tau_within` bar is fixture 1 only, and nothing below should be read as order
coverage beyond it. That is not an oversight to be tidied later; the two
sections after this one say exactly why it cannot honestly be widened yet.

The `tau_within` bar locks the **pair count** as well as the score, because a
rule that quietly stopped matching would score `None`, or +1.000 over two pairs,
and read as green. It also asserts the arm against the fixture's own self-score
rather than against the constant alone, so +0.800 stays a *ceiling* claim if the
fixture's expected order is ever re-authored, instead of silently becoming a
shortfall the constant hides.

## The ruling on `tau_group`: a bar, and it is not a quality bar

`tau_group` **flatters refine by construction** and the harness prints that
above every order block, so a green bar on it risks reading as a quality
statement the number cannot support. It gets a bar anyway, narrowed to the one
shape where the bias does not apply, and the assertion message carries the
distinction rather than leaving it to this document:

- **The heuristic arm gets a bar, on both fixtures.** The bias is a claim about
  *comparing* the arms. The heuristic arm's own number compared against nothing
  is just a published row, and a row that reproduces is worth guarding: it is
  the only thing standing between "someone wrecked group order" and a green
  suite.
- **It is two-sided, and it fails if the order gets **better**.** −0.191 is
  worse than chance and nobody wants to preserve it. The bar is not there to
  defend the number; it is there so an improvement gets *re-recorded* rather
  than quietly falsifying the published row — the same rule
  `recorded_numbers_hold` already applies to F1, where a one-sided check would
  pass a pass that improved as readily as one that regressed and leave the
  document wrong either way.
- **No refine arm gets a bar.** Two independent reasons, either sufficient: the
  bias is a between-arm claim and that is exactly what a refine bar would
  assert, and every refine arm here is a replay of a recorded run, so a bar on
  one would guard the recording rather than the engine. What the metric itself
  must do is already pinned by `one_group_moved_scores_far_above_a_reversal`,
  `a_shuffled_order_sits_near_zero` and the self-score check.

## Step 2 is deferred, and the obstacle is not the one the ticket named

The remaining work — tau bars spanning both fixtures on the within-group axis —
is **not done**, and the reason is measured rather than asserted.

`gd-26r.27` recorded that fixture 2 contributes zero rule-4 pairs because
`FooTests.cs` is not what `is_test` matches, and left `is_test` alone on the
grounds that it feeds the heuristic passes and changing it would move published
F1. Both halves of that turn out to be wrong, in opposite directions, and both
were checked by experiment.

**`is_test` is one of three gates, and it is not the binding one.** Rule 4
requires `is_test`, **stem equality**, and **locality** — same directory, or a
`__tests__` directory directly beneath it. Fixture 2's .NET pairs fail all
three: `ScPricerTests.cs` lives in
`.../rcm/tests/PACF.Rcm.Rules.Tests/` while `ScPricer.cs` lives in
`.../rcm/src/PACF.Rcm.Rules/`, so the directories differ, and `stem()` strips
dot-separated markers rather than a camel-case `Tests` suffix. Teaching
`is_test` the .NET convention and stopping there was measured: it produces
**zero** rule-4 pairs on fixture 2, exactly as before.

**And it does not move published F1.** The same experiment re-scored every
recorded arm and fixture pair. Drift, in full:

| what | before | after |
| --- | --- | --- |
| fixture 1, every published cell (F1, P, R, groups, largest, every ablation and sweep) | — | **identical** |
| fixture 2, heuristic default config | 0.378 / P 0.425 / R 0.340 / 29 groups | **identical** |
| fixture 2, off-default sweep cell `ubiquity >= 0.05` | 0.295, 48 groups | 0.289, 49 groups |
| `tau_group`, all 28 arm/fixture pairs, means | — | **unchanged** |
| `tau_group`, three fixture-2 arms, one worst/best endpoint each | 0.052 / 0.522 / 0.415 | 0.051 / 0.521 / 0.414 |
| fixture 2, rule-4 pairs | 0 | **0** |
| fixture 2, `tau_within` (and its ceiling) | −0.667 over 18 pairs (ceiling −0.333) | −1.000 over 5 pairs (ceiling −0.200) |

So the surgery is cheap on the partition — the fear that gated it is refuted —
and buys nothing on order, while **destroying 13 of fixture 2's 18 constrained
pairs**, because the newly-recognised tests drop out of the `central-first`
candidate set. It is not attempted, and the correction to `gd-26r.27`'s stated
reason is recorded here rather than left standing.

**The deeper reason a fixture-2 within-group bar cannot be honest yet.** Even
with all three gates taught the .NET convention, there would be nothing to grade
against: `gd-26r.22` established that fixture 2's within-group file order is a
plain path sort in 14 of 14 groups and is explicitly **not** claimed as ground
truth, which is why its `tau_within` ceiling is below 1.000. Its only
constrained pairs today are 17 `central-first` and 1 `mechanical-last`, both
rules this document refuses to grade an arm on. A bar there would lock in an
accident of how the fixture was typed and dress it as coverage.

**So the bars stand on one fixture until `gd-26r.19` lands.** That ticket —
harvested ground truth, cross-repo, ruled on during real reviews — is what
supplies a second changeset whose within-group order is authored to the rule.
It does *not* subsume the `is_test` defect, which is engine code and will
resurface on any .NET repo in the harvest; that defect is real, now priced at
zero F1 drift, and belongs to whoever next changes the pass set rather than to
a ticket about guarding numbers.

## What `gd-26r.28` must not break

The guards are written to survive the move of the sort out of test code and into
`src/`, and the move must keep these four properties:

1. **A constructor for a presented grouping that applies the sort.** Every bar
   here runs through `Ordered::heuristic`, not through the sort function, so it
   keeps measuring the arm rather than the helper. Whatever replaces it in
   `src/` must still be the *only* way to obtain a computed grouping, as
   `a_presented_grouping_is_sorted_by_construction` requires.
2. **The scorer stays reachable from the fixture harness.**
   `order::score_order` and `order::within_group_constraints` are the
   measurement, and they are independent of where the engine lives.
3. **The per-rule breakdown, not just the pooled number.** The rule-4 pair count
   and the fixture-2 absence bar both read `within_by_rule`. A move that reported
   only pooled `tau_within` would make both unenforceable.
4. **Rule 4's three gates unchanged in meaning** — `is_test`, stem equality and
   locality. Changing any of them is a change to what the published +1.000 over
   42 pairs *claims*, so it re-records the row; it is not a refactor.

# Tokenising the group name (`gd-o7s`)

Rule 13 compares a group's name against its members' names. Until this ticket
the two sides were not tokenised alike: `central_file` split the **name** on
`-`, `_`, `/`, `:` and space and lowercased it, while the **file** side went
through `split_tokens` — separator split, camel-case split, lowercase,
singularise, test markers dropped. So `work-items` offered `work`/`items`
against `workItems.ts`'s `work`/`item` and shared **no** token at all, and a
plural-only name matched nothing. Rule 13 stood down on groups it names exactly,
silently, and the sort fell through to its directory-and-stem key.

`gd-26r.32` found this and approved it as-is, correctly: the fix changes scoring
and that ticket was bound to move no published figure. This ticket is the other
half — the fix **and** the re-measure, recorded here deliberately.

The fix is one expression. `split_tokens` is now `pub` and `central_file` runs
the name through it, after a split on `:` — the `dir:` fallback prefix's
namespace separator, which is not a character the file tokeniser needs to know
about and not one a filename carries.

## What moved

The whole corpus was re-run: both fixtures, every baseline, ablation and sweep,
all 28 recorded refine arm/fixture pairs, and both order sections.

| figure | before | after |
| --- | --- | --- |
| heuristic F1, fixtures 1 / 2 | 0.394 / 0.378 | **unchanged** |
| refined F1, fixtures 1 / 2 | 0.427 / 0.711 | **unchanged** |
| heuristic `tau_group` as shipped, fixtures 1 / 2 | −0.191 / +0.077 | **unchanged** |
| rule 4, fixture 1 | +1.000 over 42 pairs | **unchanged** |
| `tau_within`, fixture 1 | +0.800 (at the fixture's ceiling) | **unchanged** |
| `tau_within`, fixture 2 | −0.667 over 18 pairs | **unchanged** |
| what rule 13 is worth, both fixtures | zero | **still zero** |
| every published cell in every baseline, ablation and sweep | — | **identical** |
| refine arms, `tau_group` **as shipped** | 11 of 28 pairs move | see below |

Eleven refine arm/fixture pairs move one endpoint each, in **both** directions,
none by more than **0.002**:

| arm | fixture | figure | before | after |
| --- | --- | --- | --- | --- |
| **full · vertex-opus-low (what ships)** | 1 | mean / worst | +0.042 / −0.050 | **+0.043 / −0.048** |
| full · vertex-opus-default | 1 | worst / best | +0.075 / +0.247 | +0.074 / +0.249 |
| full · gemini-3-flash-low | 1 | mean / worst / best | −0.010 / −0.078 / +0.060 | −0.011 / −0.079 / +0.059 |
| full · opus | 1 | worst / best | +0.105 / +0.376 | +0.106 / +0.375 |
| full · opus-low | 1 | mean | +0.135 | +0.136 |
| full · sonnet | 1 | mean | −0.081 | −0.080 |
| full-coarse · opus | 1 | best | +0.228 | +0.230 |
| cold · gemini-3-flash-low | 1 | mean | +0.117 | +0.118 |
| cold · vertex-opus-low | 1 | mean | +0.227 | +0.226 |
| merge-only · opus | 2 | mean | +0.201 | +0.202 |
| cold · vertex-opus-low | 2 | best | +0.470 | +0.469 |

The `published` column — the refine order as returned, before the within-group
sort — is untouched everywhere, because it is not this sort's output.

## Why so little moved, which is the finding

Three reasons, and each is a claim about the corpus rather than about the fix:

1. **F1 cannot move.** It is a partition metric. Rule 13 is an order key; it
   decides which file leads a group, never which group a file is in. "Possibly
   the refined F1" was the ticket's expectation and it is refuted by
   construction, not by luck.
2. **Fixture 1's heuristic group names are derived from its filenames'
   tokens**, so both sides already reduced alike — **0 of 19** groups change
   central file. Every fixture-1 heuristic figure is therefore bit-identical,
   including the two the bars guard.
3. **Fixture 2 changes exactly 2 of 29 groups**, both `dir:` fallbacks whose
   directory leaf is plural and now singularises to meet its members:
   `dir:…/app/pages/admissions` now leads with `admission-list.tsx`, and
   `dir:…/app/shared/stores` with `tab-store.ts` — each previously had no
   central file at all. Neither is an *expected* group, so no constrained
   within-group pair is affected and `tau_within` holds; what moves is those
   files' positions in the flattened sequence, which is the whole of the
   ≤0.002 refine drift above.

The refine arms move at all because a **model-named** group is exactly the case
the asymmetry hurt: `github-repository-identity` against
`githubRepositoryIdentity.ts`. Those arms are replays, so this is the engine
re-sorting a recorded partition, not the model answering differently.

## Where it did get worse, and the ruling

Five of the eleven moves are the wrong way, each by 0.001: `full ·
gemini-3-flash-low` on fixture 1 at all three endpoints (its whole row is
worse), `cold · vertex-opus-low` on fixture 1 at the mean (+0.227 → +0.226) and
on fixture 2 at `best` (+0.470 → +0.469), `full · opus` on fixture 1 at `best`,
and `full · vertex-opus-default` on fixture 1 at `worst`. That is reported as a
result, not smoothed: the fix is not free everywhere, and nothing else was tuned
to net it out.

**It still lands.** The moves are noise against a metric whose own bias warning
is printed above every block, they run both ways, and the shipped arm is among
the ones that gain. What the fix buys is not a number: rule 13 is normative in
`docs/GROUPING.md`, it ships **on** by `gd-26r.27`'s ruling precisely so a
reader can check the sort against the written spec, and a comparison that
tokenises one side and not the other does not implement the rule it claims to.
The corpus says the correction is cheap; the spec says it is required. Both
fixtures happening to name their groups the way their files are named is a fact
about two changesets, not a reason to leave the comparison lopsided for the
third.

## Blast radius

`central_file` only, plus `split_tokens` becoming `pub` within the crate. No
pass, no key, no config, no partition: the sole caller is `sort_group_files`,
which runs identically on the heuristic, refined, cancelled, timed-out and
`refine = false` paths. Three tests in `src/grouping/order.rs` pin the fix —
camel-case, plural, and the `dir:` prefix — and every bar in the table above is
the existing suite, unwidened.
# The size caps sit after every number above (`gd-26r.23`, built in `gd-26r.38`)

`grouping::caps::enforce` runs at the end of both full passes and splits any
group over the hard cap for the arm that produced it — 25 refined, 30 heuristic
— by directory. **No figure in this document was re-measured, and none needed
to be**, because the harness scores `passes::assign`'s claims and the caps run
after them: `tests/grouping/arms.rs` buckets the raw claims, so every F1,
precision, recall and tau above still describes exactly the code it described
before. What changed is that those numbers now describe the passes rather than
the partition the sidebar shows.

The gap between the two, measured on fixture 1 (orca, 161 files) with
`GroupingConfig::default()`:

| | groups | largest group |
| --- | --- | --- |
| heuristic claims, what every number above scores | 19 | 36 (22.4%) |
| what the sidebar shows, caps applied | 25 | 20 (12.4%) |

Nothing on fixture 1 comes out `unbounded`: every over-cap group had a
directory boundary to cut on, so the one split pass bounded all of them. The
pieces are named `parent · suffix` — `project · github-project`,
`project · tasks` — which is why those names now appear in the refine
prompt's rendering of the heuristic grouping.

**Two deliberate breaks with the recorded refine runs**, both pinned by
`the_shipped_prompt_is_the_prompt_the_numbers_were_measured_on` rather than
waived:

1. The prompt now carries `refine::size_guidance()`: the soft cap of 20 and the
   instruction that the changeset's size sets the granularity. No recorded run
   was sent it, so **the refine figures — F1 0.427 and 0.711 — predate it**.
2. The heuristic grouping the prompt renders is the capped one, because the
   heuristic cap runs inside `group_changeset`.

Both are `gd-26r.23`'s decision, not drift, and the parity test splices them in
so anything *else* that diverges still fails. Re-measuring the refine arm needs
fresh recorded runs; the human's judgment on real reviews is what that decision
named as its evidence, and pairwise F1 is structurally blind to group size in
any case — `gd-26r.11`'s full-coarse arm traded fixture 1 against fixture 2
rather than winning, which is what confirmed the blind spot.
