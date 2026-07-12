# Rust Runtime Consolidation Completion Design

**Date:** 2026-07-12
**Status:** implementation started
**Extends:** [#1861](https://github.com/berlinguyinca/autospec/issues/1861), `docs/specs/2026-07-11-rust-core-runtime-consolidation-design.md`

## Goal

Finish the remaining Rust-runtime consolidation through small, parity-proven cutovers, beginning with a Rust-owned inventory of stateful legacy runtime paths and a clean Rust quality gate.

## Context and evidence

This is not a whole-repository language rewrite. `docs/architecture.md` defines Rust as the home for platform primitives while skill and wrapper surfaces remain compatible. The original #1861 workstream issues (#1862 through #1869) are closed, but the epic remains open because `scripts/validate.sh` still delegates to Rust only to re-enter its legacy shell implementation. The completion work must therefore target runtime ownership and observed parity rather than counting `.sh` files.

The first migration tranche is present in `crates/autospec-core`: runtime-policy classification, validation affected-path routing, lint fixtures, autonomous auditing, claim leases, and the context-monitor engine. The remaining durable-core backlog is state/queue/evidence persistence, core-backed read-only commands, controlled run/resume execution, agent-result ingestion, and retirement of legacy fallbacks after an observation and shadow period.

## Decisions

1. Preserve the existing public skill and shell command names. Rust replaces stateful platform implementation behind compatibility wrappers; it does not replace harness prompt prose or installation glue.
2. Use the existing `RuntimeClass` R0-R4 policy as the source of truth for migration status. Add a repository audit command so the candidate set is deterministic and reviewable instead of inferred from ad hoc searches.
3. Keep the audit side-effect free. It reads regular files below a supplied repository root, ignores build/VCS directories, and reports only recognized platform source paths. It must not modify files, call GitHub, or execute validation.
4. Do not add dependencies. The audit uses `std::fs`, `std::path`, and the existing runtime-policy module.
5. Every subsequent R1 cutover follows observe → shadow → delegate. The wrapper fallback can be removed only after fixture-backed parity and a release-cycle escape hatch.

## First implementation slice

### Rust runtime audit

Add `autospec runtime audit [--root PATH] [--json]`.

- It recursively inspects `scripts/`, `skills/`, and `packages/` under the root.
- It skips `.git`, `target`, `node_modules`, and any non-file entry.
- It uses `autospec_core::runtime_policy::classify_path` for every candidate and returns deterministic, path-sorted R0-R4 groups.
- Text output contains each group heading, its count, and its member paths. JSON includes `command`, `root`, and a `classes` object with arrays keyed by `R0` through `R4`.
- A missing root, an unreadable root, or an unknown option returns a non-zero command error.

This turns #1861's runtime-ownership policy into an actionable migration queue without widening the current wrapper behavior.

### Quality-gate repair

Fix the existing `clippy::field-reassign-with-default` failure in the autonomous CLI parser with a struct initializer. The change is behavior-preserving and remains protected by the existing autonomous CLI integration tests.

## Completion sequence after this slice

1. Persist spec state, queue entries, validation results, and evidence artifacts with round-trip and crash-resume tests.
2. Replace success-shaped read-only CLI stubs (`status`, `plan`, `report`, and `validate`) with core-backed contract output.
3. Add safe `init`, `run`, and `resume` on top of the persisted queue and agent-result contract; preserve explicit non-zero behavior until a safe execution path exists.
4. Migrate one R1 validation/guard path at a time behind parity fixtures, starting with affected-check selection and result aggregation.
5. Make a written context-monitor cutover decision from the Rust/Python parity and operational metrics.
6. Retire each shell fallback only after observed parity, a one-release escape hatch, and a recorded removal issue.

## Acceptance criteria

- `autospec runtime audit --root <fixture> --json` emits deterministic R0-R4 file groups without scanning ignored build/VCS directories.
- `autospec runtime audit --root <missing>` fails with a clear error.
- `docs/cli-reference.md` documents the new audit command and its read-only behavior.
- `cargo test --workspace`, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings` pass.
- `bash scripts/validate.sh --fast` passes through the compatibility wrapper.

## Risks and mitigations

- **False confidence from classification:** the audit is a planning inventory, not permission to port a path. Every R1 result still needs a parity fixture and a bounded migration issue.
- **Traversal noise:** the command limits itself to known platform roots and skips generated/build directories.
- **Fallback drift:** no fallback is removed in this slice; future removal is gated by fixture parity and release-cycle observation.
