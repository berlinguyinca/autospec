# Autospec PR Size Budget Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce a deterministic small-PR budget and transparently continue unfinished autonomous work through ordered child issues.

**Architecture:** A dependency-free Rust evaluator owns patch measurements, limits, and proactive/hard verdicts; the Rust and shell linters consume that policy with boundary-parity tests. The autonomous executor then uses the same verdict before push, draft, ready, and merge, while durable continuation receipts drive idempotent child publication through the existing `autospec parent` and `Depends on issue #N` machinery.

**Tech Stack:** Rust 2024 workspace, Bash, Bats, `gh`, existing Autospec private-state and parent-reconciliation APIs.

## Global Constraints

- Hard limits are 400 additions-plus-deletions, 8 raw files, and 3 logical units.
- Proactive continuation begins at 320 changed lines, 7 raw files, or 3 logical units while acceptance criteria remain unfinished.
- A skill `SKILL.md` / `codex/prompt.md` / `opencode/agent.md` trio and derived skill goldens count as one logical unit.
- A binary diff is hard-oversized because its changed-line count cannot be proved.
- Hard-oversized work is never pushed, drafted, readied, or merged; its local branch and commits remain intact.
- The only exception grammar is `Guardian: skip-PR_SIZE # <reason>`.
- Exception categories are generated migration, dependency-solver lockfile, and mandatory lock-step artifacts; manual implementation and test code are never exempt.
- Continuation children contain `Depends on issue #N`; part PRs use `Part of #N`, and only child issues are closed by part PRs.
- The original issue closes only through the existing `autospec parent` reconciliation after every child is terminal.
- Continuation publication and notifications are durable, restart-safe, and idempotent.
- Every task starts with a failing test, keeps its PR within the size contract, and passes focused validation before commit.
- No new dependency is permitted.

---

### Task 1: Typed patch-size evaluator

**Files:**
- Create: `crates/autospec-core/src/lint/pr_size.rs`
- Modify: `crates/autospec-core/src/lint/diff.rs`
- Modify: `crates/autospec-core/src/lint/mod.rs`
- Test: inline unit tests in `crates/autospec-core/src/lint/pr_size.rs`

**Interfaces:**
- Consumes: `UnifiedDiff`, `DiffFile`, and existing skill-trio logical-unit semantics.
- Produces: `PatchSizeLimits`, `PatchSize`, `PatchSizeDimension`, `PatchSizeEvaluation`, and `evaluate_patch_size(&UnifiedDiff, PatchSizeLimits)`.

- [ ] **Step 1: Write evaluator boundary tests**

Add table-driven tests that build unified diffs and assert these exact outcomes:

```rust
assert_eq!(evaluate(400, 8, 3).hard_dimensions(), &[]);
assert_eq!(evaluate(401, 8, 3).hard_dimensions(), &[PatchSizeDimension::ChangedLines]);
assert_eq!(evaluate(400, 9, 3).hard_dimensions(), &[PatchSizeDimension::RawFiles]);
assert_eq!(evaluate(400, 8, 4).hard_dimensions(), &[PatchSizeDimension::LogicalUnits]);
assert!(evaluate(320, 1, 1).is_proactive());
assert!(evaluate(1, 7, 1).is_proactive());
assert!(evaluate(1, 1, 3).is_proactive());
```

Also prove a skill adapter trio plus `tests/fixtures/skill-goldens/<skill>.sha256` measures as one logical unit, and prove a binary diff is hard-oversized.

- [ ] **Step 2: Run the new tests and capture the red result**

Run:

```bash
cargo test -p autospec-core lint::pr_size -- --nocapture
```

Expected: compilation fails because `lint::pr_size` and its types do not exist.

- [ ] **Step 3: Extend the unified-diff evidence**

Add removed-line and binary evidence without filesystem access:

```rust
pub struct DiffFile {
    pub path: String,
    pub is_new: bool,
    pub is_binary: bool,
    pub hunks: Vec<DiffHunk>,
}

impl DiffFile {
    pub fn removed_line_count(&self) -> usize;
    pub fn changed_line_count(&self) -> usize;
}
```

`parse_unified_diff` sets `is_binary` for `Binary files ... differ` and for `GIT binary patch`.

- [ ] **Step 4: Implement the typed evaluator**

Expose these exact defaults and relationships:

