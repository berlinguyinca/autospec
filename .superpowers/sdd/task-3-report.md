# Task 3 — Native foreground Tier 1 coordinator

## Status

Complete. The independent Tier-1 re-review found no Critical or Important
correctness defects.

## Result

The Rust foreground conductor now intercepts a repository-wide empty ready-queue
snapshot before its terminal `ScanEmpty` transition. It writes an immutable
Tier-1 receipt and a SHA-256-bound queue-evidence artifact, then advances the
waterfall cursor to Tier 1.5 without acquiring or releasing the held conductor
lease. Slice-empty, active-claim, worker-cap, and batch-empty observations do
not create a waterfall pass.

## TDD and verification evidence

- **RED:** the initial repository-empty foreground regression expected `Scan`
  but received the terminal `AllDone` state before the coordinator was added.
- **RED:** the Tier-1 evidence tamper regression initially accepted a cursor
  after its sealed evidence bytes changed; state loading now rejects it.
- `cargo test -p autospec-cli` passed.
- `cargo test -p autospec-core` passed.
- `cargo fmt --check` passed.
- `cargo clippy -p autospec-cli -- -D warnings` passed.
- `git diff --check` passed.

## Files

- `crates/autospec-cli/src/commands/autonomous.rs`
- `crates/autospec-cli/src/commands/autonomous/waterfall_coordinator.rs`
- `crates/autospec-cli/src/commands/autonomous/waterfall.rs`
- `crates/autospec-cli/src/commands/autonomous/waterfall_tests.rs`
- `crates/autospec-cli/tests/autonomous_conductor_commands.rs`
- `crates/autospec-core/src/autonomous/waterfall.rs`

## Behavioral proof

- The empty repository path makes one ready-queue query and retains the
  `Scan` conductor state while Tier 1.5 is pending.
- A sealed exhausted receipt is replayed before fresh queue evidence is read;
  after an interrupted cursor write it advances the same pass idempotently.
- A queue-read error seals a failed receipt and does not advance the cursor.
- Evidence is persisted beneath the run-scoped waterfall directory and its
  SHA-256 digest is verified when recovering a Tier-1 state.
- Source-authority and integration tests prove this path has no shell,
  GitHub claim/edit/comment, `NoWorkState::record`, or `why-no-work.json`
  authority.

## Remaining risk

Tier 1.5 through Tier 4 execution remains intentionally pending; this task
only makes the native Tier-1 handoff durable and replay-safe. The integrity
seam added to `WaterfallStore` is narrow because Tier-1 typed queue evidence
must survive an interrupted foreground process without trusting changed bytes.
