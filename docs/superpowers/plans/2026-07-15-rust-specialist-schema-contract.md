# Rust specialist roster schema-contract repair

## Goal

Make Rust-owned roster reuse accept every schema-valid cached roster and reject every cache that cannot be safely re-emitted under `autospec-explore-specialists.schema.json`.

## Scope

- Validate schema-required minimum values and the specialist slug pattern in `roster_json.rs` before a cache is reused.
- Accept the optional informational `generated_at` cache property without persisting it as runtime state.
- Treat a syntactically valid empty proposal array/object array as an intentional empty specialist list, not a fallback request.
- Add core and CLI regression coverage for invalid-cache regeneration, optional-field cache reuse, and empty proposal behavior.

## Non-goals

- No shell, Python, Bats, dependency, network, or compatibility-scanner changes.
- No schema format change or cache migration beyond rejecting invalid data and regenerating it.

## Verification

1. Add failing focused tests for each contract gap.
2. Run focused core and CLI tests, formatting, the implementation linter, full workspace tests, fast validator, scanner smoke test, Clippy, and `git diff --check`.
