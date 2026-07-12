# Rust Evidence Persistence Implementation Plan

**Goal:** Persist and validate local Rust evidence bundles without adding dependencies or enabling execution.

**Architecture:** Extend `autospec_core::evidence` with a versioned bundle document, strict parser, deterministic renderer, and recovery-aware local file persistence. Reuse the existing crate-private JSON parser and the queue/state recovery pattern; keep release-report rendering separate from storage.

## Constraints

- No new dependencies or remote writes.
- Do not execute validation commands or enable the CLI stubs.
- Bundle paths are relative and confined to the bundle directory.
- A malformed bundle is always an error, never an empty result.

### Task 1: Add failing persistence and safety tests

- [ ] Add temporary-root test support to `crates/autospec-core/tests/evidence_bundle.rs`.
- [ ] Add red tests for round-trip, recovery, primary precedence, path traversal, run mismatch, duplicates, and escaped control characters.
- [ ] Confirm `cargo test -p autospec-core --test evidence_bundle` fails before production changes.

### Task 2: Implement durable evidence bundles

- [ ] Add timestamped evidence command records and versioned bundle JSON in `crates/autospec-core/src/evidence/mod.rs`.
- [ ] Validate run IDs and all artifact/log paths at write and read boundaries.
- [ ] Persist under `.autospec/evidence/<run-id>/bundle.json` with temporary-file recovery and primary-wins loading.
- [ ] Keep release-report behavior compatible while correcting JSON control-character escaping.

### Task 3: Verify and checkpoint

- [ ] Run focused evidence, workspace, format, clippy, and fast repository checks.
- [ ] Independently review persistence and path-safety semantics.
- [ ] Commit with a conventional Lore-format message.
