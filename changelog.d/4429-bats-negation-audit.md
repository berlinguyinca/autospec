# #4429 — make every mid-body negated Bats assertion real

`scripts/lint-bats-negations.sh` reported 116 mid-body negated assertions
across 57 `.bats` files. A `! cmd` that is not the final statement of its
`@test` block is a silent no-op under `set -e` (POSIX ignores `-e` for the `!`
reserved word) and Bats derives the result from the body's last command — so
none of those 116 assertions could ever fail. All 116 sites are now
observable `if cmd; then false; fi` checks, and the allowlist is reseeded to
zero.

The audit found two product defects the no-ops had been hiding:

- **`scripts/lib/autospec-loop.sh` (conductor):** the spend-ledger `add` was
  called on every non-dry cycle, including empty-queue dry cycles. A zero add
  still rewrites the ledger (lock + atomic write + `updated_at`). The dry-cycle
  contract is "no drain AND no spend increment" — the add is now skipped when
  the cycle filed no issues and did no work. The `check` still runs every
  cycle so caps can park.
- **`tests/unit/test_github_publishing.bats`:** the exact-title-fallback test
  asserted "no `issue create` at all", but the fixture's second issue has no
  matching remote title and legitimately creates one. The assertion now checks
  the specific title and verifies the linked number (88) in the ledger.

The five `kill -0`-based liveness assertions were rewritten to predicates that
treat an unreaped zombie as dead (read the state from `/proc/<pid>/stat`, or
reap via `wait` where the test is the parent).

**`ci-wait-poll.sh` shares the defect the issue was written to catch.** It
decides `pending` vs `died` with `kill -0` on the poller's PID, and
`kill -0` succeeds against a zombie — a SIGKILLed poller lingers as one until
its reparented parent reaps it, so the reader reported `pending` for a process
that is gone. Liveness is now judged by the `/proc` state on Linux (Z or a
missing entry = died) with a `kill -0` fallback elsewhere, and AC4 — which was
`skip`ped pending this issue — runs again with both preconditions made
observable and zombie-aware.

The ratchet itself is fixed where it over-reached: a `!` inside a quoted awk
program (data, not a statement) and a `find ... ! -name` predicate on a
continuation line are no longer counted as sites.
