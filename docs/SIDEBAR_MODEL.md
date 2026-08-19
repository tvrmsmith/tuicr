# Sidebar model: groups and directories

Design record for `gd-26r.7` on the wayfinder map `gd-26r` (grouped review of
large changesets in tuicr). Prototype ticket: the candidates were drawn as
ASCII mockups against a real changeset and reacted to, rather than argued in
prose.

**Revised by `gd-26r.31`.** The original record put a directory subtree inside
each group and made the tree mode orthogonal to grouping. That is reversed:
inside a group there are no directory rows at all. The revision is marked
inline where it changes what this document said, and its reasoning is
[below](#revision-gd-26r31-no-directory-rows-inside-a-group).

Assumed, settled elsewhere: grouping is **file-level with a strict partition**
(`gd-26r.5`) — every file in exactly one named group. Group naming and
membership rules are `docs/GROUPING.md`. The code seams are the seam map from
`gd-26r.6`.

## The fixture the mockups are drawn against

`tests/fixtures/grouping/orca-971b16754` — a real 161-file changeset, hand-
grouped into 13 groups in intent-centrality order. Sidebar rendered at 38
columns of inner width (the file panel is 20% of a 200-column terminal) and 46
visible rows (full terminal height).

The thing being judged is the real task: scanning a 100–400 file review and
knowing where you are.

The mockups are indicative of the shape, not cell-exact. They draw a grouped
file row as 4 cells of chrome — two of indent and the `▢ ` checkbox — but the
shipped row also carries the `M `/`A `/`D ` status badge, so it spends 6. At
the mockup's 38-cell inner width the real path budget is therefore 32, not 34,
and the middle elision splits it 11/20 around the ellipsis rather than 11/22.
Read the mockup rows for their proportions; `src/ui/file_list.rs` is where the
cells are counted.

## Decision

**Groups are collapsible top-level nodes in the one sidebar.** A group is the
only container the grouped sidebar has.

**Inside a group there are no directory rows** (`gd-26r.31`). A group's files
are listed directly beneath it, each labelled with its **full relative path**,
elided in the middle where the panel is too narrow. The group row is the only
thing that collapses. That is mockup A below, and it is the whole grouped
layout — there is no in-group tree mode to choose.

**The tree mode `nested | compact | flat` governs the ungrouped tree only.**
`nested` stays its default. It is still worth shipping on its own merits — the
ungrouped fixture spends 36 rows on directory chrome with no way to turn it
down — but it is not an axis of the grouped sidebar.

**Grouping is a toggle between the two sidebars**, not a startup-only setting:
`<leader>g`, with `:set groups!` as the command form, switches between the
grouped list and the plain directory tree. It joins `<leader>e` and
`<leader>s` as a view toggle; `g` is unclaimed as a leader key today.

*Originally this record chose differently: a directory subtree scoped inside
each group, with the tree mode orthogonal to grouping and `nested` the default
in both. `gd-26r.31` reversed it — see the
[revision](#revision-gd-26r31-no-directory-rows-inside-a-group).*

### What was rejected

- **C — directory tree unchanged, groups as a switchable view or filter.**
  Rejected: the default screen still says nothing about what the change *is*,
  and you can only ever see one group at a time. It buys the smallest diff to
  the existing code and loses the whole point. The `<leader>g` toggle
  `gd-26r.31` adds is *not* a step back toward C: the grouped sidebar is the
  default and shows every group at once, and the toggle reaches the whole
  ungrouped tree rather than one group filtered out of it.
- **D — ordering only, header rows, no new node type.** Rejected: headers are
  decoration, so nothing collapses. Same row count as flat groups (174) but
  with no overview, and at 161 files the overview is the feature.

## The mockups

### 0. Today — pure directory tree

Thirty-six directory rows. Deep single-child chains. Nothing above `src/`
tells you what the changeset is about.

```
┌ Files · 0/161 ───────────────────────┐
│▼ mobile/                             │
│  ▼ app/                              │
│    ▼ h/                              │
│      ▼ [hostId]/                     │
│        ▢ tasks.tsx                   │
│  ▼ src/                              │
│    ▼ components/                     │
│      ▼ pr-sidebar/                   │
│        ▢ PRCommentCard.tsx           │
│    ▼ session/                        │
│      ▢ github-pr-mutations.test.ts   │
│      ▢ github-pr-mutations.ts        │
│      ▢ github-pr-rpc.test.ts         │
│      ▢ github-pr-rpc.ts              │
│      ▢ github-pr-value-readers.test.…│
│      ▢ github-pr-value-readers.ts    │
│      ▢ pr-action-mutation-contract.ts│
│      ▢ pr-actions-engine.test.ts     │
│      ▢ pr-actions-engine.ts          │
│      ▢ pr-comment-actions.test.ts    │
│      ▢ pr-comment-actions.ts         │
│      ▢ use-mobile-pr-actions.test.ts │
│      ▢ use-mobile-pr-comment-actions…│
│    ▼ tasks/                          │
└──────────────────────────────────────┘
   … 173 more rows below (197 total)
```

### A — groups top-level, files listed by full path (**the chosen layout**)

```
┌ Files · 0/161 ───────────────────────┐
│▼ enterprise-host-routing         0/18│
│  ▢ mobile/src/…routing-source.test.ts│
│  ▢ src/main/gi…ise-repository.test.ts│
│  ▢ src/main/gi…terprise-repository.ts│
│  ▢ src/main/gi…entity-parsing.test.ts│
│  ▢ src/main/gi…te-identity-parsing.ts│
│  ▢ src/main/gi…ork-owner-repo.test.ts│
│  ▢ src/main/gi…repository-identity.ts│
│  ▢ src/main/gi…view-host-auth.test.ts│
│  ▢ src/main/gi…nterprise-host.test.ts│
│  ▢ src/rendere…uting-boundary.test.ts│
│  ▢ src/rendere…o-slug-routing.test.ts│
│  ▢ src/rendere…-host-boundary.test.ts│
│  ▢ src/rendere…host-clone-url.test.ts│
│  ▢ src/rendere…ject-host-clone-url.ts│
│  ▢ src/shared/…y-identity-key.test.ts│
│  ▢ src/shared/…sitory-identity-key.ts│
│  ▢ src/shared/…tup-projection.test.ts│
│  ▢ src/shared/…st-setup-projection.ts│
│▼ work-items                       0/8│
│  ▢ src/main/gi…s-query-paging.test.ts│
│  ▢ src/main/gi…ent-work-items.test.ts│
│  ▢ src/main/gi…ls-concurrency.test.ts│
│  ▢ src/main/gi…ls-file-viewed.test.ts│
│  ▢ src/main/gi…tails-pr-files.test.ts│
│  ▢ src/main/gi…k-item-details.test.ts│
│  ▢ src/main/gi…b/work-item-details.ts│
│  ▢ src/rendere…-item-source-lookup.ts│
│▼ github-client-plumbing          0/38│
│  ▢ src/main/gi…r-gh-host-args.test.ts│
└──────────────────────────────────────┘
   … 144 more rows below (174 total)
```

### A collapsed — the entire 161-file review in 13 rows

Group order is intent-centrality, not alphabetical. Collapsed, that ordering
*is* the summary of the changeset — which is why group order is a scored
concern in its own right (`gd-26r.22`) rather than an incidental.

```
┌ Files · 0/161 ───────────────────────┐
│▶ enterprise-host-routing         0/18│
│▶ work-items                       0/8│
│▶ github-client-plumbing          0/38│
│▶ pr-actions                      0/29│
│▶ project-view                    0/28│
│▶ pr-checks                        0/6│
│▶ repo-slug-resolution             0/9│
│▶ gh-auth                          0/5│
│▶ rate-limiting                    0/5│
│▶ issues                           0/4│
│▶ tasks                            0/4│
│▶ settings-repo-icon               0/5│
│▶ terminal-strays                  0/2│
└──────────────────────────────────────┘
```

### B — groups top-level, directory subtree inside (`nested`) — **superseded**

Chosen originally, reversed by `gd-26r.31`. Every group re-pays its directory
chrome. `enterprise-host-routing` spends 12 rows of chrome on 18 files.

```
┌ Files · 0/161 ───────────────────────┐
│▼ enterprise-host-routing         0/18│
│  ▼ mobile/                           │
│    ▼ src/                            │
│      ▼ tasks/                        │
│        ▢ github-project-host-routing…│
│  ▼ src/                              │
│    ▼ main/                           │
│      ▼ github/                       │
│        ▢ github-enterprise-repositor…│
│        ▢ github-enterprise-repositor…│
│        ▢ github-remote-identity-pars…│
│        ▢ github-remote-identity-pars…│
│        ▢ github-repository-identity.…│
│        ▢ github-repository-identity.…│
│        ▢ project-view-host-auth.test…│
│        ▢ work-item-details-enterpris…│
│    ▼ renderer/                       │
│      ▼ src/                          │
│        ▼ components/                 │
│          ▼ new-workspace/            │
│            ▢ SmartWorkspaceNameField…│
│          ▢ github-enterprise-slug-ro…│
│          ▢ pull-request-page-host-bo…│
│        ▼ lib/                        │
│          ▢ project-host-clone-url.te…│
│          ▢ project-host-clone-url.ts │
│    ▼ shared/                         │
│      ▢ github-repository-identity-ke…│
│      ▢ github-repository-identity-ke…│
│      ▢ project-host-setup-projection…│
└──────────────────────────────────────┘
   … 270 more rows below (300 total)
```

### B with chains joined (`compact`) — **superseded**

VS Code's "compact folders". Barely helps the ungrouped tree, because at repo
scope the chains have many children. Inside a group the subtrees are sparse —
a group's members are scattered across directories by definition — so the
mockup below assumed it pays there.

It pays far less than drawn, which is part of why `gd-26r.31` dropped in-group
directory rows entirely. The mockup was hand-drawn deciding each chain join
against the group's own files; the shipped `TreeLayout::joined_dirs`
(built once by `TreeLayout::new`) decides
joins once over the whole of `diff_files`, so a directory with one child inside
a group but five across the changeset does not join. Measured, that is 285
rows, not the 251 drawn.

```
┌ Files · 0/161 ───────────────────────┐
│▼ enterprise-host-routing         0/18│
│  ▼ mobile/src/tasks/                 │
│    ▢ github-project-host-routing-sou…│
│  ▼ src/                              │
│    ▼ main/github/                    │
│      ▢ github-enterprise-repository.…│
│      ▢ github-enterprise-repository.…│
│      ▢ github-remote-identity-parsin…│
│      ▢ github-remote-identity-parsin…│
│      ▢ github-repository-identity.fo…│
│      ▢ github-repository-identity.ts │
│      ▢ project-view-host-auth.test.ts│
│      ▢ work-item-details-enterprise-…│
│    ▼ renderer/src/                   │
│      ▼ components/                   │
│        ▼ new-workspace/              │
│          ▢ SmartWorkspaceNameField-r…│
│        ▢ github-enterprise-slug-rout…│
│        ▢ pull-request-page-host-boun…│
│      ▼ lib/                          │
│        ▢ project-host-clone-url.test…│
│        ▢ project-host-clone-url.ts   │
│    ▼ shared/                         │
│      ▢ github-repository-identity-ke…│
│      ▢ github-repository-identity-ke…│
│      ▢ project-host-setup-projection…│
│      ▢ project-host-setup-projection…│
│▼ work-items                       0/8│
│  ▼ src/                              │
│    ▼ main/github/                    │
└──────────────────────────────────────┘
   … 221 more rows below (251 total)
```

### C — directory tree unchanged, group as a filter (rejected)

```
┌ Files · 0/161 ───────────────────────┐
│[filter] enterprise-host-routing      │
│──────────────────────────────────────│
│▼ mobile/                             │
│  ▼ src/                              │
│    ▼ tasks/                          │
│      ▢ github-project-host-routing-s…│
│▼ src/                                │
│  ▼ main/                             │
│    ▼ github/                         │
│      ▢ github-enterprise-repository.…│
│      ▢ github-enterprise-repository.…│
│      ▢ github-remote-identity-parsin…│
│      ▢ github-remote-identity-parsin…│
│      ▢ github-repository-identity.fo…│
│      ▢ github-repository-identity.ts │
│      ▢ project-view-host-auth.test.ts│
│      ▢ work-item-details-enterprise-…│
│  ▼ renderer/                         │
│    ▼ src/                            │
│      ▼ components/                   │
└──────────────────────────────────────┘
   … 12 more rows below (32 total)
```

### D — ordering only, header rows, no new node type (rejected)

```
┌ Files · 0/161 ───────────────────────┐
│── enterprise-host-routing ────── 0/18│
│▢ mobile/src/…t-routing-source.test.ts│
│▢ src/main/gi…prise-repository.test.ts│
│▢ src/main/gi…enterprise-repository.ts│
│▢ src/main/gi…identity-parsing.test.ts│
│▢ src/main/gi…mote-identity-parsing.ts│
│▢ src/main/gi….fork-owner-repo.test.ts│
│▢ src/main/gi…b-repository-identity.ts│
│▢ src/main/gi…t-view-host-auth.test.ts│
│▢ src/main/gi…-enterprise-host.test.ts│
│▢ src/rendere…routing-boundary.test.ts│
│▢ src/rendere…epo-slug-routing.test.ts│
│▢ src/rendere…ge-host-boundary.test.ts│
│▢ src/rendere…t-host-clone-url.test.ts│
│▢ src/rendere…roject-host-clone-url.ts│
│▢ src/shared/…ory-identity-key.test.ts│
│▢ src/shared/…pository-identity-key.ts│
│▢ src/shared/…setup-projection.test.ts│
│▢ src/shared/…host-setup-projection.ts│
│── work-items ──────────────────── 0/8│
└──────────────────────────────────────┘
   … 154 more rows below (174 total)
```

## Row cost, same 161 files

Re-measured by `gd-26r.31` against the shipped code rather than the mockups,
using this fixture's hand grouping through the real sidebar
(`nested_costs_197_rows_on_the_fixture` and its neighbours in
`src/app/tests/tree_tests.rs`).

| layout | rows |
| --- | --- |
| today, nested (as shipped) | 197 |
| today, compact | 191 |
| today, flat | 161 |
| **groups — the chosen layout** | **174** |
| **groups, all collapsed** | **13** |

Superseded rows, kept because they are the measurement that decided
`gd-26r.31` and because the interim code still produces them:

| layout | as published | measured | why it moved |
| --- | --- | --- | --- |
| groups + nested | 300 | **305** | +5 for per-run directory rows |
| groups + compact | 251 | **285** | +4 per-run, +30 because chain joins are decided over the whole changeset, not per group |
| groups + flat | 174 | 174 | unchanged: no directory rows to emit |
| groups, all collapsed | 13 | 13 | unchanged: a collapsed group emits no rows below it |

The 300 reproduces exactly under the cost model it was measured with — one row
per directory per group — so nothing about the original arithmetic was wrong;
per-run emission simply adds rows the model did not price. The second fixture
(158 files, 14 groups, uncommitted — `gd-26r.20`) agrees on the scale: +2
nested, +2 compact, collapsed 14.

Chain-joining is worth 6 rows ungrouped, and 20 grouped as the interim code
computes it. Under the chosen layout the question is moot: the grouped sidebar
has no directory rows to join. The ungrouped tree mode still earns its place on
`flat` alone, which turns 197 rows into 161.

## The tree mode is its own feature

`nested | compact | flat` is a property of the **ungrouped** tree. It is worth
shipping whether or not grouping ever lands — the ungrouped fixture spends 36
rows on directory chrome today with no way to turn it down — and that is now
the whole of its case: `gd-26r.31` removed the in-group directory rows it used
to also govern, so it has no effect while grouping is on.

*Originally this section read "it applies to today's ungrouped tree exactly as
it applies inside a group". That half is gone.*

One config key governs it, defaulting to `nested` so nothing changes for
anyone who does not opt in. There is no group-specific override. Adding the key
means adding it to `KNOWN_KEYS` (`src/config/mod.rs`) or startup emits
an unknown-key warning; config is read exactly once, by `main`, and
never re-read during a session.

## Two signals the sidebar has to carry

### Weak group names

Derived group names come out generic exactly when the group is weak, so the
sidebar must not hide that signal. It doesn't need a badge to carry it: the
collapsed overview puts the derived name and the file count on the same row,
and a generic name next to the largest count is self-evident. In the fixture,
`github-client-plumbing` at 38 files reads as a residual group at a glance —
`docs/GROUPING.md` rule 6 — and `terminal-strays` at 2 reads as the drive-by
bucket it is.

The requirement this places on the implementation is negative: **render the
derived name verbatim.** No prettification, no title-casing, no substituting a
directory name when the derived name is poor. Papering over a bad name hides
the only cheap signal that the grouping is bad there.

### Ambiguous assignments

The engine records a chosen group, a runner-up, and a one-line reason on close
calls — roughly 14% of files. **The sidebar does not surface any of it yet.**
No glyph, no marker, no status-bar line. It stays on the group record for the
human-corrections work (`gd-26r.18`) to consume.

This is an explicit decision, not an omission. A marker on one row in seven is
noise at 161 files, and until a human can *act* on the flag by reassigning the
file, the marker offers nothing to do. When it does surface, it goes in the
marker slot the revision below defines rather than inventing a second glyph
convention.

## What this demands of `build_visible_items`

The seam map's Correction 1 is the binding constraint. `build_visible_items`
(`src/app/tree.rs`) carries a single **global** `seen_dirs` set across
the whole file loop and therefore silently requires `diff_files` to be
contiguous by directory. A directory appearing in two non-adjacent runs emits
its header once; if it is collapsed, the later files vanish from the sidebar
while remaining in the diff pane. Nothing asserts, tests, or comments this.
`sort_files_by_directory` is the only reason it has ever held.

That constraint is why the grouped sidebar has no directory rows: **grouping
gives up directory contiguity by construction, and a directory row is only
sound where it holds.**

1. **The invariant is one-level under grouping.** `diff_files` must be
   contiguous **by group**, and nothing more. Directory contiguity is
   deliberately given up: concern groups span shared directories by design, so
   `src/main/github` legitimately appears under most of the 13 groups in this
   fixture, and the within-group sort (`gd-26r.27`) can put it in two
   non-adjacent runs inside one of them.

2. **The grouped branch emits no `Directory` rows at all.** Files sit at depth
   1 under their group row and are labelled with the full relative path, not
   `file_name()`. There is no `seen_dirs` in the grouped branch, so there is
   nothing to reset and nothing to lose track of. The ungrouped branch keeps
   its global `seen_dirs`, sound because `order_files_by_directory` still runs
   there.

   *Originally this point read "`seen_dirs` must reset at every group
   boundary", and called that the single most important line of the change. It
   was wrong twice over: the reset alone still drops files when a directory
   splits into two runs inside one group (`gd-26r.31`), and the layout it was
   protecting no longer exists.*

3. **One key space per sidebar, one set each.** `App::expanded_dirs:
   HashSet<String>` holds directory paths; `App::expanded_groups:
   HashSet<String>` holds group ids. Neither ever holds the other's keys, so
   the group-id/directory-path collision hazard the seam map flagged is
   disposed of rather than merely qualified away.

   *Amended by `gd-26r.35`.* This point originally read "`expanded_dirs` holds
   group ids while grouping is on, and paths while it is off": one flat set,
   safe because with no in-group directory rows the two key spaces were never
   populated at once. `<leader>g` is what ends that. A session can now arrange
   both sidebars in one lifetime, and one set can only serve the one on screen:
   a toggle would have to either carry the outgoing keys into a sidebar that
   cannot read them — leaving the map holding both key spaces at once, which is
   exactly what the old point 3 promised could not happen — or reseed, throwing
   away an arrangement the reader will be shown again on the next keypress.
   Splitting the set is what makes a toggle need to do neither: it touches
   neither set, so toggling twice is the identity and each sidebar comes back
   as its reader left it. Nothing else about the point changes — group ids are
   still opaque, still what survives a rename, and still the only key the
   grouped sidebar reads.

   *Originally: keyed by `(group, directory)`. That was the right answer to the
   wrong layout — and note it would not have been sufficient on its own, since
   a directory in two runs inside one group shares a single `(group, directory)`
   key and both runs collapse together.*

4. **Ship the guard with the change.** A `debug_assert` in
   `build_grouped_items` that the group runs are contiguous, plus a test of the
   property the sidebar needs: `file_idx` ascends across the visible rows,
   which is what `next_file`/`prev_file` step by. Ascending `file_idx` has two
   preconditions and the assert covers one of them — contiguity; the other, that
   `grouping.groups()` runs in the same order as the runs in `diff_files`, is
   covered by the test `grouping_reorders_diff_files_into_group_runs`. The
   invariant has been load-bearing and unguarded; this change is the moment to
   fix that, because it is the change that starts violating the old form of it.
   The guard survives the revision unchanged: group contiguity is still what
   the grouped branch reads `diff_files` by.

   *Originally: the guard plus a test that a deliberately non-contiguous
   `diff_files` loses no files from the sidebar. That test cannot be written
   against the shipped guard, which panics on exactly the input it would have
   to construct — and it would prove nothing now, since members are resolved
   by path rather than by scanning a run. Ascending `file_idx` is the property
   the assert actually protects.*

5. **The tree mode does not reach the grouped branch.** `nested` and `compact`
   change what ancestors the *ungrouped* tree emits; `flat` drops its directory
   rows and labels each file with its full path. Grouping renders as though
   `flat` were set regardless of the mode, and does not change the mode — the
   ungrouped tree is still whatever the config says when `<leader>g` toggles
   back to it.

Everything else the seam map lists still applies unchanged:

- `FileTreeItem` needs a group variant, or a `label` distinct from `path`, so
  the renderer stops deriving the display name via `Path::file_name()`
  (`render_file_list` in `src/ui/file_list.rs`) and stops appending `/` to it.
- `App::expand_all_dirs` seeds `expanded_dirs` from path
  ancestors only, so it must also seed group ids or groups start collapsed and
  their files hidden. It seeds **both** sets — every directory row and every
  group row — because after `<leader>g` the sidebar it did not seed is one
  keypress away, and a reader who never collapsed anything should not find it
  shut (`gd-26r.35`). There are still no in-group directory keys: the two sets
  stay disjoint.
- `App::jump_to_file` (`src/app/navigation.rs`) inserts every path ancestor
  to reveal a target; under grouping the ancestors reveal nothing and the
  file's group id is the only key that does.
- `App::ensure_valid_tree_selection` walks the parent chain to
  recover selection and must fall back to the group node rather than
  `select(0)`.
- The unguarded `&app.diff_files[*file_idx]` in `render_file_list`
  still panics on a stale index.
- The commit-message pseudo-file is hoisted to index 0 by
  `App::sort_files_by_directory` and
  has no real path. Where it sits relative to the groups is `gd-26r.12`.

## Revision (`gd-26r.31`): no directory rows inside a group

The handback that forced this was narrow: point 2 above claimed that resetting
`seen_dirs` at group boundaries was enough, which assumes each directory
occupies one run inside a group. The within-group sort (`gd-26r.27`) bands a
group by central file, then mechanical and broad-test tails, so a directory can
appear in two non-adjacent runs inside one group — and under the old point 2 the
second run's files vanish whenever that directory is collapsed. `gd-26r.28`
slice A had already shipped the narrow fix: a directory row **per run**, keyed
`(group, directory)` so both runs toggle together, guarded by
`a_directory_split_into_two_runs_inside_one_group_keeps_every_row`.

Confirming per-run emission was the intended reading is the small answer. The
larger one is that in-group directory rows had stopped paying for themselves:

- **Ordering inside a group is now meaningful**, and directory rows interrupt
  it. Central-file-first banding is a scored concern (`gd-26r.27`); a directory
  header sitting between a production file and its test is chrome cutting
  across the very signal the sort exists to carry. Two rows for one directory
  is the visible symptom of that mismatch, not a rendering bug to paper over.
- **The chrome is most of the sidebar.** 305 rows to show 161 files — 144 rows
  of directory scaffolding, against 174 rows with none.
- **Compact does not rescue it.** The 251 this record published assumed chain
  joins decided per group; the shipped layout decides them over the whole
  changeset and lands at 285. Fixing that would recover ~30 rows and leave the
  ordering objection untouched.
- **Everything the old points 2 and 3 were guarding disappears with the rows.**
  No per-run emission, no group-scoped keys, no collision hazard between group
  ids and directory paths.

What did *not* move: the collapsed overview is still **13 rows for 161 files**,
verified in all three modes, and the second fixture (158 files, 14 groups,
uncommitted — `gd-26r.20`) agreed at 14 rows when it was measured. A
collapsed group emits nothing beneath it, so no decision about in-group layout
can reach that number. It was never at risk, and it remains the case for the
whole feature.

**`gd-26r.34` made the code match this**: `build_grouped_items` emits no
directory rows, grouped files carry the full relative path at depth 1 in every
tree mode, and `expanded_dirs` holds group ids only while grouping is on. The
per-run regression test retired with the rows it guarded, as did the two
interim row-cost pins (305 and 285); 174 and 13 are pinned and unchanged.

**`gd-26r.35` shipped the toggle**: `<leader>g` and `:set groups!` switch
between the two sidebars mid-session, the grouping survives the off state so
toggling back never regroups, and the shared `expanded_dirs` split into two
sets — see the amendment to point 3 above, which the toggle is what forced.

## Revision (`gd-26r.15`): the status chip and the marker slot

Grouping status is sidebar chrome, so its shape is recorded here. The decision
itself is `gd-26r.15`; what follows is what it demands of these rows.

### The header suffix

The header is `" Files · {reviewed}/{total} "`, optionally followed by the
filter qualifier. One grouping chip may follow that, plain (no theming: the
header is a border title, and a coloured run inside a border line reads as a
rendering artefact rather than a badge):

| Chip | When |
| --- | --- |
| `· refining` | a `:regroup` refine call is in flight |
| `· refine cancelled` | the reader pressed the cancel key on a refine wait |
| `· heuristics only` | `[grouping].refine` is on and no group came back refined |
| `· 9% new · :regroup` | files have drifted since the last full pass |

**One chip at a time**, ranked filter qualifier > in flight > cancelled /
unavailable > drift. Joining them and letting the header truncate would cut the
count the header exists for. The drift chip's `· :regroup` tail renders only
when the whole of it fits and drops to `· 9% new` otherwise, because a
truncated `· :regr` is worse than no advice; it is advisory only, and no key is
bound to it.

Neither startup refine path reaches the header: both block the TUI behind a
screen of their own, so there is nothing to put in a header that is not drawn
yet. Nothing here reports repair counts — a repaired refine is still a refined
partition, and the count stays a startup warning.

The chip is grouped-view only. Toggled off (`<leader>g`), the sidebar is the
existing file tree, unchanged, and the staleness of a grouping that is not on
screen is a fact about something the reader is not looking at.

### The marker slot on group rows

A group row is `{expand icon} {marker}{name} … {reviewed/total}`. The marker
slot holds `~ ` on a group incremental assignment produced —
`GroupSource::Incremental` or `new_since_full_pass`, one glyph for both because
the reader's decision is identical — and is empty otherwise.

**Revision (`gd-26r.23`): `!` in the same slot.** A group the size cap's single
split pass could not bound carries `! ` instead, and `!` outranks `~` in the
rare row that could take both. One slot, one glyph, and the more urgent claim
wins: `~` says the row is a little stale, `!` says the engine gave up here and
the row will not help you. That is the one thing the flush-right count cannot
say for itself — 38 files is either a coherent large concern or a failed split,
and the number does not tell them apart. A group the pass *did* split carries no
marker at all: those pieces are ordinary groups of the pass, named
`parent · suffix`, and there is no soft-cap signal anywhere.

Before the label, never between the label and the count: the count is flush
right and that column is what makes the 13-row collapsed overview scannable.
Restyling the label instead was rejected — invisible on a theme without
italics, unreadable to anyone comparing two shades. **`gd-26r.18` reuses this
slot** for whatever it surfaces about ambiguous assignments.

### The drift number

`Grouping::drift()` is the one drift number: files sitting in drifted groups
over every file in the partition, `0.0` when nothing drifted, and the header
hides the chip at exactly zero rather than parking a `0%` the reader learns to
stop looking at. The auto-regroup backstop (`gd-26r.36`) reads the same
function, so the backstop cannot fire at a value the reader never watched
approach.

Files over files, not groups over groups: one arrival marks a whole group, and
counting the mark would report a third of the review as drifted when one file
moved. It is measured off the groups rather than off each assignment's `pass`
because `pass` is derived debug state a session restore does not carry, and a
drift number that reset itself on reopen is the one thing a persisted
indicator must not do. The gap that leaves is a file incremental assignment
*joined* to an established group: it moves neither the number nor any row's
marker, so the header and the rows agree.

## Deliberately not decided here

- Group navigation keybindings beyond the `<leader>g` toggle, and any group UX
  beyond the sidebar's shape.
- Group-level review counts beyond the `reviewed/total` shown on the group row.
- ~~Group sizing~~ — settled by `gd-26r.23` and built in `gd-26r.38`: soft cap
  20 (refine prompt only), hard caps 25 refined and 30 heuristic, enforced by a
  directory split at the end of each full pass. `github-client-plumbing` at 38
  files was one of the two groups the cap was written for.
- How a human reassigns a file to a different group (`gd-26r.18`).
