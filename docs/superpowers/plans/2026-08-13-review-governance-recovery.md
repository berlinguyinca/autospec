# Review Governance Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the abandoned PR #3010 work, reconcile it with the replacement PR series and current `origin/main`, and land only behavior still missing from `main` in small validated PRs.

**Architecture:** Treat the existing branch as immutable recovery evidence, not as an integration branch. Create archival refs for committed and dirty state, analyze each governance capability against current `main`, then reconstruct missing capabilities on fresh branches from `origin/main`. Keep runtime review governance separate from autonomous claim/runtime refactors and repository-quality fixtures.

**Tech Stack:** Git, GitHub CLI, Rust workspace, Bash/Bats validation, autospec multi-harness skill lock-step artifacts.

**Spec:** `docs/specs/2026-08-11-review-governance-self-improvement-design.md`

## Global Constraints

- Preserve the exact committed and uncommitted recovery state before integration work.
- Merge or branch from the latest fetched `origin/main`; never force-push or amend.
- Do not revive PR #3010 wholesale; it was superseded by PRs #3016–#3046.
- Keep each replacement diff within the implementation-quality size and scope gates.
- Maintain lock-step equality for every modified `SKILL.md`, `opencode/agent.md`, and `codex/prompt.md` trio.
- Add or retain regression coverage before modifying behavior.
- Merge only after all non-advisory CI checks pass.

---

### Task 1: Preserve Recovery State

**Files:**
- Preserve: all tracked and untracked files in the current worktree
- Create: archival Git refs and a recovery commit on a dedicated branch

**Interfaces:**
- Consumes: branch `feat/review-governance-self-improvement` and its index/worktree
- Produces: immutable archival refs for committed HEAD and dirty recovery state

- [ ] **Step 1: Record exact state**

Run `git status --porcelain=v2`, `git diff --binary`, and `git diff --cached --binary`; save hashes and paths in the recovery commit message.

- [ ] **Step 2: Create committed-state archive ref**

Create `archive/review-governance-3010-committed` at current `HEAD` without moving or rewriting it.

- [ ] **Step 3: Create dirty-state recovery branch and commit**

Create `recovery/review-governance-3010-dirty`, add the 25 existing changes plus this plan, and commit with `chore: preserve review governance recovery state`.

- [ ] **Step 4: Verify preservation**

Run `git fsck --no-reflogs --unreachable`, `git status --short`, and compare the saved binary patches with the recovery commit tree.

### Task 2: Build Supersession Matrix

**Files:**
- Create: `reports/review-governance-recovery/supersession-matrix.md`
- Read: PR #3010 and PRs #3016–#3046
- Read: committed diff of `archive/review-governance-3010-committed` against current `origin/main`

**Interfaces:**
- Consumes: archived branch, current `origin/main`, replacement PR metadata
- Produces: per-capability disposition `integrated`, `missing`, or `obsolete`, with commit/file/test evidence

- [ ] **Step 1: Enumerate capabilities**

List review policy, provider identity, exact-commit receipt binding, integration smoke proof, gap attribution, experiment governance, rollback, and autonomous advancement separately.

- [ ] **Step 2: Map replacement PRs**

For every capability, cite the replacement merge commit and current-main files/tests that implement it.

- [ ] **Step 3: Prove missing behavior**

For each `missing` row, provide a failing current-main test or a concrete absent symbol/contract; do not classify solely from commit history.

- [ ] **Step 4: Commit the matrix**

Commit as `docs: map review governance supersession` on a fresh `docs/review-governance-recovery` branch from current `origin/main`.

### Task 3: Recover Missing Governance Runtime Behavior

**Files:**
- Modify only files identified as `missing` in the supersession matrix
- Test alongside the owning Rust or Bats subsystem

**Interfaces:**
- Consumes: `missing` runtime rows from Task 2
- Produces: one bounded `fix/*` or `feat/*` branch per independently testable runtime capability

- [ ] **Step 1: Create a failing regression test**

