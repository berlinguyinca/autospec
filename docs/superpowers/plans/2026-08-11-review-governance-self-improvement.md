# Evidence-Grounded Review Governance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Build commit-bound, risk-aware independent review with structured evidence, escaped-defect quality metrics, and bounded autonomous self-improvement.

**Architecture:** autospec-core owns a pure harness-neutral review classifier. The native executor resolves the result against installed harnesses, executes integration evidence, validates structured reviewer output, and seals policy plus verdict in its receipt. Skill adapters mirror that invariant; Phase 5.5 appends attributed outcomes used by the advisor and Tier-3 self-improvement waterfall.

**Tech Stack:** Rust workspace, Bash, jq, Bats, Markdown skill triplets, and existing Autospec receipt/private-artifact machinery.

## Global Constraints

- Work only in /home/wohlgemuth/IdeaProjects/autospec-review-governance on feat/review-governance-self-improvement.
- Follow strict TDD: write each behavioral test, run it and observe the expected failure, then implement minimum behavior.
- Do not add dependencies.
- Preserve reviewer read-only containment, private artifacts, mutation snapshots, claim binding, and fail-closed recovery.
- No auto-merge path may accept an author-context or in-thread reviewer.
- Do not hard-code dated model identifiers.
- Keep every multi-harness SKILL.md / opencode/agent.md / codex/prompt.md body byte-identical after frontmatter.
- Only strengthening or behavior-neutral review experiments may auto-promote.
- Preserve user-owned changes in /home/wohlgemuth/IdeaProjects/autospec.

---

### Task 1: Pure review-requirements classifier

**Files:**
- Create: crates/autospec-core/src/autonomous/review_policy.rs
- Modify: crates/autospec-core/src/lib.rs

**Interfaces:**
- Consume changed paths, serialization reasons, logical-component count, producer/consumer evidence, and a critical-boundary flag.
- Produce ReviewRisk, ReviewReasoning, ReviewPolicyInput, ReviewRequirements, and classify_review_requirements(&ReviewPolicyInput).

- [ ] **Step 1: Write a failing table-driven unit test**

Add review_requirements_classify_repository_risk_shapes with literal cases for one docs path, scripts/install.sh, daemon plus adapter paths, priority:high, reasoning:deep plus two components, and critical_boundary. Assert risk, smoke requirement, diversity flags, and reasoning. The production break is routing a risky diff to weaker review.

- [ ] **Step 2: Run RED**

Run: cargo test -p autospec-core review_requirements_classify_repository_risk_shapes -- --exact
Expected: compilation failure because autonomous::review_policy does not exist.

- [ ] **Step 3: Implement minimum classification**

Use fixed path categories for orchestration, install/bootstrap, adapter/provider, daemon/session, state/recovery, and merge/claim/premerge authority. Deduplicate and sort reason codes. Risk ordering is Normal < High < Integration < Critical; ambiguity chooses the stronger result.

- [ ] **Step 4: Run GREEN and commit**

Run: cargo test -p autospec-core review_policy
Commit: feat: classify autonomous review requirements

### Task 2: Integration-smoke evidence in the native executor

**Files:**
- Modify: crates/autospec-cli/src/commands/autonomous.rs
- Modify: crates/autospec-cli/src/commands/autonomous/executor_bridge.rs
- Modify: focused executor-bridge test modules

**Interfaces:**
- Consume ForegroundSelection.serialization_reasons, ReviewRequirements, changed paths, issue headings, and existing direct-command execution.
- Produce ExecutorBridgeRequest.serialization_reasons, a strict integration-smoke plan, and commit-bound pre-merge evidence.

- [ ] **Step 1: Write failing behavioral tests**

Add selected_issue_serialization_reasons_reach_the_executor_request, integration_shaped_issue_without_integration_smoke_fails_before_review, integration_primary_smoke_accepts_a_repository_integration_test, failing_integration_smoke_blocks_ci_passed_transition, and passing_integration_smoke_is_bound_into_premerge_evidence. Use fixture repositories and command artifacts. Compatibility accepts a primary smoke only when it invokes tests/integration/, tests/smoke/, or tests/e2e/.

- [ ] **Step 2: Run RED**

Run: cargo test -p autospec-cli integration_smoke -- --test-threads=1
Run: cargo test -p autospec-cli selected_issue_serialization_reasons_reach_the_executor_request -- --exact

- [ ] **Step 3: Implement risk threading and parsing**

Add serialization_reasons to ExecutorBridgeRequest and populate it from ForegroundSelection. Reuse changed-path and blast-radius evidence. Parse the Integration smoke test (pre-merge) heading as exactly one non-comment direct command.

- [ ] **Step 4: Execute and seal evidence**

Run the command under existing containment, timeout, output, identity, and artifact limits. Bind its observation and requirements digest into pre-merge evidence. Absence, ambiguity, failure, or drift blocks review.

- [ ] **Step 5: Run GREEN and commit**

