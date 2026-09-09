## Goal

Add `tests/contracts/config-conformance.bats` that runs conformance on every config file the schema accepts.

## Files to read first

- tests/contracts/config-conformance.bats
- crates/contracts/src/config.rs

## Implementation outline

1. Add the module behind the existing crate layout.
2. Add the test listed under `Tests required`.

## Tests required

- bats tests/contracts/config-conformance.bats

## Dependencies

Depends on issue #3009
Depends on issue #3011

## Dependency justification

- #3009 — consumes the config schema validator added by #3009 (consumes-new-interface), which does not exist on the current base branch.
- #3011 — drives the conformance harness added by `tests/contracts/conformance.bats` (verification-requires-predecessor).

## Files touched

- tests/contracts/config-conformance.bats

## Acceptance criteria

- [ ] `bats tests/contracts/config-conformance.bats` exits 0 after the change.
- [ ] `tests/contracts/config-conformance.bats` exists in the repository tree.

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/contracts/config-conformance.bats
```