Write the smallest test that fails on current `origin/main` and passes on the archived implementation.

- [ ] **Step 2: Verify red state**

Run the exact targeted Rust or Bats test and record the expected assertion failure.

- [ ] **Step 3: Port minimal implementation**

Copy behavior, not branch structure: reuse current-main modules and avoid restoring superseded abstractions.

- [ ] **Step 4: Verify green state and gates**

Run targeted tests, `cargo test --workspace`, applicable Bats validation, lock-step validation, file-size ratchet, and `git diff --check`.

- [ ] **Step 5: Commit and open PR**

Use a conventional commit, push without force, and open a PR referencing the supersession-matrix row.

### Task 4: Recover or Retire Uncommitted Runtime/Claim Refactor

**Files:**
- Review: the 21 Rust staged/unstaged files preserved in Task 1
- Test: existing claim, queue, executor-bridge, and validation suites

**Interfaces:**
- Consumes: dirty recovery commit and current-main refactors
- Produces: bounded missing refactor PRs, or documented `integrated/obsolete` dispositions

- [ ] **Step 1: Compare each extracted module semantically**

Check `blocked_cycle`, `gh_read`, `one_shot_selector`, `program`, claim lease, and harness-support changes against current-main modules and merged PRs.

- [ ] **Step 2: Require a behavioral regression**

Retain a slice only if a current-main test can demonstrate missing behavior or the extraction is required to pass the active file-size ratchet.

- [ ] **Step 3: Port each retained slice from latest main**

Create a fresh branch per coherent claim/runtime slice; never merge the recovery branch into it.

- [ ] **Step 4: Validate and submit**

Run serial targeted tests for shared-state claim/executor suites, then CI-equivalent checks and open a bounded PR.

### Task 5: Recover or Retire Quality-Audit Support Slice

**Files:**
- Review: `skills/autospec-shared/scripts/repo-quality-audit.sh`
- Review: `skills/autospec-shared/tests/unit/repo-quality-audit*.bats`
- Review: `tests/dogfood/allowlist/qa-brute-force-sweep.json`
- Review: `tests/dogfood/test_driver.bats`

**Interfaces:**
- Consumes: preserved quality-audit changes
- Produces: one standalone test-backed PR or an `integrated/obsolete` matrix update

- [ ] **Step 1: Run current-main tests**

Run the exact repo-quality-audit Bats targets and dogfood driver checks to identify a reproducible gap.

- [ ] **Step 2: Port only necessary changes**

Keep script behavior, tests, and allowlist updates together; exclude unrelated runtime files.

- [ ] **Step 3: Validate**

Run `bash -n`, targeted Bats tests, dogfood detector validation, and `autospec validate --fast`.

- [ ] **Step 4: Commit and open PR**

Commit as a bounded fix/test change and reference the matrix evidence.

### Task 6: Merge, Close, and Clean Up

**Files:**
- Update: `reports/review-governance-recovery/supersession-matrix.md`
- Remove: obsolete recovery worktree only after every row has a terminal disposition

**Interfaces:**
- Consumes: all replacement PR outcomes
- Produces: current `main` containing all verified missing behavior and no unclassified recovery work

- [ ] **Step 1: Update every replacement branch from latest main**

Use normal merge commits or GitHub update-branch; resolve overlaps sequentially and rerun focused tests.

- [ ] **Step 2: Merge green PRs**

Require all non-advisory checks green, then squash-merge and fetch/prune locally.

- [ ] **Step 3: Final verification**

Run targeted merged-state tests, `cargo test --workspace`, shell validation, worktree inventory, and confirm no open recovery PR remains.

- [ ] **Step 4: Preserve archives and remove obsolete checkout**

Keep both archive/recovery refs, remove the clean obsolete worktree, and delete only local integration branches proven merged.

- [ ] **Step 5: Report residual risks**

List baseline test failures separately from recovery defects and identify any deliberately archived-but-not-landed behavior.