```rust
pub const DEFAULT_MAX_CHANGED_LINES: usize = 400;
pub const DEFAULT_MAX_RAW_FILES: usize = 8;
pub const DEFAULT_MAX_LOGICAL_UNITS: usize = 3;
pub const PROACTIVE_PERCENT: usize = 80;

pub fn proactive_reached(value: usize, limit: usize) -> bool {
    value.saturating_mul(100) >= limit.saturating_mul(PROACTIVE_PERCENT)
}
```

`PatchSizeEvaluation` stores measured counts plus ordered proactive and hard dimensions. Hard means `>` for numeric limits; any binary path adds `PatchSizeDimension::BinaryDiff`.

- [ ] **Step 5: Reuse one logical-unit function**

Move the current skill-trio and derived-golden normalization behind a `pub(crate)` helper used by both issue lint and `pr_size`; replace issue lint’s literal `3` with `DEFAULT_MAX_LOGICAL_UNITS` so issue sizing and implementation sizing cannot drift.

- [ ] **Step 6: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-core lint::pr_size lint::tests::files_touched -- --nocapture
cargo clippy -p autospec-core --all-targets --all-features -- -D warnings
git diff --check
```

Commit with:

```text
feat: define the autonomous patch size budget
```

### Task 2: Rust `PR_SIZE` lint rule and narrow exceptions

**Files:**
- Modify: `crates/autospec-core/src/lint/implementation.rs`
- Modify: `crates/autospec-core/src/lint/mod.rs`
- Test: inline unit tests in `crates/autospec-core/src/lint/implementation.rs`

**Interfaces:**
- Consumes: `evaluate_patch_size`, `PatchSizeLimits`, and `UnifiedDiff` from Task 1.
- Produces: `ImplementationLintRule::PrSize`, `ImplementationLintOptions::patch_size_limits`, and auditable `ERROR:PR_SIZE` / `INFO:PR_SIZE` findings.

- [ ] **Step 1: Write failing Rust linter tests**

Add tests for exact pass/fail boundaries and exception behavior:

```rust
let at_limit = lint_diff(diff_with_changed_lines(400), None);
assert_eq!(findings_for(&at_limit, "PR_SIZE"), vec![]);

let over_limit = lint_diff(diff_with_changed_lines(401), None);
assert_eq!(over_limit.blocking_count, 1);
assert!(over_limit.findings[0].message.contains("401/400"));
```

Test all three allowed categories with matching paths, plus bare, unknown, manual-code, and test-code reasons that remain blocking.

- [ ] **Step 2: Run the focused tests and capture the red result**

Run:

```bash
cargo test -p autospec-core lint::implementation::tests::pr_size -- --nocapture
```

Expected: compilation fails because `ImplementationLintRule::PrSize` does not exist.

- [ ] **Step 3: Add the blocking rule**

Insert `PrSize` into the stable ordered detector pass before scope/test/complexity rules. Its directive is exactly:

```text
Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff.
```

The finding message reports `changed_lines=<n>/400 raw_files=<n>/8 logical_units=<n>/3` and lists binary paths when present.

- [ ] **Step 4: Validate `skip-PR_SIZE` by category and paths**

Keep generic skip parsing for other rules. For `PR_SIZE`, downgrade to `Info` only when both the normalized reason category and every size-causing path match:

```rust
enum PrSizeException {
    GeneratedMigration,
    DependencySolverLockfile,
    MandatoryLockStepArtifacts,
}
```

Generated migrations must live under an existing migration directory and contain a generator provenance marker in changed/context evidence; lockfiles must be recognized dependency-solver lockfile basenames; lock-step artifacts must be only byte-identical adapter trios or derived goldens whose normalized manual patch is within budget. Any manual source/test path preserves the error.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-core lint::implementation::tests -- --nocapture
cargo clippy -p autospec-core --all-targets --all-features -- -D warnings
git diff --check
```

Commit with:

```text
feat: block oversized implementations in Rust
```

### Task 3: Shell linter parity

**Files:**
- Modify: `scripts/lint-implementation.sh`
- Modify: `tests/unit/test_lint_implementation.bats`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: the exact constants, output fields, directive, and exception categories from Tasks 1-2.
- Produces: shell `PR_SIZE` findings byte-compatible in meaning with the Rust linter and documented `RULE_ID` policy.

- [ ] **Step 1: Add failing Bats boundary tests**

Create fixtures through temporary git commits and assert:

```bash
run "$LINTER" --base "$base" --head "$head"
[ "$status" -eq 1 ]
[[ "$output" == *"ERROR:PR_SIZE:"* ]]
[[ "$output" == *"changed_lines=401/400"* ]]
```

