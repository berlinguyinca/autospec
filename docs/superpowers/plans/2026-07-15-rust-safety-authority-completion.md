# Rust Safety Authority Completion Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `autospec queue review-safety` the sole automatic writer of issue-intent safety outcomes and remove every shell or prompt bypass.

**Architecture:** Admission surfaces may persist ordinary issue metadata and add the interim `auto-implement` label, but only the Rust queue command may create a passing safety stamp or quarantine an issue. The external validator prevents legacy writers from returning, while the promoter periodically retries safe-but-unreviewed interim issues.

**Tech Stack:** Rust workspace validation, Bash/Bats contract tests, lock-step skill trios.

## Global Constraints

- Preserve Rust as the only automatic writer for `safety:reviewed`, `security:quarantined`, and the `## Safety review` marker.
- Keep every multi-harness skill body lock-step through `scripts/derive-trio.sh`.
- Write and observe a failing regression test before each behavior change.
- Do not add dependencies or mutate the primary checkout.

### Task 1: Guard the authority boundary

**Files:**
- Modify: `crates/autospec-core/src/validation/external.rs`
- Test: `crates/autospec-core/src/validation/external.rs`

- [x] Add a failing validator test that plants a direct safety write in an operational skill surface.
- [x] Run the focused Rust test and confirm it fails because the current validator permits that writer.
- [x] Expand the validator surface inventory to reject the retired writer and direct safety mutations in skill trios and explorer runtime code.
- [x] Re-run the focused Rust test and confirm it passes.

### Task 2: Replace prompt-level decision writers

**Files:**
- Modify: `skills/autospec-classify/SKILL.md`
- Modify: `skills/autospec/SKILL.md`
- Modify: `skills/autospec-define/SKILL.md`
- Modify: `skills/autospec-run/SKILL.md`
- Derive: matching `codex/prompt.md` and `opencode/agent.md` files
- Test: `tests/unit/test_phase3_lint_integration.bats`

- [x] Change the contract test to require exact-target Rust safety review and reject prompt-owned safety mutation.
- [x] Run it and confirm it fails against the legacy wording.
- [x] Replace each writer with final-body persistence, interim admission, and `autospec queue review-safety --repo {repo} --limit 1 --issue <N>`.
- [x] Derive all mirrors and re-run the Bats contract test.

### Task 3: Replace explorer safety authority and repair retryability

**Files:**
- Modify: `scripts/autospec-explore.sh`
- Modify: `scripts/autonomous-promote-open-issues.sh`
- Test: `tests/explore/test_explore_once.bats`
- Test: `tests/autospec/autonomous-promote-open-issues.bats`

- [x] Add failing tests proving explorer files an interim issue before Rust review and promoter retries interim issues.
- [x] Remove explorer-authored stamps and labels, invoke exact Rust review after each created issue, and add a bounded promoter retry pass.
- [x] Re-run focused Bats suites and shell syntax validation.

### Task 4: Prove and ship

**Files:**
- Modify as required: docs and GitHub issue #2064 scope

- [x] Update user-facing Rust safety documentation and the issue scope.
- [x] Run format, workspace tests, fast validation, smoke scan, clippy, and diff checks.
- [x] Obtain fresh independent architecture and code reviews over the final diff; resolve every blocking finding.
- [ ] Commit using the Lore protocol, push, open a PR, run implementation lint and CI, then admin-merge only after all required checks pass.
