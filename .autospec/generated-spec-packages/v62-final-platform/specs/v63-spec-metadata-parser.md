# Spec Metadata And Parser

## Version

V63

## Objective

Implement the core model for loading markdown specs into structured metadata.

## Scope

- Parse markdown spec files.
- Extract frontmatter or heading-based metadata.
- Validate required fields.
- Report structured errors.
- Provide JSON serialization.

## Non-Goals

- No execution queue.
- No graph ordering beyond collecting dependency strings.

## Dependencies

- `v62-rust-core-workspace`

## Files To Create/Modify

- Create: `crates/autospec-core/src/spec/mod.rs`
- Create: `crates/autospec-core/src/spec/parser.rs`
- Create: `crates/autospec-core/src/spec/model.rs`
- Create: `crates/autospec-core/tests/spec_parser.rs`
- Create: `schemas/autospec-spec-metadata.schema.json`
- Modify: `docs/concepts.md`

## Implementation Steps

1. Define `SpecId`, `SpecVersion`, `SpecStatus`, and `SpecMetadata`.
2. Parse title, version, objective, dependencies, acceptance criteria, validation command.
3. Support markdown files with or without YAML frontmatter by falling back to headings.
4. Validate missing required fields with line-aware errors.
5. Add tests for valid, missing-field, and malformed-dependency specs.

## Acceptance Criteria

- [ ] Valid generated package specs parse successfully.
- [ ] Missing title/version/objective fails with structured error.
- [ ] Parser output serializes to JSON.
- [ ] Schema exists and matches parser output.

## Validation Commands

```bash
cargo test --all spec_parser
bash scripts/validate.sh --fast
```

## Expected Outputs

- `autospec plan --input .autospec/generated-spec-packages/v62-final-platform --json` can list parsed specs after V69 wires CLI.

## Rollback/Handoff Notes

If parser ambiguity appears, accept only the generated package format first and document broader markdown support as a future spec.