Cover 400/401 lines, 8/9 files, 3/4 logical units, binary diffs, the three valid categories, and invalid or path-mismatched exceptions.

- [ ] **Step 2: Run the focused Bats tests and capture the red result**

Run:

```bash
bats tests/unit/test_lint_implementation.bats --filter 'PR_SIZE'
```

Expected: tests fail because the shell linter emits no `PR_SIZE` finding.

- [ ] **Step 3: Implement deterministic shell counting**

Count additions plus deletions from `git diff --numstat`, count unique raw paths, normalize the adapter trio and derived goldens with the same path rules as Rust, and fail closed on `-\t-\t<path>` binary rows.

- [ ] **Step 4: Implement the narrow shell exception**

Parse exact `Guardian: skip-PR_SIZE # <reason>` syntax, classify the same three categories, verify every size-causing path, and emit `INFO:PR_SIZE` only for a valid category/path pairing. Emit the same directive as Task 2 for errors and `--directives`.

- [ ] **Step 5: Document the rule**

Add `PR_SIZE` to the implementation-quality `RULE_ID` and directive tables in `AGENTS.md`, including 400/8/3, the three allowed categories, and the statement that manual source/test changes cannot be exempted.

- [ ] **Step 6: Verify and commit**

Run:

```bash
bash -n scripts/lint-implementation.sh
bats tests/unit/test_lint_implementation.bats --filter 'PR_SIZE'
scripts/validate-agents-md-contract.sh
git diff --check
```

Commit with:

```text
feat: keep shell patch budgets in parity
```

### Task 4: Rust remote-mutation and merge admission

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: inline executor-bridge tests in `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`

**Interfaces:**
- Consumes: Rust `PR_SIZE` lint findings and typed `PatchSizeEvaluation`.
- Produces: commit-bound `PatchSizeAdmission` evidence used before push/draft and bound into existing exact-head premerge/merge admission.

- [ ] **Step 1: Add failing remote-boundary tests**

Extend the existing “lint blocks before git/gh mutation” harness with command ledgers. Assert a 401-line or 9-file diff performs zero `git push`, `gh pr create`, `gh pr ready`, and `gh pr merge` calls.

Add drift tests where the draft was admitted at 400 lines and the exact head later measures 401, and where the size receipt is missing or bound to another head; merge must fail before `gh pr merge`.

- [ ] **Step 2: Run the executor tests and capture the red result**

Run:

```bash
cargo test -p autospec-cli executor_bridge::tests::pr_size -- --nocapture --test-threads=1
```

Expected: tests fail because the executor has no commit-bound size admission.

- [ ] **Step 3: Add commit-bound admission evidence**

Add:

```rust
struct PatchSizeAdmission {
    base_oid: String,
    head_oid: String,
    evaluation: PatchSizeEvaluation,
}
```

Compute it from the same exact base/head diff supplied to implementation lint. Persist only the measurement and OIDs needed to prove the verdict; do not infer a verdict from formatted linter text. Bind it into the existing premerge evidence receipt.

- [ ] **Step 4: Gate every remote transition**

