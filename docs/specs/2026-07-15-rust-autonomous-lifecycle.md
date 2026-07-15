# Rust autonomous lifecycle design

## Goal

Move autonomous lifecycle and waterfall policy from stateful shell control flow into a typed Rust contract without changing the public autonomous operator commands.

## Decision

`autospec-core` gains a pure `autonomous_lifecycle` module. Its inputs model repository scope, claim ownership, lease age, stop mode, health, retry state, budget state, and discovery state with closed Rust enums and structs. Its sole policy entry point returns one serializable decision; it performs no filesystem, process, GitHub, `omx`, or shell work.

The CLI persists the lifecycle contract atomically and maps existing operator commands onto it. It owns all side effects behind typed adapter results: reading compatibility state, starting or stopping native processes, querying health, and writing state. `autonomous lifecycle decide` exposes the pure decision as JSON for integration tests and diagnostics.

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

During cutover the CLI reads both legacy repository layouts: the `owner__repo` resilience layout and the single-underscore operator layout. It writes one versioned canonical contract, so an existing heartbeat, lease, or stop sentinel is not lost. Lease acquisition must be atomic; the legacy read-then-overwrite lock race is not preserved.

The legacy 300-second and 10,800-second stale thresholds, per-issue failure cap of three, scoped stop semantics, and the current budget accounting unit remain explicit inputs. Post-merge rollback is a typed action, but is not claimed to be an automatically invoked legacy-loop behavior.

## Errors and exits

The decision endpoint emits JSON. Invalid options or invalid typed values exit with the existing malformed-input class. Ownership, stale, and terminal state are non-executable ownership outcomes. A stop, health, budget, or human gate emits its specific decision rather than falling through to a shell fallback.

## Testing

Core tests cover every decision tier, precedence, stale and cross-scope rejection, failure-cap escalation, health, budget, stop, and idle-rescan behavior. CLI integration tests cover JSON serialization, malformed values, command compatibility, and the absence of a `sh -c` fallback on the new lifecycle path. Existing foreground executor-result coverage remains unchanged.

## Out of scope

This slice does not delete legacy scripts, installers, Bats fixtures, or documentation references. Those deletions occur only after the typed lifecycle has integration coverage and native foreground dispatch consumes it. External services such as GitHub, `omx`, premerge checks, spend ledgers, control channels, and process APIs remain adapters outside the core policy.
