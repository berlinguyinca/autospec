# autospec-run SKILL.md Body Split (Token Efficiency)

**Date:** 2026-08-15
**Status:** proposed
**Builds on:** D5 duplicate-read elimination, trio-derivation (`derive-trio.sh`), block-expansion goldens, the existing separate-file prompt pattern (`phase4-implementer.md`, `implementer-contract.md`, `reviewer-contract.md`)

## Goal

Reduce the persistent orchestrator-context cost of the `autospec-run` skill body. The 140K body (~35K tokens) loads in full at every `/autospec-run` invocation and occupies orchestrator context for the whole run, competing with issue bodies, PR diffs, and test output for context headroom. Moving infrequently-consulted content to on-demand reference files (read at their trigger point) shrinks the persistent cost.

## Honest cost model (measured 2026-08-15)

| Region (lines) | Bytes | ~Tokens | Status |
|---|---|---|---|
| Run-start one-shots (17–280) | 17,153 | 4.3K | movable — consulted once at start |
| Phase 4 monitor loop (281–1630) | 101,000 | 25K | mostly pinned / essential |
| Phase 5 periodic status (1631–1646) | 860 | 0.2K | movable |
| Phase 6 final report (1647–1704) | 2,492 | 0.6K | movable |
| Phase 5.5 gap remediation (1705–1820) | 10,316 | 2.6K | heading pinned, content movable |
| Phase 5.6 quality audit (1821–1847) | 1,393 | 0.35K | movable |
| Advisor escalation (1848–1870) | 3,904 | 1.0K | movable |
| Constraints (1871–1881) | 1,147 | 0.3K | movable |
| Autonomous mode (1882–1902) | 1,027 | 0.26K | movable |
| **Total** | **140,344** | **~35K** | |

**Realistic ceiling** (≈4 bytes/token): Phase 1 (end-of-run tail, 18,105 B) ≈ **4.5K tokens (~13%)**; Phase 1+2 (tail + run-start, 35,258 B) ≈ **8.8K tokens (~25%)**; Phase 1+2+3 (tail + run-start + inline sub-procedures) ≈ **12–14K tokens (~35–40%)**. This is **not** an order-of-magnitude reduction — the Phase 4 loop is 72% of the file and its core control flow must stay in context.

## Pinned sections (MUST remain in the trio body)

`crates/autospec-core/src/validation/structural.rs` asserts these exist in **all three** trio members (`require_section` / `require_line_prefix` / lock-step checks). Extraction may move their non-pinned *content* but never the pinned heading/block:

- `## Stop mode` — `validate_stop_mode_sections`
- `## Phase 5.5 — End-of-run gap remediation` — `validate_gap_remediation_sections` (heading only)
- `## Phase 4 — Background autonomous monitor` + `batch_issue_count` / `AUTOSPEC_BATCH_SIZE` / `batch-done.json` — `validate_monitor_batch_exits`
- `## Team personality as execution lens`, `## Review counter-team as review lens`, `Critical self-question before LGTM` — `validate_team_personality_phase4_and_docs_contract`
- Harness detection section + `silently` fallback reference — `validate_harness_detection`
- priority-sort lock-step block — `check_autospec_run_priority_sort_lockstep`
- regression-review lock-step block — `check_autospec_run_regression_review_lockstep`

## Extraction pattern

Follow the existing, proven separate-file pattern the body already uses for subagent prompts:

1. Move the detailed content to `skills/autospec-run/references/<name>.md`.
2. Leave the pinned heading (where one exists) plus a one-line **MUST-read pointer** in the body at the exact trigger point.
3. Re-derive the trio: `derive-trio.sh skills/autospec-run --in-place`.
4. Regenerate goldens: `gen-skill-goldens.sh autospec-run`.
5. Add/extend a bats test asserting each pointer is present in the body and its reference file exists and is non-empty.

## Phased plan

**Phase 1 — end-of-run tail (this PR).** Move Phase 5.5 content + Phase 5.6 + Phase 6 + advisor escalation to `references/end-of-run.md`. Leave the `## Phase 5.5` heading + a MUST pointer, and MUST pointers where the other three sections were. ~4.5K tokens. Lowest risk: clean `##` boundaries, consulted once at end-of-run, not in the hot loop.

**Phase 2 — run-start one-shots.** Move self-update, status, invocation, memory injection, batch timestamp, explore-on-drain, session lock to `references/run-start.md`. Leave `## Stop mode` + harness detection (pinned) + MUST pointers. ~4K tokens.

**Phase 3 — inline sub-procedures (aggressive).** Extract the rebase-and-retest gate, CI-status comparison, and reuse-BLOCK refute pass from the Phase 4 loop to `references/`. ~3–5K tokens. Higher risk (mid-loop); each needs a precise MUST pointer at its trigger step.

## Safety invariants

- Every pointer is a bold **MUST** at the exact trigger point, not a passive "see also".
- Pointers follow the existing `phase4-implementer.md` reference convention.
- Pinned headings/blocks never move; only their non-pinned content does.
- Trio lock-step + goldens are regenerated in the same PR as the move.
- A missed pointer read degrades (an end-of-run step is skipped) but never corrupts (the Phase 4 hot loop is untouched).

## Non-goals

- Do not extract the Phase 4 core control flow (queue scan, claim, dispatch, review, merge).
- Do not alter the subagent prompt files (already extracted).
- Do not change merge safety, worktree isolation, or validation gates.

## Validation

- `bats tests/skill-body-split.bats` (new — pointer + reference-file presence)
- `bats tests/issue-snapshot.bats tests/derive-trio.bats tests/gen-skill-goldens.bats` (regression)
- `derive-trio.sh skills/autospec-run --check`
- `gen-skill-goldens.sh autospec-run` (no-op after regen)
- `scripts/lint-implementation.sh --diff-file <origin/main..HEAD>`
- `cargo test -p autospec-core` (structural validation incl. `validate_gap_remediation_sections`)
