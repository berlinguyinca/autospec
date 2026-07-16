# Task 4A — Pure Tier 1.5 observer

## Status

Complete. Independent re-review found no Critical or Important defects.

## Result

`autospec_core::autonomous::tier15` now deterministically observes an in-memory
open/closed issue snapshot and emits only closed typed decisions. It has no
CLI, process, GitHub, store, queue, claim, or foreground authority.

## TDD and verification evidence

- **RED:** `cargo test -p autospec-core --test autonomous_tier15` initially
  failed with unresolved `autospec_core::autonomous::tier15`.
- **GREEN:** the focused Tier 1.5 observer target passes all nine tests.
- `cargo test -p autospec-core` passed.
- `cargo fmt --check` passed.
- `cargo clippy -p autospec-core --all-targets -- -D warnings` passed.
- `git diff --check` and the core source-authority scan passed.

## Files

- `crates/autospec-core/src/lib.rs`
- `crates/autospec-core/src/autonomous/tier15.rs`
- `crates/autospec-core/src/autonomous/tier15/model.rs`
- `crates/autospec-core/src/autonomous/tier15/observer.rs`
- `crates/autospec-core/tests/autonomous_tier15.rs`

## Behavior

- Identical open-number duplicates deduplicate in numeric order; differing
  payloads fail closed.
- Closed fingerprints, excluded or already-groomed labels, and budget limits
  become typed skips.
- Thin or ambiguous intent and unverified dependencies become typed holds;
  pre-existing security quarantine is preserved as a typed quarantine.
- Epic and template candidates are typed routes; bounded clear-intent candidates
  are typed produced observations.
- The evidence JSON contains every decision, classification, route, and reason.
- Label checks preserve legacy ASCII-case-insensitivity; an existing open or
  closed dependency is sufficient existence evidence. The closed-fingerprint
  preimage intentionally matches the legacy exact concatenation of title and
  the first 200 body codepoints.

## Boundary

Task 4A deliberately does not enumerate GitHub, persist a receipt, modify the
waterfall cursor, admit a queue candidate, or mutate an issue. Those concerns
belong to the separately owned CLI adapter task.
