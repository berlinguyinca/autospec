---
name: project-memory-consumers-epic
description: Epic
metadata:
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 2e9dc40b-7a14-4cff-8d65-1043d1718ed0
---

**STATUS: ALL 7 SHIPPED 2026-05-24** — drained in one `/autospec-run` (PRs #518–524, all admin-squash-merged, 0 failures, 0 deferred). Post-batch `/autospec-review` confirmed full functional coverage + intact lockstep trios; only warn-level docs-drift remained (8 new scripts not yet named in the cross-tool-memory spec; cross-repo/GC still listed "deferred"). User chose to rely on #515's queued `docs:drift` self-heal rather than file/fix. Epic #517 tracker left open/untouched. In practice the issues had NO `Depends-on` declarations (the "needs #512" notes below were design intent, not encoded deps) — all 7 were independent and processed oldest-first.

User reviewed the repo post-M5 merge and said *"ok please do all of these"* after I surfaced 7 follow-up ideas. Picked autospec pipeline mode + all 7 in flight at once.

**Epic #517** (tracker, not auto-implement). Children all carried `auto-implement` + `skill:autospec-run` (now CLOSED):

| # | Issue | What | Dep |
|---|---|---|---|
| 1 | #512 | Inject relevant memory into autospec-review / classify / define / monitor prompts | none — KEYSTONE |
| 2 | #510 | AAAK compression GC via mempalace compress | none |
| 3 | #511 | Mine PR descriptions + git log into lesson_*.md | none |
| 4 | #515 | Generalize ensure-tool.sh for bats/jq/gh/mempalace (refactor of #509 helper) | none |
| 5 | #513 | Diary auto-write on Phase 1→2 + Phase 4 monitor boundaries | needs #512 |
| 6 | #516 | /autospec-stop --resume driven by feedback_monitor_silent_exit.md | needs #512 |
| 7 | #514 | Cross-repo memory traversal via ~/.autospec/memory-index/ symlink farm | needs #512 |

**Why this matters:** M1-M5 built the shared-memory FLOOR. Without consumers, the 30 memory files just sit there. #512 is the keystone — once skills actually read memory, the other items have a clear integration target. #515 generalizes the auto-install pattern proved out in PR #509.

**How to apply when resuming:** check `gh issue list --label auto-implement --search "memory in:title"` to see what the monitor has picked up. The pipeline's batch=3 default means 3 in flight at once max (per feedback_autospec_design_prefs.md). If #512 lands first, the dependent children (#513/#514/#516) can proceed without waiting; otherwise the monitor may serialize them. No new labels were created — `skill:autospec-stop` doesn't exist; #516 uses `skill:autospec-run` instead.

Related: [[project-cross-tool-memory-brainstorm]] (M1-M5 SHIPPED), [[feedback-mempalace-integration-boundary]] (auto-install scope decision in PR #509).
