---
name: autospec-run monitor session-reset feature — brainstorming in progress
description: Feature to make the Phase 4 monitor self-terminate after a batch and relaunch with fresh context; brainstorm started 2026-05-07, waiting for user answer on trigger mechanism
type: project
wing: episodic
drawer_class: session-log
originSessionId: 82cb28f4-e9f3-4587-b81c-68d4714201c0
---
User request: "ensure that when you use autospec-run that you clear the session from time to time and than resume. It should be stateless itself, since all it does is hand off issues to subtasks."

**Context:** The Phase 4 monitor currently has no self-termination logic. Context overflow happens at ~185 tool calls (~4-5 issues). The `monitor_tick` counter exists but is only used for the 12-iteration watchdog pass. All persistent state already lives in GitHub labels + heartbeat files.

**Brainstorm status:** COMPLETE. Design approved, spec committed to main.

**Decisions made:**
- Trigger: issue count (every 3 issues) via `AUTOSPEC_BATCH_SIZE` env var (default 3)
- Relaunch: orchestrator relaunches (not monitor self-relaunch)
- Signal mechanism: Approach B — `~/.autospec/batch-done.json` file (harness-neutral)
- Status values: `BATCH_COMPLETE` (hit limit, more issues remain) or `ALL_DONE` (queue drained)
- Missing file = treat as `BATCH_COMPLETE` (safe crash recovery)
- Batch counter increments after each completed process(ISSUE) call (merge OR failure)

**Spec:** `docs/specs/2026-05-07-monitor-session-reset-design.md` (committed to main as b62af17)

**Affected files:**
- skills/autospec-run/SKILL.md + codex/prompt.md + opencode/agent.md (lock-step trio)
- skills/autospec/SKILL.md + codex/prompt.md + opencode/agent.md (lock-step trio)
- tests/unit/test_monitor_batch_exit.bats (new)

**COMPLETED 2026-05-07.** PR #305 merged. PR #306 (guardian+LGTM fusion + off-peak tip) CI fix pushed (1eaaede) — test_phase4_guardian_trio.bats checks for GUARDIAN_PASS which fusion removed; updated to grep -qE 'GUARDIAN_PASS|LGTM'. Awaiting merge.

**Commits landed:**
- `7978d68` feat(validate): add check_monitor_batch_exit()
- `ae2c1d2` test(monitor): add bats for batch self-termination
- `5f5e065` feat(autospec-run): batch self-termination + orchestrator relaunch loop
- `e038491` chore(autospec-run): lock-step sync
- `aa07dd8` feat(autospec): batch self-termination + orchestrator relaunch loop
- `9a82052` chore(autospec): lock-step sync
- `81a2291` fix(validate): tighten heading check

**Gotcha:** `check_monitor_batch_exit` must grep for `^## Phase 4 — Background autonomous monitor` (exact heading), not just `Phase 4`, because autospec-define and autospec-review both mention "Phase 4" in prose or have their own Phase 4 sections.

**How to apply:** Feature is live. No further action needed.
