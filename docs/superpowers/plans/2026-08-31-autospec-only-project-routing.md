# Autospec-Only Project Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route every implementation through installed Autospec and bind it to one deterministic primary GitHub Project before work is admitted.

**Architecture:** A typed capability probe and request classifier choose the Autospec entry point and primary Project policy. Rust owns Project identity/materialization and run state; installed skill/router surfaces submit intent and consume typed status without launching a competing top-level implementation agent.

**Tech Stack:** Rust, Bash, GitHub GraphQL/CLI fixtures, Bats, lock-step skill adapters

**Spec:** `docs/specs/2026-08-31-automatic-spec-projects-design.md`

## Global Constraints

- No new dependencies.
- Autospec internal implementer/reviewer workers remain supported.
- Direct serverless implementation is reachable only after a definitive typed Autospec-unavailable result.
- Exactly one verified primary Project binding precedes issue admission.
- Multi-harness skill bodies remain lock-step identical apart from frontmatter.
- Tests are written and observed failing before production changes.

---

### Task 1: Typed implementation gateway

**Files:**
- Create: `scripts/autospec-implementation-gateway.sh`
- Modify: `scripts/listener-match.sh`
- Modify: `skills/autospec-listen/SKILL.md`
- Modify: `skills/autospec-listen/codex/prompt.md`
- Modify: `skills/autospec-listen/opencode/agent.md`
- Create: `schemas/autospec-implementation-handoff-v1.schema.json`
- Create: `scripts/autospec-implementation-conformance.sh`
- Test: `tests/autospec-listen/implementation-gateway.bats`

**Interfaces:**
- Consumes: request text, repository root, installed `autospec` CLI and skill locations, durable run registry.
- Produces: `autospec.implementation-handoff.v1` JSON and an argv-safe Autospec invocation; validates a deployment-owned consumer conformance receipt.

- [ ] **Step 1: Write failing gateway tests**

```bash
@test "installed autospec makes direct implementation dispatch unreachable" {
  run bash "$GATEWAY" --request "implement issue 42" --repo "$REPO"
  [ "$status" -eq 0 ]
  [ "$(jq -r .route <<<"$output")" = "autospec-run" ]
  [ "$(jq -r .availability <<<"$output")" = "available" ]
}
```

- [ ] **Step 2: Run the focused Bats file and confirm it fails because the gateway does not exist.**
- [ ] **Step 3: Implement argv-only capability detection, request-state routing, interrupted-run recovery, and fail-closed unavailable results that prohibit mutation.**
- [ ] **Step 4: Wire the listener trio to the gateway and make the external consumer rollout a blocking prerequisite until its versioned conformance receipt proves direct dispatch is unreachable.**
- [ ] **Step 5: Run Bats, `bash -n`, ShellCheck, trio derivation, and commit.**

### Task 2: Scope-aware primary Project selection

**Files:**
- Modify: `crates/autospec-core/src/managed_project.rs`
- Modify: `crates/autospec-core/src/autonomous/config/project_board.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project/project.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project/store.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project/github.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project/github/parse.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project/github/transport.rs`
- Test: `crates/autospec-cli/tests/managed_project.rs`

**Interfaces:**
- Consumes: typed `PrimaryProjectFacts` including canonical repository, optional spec/binding, optional ProductKey, managed mode, issue count, cross-repository edges, and requested owner.
- Produces: namespaced `PrimaryProjectPolicy::{SpecPortfolio, ManagedProduct, CreateManagedProduct}` and a verified `PrimaryProjectBinding` using one schema-compatible managed marker block.

- [ ] **Step 1: Add failing tests for existing spec lineage, new spec-sized work, bounded onboarded work, bounded untracked work, and ambiguous candidates.**
- [ ] **Step 2: Run each test and confirm failure at the missing policy/binding API.**
- [ ] **Step 3: Implement the total precedence table, `portfolio:`/`product:` namespaces, `repo:<owner>__<repo>` fallback key, and external-mode-as-secondary rule from typed facts.**
- [ ] **Step 4: Reuse `ManagedProjectStore`, extend the existing marker parser, add built-in/custom field discovery, and reuse GraphQL transport, journal, and lock primitives.**
- [ ] **Step 5: Prove ambiguity and permission uncertainty block before issue creation; run focused Rust tests and commit.**

### Task 3: Mandatory portfolio transaction for spec-sized work

**Files:**
- Create: `crates/autospec-cli/src/commands/managed_project/portfolio.rs`
- Modify: `crates/autospec-cli/src/main.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs`
- Modify: `skills/autospec/SKILL.md`
- Modify: `skills/autospec-define/SKILL.md`
- Modify: `skills/autospec-split/SKILL.md`
- Modify: `skills/autospec-run/SKILL.md`
- Modify: `skills/autospec-explore/SKILL.md`
- Update: corresponding Codex/OpenCode mirrors and generated goldens
- Test: `crates/autospec-cli/tests/managed_project.rs`

**Interfaces:**
- Produces: `autospec portfolio validate|apply|reconcile` over a frozen `autospec.portfolio-plan.v1` manifest.

- [ ] **Step 1: Add failing manifest identity, lease, lost-create-response, partial-resume, and no-issue-before-project tests.**
- [ ] **Step 2: Implement pure validate/freeze, verified Project binding, deterministic issue materialization, journal checkpoints, and reconciliation.**
- [ ] **Step 3: Put the portfolio transaction before the first issue create in every spec decomposition trio, before claim/admission in autospec-run, and in autospec-explore's existing-spec handoff.**
- [ ] **Step 4: Add cross-repository dependency admission, coordination-ref permission/ruleset preflight, dry-run tri-state, and completion-policy reconciliation tests.**
- [ ] **Step 5: Run focused Rust/Bats suites, lock-step/golden validation, and commit.**

### Task 4: End-to-end orchestration proof and documentation

**Files:**
- Modify: `AGENTS.md`
- Modify: `README.md`
- Modify: `docs/USER_MANUAL.md`
- Modify: `docs/KNOWN_LIMITATIONS.md`
- Test: `tests/autospec/managed-project-workflows.bats`
- Test: `tests/autospec/managed-project-publisher-execution.bats`

**Interfaces:**
- Consumes: typed gateway, primary Project binding, portfolio transaction.
- Produces: one observable implementation lifecycle with Autospec run identity and primary Project URL.

- [ ] **Step 1: Add a failing end-to-end fixture proving installed Autospec receives the request, a primary Project is acknowledged first, and no direct serverless implementer event is recorded.**
- [ ] **Step 2: Add unavailable-blocks-mutation, interrupted-run recovery, and external-consumer conformance fixtures.**
- [ ] **Step 3: Update operator/developer documentation with the routing matrix and internal-subagent boundary.**
- [ ] **Step 4: Run `cargo test --workspace --no-fail-fast`, focused Bats, `autospec validate`, Clippy, shell validation, and `git diff --check`.**
- [ ] **Step 5: Perform the opt-in no-mock Projects v2 smoke run, reconcile/clean disposable artifacts, and commit verification evidence.**
