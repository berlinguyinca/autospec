# Rust Direct Validation Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the shell validation executor with `autospec validate`, then delete `scripts/validate.sh` and every legacy validation handoff path.

**Architecture:** `autospec-core::validation` owns the frozen check catalog, safe external-tool definitions, Rust-native structural checks, deterministic plan construction, bounded scheduling, and schema-2 results. `autospec-cli` owns public option parsing and rendering. Every former shell gate has exactly one Rust-native or typed external-tool owner; no validation command runs through a shell string.

**Tech Stack:** Rust 2021 standard library; existing Bats, Python, Node, Cargo, and Bash tools when explicitly invoked; no new dependencies.

## Global Constraints

- `autospec validate` is the only supported validation entry point after the cutover.
- Preserve every current gate’s stable ID, requiredness, and display order before deleting its shell implementation.
- External commands use a program and argument vector; prohibit `sh -c`, `bash -c`, and interpolated command strings.
- Every result records schema version, check ID, requiredness, exit status, elapsed milliseconds, spawn count, stdout/stderr byte counts, and output digest.
- `--fast` skips Bats and Python suites but retains structural gates. Scoped and parallel modes retain their documented behavior.
- Do not change the Python context-monitor driver in this plan.
- Follow RED → GREEN → REFACTOR for every behavior change and preserve multi-harness lock-step bodies.

---

## File Structure

- `crates/autospec-core/src/validation/catalog.rs` — ordered canonical check catalog and owner validation.
- `crates/autospec-core/src/validation/command.rs` — safe process definitions and process-result capture.
- `crates/autospec-core/src/validation/options.rs` — pure parser for validation options.
- `crates/autospec-core/src/validation/plan.rs` — full/fast/scoped check selection.
- `crates/autospec-core/src/validation/runner.rs` — deterministic bounded executor.
- `crates/autospec-core/src/validation/structural.rs` — Rust-native lock-step, discovery, file, and literal-content checks.
- `crates/autospec-core/src/validation/results.rs` — schema-2 check report and aggregate.
- `crates/autospec-core/tests/validation_catalog.rs` — catalog and owner completeness tests.
- `crates/autospec-core/tests/validation_options.rs` — public parser tests.
- `crates/autospec-core/tests/validation_runner.rs` — safe command, metadata, selection, and scheduler tests.
- `crates/autospec-core/tests/validation_structural.rs` — fixture-backed structural tests.
- `crates/autospec-cli/src/commands/validate.rs` — direct Rust CLI.
- `crates/autospec-cli/tests/validation_parity.rs` — full, fast, scoped, parallel, and failure parity fixtures.
- `crates/autospec-cli/tests/fixtures/validation-cutover/` — frozen catalog and expected reports.

## Task 1: Freeze the full validation catalog

**Files:**
- Create: `crates/autospec-core/src/validation/catalog.rs`
- Modify: `crates/autospec-core/src/validation/mod.rs`
- Create: `crates/autospec-core/tests/validation_catalog.rs`
- Create: `crates/autospec-cli/tests/fixtures/validation-cutover/catalog-v1.json`
- Create: `docs/reports/2026-07-12-validation-cutover-baseline.md`

**Consumes:** the 148 `check_*` gates in `scripts/validate.sh`.

**Produces:** a checked-in, ordered catalog with one owner slot for every existing gate.

- [ ] **Step 1: Write the failing catalog-completeness test.**

```rust
#[test]
fn catalog_has_one_owner_slot_for_every_frozen_gate() {
    let catalog = ValidationCatalog::standard();
    assert_eq!(catalog.ids(), frozen_catalog_ids());
    assert!(catalog.validate().is_ok());
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-core --test validation_catalog catalog_has_one_owner_slot_for_every_frozen_gate -- --exact`

Expected: FAIL because `ValidationCatalog` and the fixture do not exist.

- [ ] **Step 3: Add the catalog model.**

