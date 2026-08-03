# Stale Successor Preserved-Branch Recovery Plan

**Goal:** Requeue a stale heartbeat-pending successor over exact expired prior-generation evidence without deleting its branch.

**Architecture:** Add a descriptor-relative read-only prior-generation proof before the branch guard, then reuse the existing quarantine and claim-ref CAS transaction.

## Constraints

- Preserve the branch and all WIP.
- Keep current-generation, live, foreign, malformed, and unsafe heartbeat evidence blocking.
- Do not add dependencies or weaken the stale lease timeout.

## Task 1: Lock the live failure with an integration regression

**Files:** `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

- [ ] Seed a preserved issue branch in `foreground_reclaims_stale_heartbeat_pending_before_acquire`.
- [ ] Confirm the current branch guard returns `claim_lost`.
- [ ] Record the branch OID and prove recovery leaves it unchanged.

## Task 2: Admit only exact expired prior-generation evidence

**Files:** `crates/autospec-cli/src/commands/claim.rs`

- [ ] Inspect the heartbeat root and repository descriptor-relatively.
- [ ] Match repo, issue, branch, empty PR, and a different worker or claim generation.
- [ ] Require the prior heartbeat process to be expired and dead.
- [ ] Bypass only the preserved-branch guard, then reuse quarantine and CAS.

## Task 3: Verify and publish

- [ ] Run focused tests, `cargo test --workspace -- --test-threads=1`, Clippy, implementation lint, and `autospec validate`.
- [ ] Obtain independent LGTM, open a PR closing #2881, merge, reinstall, and retry #2751 live.
