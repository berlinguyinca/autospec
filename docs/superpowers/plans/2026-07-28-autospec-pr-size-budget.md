# Autospec PR Size Budget Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce a deterministic small-PR budget and transparently continue unfinished autonomous work through ordered child issues.

**Architecture:** A dependency-free Rust evaluator owns measurements and verdicts; Rust and shell linters consume it with parity tests. The executor binds the verdict to existing exact-head evidence, while durable receipts publish children through append-only parent reconciliation.

**Tech Stack:** Rust 2024 workspace, Bash, Bats, `gh`, existing Autospec private-state and parent APIs.

## Global Constraints

- Hard limits: 400 additions-plus-deletions, 8 raw files, and 3 logical units.
- Proactive limits: 320 changed lines or 7 raw files while criteria remain.
- A skill adapter trio and its derived goldens count as one logical unit.
- Binary diffs are hard-oversized because their line count cannot be proved.
- Oversized work is never pushed, drafted, readied, or merged; local commits remain intact.
- Only `Guardian: skip-PR_SIZE # <reason>` can request an exception.
- Allowed categories: generated migration, dependency-solver lockfile, mandatory lock-step artifacts.
- Manual implementation and test code are never exempt; valid exceptions emit `INFO:PR_SIZE`.
- Children use `Depends on issue #N`; part PRs use `Part of #N` and close only their child.
- Parent reconciliation closes the umbrella only after every child has an observed merged PR.
- Publication and session notifications are durable, restart-safe, idempotent, and desktop-free.
- Each task uses TDD, adds no dependency, and stays within this PR-size contract or its validated lock-step exception.
- A child touching an inherited file over 400 LOC carries `Guardian: skip-COMPLEXITY # surgical change in legacy <path>; changed functions remain capped`.

---

### Task 1: Typed patch-size evaluator

**Files:**
- Create: `crates/autospec-core/src/lint/pr_size.rs`
- Modify: `crates/autospec-core/src/lint/diff.rs`
- Modify: `crates/autospec-core/src/lint/mod.rs`
- Test: inline tests in `pr_size.rs`

**Interfaces:**
- Consumes: `UnifiedDiff` and current issue logical-unit semantics.
- Produces: `PatchSizeLimits`, `PatchSize`, `PatchSizeDimension`, `PatchSizeEvaluation`, and `evaluate_patch_size(&UnifiedDiff, PatchSizeLimits)`.

- [ ] **Step 1: Write failing boundaries**

```rust
assert!(evaluate(400, 8, 3).hard_dimensions().is_empty());
assert_eq!(evaluate(401, 8, 3).hard_dimensions(), &[PatchSizeDimension::ChangedLines]);
assert_eq!(evaluate(400, 9, 3).hard_dimensions(), &[PatchSizeDimension::RawFiles]);
assert_eq!(evaluate(400, 8, 4).hard_dimensions(), &[PatchSizeDimension::LogicalUnits]);
assert!(evaluate(320, 1, 1).is_proactive());
assert!(evaluate(1, 7, 1).is_proactive());
```

Register `mod pr_size` before the red run; also prove one adapter trio plus its golden is one unit and a binary diff is hard.

- [ ] **Step 2: Capture red**

Run `cargo test -p autospec-core lint::pr_size -- --nocapture`; expect missing module/types.

- [ ] **Step 3: Extend diff evidence**

```rust
pub struct DiffFile {
    pub path: String,
    pub is_new: bool,
    pub is_binary: bool,
    pub hunks: Vec<DiffHunk>,
}
```

Add `removed_line_count` and `changed_line_count`; recognize `Binary files` and `GIT binary patch`.

- [ ] **Step 4: Implement exact limits**

```rust
pub const DEFAULT_MAX_CHANGED_LINES: usize = 400;
pub const DEFAULT_MAX_RAW_FILES: usize = 8;
pub const DEFAULT_MAX_LOGICAL_UNITS: usize = 3;
pub const PROACTIVE_CHANGED_LINES: usize = 320;
pub const PROACTIVE_RAW_FILES: usize = 7;
```

