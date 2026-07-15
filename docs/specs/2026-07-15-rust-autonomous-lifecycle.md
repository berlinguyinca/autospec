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

## Errors and exits

The decision endpoint emits JSON. Invalid options or invalid typed values exit with the existing malformed-input class. Ownership, stale, and terminal state are non-executable ownership outcomes. A stop, health, budget, or human gate emits its specific decision rather than falling through to a shell fallback.

## Testing

Core tests cover every decision tier, precedence, typed claim identity, both lease-expiry classes, stale and cross-scope rejection, failure-cap escalation, health, budget, stop, and idle-rescan behavior. CLI integration tests cover JSON serialization, malformed values, command compatibility, health parks, stop-before-health behavior, and the absence of a `sh -c` fallback on the new lifecycle path. Existing foreground executor-result coverage remains unchanged.

## Out of scope

This slice does not delete legacy scripts, installers, Bats fixtures, or documentation references. Those deletions occur only after the typed lifecycle has integration coverage and native foreground dispatch consumes it. External services such as GitHub, `omx`, premerge checks, spend ledgers, control channels, and process APIs remain adapters outside the core policy.
