# Sidebar model: groups and directories

Design record for `gd-26r.7` on the wayfinder map `gd-26r` (grouped review of
large changesets in tuicr). Prototype ticket: the candidates were drawn as
ASCII mockups against a real changeset and reacted to, rather than argued in
prose.

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

## Decision

**Groups are collapsible top-level nodes in the one sidebar.** Directory nodes
live *inside* a group and are scoped to it. There is no second view, no filter
mode, and no group-less tree.

**How files lay out inside a group is a separate, orthogonal feature**: a tree
mode with three values — `nested`, `compact`, `flat` — that applies to the
sidebar whether or not grouping is on. `nested` is today's behaviour and stays
the default. Path flattening is therefore not part of grouping at all; it is a
tree-rendering option that grouping happens to make more valuable.

That collapses what looked like two rival candidates (A and B) into one model
plus one axis.

### What was rejected

- **C — directory tree unchanged, groups as a switchable view or filter.**
  Rejected: the default screen still says nothing about what the change *is*,
  and you can only ever see one group at a time. It buys the smallest diff to
  the existing code and loses the whole point.
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

### A — groups top-level, files flat inside (`flat`)

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

### B — groups top-level, directory subtree inside (`nested`)

Every group re-pays its directory chrome. `enterprise-host-routing` spends
12 rows of chrome on 18 files.

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

### B with chains joined (`compact`)

VS Code's "compact folders". Barely helps the ungrouped tree, because at repo
scope the chains have many children. Inside a group the subtrees are sparse —
a group's members are scattered across directories by definition — so it pays
there.

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

| layout | rows |
| --- | --- |
| today, nested (as shipped) | 197 |
| today, compact | 191 |
| today, flat | 161 |
| groups + nested | 300 |
| groups + compact | 251 |
| groups + flat | 174 |
| groups, all collapsed | 13 |

Chain-joining is worth 6 rows ungrouped and 49 rows grouped. That asymmetry is
the argument for shipping the tree mode as a real option rather than picking
one layout.

## The tree mode is its own feature

`nested | compact | flat` is a property of the sidebar, not of grouping. It
applies to today's ungrouped tree exactly as it applies inside a group, and it
is worth shipping whether or not grouping ever lands — the ungrouped fixture
spends 36 rows on directory chrome today with no way to turn it down.

One config key governs both, defaulting to `nested` so nothing changes for
anyone who does not opt in. There is no group-specific override. Adding the key
means adding it to `KNOWN_KEYS` (`src/config/mod.rs:169-198`) or startup emits
an unknown-key warning; config is read exactly once, at `src/main.rs:73`, and
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
file, the marker offers nothing to do.

## What this demands of `build_visible_items`

The seam map's Correction 1 is the binding constraint. `build_visible_items`
(`src/app/tree.rs:271-316`) carries a single **global** `seen_dirs` set across
the whole file loop and therefore silently requires `diff_files` to be
contiguous by directory. A directory appearing in two non-adjacent runs emits
its header once; if it is collapsed, the later files vanish from the sidebar
while remaining in the diff pane. Nothing asserts, tests, or comments this.
`sort_files_by_directory` is the only reason it has ever held.

**The chosen model does not remove that invariant. It re-scopes it, and it
makes the latent bug certain rather than hypothetical.**

1. **The invariant becomes two-level.** `diff_files` must be contiguous **by
   group**, and within a group contiguous **by directory**. Global directory
   contiguity is deliberately given up: concern groups span shared directories
   by design, so `src/main/github` legitimately appears under most of the 13
   groups in this fixture.

2. **`seen_dirs` must reset at every group boundary.** Left global, the second
   group to touch `src/main/github` emits no header for it, and if that
   directory is collapsed its files disappear from the sidebar entirely. This
   is not an edge case under grouping — it fires on nearly every group in the
   fixture. This reset is the single most important line of the change.

3. **`expanded_dirs` must be keyed by (group, directory), not by path.**
   `expanded_dirs: HashSet<String>` (`src/app/mod.rs:1233`) is a flat string
   set. Once the same directory path appears under several groups, collapsing
   `src/main/github` in one group would collapse it in all of them. Qualifying
   the key also removes the group-id/directory-path collision hazard the seam
   map flagged.

4. **Ship the guard and the test with the change.** A `debug_assert` in
   `build_visible_items` that the group runs are contiguous, plus a test that a
   deliberately non-contiguous `diff_files` with a collapsed directory loses no
   files from the sidebar. The invariant has been load-bearing and unguarded;
   this change is the moment to fix that, because it is the change that starts
   violating the old form of it.

5. **The three tree modes change what ancestors are emitted, per group.**
   `nested` is today's per-ancestor emission. `compact` joins a chain while
   each node has exactly one directory child and no files of its own. `flat`
   emits no `Directory` rows at all: files sit at depth 1 under the group, and
   the label is the full path rather than `file_name()`. Under `flat` the
   directory half of the contiguity invariant is vacuous — only group
   contiguity remains.

Everything else the seam map lists still applies unchanged:

- `FileTreeItem` needs a group variant, or a `label` distinct from `path`, so
  the renderer stops deriving the display name via `Path::file_name()`
  (`src/ui/file_list.rs:41-44, 105-108`) and stops appending `/` to it
  (`file_list.rs:112`).
- `expand_all_dirs` (`tree.rs:187-203`) seeds `expanded_dirs` from path
  ancestors only, so it must also seed group ids or groups start collapsed and
  their files hidden.
- `jump_to_file` (`src/app/navigation.rs:754-762`) inserts every path ancestor
  to reveal a target; it must also insert the file's group id.
- `ensure_valid_tree_selection` (`tree.rs:249-266`) walks the parent chain to
  recover selection and must fall back to the group node rather than
  `select(0)`.
- The unguarded `&app.diff_files[*file_idx]` at `file_list.rs:48` and `116`
  still panics on a stale index.
- The commit-message pseudo-file is hoisted to index 0 (`tree.rs:144-150`) and
  has no real path. Where it sits relative to the groups is `gd-26r.12`.

## Deliberately not decided here

- Group navigation keybindings and any group UX beyond the sidebar's shape.
- Group-level review counts beyond the `reviewed/total` shown on the group row.
- Group sizing. `github-client-plumbing` at 38 files and `pr-actions` at 29
  both exceed the ~20 soft / ~25 hard cap the human wants, and the two groups a
  cap would split are exactly the two that read as weak. Tracked separately.
- How a human reassigns a file to a different group (`gd-26r.18`).