Proactive means changed lines `>= 320` or raw files `>= 7`; hard means any numeric value is `>` its maximum.

- [ ] **Step 5: Remove logical-unit drift**

Expose one `pub(crate)` trio/golden normalizer and replace issue lint’s literal `3` with `DEFAULT_MAX_LOGICAL_UNITS`.

- [ ] **Step 6: Verify and commit**

Run fmt, the focused core tests, core clippy with `-D warnings`, and `git diff --check`; commit `feat: define the autonomous patch size budget`.

### Task 2: Rust `PR_SIZE` rule and exceptions

**Files:**
- Modify: `crates/autospec-core/src/lint/implementation.rs`
- Modify: `crates/autospec-core/src/lint/mod.rs`
- Test: inline implementation-lint tests

**Interfaces:**
- Consumes: Task 1’s evaluator.
- Produces: `ImplementationLintRule::PrSize`, `ImplementationLintOptions::patch_size_limits`, and `ERROR:PR_SIZE` / `INFO:PR_SIZE`.

- [ ] **Step 1: Write failing lint cases**

```rust
assert!(findings_for(&lint_diff(diff_lines(400), None), "PR_SIZE").is_empty());
let result = lint_diff(diff_lines(401), None);
assert_eq!(result.blocking_count, 1);
assert!(result.findings[0].message.contains("changed_lines=401/400"));
```

Cover 8/9 files, 3/4 units, binary, every valid category, bare/unknown reasons, and mixed manual paths.

- [ ] **Step 2: Capture red**

Run `cargo test -p autospec-core lint::implementation::tests::pr_size -- --nocapture`; expect missing `PrSize`.

- [ ] **Step 3: Add the first ordered detector**

Report all measured dimensions and use this exact directive:

```text
Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff.
```

- [ ] **Step 4: Validate shape, not prose**

Generated migrations require a migration path plus generator provenance in diff evidence; lockfiles require a known solver-lock basename; lock-step requires byte-identical adapter hunks or derived goldens with the normalized manual patch within budget. Otherwise retain `Error`.

- [ ] **Step 5: Verify and commit**

Run fmt, implementation-lint tests, core clippy, and diff check; commit `feat: block oversized implementations in Rust`.

### Task 3: Shell linter parity

**Files:**
- Modify: `scripts/lint-implementation.sh`
- Modify: `tests/unit/test_lint_implementation.bats`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: Tasks 1-2 constants, messages, and exception categories.
- Produces: equivalent shell findings and documented `PR_SIZE` policy.

- [ ] **Step 1: Write failing Bats cases**

```bash
run "$LINTER" --base "$base" --head "$head"
[ "$status" -eq 1 ]
[[ "$output" == *"ERROR:PR_SIZE:"* ]]
[[ "$output" == *"changed_lines=401/400"* ]]
```

Cover every numeric boundary, binary, three valid exceptions, and invalid/mixed exceptions.

- [ ] **Step 2: Capture red**

Run `bats tests/unit/test_lint_implementation.bats --filter 'PR_SIZE'`; expect no finding.

- [ ] **Step 3: Implement shell counting**

Use `git diff --numstat` for additions+deletions and binary rows, unique raw paths, and the same trio/golden normalization.

- [ ] **Step 4: Implement narrow skips**

Accept exact grammar only when category and every size-causing path match; support the Task 2 directive under `--directives`.

- [ ] **Step 5: Document and verify**

Add `PR_SIZE` to AGENTS rule/directive tables. Run `bash -n`, focused Bats, `scripts/validate-agents-md-contract.sh`, and diff check; commit `feat: keep shell patch budgets in parity`.

### Task 4: Rust remote and merge admission

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: inline executor-bridge tests

**Interfaces:**
- Consumes: Rust `PR_SIZE` and `PatchSizeEvaluation`.
- Produces: exact-head `PatchSizeAdmission` bound into existing premerge evidence.

- [ ] **Step 1: Write failing mutation tests**

For 401 lines and 9 files assert zero `git push`, `gh pr create`, `gh pr ready`, or `gh pr merge`. Assert merge rejects a missing, stale, or mismatched size receipt.

