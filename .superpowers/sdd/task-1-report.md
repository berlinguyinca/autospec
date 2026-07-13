# Task 1 — Freeze the full validation catalog

## Status

Complete. The cutover baseline has a checked-in ordered catalog for all 148 named
`check_*` gates in `scripts/validate.sh`.

## Commit

`test: freeze validation cutover catalog`

## TDD evidence

- **RED:** `cargo test -p autospec-core --test validation_catalog catalog_has_one_owner_slot_for_every_frozen_gate -- --exact` exited non-zero because `ValidationCatalog`/`ValidationCheck` and `catalog-v1.json` were absent.
- **GREEN:** `cargo test -p autospec-core --test validation_catalog -- --nocapture` passed all 3 tests: fixture parity, the frozen 148-ID count, and empty/duplicate-ID rejection.

## Final verification

- `cargo fmt --all --check` passed.
- `cargo test --workspace` passed: 167 tests passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `git diff --check` passed.
- Manual catalog audit: shell definitions, fixture IDs, and Rust catalog IDs are each 148 with no duplicates.

## Scope and concerns

Only the Task 1 module export, catalog model, catalog test, frozen fixture, and
baseline report were changed. The catalog preserves `scripts/validate.sh`
declaration order because it is the only complete order across all named gates.
The current shell executor also has dynamic per-skill discovery; this task deliberately
does not plan or execute that behavior. Catalog slots are non-executable placeholders
until later tasks assign structural checks or explicit tool commands.
