### Added

- A stateless reconciler no longer submits into the same wall forever: `fleet_dispatch` now
  carries per-claim reconciler memory (`ReconcilerMemory`, `StartOutcome`, `SuspendedClaim`).
  Consecutive start failures are tracked per claim; after a threshold (default 3) the claim is
  suspended with the last failure reason recorded, and the "never converged" state is rendered
  distinctly from "converging" — `want=1 up=0 (N consecutive start failures, last: <reason>)`
  instead of `want=1 up=0` on every attempt. The per-claim rate threshold is paired with the
  existing per-attempt cost floor, so cheap repeated failures (a worker that dies in under a
  second, a hundred times) now accumulate into a suspension instead of passing silently
  (#4211).
