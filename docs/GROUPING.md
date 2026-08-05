# Grouping guidelines

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

3. **Drive-by and unrelated changes are grouped, not scattered.** A change with
   no relation to the changeset's intent still belongs to a named group — it is
   simply ordered last. Scattering them through the concern groups is worse
   than collecting them.

4. **A test file lives in its production file's group, and follows it.** Tests
   never form their own group, and a test never precedes the code it covers.

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
    reconcile afterwards. It needs no pass; see `a_rename_is_one_file_in_one_group`.

## Ambiguous assignments

Every file still lands in exactly one group — rule 1's strict partition holds
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
