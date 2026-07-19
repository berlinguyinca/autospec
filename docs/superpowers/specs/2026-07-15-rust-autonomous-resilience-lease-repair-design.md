# Rust Autonomous Resilience Lease Repair Design

## Purpose

Close the final review gaps in the Rust resilience cutover: reserve one local
conductor before launch, keep unreadable state distinct from invalid state,
and make status and explicit budget input fail closed.

## Decision

Use a Rust-only, no-dependency transaction gate at canonical
`autonomous/owner__repo/conductor.lease.lock`. On Unix, a short non-blocking
exclusive `flock` serializes read, policy evaluation, and canonical state
replacement. `state.json` remains the durable compatibility record and gains
an opaque `lease_token` plus monotonically increasing `lease_generation` for
Rust-owned records.

An acquisition writes a fresh `claimed` record while the transaction gate is
held. `start` passes the token to its native foreground child only through the
child environment. Before any foreground mutation, that child atomically
adopts the matching token and its own PID. If a crashed launcher is reclaimed
first, its delayed child cannot adopt the replaced token and exits before
dispatch; this fences the check-then-launch race. Direct foreground runs
acquire and release their own token. `restart` never signals an existing unit
until it has acquired a fresh lease; a fresh lease parks the restart.

The gate is a local/shared-filesystem lease, not a distributed lock. Existing
GitHub claim ownership remains the remote mutation arbiter. Unix is the
supported local runtime; an unsupported platform returns a diagnostic rather
than weakening safety.

## Error and Compatibility Rules

- Successfully read but malformed/foreign records remain stable JSON rejects
  with exit 3.
- Filesystem reads or canonical migration/acquisition writes that fail for an
  I/O reason remain diagnostics with exit 2 and no decision JSON.
- Status uses the same required `repo` identity validation as admission; a
  repository-less record is malformed and a mismatched record is foreign. Its
  reported spend is the same scoped `AUTOSPEC_AUTONOMOUS_SPEND_DIR/<owner__repo>/spend.json`
  ledger used for admission, never the retired global spend file.
- Omitted lifetime limits retain environment/default behavior. A supplied
  empty, negative, or non-numeric `--budget-tokens` or `--budget-issues`
  value is a diagnostic and never falls back to defaults. Explicit zero is
  valid and disables only that cap.
- Legacy records without a lease token are valid evidence: a fresh record is
  held and a reclaimable one may be replaced only inside the transaction.

## Tests

Black-box tests must prove one of two competing foreground/start operations
owns the lease while the other exits held before local or GitHub mutation;
adoption must fence a delayed child after a replacement. A token-bearing child
must release its matching lease on every post-launch terminal path, including a
persisted-stop decision and an admission diagnostic, after lifecycle evidence
has been persisted when applicable. They must also prove I/O diagnostics are
not malformed JSON rejects, status validates its record scope and reports its
scoped spend ledger, and explicitly blank lifetime flags fail before operator
state exists.

## Non-goals

This repair adds no shell fallback, dependency, remote lock, generic force
takeover, or new public operator flag. It does not delete the remaining
waterfall/drain shell work; it establishes the atomic Rust resilience boundary
required before that deletion.