Run focused integration_smoke, production_entry, and full_suite tests serially.
Commit: feat: require integration evidence before autonomous review

### Task 3: Risk-aware provider diversity and universal independence

**Files:**
- Modify: crates/autospec-cli/src/commands/autonomous/executor_bridge.rs
- Modify: reviewer tests under the executor_bridge test tree

**Interfaces:**
- Consume ReviewRequirements, implementer HarnessKind, installed aliases, and configured environment.
- Produce ResolvedReviewPolicy containing requirements, reviewer_harness, provider_diversified, and selection_reason.

- [ ] **Step 1: Write failing resolver tests**

Cover: Normal retains the available provider; Integration prefers a different provider; Integration falls back to high reasoning on the same provider; Critical without an alternate fails closed; unstructured review commands cannot authorize production review.

- [ ] **Step 2: Run RED**

Run: cargo test -p autospec-cli review_provider -- --test-threads=1

- [ ] **Step 3: Implement selection and close the override hole**

Enumerate configured aliases. Prefer a different HarnessKind for High/Integration, require it for Critical, otherwise use high reasoning on the current provider, and record fallback reasons. Route AUTOSPEC_EXECUTOR_REVIEW_COMMAND through the structured adapter and existing executable/artifact/mutation checks. Keep direct injection test-only.

- [ ] **Step 4: Run GREEN and commit**

Run review_provider and identity_reviewer tests serially.
Commit: feat: diversify autonomous reviewers by risk

### Task 4: Structured verdict and receipt-bound evidence

**Files:**
- Modify: crates/autospec-cli/src/commands/autonomous/executor_bridge.rs
- Modify: reviewer/receipt tests
- Modify: docs/AUTONOMOUS-RUNBOOK.md

**Interfaces:**
- Consume exact commit, ResolvedReviewPolicy, and private bounded reviewer result.
- Produce strict ReviewVerdict, receipt schema 4, semantic digests, and normalized exact LGTM for the outer state machine.

- [ ] **Step 1: Write failing verdict tests**

Use hand-written JSON for valid exact evidence, wrong commit, unknown keys, empty surfaces/tests, missing integration paths, nonempty findings, and receipt policy/verdict digest drift. Each test names the admission error it catches.

- [ ] **Step 2: Run RED**

Run: cargo test -p autospec-cli structured_review -- --test-threads=1
Run: cargo test -p autospec-cli receipt_rejects_policy -- --test-threads=1

- [ ] **Step 3: Implement parsing and recovery**

Accept only schema, commit, verdict, surfaces_examined, tests_examined, integration_paths_checked, and blocking_findings. Require the expected commit, lgtm, nonempty surfaces/tests, integration paths when required, and zero findings. Persist resolved policy, reviewer identity/reasoning, diversity, semantic fields, and digests. Safe legacy recovery returns to CiPassed and reruns review.

- [ ] **Step 4: Preserve normalized compatibility**

Update harness prompts to return JSON. The trusted normalizer emits LGTM only after validation. Stderr, truncation, replacement, and nonzero status remain blocking.

- [ ] **Step 5: Run GREEN and commit**

Run structured_review, ready_harness, and result_reviewer tests serially.
Commit: feat: bind semantic review evidence to receipts

### Task 5: Fail-closed multi-harness adapters

**Files:**
- Modify lock-step autospec-run and autospec skill triplets
- Modify: skills/autospec-run/prompts/phase4-implementer.md
- Modify: skills/autospec/README.md
- Modify/add behavioral tests under tests/autospec-run/ and tests/autonomous/

**Interfaces:**
- Consume canonical risk, independence, and structured-verdict rules.
- Produce typed blocker/requeue when independent delegation is unavailable; never in-thread merge authorization.

- [ ] **Step 1: Write a failing adapter behavior test**

Run a fixture adapter with foreground delegation unavailable. Assert blocker/requeue output, no merge invocation, and structured reviewer JSON delivery. Do not test by grepping prose.

- [ ] **Step 2: Run RED**

Run: bats tests/autospec-run/test_invoke_review_harness_neutral.bats
Run: bats tests/autonomous/test_separation_of_powers.bats

- [ ] **Step 3: Update canonical bodies and regenerate mirrors**

Replace in-thread fallback with fail-closed blocker/requeue, mirror risk selection and structured verdicts, preserve one reviewer for Normal work, and use repository lock-step generation.

- [ ] **Step 4: Run GREEN and commit**

Run the adapter, separation, regression-review lock-step, and autospec validate --fast --changed=origin/main checks.
Commit: fix: fail closed when independent review is unavailable

### Task 6: Phase 5.5 attribution and escaped-defect governance

**Files:**
- Modify lock-step autospec-review triplet
- Modify: skills/autospec-shared/scripts/emit-gaps.sh
- Modify: skills/autospec-shared/scripts/gap-json-lib.sh
- Modify: skills/autospec-shared/scripts/gap-remediation-loop.sh
- Modify: skills/autospec-shared/scripts/advisor-observe.sh
- Modify: skills/autospec-shared/scripts/advisor-govern.sh
- Modify: skills/autospec-shared/scripts/advisor-sweep-tick.sh
- Modify corresponding Bats unit and integration tests

