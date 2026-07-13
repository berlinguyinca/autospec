# Rust Direct Validation Cutover Design

**Date:** 2026-07-12
**Status:** approved design; implementation pending written-spec review
**Supersedes:** the validation-wrapper fallback in
`docs/specs/2026-07-11-rust-core-runtime-consolidation-design.md`

## Goal

Make `autospec validate` the sole repository validation entry point by replacing the
shell-owned validation orchestrator with a Rust-owned executor and deleting
`scripts/validate.sh` plus all fallback and recursion paths.

## Scope

This is a validation-runtime cutover. It removes legacy *validation orchestration*,
not every Bash or Python file in the repository. Rust will own CLI parsing, check
selection, scheduling, output, result aggregation, and process execution. Existing
independent test suites and helper tools may remain in their native runtime when
Rust invokes them through a typed check definition; they are not a shell validation
fallback.

The Python context-monitor package is outside this cutover. It remains a production
driver until a separate Rust driver replaces its adapters, injection, handoff,
telemetry, daemon lifecycle, and session integration.

## Current state

`scripts/validate.sh` is a 5,419-line executor with 149 named checks. It currently
owns its legacy shell body directly; the previous recursive Rust-to-shell handoff has
already been removed. The Rust CLI supports affected-check planning, captured-result
aggregation, and direct option parsing, while execution-capable options return an
explicit pending-executor error until the Rust runner is installed. The remaining
cutover work is to move all 149 gates to direct owners, wire that runner, and delete
the shell body.

The frozen 149-name catalog is a definition audit, not the legacy execution list.
The shell invokes 133 unique top-level checks in 138 ordered call occurrences (five
top-level calls are intentionally repeated). Fifteen named helpers execute through
per-skill or aggregating top-level checks, and `check_architecture_fitness_engine` is
defined but never invoked. Direct execution must retain all 149 symbols for ownership
auditing, preserve repeated top-level occurrences, and avoid introducing the
unreachable gate.

## Decisions

1. `autospec validate` becomes the only supported command. `scripts/validate.sh` is
   deleted, and all repository documentation, CI, tests, and skill instructions use
   `autospec validate` instead.
2. The Rust executor implements the public option contract: `--fast` and
   `--no-bats`; `--changed[=<base>]`; `--since <ref>`; `--jobs[=<count>|auto]`; and
   `--json`. Unknown options and missing option values fail non-zero with a clear
   diagnostic.
3. Validation checks are represented as typed Rust definitions. A definition declares
   its stable ID, requiredness, input selection, execution mode, reachability, and
   deterministic display order. Reachability is `top_level`, `internal_component`, or
   `legacy_unreachable`; only top-level checks emit executor results. An executable
   plan also records an occurrence index so repeated top-level calls remain distinct.
   Execution modes are Rust-native checks and explicit external-tool invocations.
   Arbitrary `sh -c` commands are not allowed.
4. Rust-native checks replace pure shell logic such as skill discovery, lock-step
   comparison, required-file checks, and deterministic text/content assertions.
   Typed external-tool checks invoke the existing explicit command and arguments
   needed for Bats, Python, Bash syntax, Cargo, or Node validation.
5. Each completed check emits a schema-versioned result containing check ID,
   requiredness, exit status, elapsed milliseconds, spawn count, stdout byte count,
   stderr byte count, and stable output digest. The aggregate fails when any required
   check fails.
6. The Rust executor preserves the current semantic distinction: `--fast` excludes
   Bats and Python suites but retains structural checks; scoped modes select the
   existing affected set plus always-run checks; `--jobs` permits only independent
   checks to run concurrently and reports results in stable check-ID order.
7. Before deleting the shell implementation, parity fixtures must cover successful
   and failing full, fast, scoped, and parallel executions, including missing-tool
   failures. The comparison asserts the selected check IDs/order, required-failure
   exit status, and per-check result metadata. Timing values are recorded for
   comparison but do not require byte-identical durations.