```rust
pub struct ValidationCheck {
    pub id: &'static str,
    pub required: bool,
    pub independent: bool,
    pub modes: CheckModes,
    pub owner: CheckOwner,
}

pub enum CheckOwner { RustNative(StructuralCheck), External(ToolCommand) }
```

Capture the current execution order in `catalog-v1.json`, reject empty/duplicate IDs,
and record the catalog count in the baseline report.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-core --test validation_catalog -- --nocapture`

Expected: PASS; fixture IDs and catalog IDs match exactly.

Commit: `test: freeze validation cutover catalog`.

## Task 2: Build safe process definitions and schema-2 results

**Files:**
- Create: `crates/autospec-core/src/validation/command.rs`
- Modify: `crates/autospec-core/src/validation/results.rs`
- Modify: `crates/autospec-core/src/validation/mod.rs`
- Create: `crates/autospec-core/tests/validation_runner.rs`

**Consumes:** `ValidationCheck` from Task 1.

**Produces:** `ToolCommand`, `CheckResult`, and a schema-2 aggregate.

- [ ] **Step 1: Write failing safety and result tests.**

```rust
#[test]
fn tool_commands_reject_shell_execution_shapes() {
    assert!(ToolCommand::new("sh", ["-c", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("bash", ["-c", "echo unsafe"]).is_err());
}

#[test]
fn completed_result_serializes_execution_metadata() {
    let result = CheckResult::completed("lockstep", true, 0, 12, 1, 4, 0, "digest");
    assert!(result.to_json().contains("\"elapsed_ms\":12"));
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-core --test validation_runner -- --nocapture`

Expected: FAIL because safe tool commands and schema-2 results do not exist.

- [ ] **Step 3: Implement process boundaries.**

```rust
pub struct ToolCommand { program: PathBuf, args: Vec<OsString> }
pub struct CheckResult {
    pub id: String, pub required: bool, pub exit_code: Option<i32>,
    pub elapsed_ms: u128, pub spawn_count: u32, pub stdout_bytes: usize,
    pub stderr_bytes: usize, pub output_digest: String,
}
```

Execute through `std::process::Command`, use the repository root as working directory,
and map missing programs or signals to a non-success typed result.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-core --test validation_runner -- --nocapture`

Expected: PASS; no command-string execution is possible and metadata round-trips.

Commit: `feat: model direct validation commands safely`.

## Task 3: Replace the CLI fallback with direct option parsing

**Files:**
- Create: `crates/autospec-core/src/validation/options.rs`
- Modify: `crates/autospec-core/src/validation/mod.rs`
- Modify: `crates/autospec-cli/src/commands/validate.rs`
- Create: `crates/autospec-core/tests/validation_options.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`

**Consumes:** Tasks 1-2.

**Produces:** `ValidationOptions { fast, changed_base, since, jobs, json }` without a
shell handoff.

- [ ] **Step 1: Write failing public-option tests.**

```rust
#[test]
fn options_accept_fast_scoped_and_parallel_forms() {
    let options = ValidationOptions::parse(["--fast", "--changed=origin/main", "--jobs=4"])
        .unwrap();
    assert!(options.fast);
    assert_eq!(options.changed_base.as_deref(), Some("origin/main"));
    assert_eq!(options.jobs, Jobs::Fixed(4));
}

#[test]
fn options_reject_unknown_or_incomplete_values() {
    assert!(ValidationOptions::parse(["--unknown"]).is_err());
    assert!(ValidationOptions::parse(["--since"]).is_err());
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-core --test validation_options -- --nocapture`

Expected: FAIL because the options API does not exist.

- [ ] **Step 3: Implement parser and direct CLI dispatch.**

Support `--fast`/`--no-bats`, `--changed[=<base>]`, `--since <ref>`,
`--jobs[=<count>|auto]`, `--json`, `--path`, and `--shadow-results`. Remove
`run_legacy_shell`, `is_shadow_results_command`, and all environment-based execution
branches from the Rust CLI.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-core --test validation_options && cargo test -p autospec-cli --test cli_commands validate_ -- --nocapture`

Expected: PASS; direct execution options no longer direct callers to a shell script.

Commit: `feat: parse direct Rust validation options`.

## Task 4: Port pure structural gates to Rust

**Files:**
- Create: `crates/autospec-core/src/validation/structural.rs`
- Modify: `crates/autospec-core/src/validation/catalog.rs`
- Create: `crates/autospec-core/tests/validation_structural.rs`
- Create: `crates/autospec-cli/tests/fixtures/validation-cutover/valid-skill/`
- Create: `crates/autospec-cli/tests/fixtures/validation-cutover/drifted-skill/`

**Consumes:** Tasks 1-3.

**Produces:** Rust owners for skill discovery, trio/duo lock-step, required file
presence, and deterministic literal-content checks.

- [ ] **Step 1: Write failing fixture tests.**

```rust
#[test]
fn matching_trio_bodies_pass_without_external_processes() {
    assert!(run_structural(fixture("valid-skill")).is_ok());
}

#[test]
fn drifted_duo_reports_the_divergent_file() {
    let failure = run_structural(fixture("drifted-skill")).unwrap_err();
    assert!(failure.message.contains("codex/prompt.md"));
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-core --test validation_structural -- --nocapture`

Expected: FAIL because the structural runner does not exist.

- [ ] **Step 3: Implement filesystem checks.**

Use `std::fs` to discover skill trios/duos, strip frontmatter, compare bodies, verify
required files, and return stable diagnostics. Do not invoke `diff`, `awk`, or a
shell. Register these entries as `CheckOwner::RustNative`.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-core --test validation_structural -- --nocapture`

Expected: PASS; matching fixtures pass and drift diagnostics identify the failing file.

Commit: `feat: run structural validation in Rust`.

## Task 5: Assign every remaining shell gate a typed Rust owner

**Files:**
- Modify: `crates/autospec-core/src/validation/catalog.rs`
- Modify: `crates/autospec-core/src/validation/structural.rs`
- Modify: `crates/autospec-core/src/validation/command.rs`
- Modify: `crates/autospec-core/tests/validation_catalog.rs`
- Modify: `crates/autospec-core/tests/validation_runner.rs`

**Consumes:** frozen catalog and execution primitives.

**Produces:** one non-shell owner for all 148 frozen IDs.

- [ ] **Step 1: Write failing owner-coverage tests.**

```rust
#[test]
fn every_frozen_gate_has_exactly_one_non_shell_owner() {
    for check in ValidationCatalog::standard().checks() {
        assert!(check.owner.is_rust_native() || check.owner.is_explicit_tool());
    }
}

#[test]
fn missing_required_tool_is_a_required_failure_not_a_skip() {
    let result = run_missing_tool_check();
    assert!(result.required);
    assert_eq!(result.exit_code, None);
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-core --test validation_catalog --test validation_runner -- --nocapture`

Expected: FAIL until every frozen ID has a direct owner.

- [ ] **Step 3: Port catalog owners in frozen order.**

Convert shell text checks to reusable Rust predicates (`file_contains`,
`all_members_contain`, `literal_sequence`, and `required_files`). Convert existing
Bats, Python, Cargo, Node, Bash-syntax, and standalone helper invocations to literal
`ToolCommand` program/argument definitions. Preserve original requiredness and do not
silently omit a gate.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-core --test validation_catalog --test validation_runner -- --nocapture`

Expected: PASS; no catalog entry is shell-owned and tool failures are explicit.

Commit: `feat: assign validation gates to Rust owners`.

## Task 6: Build deterministic full, fast, scoped, and parallel execution

**Files:**
- Create: `crates/autospec-core/src/validation/plan.rs`
- Create: `crates/autospec-core/src/validation/runner.rs`
- Modify: `crates/autospec-core/src/validation/mod.rs`
- Modify: `crates/autospec-cli/src/commands/validate.rs`
- Modify: `crates/autospec-core/tests/validation_runner.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`

**Consumes:** Tasks 1-5.

**Produces:** `ValidationPlan::build` and `ValidationRunner::run`.

- [ ] **Step 1: Write failing plan/scheduler tests.**

```rust
#[test]
fn fast_plan_keeps_structural_and_excludes_python_and_bats() {
    let plan = ValidationPlan::build(&catalog(), ValidationOptions::fast()).unwrap();
    assert!(plan.ids().contains(&"lockstep"));
    assert!(!plan.ids().contains(&"python-suites"));
}

#[test]
fn parallel_execution_renders_results_in_catalog_order() {
    assert_eq!(run_two_independent_checks(2).ids(), ["first", "second"]);
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-core --test validation_runner -- --nocapture`

Expected: FAIL because direct plan construction and scheduling do not exist.

- [ ] **Step 3: Implement deterministic planning.**

Use repository Git input for `--changed` and `--since`, add always-run checks, resolve
`--jobs auto` to CPU minus two with a minimum of one, run only independent entries in
parallel, and sort every completed result by catalog index before rendering.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-core --test validation_runner && cargo test -p autospec-cli --test cli_commands validate_ -- --nocapture`

Expected: PASS; `autospec validate --fast --json` emits a schema-2 direct-execution report.

Commit: `feat: execute validation plans directly in Rust`.

## Task 7: Freeze parity evidence

**Files:**
- Create: `crates/autospec-cli/tests/validation_parity.rs`
- Create: `crates/autospec-cli/tests/fixtures/validation-cutover/*.json`
- Modify: `docs/reports/2026-07-12-validation-cutover-baseline.md`
- Modify: `docs/cli-reference.md`

**Consumes:** direct executor from Task 6.

**Produces:** full, fast, scoped, parallel, required-failure, optional-failure, and
missing-tool fixtures.

- [ ] **Step 1: Write a failing parity test.**

```rust
#[test]
fn fast_fixture_matches_frozen_check_order_and_required_outcome() {
    let actual = run_fixture("fast-repository");
    let expected = fixture("fast-passing.json");
    assert_eq!(actual.check_ids(), expected.check_ids());
    assert_eq!(actual.required_status(), expected.required_status());
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-cli --test validation_parity -- --nocapture`

Expected: FAIL until the direct executor emits the frozen schema-2 report.

- [ ] **Step 3: Add complete parity fixtures and report evidence.**

Compare selected IDs/order, required outcome, spawn count, stdout/stderr bytes, and
output digest. Record observed elapsed milliseconds in the baseline report without
requiring byte-identical durations.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-cli --test validation_parity -- --nocapture`

Expected: PASS; every direct mode matches the frozen cutover contract.

Commit: `test: prove direct Rust validation parity`.

## Task 8: Migrate all tracked callers

**Files:**
- Modify: `.github/workflows/*.yml`
- Modify: `AGENTS.md`, `README.md`, `docs/**/*.md`
- Modify: `skills/**/SKILL.md`, `skills/**/codex/prompt.md`, `skills/**/opencode/agent.md`
- Modify: tests that call `bash scripts/validate.sh`
- Modify: `crates/autospec-cli/tests/validation_parity.rs`

**Consumes:** proven direct executor and parity corpus.

**Produces:** no operational repository caller of `bash scripts/validate.sh`.

- [ ] **Step 1: Write a failing tracked-reference test.**

```rust
#[test]
fn repository_callers_use_direct_rust_validation() {
    let output = Command::new("git")
        .args(["grep", "-n", "bash scripts/validate.sh", "--", ".github", "AGENTS.md", "README.md", "docs/cli-reference.md", "docs/workflows.md", "skills"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-cli --test validation_parity repository_callers_use_direct_rust_validation -- --exact`

Expected: FAIL while callers reference the shell command.

- [ ] **Step 3: Replace every caller.**

Use `autospec validate` in CI, scripts, docs, and tests. Keep multi-harness skill
bodies byte-identical after their frontmatter.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo test -p autospec-cli --test validation_parity repository_callers_use_direct_rust_validation -- --exact && autospec validate --fast`

Expected: PASS; direct Rust validation is the only operational invocation.

Commit: `refactor: route validation callers through Rust`.

## Task 9: Delete the legacy validation executor

**Files:**
- Delete: `scripts/validate.sh`
- Modify: `crates/autospec-cli/src/commands/validate.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`
- Delete: `tests/smoke/validate-rust-wrapper.bats`
- Modify: `tests/install/test_autospec_bin_path.sh`
- Modify: `tests/install/test_validate_nested_fast_guard.sh`
- Modify: `docs/specs/2026-07-11-rust-core-runtime-consolidation-design.md`
- Modify: `docs/reports/2026-07-12-rust-context-monitor-cutover.md`

**Consumes:** Tasks 1-8.

**Produces:** no shell validation executor or recursion environment variables.

- [ ] **Step 1: Write failing absence tests.**

```rust
#[test]
fn legacy_validation_symbols_are_absent_from_tracked_source() {
    let symbols = [
        ["scripts", "validate.sh"].join("/"),
        ["run", "legacy", "shell"].join("_"),
        format!("AUTOSPEC_{}{}", "FORCE_", "LEGACY_SHELL"),
        format!("AUTOSPEC_{}{}", "VALIDATE_", "FROM_SHELL"),
        format!("AUTOSPEC_{}{}", "VALIDATE_", "FROM_RUST"),
        format!("AUTOSPEC_{}{}", "VALIDATE_", "LEGACY_ACTIVE"),
    ];
    for symbol in symbols {
        let output = Command::new("git")
            .args(["grep", "-n", &symbol, "--", "crates/autospec-core", "crates/autospec-cli/src", "scripts", "tests/install", "tests/smoke", "docs/cli-reference.md", "docs/workflows.md", "docs/specs/2026-07-11-rust-core-runtime-consolidation-design.md", "docs/reports/2026-07-12-rust-context-monitor-cutover.md"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{symbol}");
    }
}
```

- [ ] **Step 2: Verify RED.**

Run: `cargo test -p autospec-cli --test validation_parity legacy_validation_symbols_are_absent_from_tracked_source -- --exact`

Expected: FAIL while the legacy executor and fallback symbols remain.

- [ ] **Step 3: Remove all fallback artifacts.**

Delete the shell executor and wrapper-only tests, remove the Rust handoff and the four
recursion variables, convert wrapper coverage into direct CLI coverage, and update
cutover documents to mark the removal complete.

- [ ] **Step 4: Verify GREEN and commit.**

Run: `cargo fmt --all --check && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && autospec validate --fast`

Expected: PASS; only the direct Rust command validates the repository.

Commit: `refactor: remove legacy validation executor`.

## Task 10: Record final evidence

**Files:**
- Modify: `docs/reports/2026-07-12-validation-cutover-baseline.md`
- Modify: `docs/cli-reference.md`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`

**Consumes:** Tasks 1-9.

**Produces:** auditable cutover proof.

- [ ] **Step 1: Write a failing removal-audit test.**

```rust
#[test]
fn runtime_audit_does_not_list_the_deleted_validation_script() {
    let output = autospec().args(["runtime", "audit", "--root", ".", "--json"])
        .output().unwrap();
    assert!(!String::from_utf8_lossy(&output.stdout).contains("scripts/validate.sh"));
}
```

- [ ] **Step 2: Verify the final test and completion suite.**

Run: `cargo test -p autospec-cli --test runtime_commands runtime_audit_does_not_list_the_deleted_validation_script -- --exact && cargo fmt --all --check && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && autospec validate --fast && git diff --check`

Expected: PASS; audit no longer sees the deleted script and the direct Rust gate is green.

- [ ] **Step 3: Record evidence and commit.**

Write catalog count, parity fixture verdicts, process/time/output observations, deleted
symbols, and final commands into the baseline report.

Commit: `docs: record Rust validation cutover evidence`.
