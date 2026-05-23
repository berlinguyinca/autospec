---
name: autospec-review implementation status
description: autospec-review SKILL.md fully implemented as of 2026-05-07; all T7-T24 issues merged via PRs #271-#289
type: project
originSessionId: 5e769f84-c8f2-46ae-90c1-f0732ff229db
---
`skills/autospec-review/SKILL.md` is **fully implemented** as of 2026-05-07. All 20 tasks (T7–T24, issues #247–#265) were merged via PRs #271–#289 in a single autospec-run session (~2 hours). The skill is live and invokable via `/autospec-review`.

**Why:** Scaffolded in PR #266 with reference docs in PRs #267–#269 (gap-taxonomy, csv-schema, subagent-contract, reviewer-prompt). The implementation queue ran T8-T24 autonomously via autospec-run.

**How to apply:** `/autospec-review` now has a runnable Phase 0-7 body. Use it to audit design specs against open/closed issues for gaps.

**What landed (key PRs):**
- #271 (T7): `templates/regression-spec.md.tmpl`
- #272–#278 (T8–T13b): TDD utility modules — compute_gap_id, spec discovery, linkage matrix, JSON validation, CSV writer, run_id generator, argparse CLI
- #279 (T14): SKILL.md body Phases 0-3 (preflight, discovery, dispatch, merge)
- #285 (T15): SKILL.md body Phases 4-6 (render, review, autospec-split, post-process) — also wrote opencode/agent.md + codex/prompt.md lock-step copies
- #280–#283 (T18–T21): autospec-run SKILL.md modifications — queue priority sort, Tier-A LGTM escalation + 2-pass, Phase 6 post-batch `/autospec-review` trigger, lock-step opencode+codex variants
- #284 (T22): bats byte-identity tests for autospec-run lock-step blocks
- #286 (T16): lock-step closure PR (T15 had already written all three harness files)
- #287 (T17): bats skill body byte-identity lock-step test for autospec-review
- #288 (T23): 4 new validate.sh checks for autospec-review
- #289 (T24): SKILLS.md + README registered autospec-review

**Notable implementation quirks from the run:**
- T16 (lock-step copy) was pre-empted by T15 which wrote all three harness files together; T16 PR was an empty-commit closure
- T17 bats test uses `cat codex/prompt.md` (not `sed '1{/^$/d;}'`) to match validate.sh's comparison method
- T23 adapted two check functions: `check_autospec_review_skill_present` uses `cat` not `sed`; `check_autospec_run_regression_review_lockstep` checks for `"Tier A (spec work)"` (actual file text)
