---
name: feedback-validate-sh-lockstep-duo-gap
description: "validate.sh check_lockstep() guards on all 3 trio files (SKILL.md + opencode/agent.md + codex/prompt.md); falls open when a skill ships only 2 (e.g., autospec-test has no opencode/agent.md) — body divergence between SKILL.md and codex/prompt.md goes undetected"
metadata: 
  node_type: memory
  tags: 
    - autospec
    - validate
    - lockstep
    - ci-gap
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 2d7883e9-9977-428f-8919-ef9b88df12a4
---

**Discovered 2026-05-22 on PR #412 review:** the fused guardian+LGTM reviewer caught a DOC_OUT_OF_SYNC violation that CI passed clean — Stage 2.5 + docs drift-gate block was added to `skills/autospec-test/SKILL.md` but NOT to `skills/autospec-test/codex/prompt.md`. The top-level `check_lockstep()` loop in `validate.sh` conditions on all three trio files existing, and `autospec-test` has only 2 of 3 (no `opencode/agent.md`), so the loop body never executes for that skill.

**Failure mode:** silent divergence. The codex/prompt.md is meant to be the body of SKILL.md verbatim (per AGENTS.md lockstep rule). A reader invoking the skill via Codex CLI gets stale content.

**Fix the gap:** `check_lockstep()` should ALSO run as a **duo** check (`SKILL.md` ↔ `codex/prompt.md`) when `opencode/agent.md` is absent. Same byte-diff after frontmatter stripping. File as a hardening follow-up.

**How to apply:**
1. When designing lockstep changes for skills with only the SKILL.md+codex/prompt.md pair (no opencode), MANUALLY verify byte-equivalence after frontmatter stripping — don't rely on validate.sh until it grows the duo check.
2. When writing new skill scaffolds, prefer all three trio files (or accept lockstep skipping for skills that only ship under Claude Code).
3. Follow-up issue should fix `check_lockstep()` to support the duo case.