Require a non-hard `PatchSizeAdmission` before `push_exact_issue_head` and draft creation. `revalidate_merge_admission` must require the same exact-head admission in accepted premerge evidence, so ready and admin merge reject missing, stale, or mismatched receipts without recomputing an ad-hoc second diff.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli executor_bridge::tests::pr_size -- --nocapture --test-threads=1
cargo test -p autospec-cli executor_bridge::tests::implementation_lint_blocks_before_remote_mutation -- --nocapture --test-threads=1
cargo clippy -p autospec-cli --all-targets --all-features -- -D warnings
git diff --check
```

Commit with:

```text
feat: enforce patch admission at remote boundaries
```

### Task 5: Durable continuation receipt

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: inline executor-bridge tests in `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`

**Interfaces:**
- Consumes: proactive/hard `PatchSizeEvaluation`, executor issue identity, and a typed worker `ContinuationReport`.
- Produces: private `ContinuationReceipt` persisted beside executor invocation state and loaded idempotently by receipt identity.

- [ ] **Step 1: Add failing receipt lifecycle tests**

Prove these state transitions without invoking `gh`:

```text
320 lines + unmet criteria -> receipt status planned
319 lines + unmet criteria -> no receipt
320 lines + all criteria complete -> no receipt
401 lines -> receipt status oversized_checkpoint
same base/head/issue after restart -> same receipt path and content
```

Also assert the local branch and commits still exist after hard oversize handling.

- [ ] **Step 2: Run the receipt tests and capture the red result**

Run:

```bash
cargo test -p autospec-cli executor_bridge::tests::continuation_receipt -- --nocapture --test-threads=1
```

Expected: tests fail because no continuation receipt is written.

- [ ] **Step 3: Define the worker report and receipt schemas**

The worker emits completed criteria, ordered unmet criteria, and whether a coherent capped slice is ready; the controller independently supplies measured budget and OIDs. Persist schema version, repository, umbrella issue, base/head OIDs, measured budget, trigger kind, completed criteria, ordered unmet criteria, child issue numbers, and publication status. Derive the filename from repository + issue + base/head identity.

- [ ] **Step 4: Add the proactive worker contract**

The local worker prompt requires a deterministic diff-budget check after each coherent edit/test checkpoint. At 320 lines, 7 files, or 3 units it stops adding criteria, commits the passing slice, and emits `ContinuationReport`; if no criteria remain it emits an empty unmet list and no continuation is created.

- [ ] **Step 5: Implement fail-closed private persistence**

Use existing private-directory, symlink-rejection, create-once, and atomic-state helpers. An identity/content mismatch is an invariant failure. A restart loads the same receipt and never truncates or replaces it.

- [ ] **Step 6: Emit local lifecycle notifications**

Use the existing session notification path for proactive threshold, hard checkpoint, invalid exception, and receipt recovery. Include measured counts and the receipt path; do not use desktop notifications.

- [ ] **Step 7: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli executor_bridge::tests::continuation_receipt -- --nocapture --test-threads=1
cargo clippy -p autospec-cli --all-targets --all-features -- -D warnings
git diff --check
```

Commit with:

```text
feat: preserve autonomous continuation intent
```

### Task 6: Idempotent parent extension

**Files:**
- Modify: `crates/autospec-core/src/state/mod.rs`
- Modify: `crates/autospec-cli/src/commands/parent.rs`
- Modify: `crates/autospec-cli/src/commands/options.rs`
- Test: inline parent/state tests in the modified Rust files

**Interfaces:**
- Consumes: immutable trusted parent decomposition records and existing `record/reconcile-child/sweep` behavior.
- Produces: `extend_parent_decomposition(parent, children)` and `autospec parent extend --parent N --children A,B`.

- [ ] **Step 1: Add failing extension tests**

Prove extension posts a new trusted full-list record, preserves prior children and their terminal states, and is idempotent when the requested set already exists.

Also prove it rejects child removal, duplicate children, a child already linked to another parent, and a parent/child identity collision.

- [ ] **Step 2: Run the parent tests and capture the red result**

Run:

```bash
cargo test -p autospec-core state::tests::parent -- --nocapture
cargo test -p autospec-cli commands::parent::tests::extend -- --nocapture
```

Expected: compilation fails because parent extension does not exist.

- [ ] **Step 3: Implement append-only core extension**

Load the latest trusted decomposition, require the proposed list to be an ordered superset, validate every child ownership invariant, and post one new full-list marker. Repeating the same request performs no remote write.

- [ ] **Step 4: Add the CLI subcommand**

Parse:

```text
autospec parent extend --parent <N> --children <A,B,...>
```

Return the same typed summary shape as `parent record`, with an explicit `changed` boolean.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-core state::tests::parent -- --nocapture
cargo test -p autospec-cli commands::parent::tests::extend -- --nocapture
cargo clippy -p autospec-cli -p autospec-core --all-targets --all-features -- -D warnings
git diff --check
```

Commit with:

```text
feat: extend autonomous parent decompositions
```

### Task 7: Idempotent continuation publication

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: inline executor-bridge tests in `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`

**Interfaces:**
- Consumes: `ContinuationReceipt`, `autospec parent extend`, and ready-queue `Depends on issue #N` parsing.
- Produces: ordered child issue publication, `Part of #N` part-PR metadata, receipt-backed restart recovery, and umbrella-completion notifications.

- [ ] **Step 1: Add failing publication/restart tests**

With the existing fake `gh` adapter, prove one receipt with two unmet slices creates exactly two child issues, where child 2 contains `Depends on issue #<child1>`. Re-run from the persisted receipt and assert zero duplicate issue-create calls.

Add reconciliation tests proving the umbrella stays open after child 1 merges and closes only after both child PRs are observed merged. When the current issue already has a parent, append children there; otherwise create one tracker containing the current issue and continuations.

