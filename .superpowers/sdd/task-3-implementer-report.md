# Task 3 implementer report — Tier 4 sealed CLI receipts

## Result

Implemented the sealed, local-only Tier 4 receipt boundary without activating or calling it from foreground execution.

## TDD evidence

- **RED:** `cargo test -p autospec-cli --bin autospec tier4` initially failed because the Tier 4 adapter and receipt coordinator modules did not exist.
- **GREEN:** added the disabled checked-in-policy adapter/receipt path; the disabled-policy test passed.
- **RED:** added the three-result rollover contract; it failed because enabled receipt persistence was absent.
- **GREEN:** added sealed Tier 4 evidence, receipt persistence, exact verification, retained rollover history, and the contract passed.
- **RED:** added a fully rehashed nested-key-order replay mutation; it replayed before nested canonical ordering was enforced.
- **GREEN:** added recursive canonical-shape validation; the mutation is rejected.
- **RED:** added the source-policy/envelope mismatch mutation; it replayed before evaluator-based consistency replay.
- **GREEN:** completed receipts now reconstruct through the core Tier 4 evaluator and require byte-exact canonical documents.

## Changes

- Added a pure `Tier4Input::DisabledByCheckedInPolicy` adapter and a local-only receipt coordinator.
- Added the exact Tier 4 artifact namespace: `policy.json`, `source_policy.json`, `sources.json`, `generated.json`, `dedup.json`, `verification.json`, `roi_rank.json`, and `failure.json`.
- Enforced producer identities, terminal/funnel consistency, exact artifact order, digest/predecessor chains, canonical nested JSON, policy/source correspondence, raw-body exclusion, and replay-before-scan behavior.
- Moved Tier 2/Tier 3 persistence and verifier methods out of CLI `waterfall.rs` into `waterfall/tier_evidence.rs`; the parent is 415 lines.
- Preserved all five prior-pass receipt references after a valid Tier 4 allowed dry rollover, replayed those at `next_pass_id - 1`, and cleared retained history only when the next-pass Tier 1 receipt is recorded.
- Restricted Tier 4 advancement to `no_proposals_generated`, `verification_rejected`, and `roi_filtered`.

## Verification

Passed:

- `cargo test -p autospec-core --test autonomous_tier4` (8 tests)
- `cargo test -p autospec-cli --bin autospec tier2` (17 tests)
- `cargo test -p autospec-cli --bin autospec tier3` (16 tests)
- `cargo test -p autospec-cli --bin autospec tier4` (13 tests)
- `cargo fmt --check`
- `cargo clippy -p autospec-cli --bin autospec -- -D warnings`
- `git diff --check`

## Remaining risk

Failure receipts are validated structurally from their sealed partial-evidence prefix because the core API intentionally does not expose construction of a sealed `Tier4Failure` from persisted documents. Completed observations receive the stronger full evaluator replay check.