**Interfaces:**
- Consume Phase 5.5 findings and native receipt metadata.
- Produce attributed gaps, append-only .autospec/review-outcomes.jsonl, escaped_high_rate, escaped_total_rate, cost_per_reviewed_pr, attributed_reviewed_prs, and diagnostic first_pass_lgtm.

- [ ] **Step 1: Write failing attribution tests**

Prove originating_pr, originating_commit, review_receipt_digest, reviewer identity/reasoning, diversity, and risk survive emit -> validate -> remediation. Missing attribution stays explicit and never counts clean.

- [ ] **Step 2: Write failing advisor tests**

Literal JSONL cases: one high escape across four PRs equals 0.25; cache tokens do not affect quality; a high escape strengthens immediately; relaxation below 20 samples holds; 20 clean samples may relax within cost; review_unavailable freezes relaxation.

- [ ] **Step 3: Run RED**

Run autospec-review-remediation, gap-remediation-compose, advisor-observe, advisor-govern, and advisor-sweep-tick Bats suites.

- [ ] **Step 4: Implement attribution and metrics**

Extend optional gap fields without breaking history. Append immutable outcomes and superseding corrections. Emit review_unavailable instead of an empty clean result. Make high escape rate primary, total rate secondary, cost the guard, and first-pass LGTM diagnostic only.

- [ ] **Step 5: Run GREEN and commit**

Run all Task 6 Bats suites.
Commit: feat: govern review policy from escaped defects

### Task 7: Falsifiable autonomous review-policy improvement

**Files:**
- Modify: scripts/autonomous-self-improvement.sh
- Add: tests/autonomous/test_review_escape_learning.bats
- Modify: tests/autonomous/test_self_improvement_candidates.bats
- Modify: scripts/lib/autospec-loop.sh only if its current Tier-3 input needs an explicit outcome path
- Modify: docs/CONFIG_REFERENCE.md
- Modify: docs/USER_MANUAL.md

**Interfaces:**
- Consume review outcomes, attributed gaps, receipts, blocked pre-merge evidence, and learning-ledger repetition.
- Produce one deduplicated candidate per evidence cluster with evidence, failed invariant, named consumer, falsifier, bounded files, metric, sample floor, cost bound, rollback, and change class.

- [ ] **Step 1: Write failing behavioral tests**

Exercise the real script against temp repos for: a high attributed escape creates one strengthening candidate; repeats increase frequency without duplicates; no evidence creates no candidate; weakening remains report-only; below-floor samples cannot promote; canary regression references prior-policy rollback; the next outcome identifies a successful experiment.

- [ ] **Step 2: Run RED**

Run: bats tests/autonomous/test_self_improvement_candidates.bats
Run: bats tests/autonomous/test_review_escape_learning.bats

- [ ] **Step 3: Implement questions and hypothesis contract**

Generate the eight approved questions per evidence cluster. Require evidence, failed_invariant, named_consumer, falsifier, files, dedupe_key, before_after, sample_floor, max_cost_regression, rollback, and change_class.

- [ ] **Step 4: Enforce experiment lifecycle**

Route strengthening/neutral candidates through needs-classify and origin:self; keep weakening report-only. Represent candidate, shadow, canary, promoted, held, and rolled-back states append-only. Promotion requires sample floor, zero high escapes, non-regressing total escapes, acceptable cost, provider-diverse review, and rollback digest.

- [ ] **Step 5: Run GREEN and commit**

Run self-improvement and conductor-wiring Bats suites.
Commit: feat: learn review improvements from escaped defects

### Task 8: Whole-feature verification

**Files:**
- Modify only files required to repair verification findings.

**Interfaces:**
- Consume Tasks 1-7.
- Produce fresh validation evidence and the required scoped closeout report.

- [ ] **Step 1: Run focused suites**

Run core review_policy tests; CLI structured_review and integration_smoke tests serially; advisor, remediation, review_escape_learning, separation-of-powers, and lock-step Bats suites.

- [ ] **Step 2: Run full validation serially**

Run: cargo test --workspace -- --test-threads=1
Run: autospec validate

The serial runner is required because the clean baseline showed shared-state contention under the default parallel Rust runner.

- [ ] **Step 3: Audit diff and requirements**

Run git diff --check main...HEAD, git status --short, and git diff --stat main...HEAD. Map every design acceptance criterion to a passing test or explicit gap.

- [ ] **Step 4: Commit only verification repairs**

Commit subject: fix: close review governance verification gaps

- [ ] **Step 5: Write the closeout report**

Record result, labeled claims, proof type, before/after, exact artifacts and rerunnable commands, scoped git status, and one likely hidden failure. Do not claim completion without fresh output.
