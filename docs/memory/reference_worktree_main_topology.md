---
name: reference_worktree_main_topology
description: "The autospec repo runs many parallel git worktrees; a sibling worktree holds `main`, so the primary checkout cannot be on main and rests detached/feature-branch"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 126475a4-f965-4503-a160-92b59f439782
---

The `/Users/wohlgemuth/IdeaProjects/autospec` checkout is one of ~9+ concurrent
git worktrees (others under `/private/tmp/wt-*` and `/Users/wohlgemuth/IdeaProjects/autospec-*`).
The sibling worktree `/Users/wohlgemuth/IdeaProjects/autospec-autonomous` holds the
`main` branch. Git forbids the same branch in two worktrees, so the **primary
checkout can never `git checkout main`** — it sits on a feature branch or detached
at `origin/main`.

How to apply: when cleaning up / anchoring the primary checkout, don't expect to
land it on `main`. Use `git switch -c <branch> origin/main`, or accept detached
HEAD at `origin/main` (clean + safe). When diagnosing a "stale branch N-ahead /
upstream [gone]", remember most of the N "ahead" commits are squash-merge
divergence, not unmerged work — reconcile by branching FRESH off `origin/main`
(never try to merge the stale commits). See [[feedback_per_session_worktree_isolation]]
and [[feedback_pre_pipeline_sync]].
