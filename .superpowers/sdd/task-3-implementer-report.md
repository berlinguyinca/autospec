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

## Repair pass — retained audit and trusted policy hardening

### Result

Completed Tier 4 rollover history now fails closed unless it retains exactly the five prior-pass receipts with each coordinator's actual advancing exhausted status. Enabled Tier 4 evidence now requires an injected trusted `Tier4SourcePolicy`, and changed retries recover only the known unreferenced Tier 4 artifacts while holding the store lock.

### TDD evidence

- **RED:** clearing `completed_receipts` from a valid Tier 4 rollover state loaded successfully.
- **GREEN:** core state validation and CLI replay now require all five prior-pass entries whenever the cursor is Tier 1 with a pass greater than one.
- **RED:** fully rehashed retained receipts with non-advancing Tier 1/1.5/2/3/4 dry statuses were accepted for Tier 1 and Tier 1.5.
- **GREEN:** replay accepts only the exact advancing exhausted reasons: Tier 1/1.5 `no_proposals_generated`, Tier 2/Tier 4 the three closed discovery reasons, and Tier 3 `no_metadata_findings`.
- **RED:** a changed trusted retry after an untrusted unsealed attempt failed on conflicting Tier 4 evidence.
- **GREEN:** lock-held cleanup removes only the eight known Tier 4 artifact paths; an unrelated file survives and the changed retry seals its receipt.

### Changes

- Added an internal typed `WaterfallStore` acquisition seam for the checked-in `Tier4SourcePolicy`; missing policy rejects completed enabled replay and retained completed history, while disabled receipts remain exactly policy-only.
- Bound completed Tier 4 source-policy evidence to the trusted typed policy and the canonical rendered-evidence replay. A complete, rehashed alternate valid dry chain is rejected under the expected policy.
- Updated state/store fixtures that formerly modelled a new pass with a pass id greater than one; first-pass fixtures preserve their original test intent without bypassing retained-history rules.

### Repair verification

Passed:

- `cargo test -p autospec-core --test autonomous_tier4` (8 tests)
- `cargo test -p autospec-core --test autonomous_waterfall` (4 tests)
- `cargo test -p autospec-cli --bin autospec tier2` (17 tests)
- `cargo test -p autospec-cli --bin autospec tier3` (16 tests)
- `cargo test -p autospec-cli --bin autospec tier4` (17 tests)
- `cargo test -p autospec-cli --bin autospec waterfall` (6 tests)
- `cargo fmt --check`
- `cargo clippy -p autospec-cli --bin autospec -- -D warnings`
- `git diff --check`

The root checkout does not contain `scripts/validate.sh`; Rust tests, formatting, lint, and diff checks are the available verification surface for this change.
