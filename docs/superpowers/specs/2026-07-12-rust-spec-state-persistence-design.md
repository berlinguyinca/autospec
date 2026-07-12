# Rust Spec-State Persistence Design

**Date:** 2026-07-12
**Status:** approved for implementation
**Parent:** [Rust Runtime Consolidation Completion Design](2026-07-12-rust-runtime-consolidation-completion-design.md), [#1861](https://github.com/berlinguyinca/autospec/issues/1861)

## Goal

Make Rust the durable owner of spec lifecycle state by storing validated package progress in `.autospec/state/specs.json` and recovering a completed temporary write after an interrupted promotion.

## Scope

This slice adds a pure Rust `SpecStateStore` in `autospec-core` and its on-disk representation. It supports loading, validating, deterministically rendering, and recoverably replacing the spec-state document. It remains a local, non-executing primitive: it does not invoke agents, run commands, modify queues, or replace a shell wrapper.

The store document is schema version 1:

```json
{
  "schema": 1,
  "specs": [
    {
      "spec_id": "v65-spec-state-validation",
      "state": "passed",
      "deferred_reason": null,
      "superseded_by": null
    }
  ]
}
```

`specs` are always rendered in ascending `spec_id` order. The supported lifecycle states remain `planned`, `ready`, `running`, `passed`, `failed`, `blocked`, `deferred`, and `superseded`.

## Contract

- `SpecStateStore::load_or_default(root)` returns an empty store only when neither the primary state file nor its recovery file exists.
- `SpecStateStore::save(root)` creates `.autospec/state/`, writes a complete `specs.json.tmp`, synchronizes it, then promotes it to `specs.json`. On Unix, it synchronizes newly created `.autospec/` and `state/` directory entries through their parent chain, then synchronizes the state directory after the temporary write and after promotion. Where replacement requires a remove-and-rename fallback, the completed temporary file remains available for recovery if promotion is interrupted.
- On load, a valid primary file wins. If the primary file is absent or malformed and `specs.json.tmp` is complete and valid, the store promotes that temporary file and returns it. If no valid file exists, loading fails clearly instead of silently losing lifecycle state.
- The store rejects an unsupported schema, malformed JSON, duplicate or invalid spec IDs, missing deferred reasons, conflicting lifecycle metadata, and a superseded entry whose replacement is absent or points to itself.
- The existing `SpecLifecycle` transition API stays compatible. The store is the boundary that validates data constructed outside that API.
- No new dependencies are added. The JSON encoder/parser is deliberately limited to this documented schema, accepts standard string escapes, and rejects unknown keys or trailing content.

## Alternatives considered

- **Add `serde`/`serde_json`:** rejected because this conversion slice must use the current dependency-free workspace.
- **Persist only on the future CLI layer:** rejected because queue, report, and resume need a shared durable core contract before a CLI cutover.
- **Use a best-effort direct overwrite:** rejected because an interrupted write can leave an empty or truncated state file and makes later resume behavior unknowable.

## Acceptance criteria

- A saved state store round-trips all lifecycle fields and emits stable, valid JSON.
- A valid temporary state file is recovered when the primary file is missing or malformed.
- A malformed primary file without a valid recovery file fails; it never becomes an empty store.
- Store validation rejects missing deferred reasons, invalid or duplicate IDs, and unknown/self supersession references.
- Existing lifecycle transition tests continue to pass.
- `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `bash scripts/validate.sh --fast` pass.

## Non-goals and follow-up

This is not queue persistence, agent execution, evidence capture, or command execution. The next slice persists `ExecutionQueue` under `.autospec/runs/<run-id>/queue.json` and resumes the latest incomplete queue using this state-store contract.
