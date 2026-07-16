# Task 4B report

## Task 1 — read-only paginated Tier 1.5 scan

### RED

`cargo test -p autospec-cli --bin autospec commands::autonomous::tier15`

Failed before the adapter existed with unresolved `Tier15Scan`, `scan_with`,
strict-parser, pagination, repository-validation, and direct-GET argument symbols.

`cargo test -p autospec-cli --bin autospec commands::autonomous::tier15::tests::projected_page_keeps_raw_count_while_pull_requests_are_filtered`

Failed after adding the regression assertion because the shared parser accepted
the legacy array shape, which lacks the required unfiltered `raw_count`.

### GREEN

`cargo test -p autospec-cli --bin autospec commands::autonomous::tier15`

Passed after the adapter collected open pages before closed pages, preserved the
strict `{raw_count,items}` projection, and failed closed on page/parser/repository
errors, repeated issues, and cursor overflow. The final focused command is rerun
in the verification section after formatting and lint checks.

### Verification

- `cargo test -p autospec-cli --bin autospec commands::autonomous::tier15` — 9 passed.
- `cargo test -p autospec-cli --bin autospec` — 24 passed.
- `cargo fmt --check` — passed.
- `cargo clippy -p autospec-cli --bin autospec -- -D warnings` — passed.
- `git diff --check` — passed.
- Authority scan — no queue, claim, safety, shell, GraphQL, or GitHub-write authority; the
  only process launch is `gh api --method GET` in the narrow fetcher.

`cargo clippy -p autospec-cli --all-targets -- -D warnings` remains blocked by two
pre-existing `HEAD` diagnostics outside Task 1: `clippy::len_zero` in
`crates/autospec-cli/tests/explore_commands.rs:228`, and
`clippy::items_after_test_module` in `crates/autospec-cli/src/commands/autonomous.rs:3709`.

## Task 1 review corrections

### RED

`cargo test -p autospec-cli --bin autospec commands::autonomous::tier15`

Failed before `tier15/strict.rs` existed: the byte parser and the source test's
included validator source were unavailable. The same run also reported concurrent
Task 2 waterfall visibility errors, which were outside this slice.

`cargo test -p autospec-cli --bin autospec commands::autonomous::tier15::tests::projected_page_accepts_github_rest_lowercase_state`

Failed with `state must be OPEN or CLOSED`, proving that the initial strict
validator incorrectly rejected valid lowercase GitHub REST state.

### GREEN

`cargo test -p autospec-cli --bin autospec commands::autonomous::tier15`

Passed 13 tests after adding strict UTF-8 decoding, exact projected-page field and
type validation, case-insensitive REST state validation, stronger repository
component rules, and the narrow source-authority test.

The focused run emitted only the concurrent Task 2 unused-method warning from
`waterfall.rs`; no Task 1 warning was emitted.

## Task 2 — sealed Tier 1.5 receipts and replay

### RED

`cargo test -p autospec-cli --bin autospec
commands::autonomous::waterfall_tests::store_seals_tier_one_point_five_observation_and_failure_evidence
-- --nocapture` failed before the shared Tier 1.5 artifact type and persistence
method existed.

`cargo test -p autospec-core --test autonomous_tier15
observer_exposes_immutable_funnel_snapshot_counts -- --nocapture` failed before
the pure observer exposed the three non-mutating snapshot counts required for a
monotonic receipt funnel.

`cargo test -p autospec-cli --bin autospec commands::autonomous::tier15_receipts
-- --nocapture` failed before `record_tier15` and `Tier15Progress` existed.

### GREEN

- The shared evidence boundary persists and verifies immutable Tier 1 and Tier
  1.5 artifacts, including `observation.json` and `read-failure.json`.
- The receipt-only coordinator retains produced and failed scans at the
  Tier1_5 cursor, advances exhausted scans only after a sealed receipt, and
  replays a sealed exhausted receipt after a pre-cursor restart.
- The coordinator has no foreground, queue, claim, promotion, why-no-work, or
  GitHub write authority; the sole process boundary remains the Task 1 reader.
- `cargo test -p autospec-cli --bin autospec commands::autonomous::tier15_receipts
  -- --nocapture` — 5 passed.
- `cargo test -p autospec-core --test autonomous_tier15
  observer_exposes_immutable_funnel_snapshot_counts -- --nocapture` — passed.
- `cargo test -p autospec-cli --bin autospec` — 36 passed.
- `cargo test -p autospec-core` — passed.
- `cargo fmt --check` — passed.
- `cargo clippy -p autospec-cli --bin autospec -- -D warnings` — passed.
- `cargo clippy -p autospec-core --all-targets -- -D warnings` — passed.

The broad CLI all-target lint remains blocked by the same unrelated HEAD
diagnostics noted above; the affected files are outside Task 4B scope.

## Review correction — snapshot consistency

- **RED:** `cargo test -p autospec-cli --bin autospec
  commands::autonomous::tier15::tests::issue_present_in_open_and_closed_snapshots_fails_closed
  -- --nocapture` initially accepted the conflicting open/closed number as a
  complete mixed-time observation.
- **GREEN:** the same command passes after rejecting any cross-state number
  overlap before `observe_tier15` receives the snapshots.
