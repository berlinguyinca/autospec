---
name: feedback_subagent_cwd_pinned_to_main_checkout
description: "Agent-tool subagents run with cwd pinned to the main repo checkout, NOT the EnterWorktree worktree — implementer subagents commit to whatever branch the main checkout is on, contaminating parallel work"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ba194365-30f2-4add-918a-ed7263b0d5dd
---

When the session is in an `EnterWorktree` worktree and you dispatch an **Agent**
subagent to write+commit code, the subagent's working directory is pinned to the
**main repo checkout** (the original launch dir), not the active worktree — and
the harness resets a subagent's `cd` back after each Bash call. So a build
subagent commits to whatever branch the *main checkout* currently has checked
out. If a parallel session/user has switched the main checkout to their own
branch, the subagent's commit lands on THEIR branch (cross-contamination).

**Why:** observed live — main checkout was on `feat/closeout-report-contract`
(parallel work); a dispatched implementer's commit landed there instead of the
intended worktree branch, on top of the user's uncommitted edits.

**How to apply:**
- For multi-track worktree work, do the file edits/commits **directly as the main
  agent** (the main loop's cwd IS correctly pinned to the active worktree — a
  `cd elsewhere` in Bash is auto-reset back to the worktree). Reserve subagents
  for **read-only** tasks (reviewers/explorers never commit, so they're safe).
- If you must use a writing subagent, have it `git rev-parse --abbrev-ref HEAD`
  first and refuse if not on the expected branch.
- Recovery when it lands on the wrong branch: `git cherry-pick <sha>` onto the
  intended branch, then in the contaminated checkout `git reset --soft HEAD~1` +
  selectively `git restore`/`rm` only the stray files (never disturb the owner's
  uncommitted changes). Confirm with the user before touching a branch you don't
  own. Relates to [[feedback_skill_golden_derivation_workflow]].
