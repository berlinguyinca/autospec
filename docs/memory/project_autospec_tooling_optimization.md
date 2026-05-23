---
name: project-autospec-tooling-optimization
description: Queued follow-on after autospec-test v1+v2 complete — convert LLM-driven steps to deterministic tooling and audit script-reference discipline
metadata: 
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 2d7883e9-9977-428f-8919-ef9b88df12a4
---

User asked 2026-05-21, immediately after v1+v2 plans were filed and the v1 implementation monitor was in its final batch. Verbatim:

> "after this is all done, please than optimize this skill, generate tooling to avoid usage of tokens. Ensure the tools can be referenced correctly and dont need to be copied into project directories. Basically what doesn't need a LLM and can be done determistic, should be done determistic with proper tools. If these tools don't exist, write them and test them properly, utilize autospec for as much as possible"

**Scope (4 pillars):**

1. Replace LLM-driven steps with deterministic tools where possible:
   - Issue body generation (Phase 3 decomposer) — most fields are templatable; LLM only for Goal sentence + Implementation outline specifics
   - Phase 3.5 model-fit classification — mostly mechanical (file count + verb keyword matching against rubric) — escalate to LLM only on ambiguity
   - PR comment composition — 100% template-driven from gate JSON
   - Implementer prompt assembly — deterministic template fed from issue body fields
   - Guardian rule expansion — grow lint-implementation.sh to cover more RULE_IDs (BASH_RETURN_TRAP_LEAK, EVAL_USER_INPUT, etc. that surfaced real bugs in PR #331)

2. Tooling discipline — no copies into target repos:
   - All helper scripts live at `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}` per existing convention
   - Skill files reference via `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/<tool>"` exactly like lint-issue.sh, autospec-watchdog.sh, lint-implementation.sh
   - Audit current autospec-test skill — find places where scripts currently expected in `skills/autospec-test/scripts/` and decide which should shift to centralized location

3. Missing tools — build + test them. Candidates known so far:
   - `gen-issue-skeleton.sh` — produces child-issue body template from spec section anchor
   - `classify-model-fit.sh` — deterministic ctx/reasoning classifier with LLM fallback only when conf < threshold
   - `gen-pr-report.sh` — composes autospec-test PR comment from gate JSON
   - `gen-implementer-prompt.sh` — fills implementer template from issue body
   - Extended `lint-implementation.sh` covering RULE_IDs the fused reviewer surfaced repeatedly

4. Dogfood — run the optimization through autospec itself: write a design spec at `docs/specs/2026-05-22-autospec-tooling-optimization-design.md`, /autospec-split into linked issues, monitor implements.

**Why this matters now:** every Phase 4 implementer subagent burns ~50–250 tool calls per issue, mostly on tasks that could be template-driven. The token bill scales linearly with issue count. Token reduction is also the path to running on smaller local LLMs (qwen3-32b laptop profile already declared in ~/.autospec/model-profiles.yml).

**How to apply:** After v1 #328 merges AND v2 10 issues all merge AND v2 monitor exits ALL_DONE → invoke /autospec-define against this scope. Decompose the optimization itself into issues with /autospec-split. Run via /autospec-run. Same pipeline that's been working.

**Reference:** the work itself unblocks at the same moment as v1+v2 completion — no separate gating needed. Pick this up on resume even if user doesn't re-prompt.