- [ ] **Step 2: Capture red**

Run `cargo test -p autospec-cli executor_bridge::tests::pr_size -- --nocapture --test-threads=1`; expect no admission.

- [ ] **Step 3: Add typed evidence**

```rust
struct PatchSizeAdmission {
    base_oid: String,
    head_oid: String,
    evaluation: PatchSizeEvaluation,
}
```

Compute it from the exact lint diff and bind it into existing premerge evidence.

- [ ] **Step 4: Gate exact transitions**

Require non-hard evidence before push/draft. Make `revalidate_merge_admission` reject missing or OID-mismatched evidence, covering ready and admin merge without a second ad-hoc diff.

- [ ] **Step 5: Verify and commit**

Run fmt, focused mutation/admission regressions, CLI clippy, and diff check; commit `feat: enforce patch admission at remote boundaries`.

### Task 5: Durable continuation receipt

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: inline executor-bridge tests

**Interfaces:**
- Consumes: `PatchSizeEvaluation`, issue identity, and typed `ContinuationReport`.
- Produces: private `ContinuationReceipt` loaded idempotently by issue/base/head identity.

- [ ] **Step 1: Write failing lifecycle tests**

Assert: `320 + unmet -> planned`, `319 + unmet -> absent`, `320 + complete -> absent`, `401 -> oversized_checkpoint`, restart reuses exact content, and hard handling preserves commits.

- [ ] **Step 2: Capture red**

Run `cargo test -p autospec-cli executor_bridge::tests::continuation_receipt -- --nocapture --test-threads=1`; expect no receipt.

- [ ] **Step 3: Add worker/report contract**

The worker checks budget after coherent edit/test checkpoints, stops adding criteria at proactive status, commits a passing slice, and reports completed plus ordered unmet criteria. Empty unmet criteria never create a continuation.

- [ ] **Step 4: Persist receipt fail-closed**

Store schema, repository, umbrella, OIDs, budget, trigger, criteria, children, and status through existing private/symlink-safe create-once helpers; reject identity/content mismatch.

- [ ] **Step 5: Notify the session**

Notify threshold, hard checkpoint, invalid exception, and recovery with measurements and receipt path; never invoke desktop notification.

- [ ] **Step 6: Verify and commit**

Run fmt, receipt tests, CLI clippy, and diff check; commit `feat: preserve autonomous continuation intent`.

### Task 6: Append-only parent extension

**Files:**
- Modify: `crates/autospec-core/src/state/mod.rs`
- Modify: `crates/autospec-cli/src/commands/parent.rs`
- Modify: `crates/autospec-cli/src/commands/parent/options.rs`
- Test: inline parent/state tests

**Interfaces:**
- Consumes: immutable trusted parent records.
- Produces: `extend_parent_decomposition(parent, children)`, merged-PR terminal evidence, and `autospec parent extend --parent N --children A,B`.

- [ ] **Step 1: Write failing extension tests**

Prove ordered-superset append, prior merged state preservation, idempotent repeat, and rejection of removal, duplicate, other-parent, and parent-self children. A manually closed child without a merged PR remains pending.

- [ ] **Step 2: Capture red**

Run `cargo test -p autospec-core state::tests::parent_extend -- --nocapture` and `cargo test -p autospec-cli commands::parent::tests::extend -- --nocapture`; expect missing extension API/subcommand.

- [ ] **Step 3: Implement append-only core behavior**

Load the latest trusted full list, validate ownership, post one new full-list marker only when changed, and return an explicit `changed` boolean. Record terminal state only from authoritative merged-PR evidence, never issue closure alone.

- [ ] **Step 4: Add CLI parsing**

Support exact `autospec parent extend --parent <N> --children <A,B,...>` and the same typed summary shape as `record`.

- [ ] **Step 5: Verify and commit**

Run fmt, core/CLI parent tests, core+CLI clippy, and diff check; commit `feat: extend autonomous parent decompositions`.

### Task 7: Idempotent continuation publication

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: inline executor-bridge tests