- [ ] **Step 2: Run the publication tests and capture the red result**

Run:

```bash
cargo test -p autospec-cli executor_bridge::tests::continuation_publication -- --nocapture --test-threads=1
```

Expected: the executor cannot publish continuation children from a receipt.

- [ ] **Step 3: Build child issue bodies deterministically**

Each child body contains one concrete goal, remaining acceptance criteria, exact implementation paths, tests, a one-line primary smoke command, `Part of #<umbrella>`, and an ordered `Depends on issue #N` line except for the first child.

- [ ] **Step 4: Publish and record children idempotently**

Before each create, search for the receipt identity marker. After create, authoritative-reread exactly one marker issue, persist its number, and call parent `extend` with the complete ordered child set. Restart resumes at the first missing child.

- [ ] **Step 5: Keep PR closure semantics exact**

Generated part PR bodies contain `Part of #<umbrella>` and `Closes #<child>`; they never contain `Closes #<umbrella>`. Continue to use `reconcile-child` after merge and `sweep` at batch start.

- [ ] **Step 6: Notify publication and completion**

Emit session notifications when a child is created or recovered and when parent reconciliation observes every child terminal. Include issue and PR URLs when known.

- [ ] **Step 7: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli executor_bridge::tests::continuation_publication -- --nocapture --test-threads=1
cargo clippy -p autospec-cli --all-targets --all-features -- -D warnings
git diff --check
```

Commit with:

```text
feat: publish ordered autonomous continuations
```

### Task 8: Multi-harness proactive checkpoint behavior

**Files:**
- Modify: `skills/autospec-run/SKILL.md`
- Modify: `skills/autospec-run/codex/prompt.md`
- Modify: `skills/autospec-run/opencode/agent.md`
- Modify: `skills/autospec/SKILL.md`
- Modify: `skills/autospec/codex/prompt.md`
- Modify: `skills/autospec/opencode/agent.md`
- Modify: the existing validation script that gates Phase 4 linter and merge-gate wording
- Test: that validation script and generated skill goldens

**Interfaces:**
- Consumes: shell `PR_SIZE`, Rust continuation behavior, `autospec parent extend`, and exact 320/7/3 proactive thresholds.
- Produces: lock-step harness instructions that checkpoint before push, split unmet acceptance criteria, and rerun the same hard gate before final merge.

- [ ] **Step 1: Add failing contract assertions**

Assert all six adapter bodies contain exact requirements for:

```text
320 changed lines
7 raw files
3 logical units
Guardian: skip-PR_SIZE # <reason>
Part of #<umbrella>
Depends on issue #N
```

Also assert the Phase 4 order is checkpoint/lint, then push/draft, and final lint, then merge.

- [ ] **Step 2: Run the validation and capture the red result**

Run the selected validation script directly.

Expected: it fails because current adapters do not specify proactive continuations or pre-push `PR_SIZE` admission.

- [ ] **Step 3: Update canonical skill bodies**

At each implementation checkpoint, invoke the installed/current-repo deterministic linter against exact base..HEAD. At proactive status with unmet criteria, stop expanding the slice and hand the remaining criteria to the receipt/publisher path. On hard error, preserve the branch and do not push or draft.

- [ ] **Step 4: Preserve lock-step adapters**

Copy each canonical skill body to its Codex and OpenCode adapters while retaining only their harness-specific frontmatter. The final merge gate reruns the exact-head linter and requires reviewer acceptance for any valid `INFO:PR_SIZE` exception.

- [ ] **Step 5: Regenerate and verify goldens**

Run:

```bash
scripts/generate-skill-goldens.sh
scripts/validate-skill-lockstep.sh
cargo run -p autospec-cli -- validate --json
git diff --check
```

Expected: every required validation passes and the JSON summary reports zero required failures.

- [ ] **Step 6: Commit**

Commit with:

```text
feat: continue large autonomous work in capped parts
```

## Final integration verification

- [ ] Rebase every child branch on the latest `origin/main` before its PR review.
- [ ] Run each child’s focused tests and confirm its diff is within 400 changed lines, 8 raw files, and 3 logical units.
- [ ] Run `cargo run -p autospec-cli -- validate --json` sequentially on the final child and confirm all required checks pass.
- [ ] Run an independent whole-feature review against this plan and the design spec.
- [ ] Merge children in dependency order and run `autospec parent sweep` after each merge.
- [ ] Confirm issue `#2699` closes only after the final child is terminal.
