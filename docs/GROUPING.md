# Grouping guidelines

Part of the grouping design set. [`docs/GROUPING_INDEX.md`](GROUPING_INDEX.md)
maps a question to the doc that owns the answer.

How a changeset should be cut into groups for review. These are the rules the
grouping engine aims at, and the rules a human uses when correcting it.

Settled elsewhere and assumed here: grouping is **file-level with a strict
partition** — every file in the changeset belongs to exactly one named group,
no multi-membership.

## The rules

1. **Groups are concern-shaped, not directory-shaped.** A concern routinely
   spans several top-level directories, and a directory cut shreds it. Grouping
   by directory is also trivially reproducible by `dirname`, so it carries no
   information the file tree did not already show.

2. **Groups sort by intent-centrality.** The changeset's actual subject comes
   first; work incidental to it comes later; drive-by fixes come last.

   **The order is part of the answer, not a presentation detail.** A grouping
   that partitions the changeset correctly and orders the groups wrongly is a
   wrong grouping, and is scored as one — see `tau_group` in
   `docs/GROUPING_PASSES.md`. Both arms produce an order (`gd-26r.12`
   Decision 3), and in the groups contract the order *is* the `Vec` position
   (`docs/GROUPS_CONTRACT.md`), so there is no representation in which a
   grouping can decline to have one.

3. **Drive-by and unrelated changes are grouped, not scattered.** A change with
   no relation to the changeset's intent still belongs to a named group — it is
   simply ordered last. Scattering them through the concern groups is worse
   than collecting them.

4. **A test file lives in its production file's group, and follows it.** Tests
   never form their own group, and a test never precedes the code it covers.
   "Its production file" means the one it sits beside — same directory, or a
   `__tests__` directory directly beneath it. Two files that merely share a name
   in different parts of the tree are not that pair. See also rules 12–14, which
   order the rest of a group's files.

5. **When two groups claim a file, the more intent-central group takes it.**

6. **A residual group is a smell.** If a group amounts to "everything that did
   not sort elsewhere", either name the concern it actually represents or split
   it. Residual groups are usually the largest and least useful in the set.

7. **A burst of new test files pins a concern.** Several added test files
   sharing a filename token is strong evidence of a group — stronger than
   directory proximity, because the tests were written to pin one behaviour.

8. **Mechanical and generated changes get their own group, ordered last.**
   Lint autofixes, formatter sweeps, lockfiles, generated output, vendored
   updates. They are skimmed, not read, and mixing them into a concern group
   dilutes it.

9. **Documentation groups by its scope.** A doc that clearly belongs to one
   concern goes in that concern's group. A doc that covers the whole change —
   a design doc, a findings log, a plan — goes in a docs group of its own.

10. **Group names use the changeset's own vocabulary.** `enterprise-host-routing`
    is legible; `shared` and `renderer store/lib/hooks` are not. Prefer terms
    that appear in the changeset's commits, filenames, or PR title.

11. **A rename is one file, not two.** Both halves belong to the same group,
    which git's rename detection already guarantees: a rename arrives as a
    single entry keyed by the new path, so there is no second half to place.
    The rule stands as a constraint on any representation that splits a rename
    into a delete and an add — that representation is wrong, not a case to
    reconcile afterwards. It needs no pass; see `a_rename_is_one_file_in_the_changeset`.

## Within-group file order

Rules 1–11 order the groups. These order the files inside one, and rule 4 above
is the first of them. Added by `gd-26r.22`, which found that nothing produced a
within-group order at all: both arms built a group from a path-keyed map, and
`x.test.ts` sorts before `x.ts`, so **every** test preceded the code it covered.

12. **Integration and end-to-end tests come after unit tests.** Within a group,
    the unit tests of a file follow it (rule 4), and the broader tests —
    integration, end-to-end, acceptance, smoke — come after all of them. A
    reviewer reads the narrow proof of a change before the wide one.

13. **The group's central file comes first.** The file the group is named for —
    the entry point, the type, the module the concern is about — leads the
    group; everything else follows. This is rule 2's intent-centrality applied
    one level down.

    Measured, and honest about it: neither fixture's expected order follows this
    rule (`tau` −0.25 and −0.29 against their own file order), so it is written
    down and **unscored**. Naming it a rule while admitting the ground truth
    does not exhibit it is deliberate — the alternative is grading engines
    against a constraint the fixtures contradict.

14. **A mechanical file inside a concern group sorts to that group's tail.**
    Rule 8 sends a *burst* of generated files to its own group and is the normal
    path; this catches the straggler — a lockfile, a snapshot, a generated
    client — that ended up in a concern group anyway.

Where no rule above relates two files in a group, their order carries no intent
and is not scored. Path order is the tiebreak, chosen for stability, not because
it means anything.

## Testing order

Unit tests come before integration and end-to-end tests — as a rule about the
*test suite*, not about file order.

Write and run the unit tests first: they are the cheapest to run, the most
precise about where a failure is, and the ones that must already be green before
a broader failure is worth diagnosing. An integration or end-to-end test that
fails while the unit tests are red tells you nothing you did not already know,
and an end-to-end suite standing in for absent unit tests is slow, flaky, and
vague about the cause.

## Ambiguous assignments

Every file still lands in exactly one group — the strict partition assumed
above, before the numbered rules, holds
unconditionally. But where the call was close, the engine records the
assignment it made alongside the alternative it rejected:

- the group chosen
- the runner-up group
- a one-line reason

This is an annotation on the assignment, not a separate bucket of unassigned
files, and it is only emitted where the call was genuinely close. Flagging most
of a changeset is noise and defeats the purpose.

The flags are derived state: recomputed on every regroup, cheap to throw away,
and stored with the rest of the grouping on the session.

Human *corrections* to those calls are a different matter — they are the only
input in this system that cannot be regenerated — and are deliberately out of
scope here. See the follow-up ticket; note in particular that
`discard_session_and_quit` is the normal exit for a review that left no
comments, so corrections must not be stored where that path can destroy them.

## What is not decided here

- The on-disk or in-memory shape of a group record, including how the
  ambiguity flag is represented.
- How groups and flags are presented in the sidebar.
- How a human reassigns a file to a different group.