**Interfaces:**
- Consumes: `ContinuationReceipt`, parent extension, and `Depends on issue #N`.
- Produces: ordered child publication, correct part-PR metadata, restart recovery, and completion notifications.

- [ ] **Step 1: Write failing publication tests**

Without a parent, the original issue becomes the umbrella: child 1 represents the capped current slice and child 2 depends on child 1. With an existing parent, append continuations after the current issue. Restart creates none, and manual child closure cannot complete the umbrella.

- [ ] **Step 2: Capture red**

Run `cargo test -p autospec-cli executor_bridge::tests::continuation_publication -- --nocapture --test-threads=1`; expect no publisher.

- [ ] **Step 3: Build exact child bodies**

Include concrete goal, checkbox criteria, paths, tests, one-line smoke command, and `Part of #<umbrella>`. The current slice PR closes child 1; every continuation depends on its preceding merged child and starts from a clean isolated worktree.

- [ ] **Step 4: Publish idempotently**

Search by receipt marker before create, authoritative-reread one result, persist number, and call parent extension with the ordered full child list.

- [ ] **Step 5: Preserve closure semantics**

Part PRs contain `Part of #<umbrella>` and `Closes #<child>`, never `Closes #<umbrella>`; retain reconcile-after-merge and sweep-at-start.

- [ ] **Step 6: Notify and verify**

Notify create/recovery/umbrella completion. Run fmt, publisher tests, CLI clippy, and diff check; commit `feat: publish ordered autonomous continuations`.

### Task 8: Multi-harness proactive behavior

**Files:**
- Modify: `skills/autospec-run/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `tests/unit/test_phase4_guardian_trio.bats`
- Modify: `tests/fixtures/skill-goldens/autospec-run.SKILL.md.sha256`
- Modify: `tests/fixtures/skill-goldens/autospec-run.codex.prompt.md.sha256`
- Modify: `tests/fixtures/skill-goldens/autospec-run.opencode.agent.md.sha256`
- Modify: `tests/fixtures/skill-goldens/autospec.SKILL.md.sha256`
- Modify: `tests/fixtures/skill-goldens/autospec.codex.prompt.md.sha256`
- Modify: `tests/fixtures/skill-goldens/autospec.opencode.agent.md.sha256`

**Interfaces:**
- Consumes: shell/Rust gates, receipt publisher, parent extension, and 320/7 thresholds.
- Produces: lock-step instructions for pre-push checkpoint, continuation, and final exact-head gate.

- [ ] **Step 1: Write failing contract assertions**

Require all adapters to contain `320 changed lines`, `7 raw files`, exception grammar, `Part of #<umbrella>`, and `Depends on issue #N`; require checkpoint/lint before push and final lint before merge.

- [ ] **Step 2: Capture red**

Run `bats tests/unit/test_phase4_guardian_trio.bats --filter PR_SIZE`; expect missing proactive and pre-push requirements.

- [ ] **Step 3: Update canonical bodies**

At each checkpoint run exact base..HEAD policy. Proactive status freezes a completed slice and publishes unmet criteria; hard status preserves the branch without push/draft. Final gate reruns after base/docs repair and requires reviewer acceptance of valid `INFO:PR_SIZE`.

- [ ] **Step 4: Mirror and verify**

Use `Guardian: skip-PR_SIZE # mandatory lock-step artifacts: autospec and autospec-run adapter trios plus derived goldens`; prove the normalized two canonical bodies plus validator remain within 400/8/3, mirror bodies, regenerate only their six goldens, run lock-step validation, then run the full JSON validator with zero required failures.

- [ ] **Step 5: Commit**

Commit `feat: continue large autonomous work in capped parts`.

## Final integration verification

- [ ] Rebase each child on latest `origin/main`; run focused tests and prove its diff is within 400/8/3.
- [ ] Run the full JSON validator sequentially on the final child with zero required failures.
- [ ] Obtain independent whole-feature review against this plan and its design.
- [ ] Merge in dependency order, sweep after each merge, and confirm issue `#2699` closes only after the final child PR is observed merged.
