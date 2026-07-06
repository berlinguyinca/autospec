# Dependency Graph And Execution Ordering

## Version

V64

## Objective

Build deterministic dependency graph validation and execution ordering for specs.

## Scope

- Build graph from parsed spec dependencies.
- Detect missing dependencies.
- Detect cycles.
- Generate ordered execution list.
- Emit machine-readable graph diagnostics.

## Non-Goals

- No execution of specs.
- No retries or queue persistence.

## Dependencies

- `v63-spec-metadata-parser`

## Files To Create/Modify

- Create: `crates/autospec-core/src/graph/mod.rs`
- Create: `crates/autospec-core/src/graph/order.rs`
- Create: `crates/autospec-core/tests/dependency_graph.rs`
- Create: `schemas/autospec-execution-order.schema.json`
- Modify: `docs/architecture.md`

## Implementation Steps

1. Implement topological sort over `SpecMetadata`.
2. Make ordering stable by sorting independent specs by version then id.
3. Return structured missing-dependency errors.
4. Return structured cycle errors with the cycle path.
5. Add tests for linear, diamond, missing, and cyclic graphs.

## Acceptance Criteria

- [ ] `execution-order.json` for this package validates as acyclic.
- [ ] Cycles fail with named spec ids.
- [ ] Missing dependency errors cite the referencing spec.
- [ ] Output order is deterministic across runs.

## Validation Commands

```bash
cargo test --all dependency_graph
bash scripts/validate.sh --fast
```

## Expected Outputs

- Stable execution order for package specs.

## Rollback/Handoff Notes

If graph behavior conflicts with existing shell ordering, keep Rust graph read-only until V69 CLI adoption.
