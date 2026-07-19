---
name: Harness-aware model tier resolution — in progress
description: Feature to make autospec auto-detect its harness and use only that harness's models; spec + 6 issues filed 2026-05-07, awaiting run/defer/refine decision
type: project
wing: episodic
drawer_class: session-log
originSessionId: 82cb28f4-e9f3-4587-b81c-68d4714201c0
---
Spec landed as PR #290 (merged to main 2026-05-07):
`docs/specs/2026-05-07-harness-aware-model-tier-design.md`

Umbrella epic: issue #291
Child issues: #292 (AGENTS.md), #293 (validate.sh), #294 (autospec/SKILL.md), #295 (autospec-run/SKILL.md), #296 (autospec-define/SKILL.md), #297 (autospec-classify + autospec-review SKILL.md)

**Why:** Verbose multi-harness tier strings (e.g. "Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier") appear in 8+ subagent dispatch briefs across all 5 skill files. When running in Claude Code, the briefs carry GPT model names that Claude cannot use — fragile, token-wasteful, confusing.

**Approved design:**
- Add `## Harness detection (run once at skill start)` block before Phase 0 in each SKILL.md; detects harness via tool availability (Agent+subagent_type = CC; task+model = OpenCode; apply_patch = Codex)
- Resolves `TIER_A` and `TIER_B` once; all tier briefs reference those vars
- **Silent fallback:** if TIER_B unavailable, silently use TIER_A — never ask the user
- Scope: all 5 skills + AGENTS.md + validate.sh

**Issue classification:** #292 ctx:32k/medium; #293-#297 ctx:64k/medium. Clean DAG: #292→#293→{#294,#295,#296,#297}. All lint-clean.

**COMPLETED 2026-05-07.** All 6 issues closed, all PRs merged (#298-#299, #300-#302 batch, #304).

**What shipped:**
- AGENTS.md: `### Harness detection protocol` subsection
- autospec validate + tests/unit/test_harness_detection_block.bats: new check + dual-format acceptance
- 5 skills × 3 lock-step files each = 15 files updated with `## Harness detection` block + TIER_A/TIER_B refs

**How to apply:** Feature is live. No further action needed.
