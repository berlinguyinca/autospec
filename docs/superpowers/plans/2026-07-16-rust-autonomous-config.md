# Rust Autonomous Repository Configuration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Rust autonomous mainline health consume bounded repository-local configuration without shell or environment authority.

**Architecture:** A pure core parser owns `main_health` configuration. The health model resolves CLI/config/default branch precedence and marks exact ignored evidence advisory. The CLI reads the repository file once before main-health or foreground admission.

**Tech Stack:** Rust 2021 standard library, existing Rust health model and Cargo tests; no new dependencies.

## Global Constraints

- Only support `main_health.branch` and exact `main_health.ignore_checks`.
- Precedence is CLI branch, then config branch, then GitHub default branch.
- Missing config preserves behavior; invalid relevant config fails before dispatch, claim mutation, or state creation.
- Do not execute shell, reference legacy scripts, use a global health env var, or add a YAML dependency.
- Retain ignored evidence as advisory instead of deleting it.

---

### Task 1: Add a pure bounded config parser

**Files:** Create `crates/autospec-core/src/autonomous/config.rs`; modify `crates/autospec-core/src/lib.rs`; test in `crates/autospec-core/tests/autonomous_config.rs`.

**Interfaces:** `AutonomousConfig::parse(&str) -> Result<AutonomousConfig, String>` produces `MainHealthConfig { branch: Option<String>, ignore_checks: BTreeSet<String> }` without I/O.

- [x] **Step 1: Write failing parser tests**

Test a valid `main_health` branch and list, absent config, unrelated top-level policy keys, quoted list values, duplicate fields, scalar `ignore_checks`, empty values, nested/inline values, and unknown `main_health` fields. The valid assertion requires `master_ai` plus `Unit Tests`; invalid relevant shapes must return `Err`.

- [x] **Step 2: Verify the parser test fails**

Run `cargo test -p autospec-core --test autonomous_config --quiet`. Expected: module/parser unresolved.

- [x] **Step 3: Implement the closed parser**

Define defaulted `AutonomousConfig` and `MainHealthConfig`, export `autonomous::config`, and parse only the `main_health` indentation block. Ignore unrelated top-level keys. Reject unknown/duplicate/malformed/empty/inline/nested values inside `main_health`.

- [x] **Step 4: Verify parser coverage**

Run `cargo test -p autospec-core --test autonomous_config --quiet`. Expected: every valid and invalid parser assertion passes.

- [x] **Step 5: Commit the parser**

Stage `crates/autospec-core/src/lib.rs`, `crates/autospec-core/src/autonomous/config.rs`, and `crates/autospec-core/tests/autonomous_config.rs`; commit `feat: parse autonomous repository health config` using the Lore trailer protocol.

### Task 2: Apply configuration through the pure health model

**Files:** Modify `crates/autospec-core/src/autonomous/mainline_health.rs`; test in `crates/autospec-core/tests/mainline_health.rs`.

**Interfaces:** Extend `HealthBranchInput` with `configured_branch: Option<String>`, add `HealthBranchSource::Configured`, and expose `apply_ignored_checks(Vec<CheckEvidence>, &BTreeSet<String>) -> Vec<CheckEvidence>`.

- [x] **Step 1: Write failing model tests**

Require configured `master_ai` to win when CLI branch is absent, CLI branch to continue winning over config, and default branch to remain the final fallback. Require failed and pending `Unit Tests` evidence to become advisory/continue when ignored, while any unmatched failed/pending evidence remains required and blocks.

- [x] **Step 2: Verify the model test fails**

Run `cargo test -p autospec-core --test mainline_health --quiet`. Expected: configured branch and advisory conversion are unavailable.

- [x] **Step 3: Implement precedence and exact advisory conversion**

Resolve nonempty explicit, then configured, then default branch. Reconstruct only exact matching entries with `required: false`; preserve all fields and the required status of unmatched evidence.

- [x] **Step 4: Verify health policy**

Run `cargo test -p autospec-core --test mainline_health --quiet`. Expected: precedence plus ignored/unignored evidence cases pass.

- [x] **Step 5: Commit the policy**

Stage the health model and test, then commit `feat: apply repository health policy in Rust` with Lore trailers.

### Task 3: Read config before Rust health admission

**Files:** Modify `crates/autospec-cli/src/commands/autonomous.rs`; test in `crates/autospec-cli/tests/autonomous_conductor_commands.rs`.

**Interfaces:** Add `load_autonomous_config(repo_dir: &str) -> Result<AutonomousConfig, String>` and supply its output to `load_main_health` for both `main-health` and foreground admission. Resolve a Git checkout root before constructing the config path.

- [x] **Step 1: Write failing CLI fixture tests**

Add `ForegroundFixture::write_autonomous_config`. Prove config branch `master_ai` is queried without a default-branch lookup; prove CLI `--branch` wins; prove an ignored failed check is advisory; prove malformed config returns exit `2` before executor dispatch; prove separate fixture repositories cannot share config.

- [x] **Step 2: Verify the fixture tests fail**

Run `cargo test -p autospec-cli --test autonomous_conductor_commands --quiet`. Expected: current command ignores the fixture file.

- [x] **Step 3: Implement the Rust file adapter**

Read `<checkout-root>/.autospec/autonomous.yml` when `repo_dir` lies inside a Git checkout; otherwise read `Path::new(repo_dir).join(".autospec/autonomous.yml")`. A missing file returns `AutonomousConfig::default`; every other read/parse error is a diagnostic. Do not use an environment variable or shell helper.

- [x] **Step 4: Verify CLI integration**

Run `cargo test -p autospec-cli --test autonomous_conductor_commands --quiet`. Expected: config branch, CLI override, advisory evidence, malformed config, and repository isolation pass.

- [x] **Step 5: Commit the adapter**

Stage the autonomous command and conductor tests, then commit `feat: load autonomous health config from the repository` with Lore trailers.

### Task 4: Document and prevent legacy authority

**Files:** Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`, `docs/CONFIG_REFERENCE.md`, `docs/runbooks/mainline-health-admission.md`, and `docs/cli-reference.md`.

- [x] **Step 1: Write a failing source-authority test**

Read the Rust autonomous command source and reject `Command::new("sh")`, `autonomous-resilience.sh`, and `AUTOSPEC_MAIN_HEALTH_` from the new config adapter path.

- [x] **Step 2: Verify the source test fails**

Run `cargo test -p autospec-cli --test autonomous_conductor_commands rust_config_adapter_has_no_shell_or_global_health_env_authority --quiet`. Expected: the test is absent before this step.

- [x] **Step 3: Document the contract and add the guard**

Document schema, precedence, exact advisory semantics, missing-file behavior, and fail-closed invalid configuration. State that Rust does not consume global legacy health environment variables.

- [x] **Step 4: Run full validation**

Run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace --quiet`, and `cargo run -q -p autospec-cli -- validate --fast`. Expected: all pass.

- [x] **Step 5: Commit documentation and guard**

Stage the test/docs and commit `docs: define Rust autonomous health configuration` with Lore trailers.

## Plan self-review

Parser, policy, file I/O, and documentation have separate testable tasks. Every #1602 acceptance criterion is covered. Drain, no-work, premerge, legacy scripts, and skill bodies are deliberately outside this bounded plan.