8. The cutover deletes the shell script, the Rust shell handoff, all four recursion
   variables, legacy fixtures, legacy-wrapper tests, and documentation claims that
   shell validation is supported. No compatibility wrapper or one-release escape
   hatch remains because the approved product decision is to remove legacy code.

## Architecture

`autospec-core::validation` owns the typed `ValidationCheck`, `ValidationPlan`,
`ValidationExecution`, and schema-versioned result types. It has a strict command
builder that accepts a program plus argument vector and rejects shell strings.
`autospec-cli::commands::validate` parses options, asks the core for a deterministic
plan, executes it, renders text or JSON, and returns non-zero if required checks
fail.

The check catalog is split by ownership: structural Rust checks, explicit external
tool checks, and the affected-path selector. It also records whether a check was
directly reachable from the legacy `main`, an internal helper, or an unreachable
definition. The scheduler receives selected top-level checks and a worker count,
records results by check ID, and renders only after all selected checks settle. An
aggregator may reuse an internal component procedure but does not emit that component
as a second top-level result. This separates portable validation semantics from the
platform tools each check intentionally calls.

## Migration sequence

1. Freeze the current shell behavior in a fixture corpus and check manifest. Capture
   all named definitions, their top-level/internal/unreachable reachability, every
   public option mode, selected check order, failure mapping, output metadata,
   process count, and elapsed time.
2. Add the typed Rust check and result model, then implement option parsing and a
   no-shell executor for the structural lock-step group. Tests begin red and prove
   both successful and deliberately drifted trios.
3. Port every remaining shell-owned check into either a Rust-native implementation or
   a typed external-tool definition. Preserve the check ID and requiredness for each
   current gate; do not silently drop a gate.
4. Run Rust and shell executors against the fixture corpus until their selected
   checks, required-failure status, and output metadata agree. Record the measured
   process/time/output data in a cutover report.
5. Change all repository callers to `autospec validate`, delete the shell executor and
   handoff code, remove legacy-only tests and environment variables, and make the
   Rust CLI reject their former flags as unknown environment configuration.

## Acceptance criteria

- `autospec validate --fast`, `--changed`, `--since`, `--jobs`, and `--json` implement
  the documented option contract without invoking `scripts/validate.sh`.
- The validation result document contains check ID, requiredness, exit status, elapsed
  milliseconds, spawn count, stdout/stderr byte counts, and stable output digest for
  every selected check.
- The fixture corpus proves equal check selection/order and required-failure outcome
  for legacy and Rust full, fast, scoped, and parallel cases before removal.
- Every one of the 149 frozen validation symbols has a Rust-native or typed
  external-tool owner; the manifest rejects missing or duplicate check IDs, and the
  executable plan contains only the 138 ordered invocations of the 133 unique legacy
  top-level checks.
- `scripts/validate.sh`, `run_legacy_shell`, `AUTOSPEC_FORCE_LEGACY_SHELL`,
  `AUTOSPEC_VALIDATE_FROM_SHELL`, `AUTOSPEC_VALIDATE_FROM_RUST`, and
  `AUTOSPEC_VALIDATE_LEGACY_ACTIVE` no longer exist in tracked source, docs, or tests.
- Repository CI and documentation invoke `autospec validate` directly.
- `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, and the Rust direct validation suite pass.

## Risks and controls

- **Dropped or expanded gate:** A checked-in manifest derived from the current 149
  check IDs blocks deletion until every ID has an explicit Rust-native or external-
  tool owner, while reachability metadata prevents duplicate helper execution and
  prevents the unreachable architecture-fitness definition from becoming a new gate.
- **Shell-injection regression:** External checks use `Command` program and argument
  vectors only; the executor rejects a command string.
- **Parallel nondeterminism:** Results are sorted by stable check ID and concurrency
  applies only to manifest entries marked independent.
- **Tool absence:** Each external check reports a typed missing-tool failure with the
  check ID and requiredness; Rust does not silently skip it.
- **Unbounded scope:** Context-monitor and non-validation R1 paths are excluded from
  this specification and require their own approved cutover designs.
