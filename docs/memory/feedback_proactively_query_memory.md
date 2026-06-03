---
name: proactively-query-memory
description: "Meta-rule. Passive auto-load isn't enough; do a 5-second \"what memory applies here?\" check before each significant decision (dispatch, recovery, filing, design choice, bash code)."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 37f5bea4-90df-4cf3-8298-d8158b15d2ca
---

Auto-memory is loaded into context at session start, but loading != using. Honest audit after the 2026-06-01 session: only ~4 of 25 memories were actively referenced before decisions, despite at least 8 being applicable. Memory only works if I do the consult step.

**The rule:** before each significant action, run a one-sentence mental check: "Is there a memory in MEMORY.md that applies to what I'm about to do?" If yes, open it (Read tool) and let it shape the decision. If no, proceed.

**Significant actions that warrant the check:**
- Dispatching a subagent (esp. autospec / long-running monitors) → `feedback_monitor_silent_exit.md`, `feedback_admin_merge_denial.md`, `feedback_autospec_autonomy_scope.md`
- Recovering from a stall / silent-exit → `feedback_monitor_silent_exit.md`, `feedback_heartbeat_cross_repo_collision.md`
- Filing GitHub issues → `feedback_autospec_decomposer_gotchas.md`, `feedback_autospec_skill_per_capability.md`, `feedback_roi_check_new_components.md`
- Writing bash / shell → `feedback_bash_return_trap_leak.md`, `feedback_bash_set_e_short_circuit.md`, `feedback_autospec_no_shell_user_text.md`
- Editing validate.sh / lockstep files → `feedback_validate_sh_lockstep_checks.md`, `feedback_validate_sh_lockstep_duo_gap.md`
- Before invoking `/autospec-split` or `/autospec-define` → `feedback_pre_pipeline_sync.md`, `feedback_autospec_split_origin_main_gate.md`
- Before declaring a feature "done" → `feedback_per_pr_lgtm_misses_integration.md` (never skip Phase 5.5)
- Designing a new component or skill → `feedback_roi_check_new_components.md` (named consumer? upstream invocation possible?)

**When you catch yourself NOT having checked:**
- Surface it to the user briefly (one sentence — "should have checked memory for X")
- Save a new memory if it's a recurring miss

**When a memory turns out to be wrong:**
- Update or remove it immediately (memory rot is worse than no memory)

**The discipline:** the 5-second check feels expensive but compounds. Each correctly-applied memory saves minutes of re-discovery later. Treat unchecked memory as technical debt against future sessions.

Related: [[per-pr-lgtm-misses-integration]] (concrete example of a memory that would have saved a Phase 5.5 round if proactively used).
