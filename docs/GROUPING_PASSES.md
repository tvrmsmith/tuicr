# Heuristic grouping passes: findings

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
than a case to reconcile afterwards — and `a_rename_is_one_file_in_one_group`
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

So the pass is nondeterministic but its *evidence* is not: the twenty orca
envelopes are committed under `tests/fixtures/grouping/refine/`, so **every
fixture-1 number below can be re-derived offline with no API key**. The twenty
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
coin flip rather than a fix — 50 of 120 triples clear, and the median triple
scores 0.369 against the 0.394 bar. Refine is not uniformly better than the
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
model toward fewer, larger groups without naming a number, to avoid fitting the
prompt to the answers. It lifts fixture 1 to 0.386 — still under the bar — and
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
fewer files than fixture 1 and emits a third more output on `full`. Input is
small and nearly flat, which is why a 100–400 file changeset stays affordable.

Two and a half to three and a quarter minutes per call, on a review the user is
waiting to start, is the empirical confirmation of `gd-26r.4`: this cannot be
synchronous.
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

- **Almost no file lands with exactly the same group-mates in every run** — 3.1%
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
do not.** The distribution runs 0.343 to 0.439 with a median of 0.369 — that is,
**the median three-call vote lands below the bar.** A shipped "three calls and a
vote" draws an arbitrary triple, so on fixture 1 consensus is a coin flip that
loses slightly more often than it wins, not a fix.

This is a correction, and it is the reason the corpus was doubled. The five-run
version of this document reported that seven of ten triples cleared the bar and
called the result provisional pending ten runs. At ten runs the honest figure is
42%, and the earlier 0.411 headline was a favourable draw: it sits in the top
quintile of the 120.

Fixture 2 is nowhere near its bar under any triple — 120 of 120 clear, minimum
0.640 against a bar of 0.378 — so nothing here qualifies that side of the split.
And on fixture 1 the shape whose triples do reliably clear is not `full` at all
but `merge-only`, at 107 of 120; see *The cheaper shapes*.

## The cheaper shapes

**Naming-only cannot move the metric.** Not "did not" — *cannot*. The scorer
ignores group names by design, so a shape that only renames scores exactly the
heuristic number on every run, on both fixtures, forever.
`naming_only_cannot_move_the_metric` locks this. It is also the one shape that is
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
is constrained to coarsen (`merge_only_can_only_coarsen` locks that it can only
trade precision for recall). +0.020 over the bar is not a result worth an API
call and a nondeterministic dependency.

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
  tokens, which is at or past `claude-opus-5`'s context window before any output.
  A code-reading refine could not be one call over a 161-file changeset; it would
  need chunking or summarisation, which is a different pass with a different
  design.
- **Naming quality is invisible here**, as above. It may be the largest
  user-visible benefit and it scores zero on this harness.
- **Reading order is invisible here.** Refine returns groups in a proposed
  reading order (rule 2, intent-centrality) and the report prints it, but the
  scorer is
  order-blind, so no number in this document says whether the order is good.
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
  triples do, with the median triple at 0.369 under a 0.394 bar. Cost is roughly
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
