# Deferred Runtime Readiness

**Issue:** #2808

## Problem

Autospec direct runtime modes currently require the brokered frontend port to bind
within five two-second attempts. That contract is correct for modes whose setup
command starts a server, but it rejects environment-only session modes whose child
command starts the server later. The failed setup is persisted as
`CleanupFailed`; every later session then refuses reconciliation.

The `lc-binbase-scheduler` runtime manifest exposes this compatibility break. Its
setup command succeeds without binding because Playwright starts the Angular
server later using the brokered environment. Codex is wrapped by
`autospec-env session`, so the runtime failure prevents Codex from starting.

## Design

Add a typed `RuntimeReadiness` policy to each `RuntimeMode`:

- `bound` remains the default and preserves the existing frontend bind health
  check.
- `deferred` runs the setup command once and marks the environment active after
  the command succeeds, without probing the frontend port.

Both manifest versions accept `readiness: bound|deferred` as a mode field and
reject every other value. The selected mode carries the parsed enum through
`RuntimeContext`; no environment variable or string comparison controls runtime
behavior.

`CleanupFailed` remains fail-closed, including when the structured inventory is
empty. Version-1 setup and teardown commands can own processes that are not
represented in the inventory, so empty structured state is not sufficient proof
that automatic removal is safe. Operators recover existing tombstones through
the authenticated `runtime env down` path, which validates identity and runs the
declared teardown command.

## Error Handling

`bound` modes retain `PORT_BIND_HEALTH_RETRIES_EXHAUSTED` and
`CleanupFailed` on an unbound frontend port. A failed `deferred` setup command
also retains `CleanupFailed`, preserving explicit teardown recovery. Unsupported
readiness values fail during manifest parsing before any runtime state or child
process is created.

## Compatibility

Existing manifests are unchanged because omitted readiness resolves to `bound`.
Environment-only profiles must opt into `deferred`; Autospec does not infer
intent from commands such as `true`.

The scheduler manifest will add `readiness: deferred` to its environment-only
modes after the Autospec binary containing this parser is installed.

## Verification

- Unit tests cover version-1 and version-2 parsing, the default, and invalid
  readiness values.
- Runtime integration tests prove deferred setup runs once and reaches `Active`
  without a listener.
- Reconciliation tests prove empty `CleanupFailed` state remains blocked without
  rerunning setup.
- An isolated two-start reproduction uses the scheduler manifest and a temporary
  state root.
- `cargo test --workspace` plus repository validation scripts remain green.

## Rejected Alternatives

- Removing the bind check globally weakens readiness guarantees for real direct
  servers.
- Treating `sh -c 'true'` specially encodes command-text heuristics instead of a
  manifest contract.
- Clearing the scheduler tombstone alone only reproduces the ten-second failure.
- Automatically reconciling resource-empty `CleanupFailed` state can orphan
  processes owned by version-1 mode commands.
