---
name: validate.sh has named-content checks beyond lock-step
description: validate.sh checks specific prose phrases in skill files (not just lock-step equality), so renaming or rewriting sections requires updating validate.sh too
type: feedback
originSessionId: 82cb28f4-e9f3-4587-b81c-68d4714201c0
---
When editing prose inside SKILL.md files, validate.sh enforces named-content checks beyond just lock-step sync. Examples found in practice:

- `check_autospec_run_regression_review_lockstep()` — checks for `"Regression review escalation"` and `"Tier A (spec work)"` in autospec-run/opencode/agent.md and codex/prompt.md
- `check_subagent_model_tier()` — checks for specific tier-brief patterns
- `check_harness_detection_block()` — checks for `## Harness detection` heading + TIER_A/TIER_B/silently
- `check_monitor_batch_exit()` — checks for batch_issue_count/AUTOSPEC_BATCH_SIZE/batch-done.json/BATCH_COMPLETE/ALL_DONE

**Why:** These checks catch partial migrations and ensure the installed skill files work as documented.

**How to apply:** Before finalizing any edit to skill prose that renames a section heading or removes a specific phrase, run `bash scripts/validate.sh` first. If it fails, either keep the old phrase in the new text OR update the corresponding check in validate.sh. Prefer updating validate.sh to accept both old and new formats (using `grep -qE "old|new"`) to avoid breaking CI during transitions.
