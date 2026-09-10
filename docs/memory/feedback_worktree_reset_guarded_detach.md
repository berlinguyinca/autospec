---
name: feedback-worktree-reset-guarded-detach
description: "Reset a worktree with worktree-guard.sh reset (guarded, detached re-park at the base tip) — never an unguarded checkout/reset/clean chain naming a shared branch"
metadata: 
  node_type: memory
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 3653-fix
---

An unguarded three-step worktree reset — `git -C <wt> checkout -q -f main && git -C <wt> reset -q --hard origin/main && git -C <wt> clean -qfd` — failed in the wild (#3653): the first step aborts when a **sibling worktree already holds `main`**, and the sequence plowed on anyway, moving a local branch off its base commit. A detached park cannot have this failure mode.

**Why:** `git checkout -f <branch>` is a branch-targeting operation; it fails whenever another worktree owns that ref. Chaining `&&`-separated destructive steps without checking each result (or wrapping the unit so one failure stops everything) turns a single-step failure into cross-branch ref damage. See `reference_worktree_main_topology.md` — sibling worktrees routinely own `main`, so this collision is normal, not an edge case.

**How to apply:**
- Use `worktree-guard.sh reset --path <wt> [--base origin/main] [--clean]` (design doc §D6). It fetches, then parks the worktree **DETACHED** at the base tip via `git checkout -q --detach <tip>` — the branch name is never the checkout target, so a sibling holding `main` cannot fail it.
- The unit checks every step and stops at the first failure (exit 7 mid-unit); HEAD state (detached, at the base tip) is asserted after each step. Exit codes: 3 primary, 4 dirty (without `--clean`), 5 unknown/stale base ref (worktree left untouched), 7 mid-unit failure.
- General rule: any multi-step destructive git unit must (a) guard as a unit — check each step, stop at the first failure — (b) prefer ref-independent targets (`--detach <sha>` over a shared branch name), and (c) assert the resulting state instead of assuming it.
- Regression coverage: `tests/worktree-guard/test_reset.bats` (sibling-holds-base-branch case is test 5).
- Bats setup gotcha learned while writing these tests: on git 2.34.1 `git branch -M main <cur>` fails ("Cannot force update the current branch") — create `main` with `checkout -b main` from the clone instead of renaming the current branch.
