# Repository Audit

Date: 2026-07-03

## Summary

AutoSpec is a multi-harness workflow repository made of portable skills, shell scripts, Python helpers, JavaScript helpers, schemas, Bats tests, and documentation.

## Observed Structure

- `skills/` - 31 skill surfaces.
- `scripts/` - 113 script files.
- `tests/` - 493 test files.
- `docs/specs/` - 155 design/specification files.
- `.autospec/` - runtime state, QA findings, release/baseline evidence, and launch reports.
- `schemas/` - JSON schemas for QA, release, fleet, fab, reliability, and related artifacts.
- `examples/hello-autospec/` - deterministic launch demo.
- `marketing/` - launch copy kit.

## Rust Workspace Check

No `Cargo.toml` or `*.rs` files were found in the checkout used for this historical V25 audit on 2026-07-03. Cargo commands were attempted and failed with "could not find `Cargo.toml`," so Rust validation is recorded as not applicable for that baseline state. Current V74 launch evidence supersedes this note for the final Rust-backed tree.

## Current Blockers

- Historical `.autospec/qa-verdict.json` is stale relative to HEAD.
- Full, non-fast `autospec validate` was previously interrupted; `autospec validate --fast` passed during this pass.
