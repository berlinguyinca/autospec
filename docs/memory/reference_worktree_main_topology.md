---
name: reference_worktree_main_topology
description: "Check `git worktree list` before assuming the primary autospec checkout can hold `main`; sibling worktrees may own it"
metadata:
  node_type: memory
  type: reference
  originSessionId: 126475a4-f965-4503-a160-92b59f439782
---

Snapshot from 2026-06-26/27: the `/Users/wohlgemuth/IdeaProjects/autospec`
checkout was one of many concurrent git worktrees (others under `/private/tmp/wt-*`
and `/Users/wohlgemuth/IdeaProjects/autospec-*`). At that point the sibling
worktree `/Users/wohlgemuth/IdeaProjects/autospec-autonomous` held the `main`
branch, so the primary checkout could not also `git checkout main` and rested on
a detached/feature state instead.

How to apply: when cleaning up / anchoring the primary checkout, first run
`git worktree list --porcelain` and verify whether another worktree currently
owns `main`. If `main` is occupied, use `git switch -c <branch> origin/main` or
accept detached HEAD at `origin/main` for read-only inspection. When diagnosing a
"stale branch N-ahead / upstream [gone]", treat the ahead commits as suspect
squash-merge divergence until checked against `origin/main`; reconcile by
branching fresh off `origin/main` instead of merging stale commits by default.
This complements, rather than replaces, sessions where the primary checkout is
intentionally pinned to `main` (see [[project_gap_remediation_keyword_routing]]).
See also [[feedback_per_session_worktree_isolation]] and [[feedback_pre_pipeline_sync]].
