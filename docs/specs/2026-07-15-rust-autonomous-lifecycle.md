# Rust autonomous lifecycle design

## Goal

Move autonomous lifecycle and waterfall policy from stateful shell control flow into a typed Rust contract without changing the public autonomous operator commands.

## Decision

`autospec-core` gains a pure `autonomous_lifecycle` module. Its inputs model repository scope, claim ownership, lease age, stop mode, health, retry state, budget state, and discovery state with closed Rust enums and structs. Its sole policy entry point returns one serializable decision; it performs no filesystem, process, GitHub, `omx`, or shell work.

The CLI parses the schema-1 lifecycle record back into the same typed contract and maps existing operator commands onto it. It owns all side effects behind typed adapter results: reading compatibility state, starting or stopping native processes, querying health, reading an observed claim, and writing state. A malformed or foreign observed run-state is a typed non-executable rejection, never an implicit unclaimed state. `autonomous lifecycle decide` exposes the pure decision as JSON for integration tests and diagnostics.

## Decision precedence

The policy evaluates exactly one outcome in this order:

1. Stop or park requests take precedence and prevent executable work.
2. Human-gated or value-queue work returns its explicit escalation outcome.
3. A ready Tier 1 item is selected when a fresh, matching lease and worker capacity exist.
4. Tier 1.5 promotion follows an empty Tier 1 result.
5. Tiers 2, 3, and 4 represent discovery work.
6. Tiers 5, 6, and 7 represent growth work.
7. An otherwise healthy, uncapped cycle returns `idle` with a rescan action; it never silently converges to permanent park.

Malformed input, scope mismatch, stale lease, terminal claim, or ownership mismatch returns a non-executable rejection. Budget, health, and stop outcomes are explicit and machine readable.

## Runtime and compatibility boundary

The module does not preserve shell command strings. `autonomous.rs` remains the side-effect boundary and starts native foreground operation directly. It must preserve process-group termination behavior and make `start`, `restart`, `stop`, `status`, and `run-foreground` consume the same lifecycle contract.

This first cutover writes and reads schema-1 lifecycle decisions in the existing single-underscore operator layout and removes Rust command-string launch authority. `start` and `restart` decide before creating state or stopping units; restart is the only typed transition allowed to supersede a stored stop. Foreground reads an active stop before health or queue work and preflights the observed GitHub claim before acquisition. The next resilience cutover must read both that layout and the legacy `owner__repo` state before deleting shell files; it must make lease acquisition atomic rather than preserving the shell read-then-overwrite race.

The typed policy exposes the 300-second stale lease threshold, the 10,800-second abandoned-lease threshold, and the per-issue failure cap of three as explicit constants. Durable compatibility reads, per-issue failure persistence, budget-ledger migration, and automatic rollback invocation remain follow-on work. Post-merge rollback is a typed action, but is not claimed to be an automatically invoked legacy-loop behavior.

## Resilience compatibility contract

`autospec autonomous resilience decide --repo OWNER/REPO` is a Rust-only admission
adapter for the existing resilience records. For `owner/repo`, it reads every compatible
record type in this strict order: `owner__repo`, then `owner_repo`, then `owner-repo`.
The first existing record is authoritative; the adapter does not skip malformed data to
fall through to a later layout. Read-only decisions never migrate compatibility state.
Acquisition, adoption, and release write only the canonical `owner__repo` record; they
never write `owner_repo` or `owner-repo`.

Malformed state, failure, or spend records fail closed with a `reject` JSON decision, as
does a state record bound to a different repository. A rejected admission does not create
a canonical state record. The autonomous operator directory (by default
`.autospec/autonomous-operator/<scope>/`) remains lifecycle-only: it holds lifecycle,
stop, launch, and foreground records, not resilience compatibility state.

Lease boundaries are inclusive. A claimed lease is reclaimable at exactly 300 seconds,
and any lease is abandoned at exactly 10,800 seconds; a missing heartbeat or dead PID on
the same known host is also reclaimable. Capacity checks are inclusive as well: usage at
or above a nonzero token limit yields `usage_cap` before an issue limit is considered;
otherwise issue count at or above a nonzero issue limit yields `issue_cap`. A zero limit
disables that capacity check.

The adapter has no shell resilience authority. It neither invokes
`scripts/autonomous-resilience.sh` nor starts `sh` or `bash`; it reads typed records and
emits one JSON decision for its caller.

## Atomic conductor ownership

Before `start` or `restart` creates an operator directory, writes a lifecycle or launch
record, terminates a unit, or clears a stop flag, it completes read-only repository,
stored-stop, and lifetime-budget validation and takes a non-blocking exclusive Unix
transaction at `autonomous/owner__repo/conductor.lease.lock`. The transaction covers
record read, policy evaluation, and canonical claimed-record replacement. Fresh leases
remain held; in particular, a held lease parks `restart` before it can signal an existing
conductor or clear its stop flag. Capacity and failure-cap policy outcomes return their
existing park or reject decisions without issuing a token or writing a claimed record.

The claimed record carries an opaque lease token and monotonically increasing generation.
`start` and `restart` pass that token to the native `run-foreground` child only through
`AUTOSPEC_CONDUCTOR_LEASE_TOKEN` in `Command::env`; the token is never an argument,
launch JSON field, or log value. If a later child launch fails, Rust terminates any child
it has already started and releases only its still-matching token.

`run-foreground` gives a persisted stop precedence over executable work. A child first
adopts a supplied environment token solely to establish release authority, then releases
that exact token before returning any persisted-stop or pre-admission diagnostic. Otherwise
it atomically adopts its environment token before lifecycle, health, queue, claim, or
foreground-state write; a direct invocation acquires its own token at that same boundary.
A missing, stale, or replaced token exits with the
`conductor_lease_token_mismatch` rejection before local or GitHub mutation. Once owned,
the foreground path rechecks admission at final selection and dispatch using its matching
token, then persists every terminal foreground/lifecycle result before releasing that exact
token. `autonomous status --json` reads spend from the same scoped
`AUTOSPEC_AUTONOMOUS_SPEND_DIR/<owner__repo>/spend.json` ledger as admission. The local
transaction is a shared-filesystem lease, not a remote GitHub lock;
existing GitHub claim ownership remains the remote mutation arbiter.

## Errors and exits

The decision endpoint emits JSON. Invalid options or invalid typed values exit with the existing malformed-input class. Ownership, stale, and terminal state are non-executable ownership outcomes. A stop, health, budget, or human gate emits its specific decision rather than falling through to a shell fallback. Filesystem and transaction failures are diagnostics with exit `2` and no decision JSON; malformed or foreign records and token mismatches are JSON rejects with exit `3`; held leases and parks exit `20`.

## Testing

Core tests cover every decision tier, precedence, typed claim identity, both lease-expiry classes, stale and cross-scope rejection, failure-cap escalation, health, budget, stop, and idle-rescan behavior. CLI integration tests cover JSON serialization, malformed values, command compatibility, health parks, stop-before-health behavior, and the absence of a `sh -c` fallback on the new lifecycle path. Existing foreground executor-result coverage remains unchanged.

## Out of scope

This slice does not delete legacy scripts, installers, Bats fixtures, or documentation references. Those deletions occur only after the typed lifecycle has integration coverage and native foreground dispatch consumes it. External services such as GitHub, `omx`, premerge checks, spend ledgers, control channels, and process APIs remain adapters outside the core policy.
