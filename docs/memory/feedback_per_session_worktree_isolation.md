---
name: feedback_per_session_worktree_isolation
description: "Concurrent autospec/claude sessions stomp each other; start every edit in a fresh worktree off origin/main + claim files, never edit on the shared/in-flight branch"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 59916493-7c02-4311-889a-2cbeccc1c071
---

When multiple autospec or claude sessions run against the same repo, they step
on each other's code. In the 2026-06-15 session I edited the autospec-explore
trio directly on the in-flight `feat/closeout-report-contract` branch; meanwhile
`main` had advanced 15 commits (refactoring the very `SRC_WEIGHTS`→
`DEFAULT_SRC_WEIGHTS` file I touched) and other live worktrees existed
(`fix/explore-codebase-signals-precision`, `fix/installer-ships-runtime-libs`).
Result: a conflicted PR and full rework onto a fresh branch.

**Why:** the trio + golden + validate machinery makes every skill edit a
multi-file atomic unit; two sessions touching the same skill = guaranteed
lockstep/golden conflicts. Editing on someone else's branch or a stale base
compounds it. The repo already ships the primitives to prevent this
(`worktree-guard.sh` with in_primary_checkout / dirty / stale_base exits;
path-scoped heartbeats; atomic cross-machine issue-claim).

**How to apply:**
1. Before ANY code edit, `git fetch origin` and create/enter a dedicated
   worktree on a new branch off fresh `origin/main` — never the shared primary
   checkout, never an unrelated in-flight feature branch. Run
   `worktree-guard.sh assert` and refuse on stale_base.
2. Pre-flight overlap scan: check open PRs + `git worktree list` + other
   branches for in-flight edits to the same files/skill; if another live session
   owns that skill, pick different work or coordinate.
3. Prefer a file/skill-granular claim (lease with heartbeat TTL under a
   path-scoped `~/.autospec/claims/…`) layered on the existing issue-claim
   infra, so two sessions don't both edit one trio.
4. Keep my work in its own branch/PR; stage only my files; leave others'
   uncommitted changes untouched.

**Refinement (2026-06-26, /autospec-autonomous run):** `claim-issue.sh`'s atomic
label swap does NOT protect you against a **claim-blind** parallel run. A separate
flow (distinctive `wt-rl-*` worktrees / `feat/reuse-*` branches, merging via admin,
not using the claim/heartbeat/registry system) merged the identical work for #1439
(PR #1446) while I held `in-progress-by-bot` — wasting a full ~110k-token implementer
dispatch on a duplicate conflicting PR (#1447, closed as superseded). Lessons:
(a) AFTER claiming an issue and BEFORE dispatching the expensive implementer, run a
**topic-keyed overlap scan** — search local `git worktree list`, `git ls-remote
--heads origin`, and open PRs for branches/worktrees matching the issue's SUBJECT
(not just its number); a rival's local un-pushed worktree (e.g. `/private/tmp/wt-rl-*`)
is the early-warning sign. (b) One collision = pre-existing work, recover and continue;
seeing rival worktrees pre-staged for DOWNSTREAM issues (#1440/#1442 here) is
predictive — stop and surface to the operator rather than racing. (c) When the operator
confirms "park & monitor," hold the session lock, persist a watermark
(`~/.autospec/autonomous-park-watermark.json`), and re-engage only on stall.

Related: [[feedback_subagent_cwd_pinned_to_main_checkout]],
[[feedback_heartbeat_cross_repo_collision]], [[reference_harness_session_id_envs]],
[[reference_worktree_main_topology]].
