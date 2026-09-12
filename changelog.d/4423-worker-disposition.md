### Added

- `autospec-core::worker_disposition` — a vanished worker is classified
  from scheduler terminal evidence, never from absence: `sacct` `State` is
  mapped to `Cancelled` (with the cancelling uid when the reason records
  one) / `Completed` / `Crashed`, and when `sacct` is unavailable or the
  state does not classify the disposition is `Unknown` — rendered "worker
  disappeared; cause unknown", never a crash. This is the gap the gateway
  in issue #4423 showed when it logged `"worker crashed (no walltime
  deadline known)"` for five jobs `sacct` said were `CANCELLED by 298907`.
  Every cancellation is a `CancelNotice` logging who/what/why to the shared
  cancellation file before `scancel`, and `DispositionCounters` keeps
  `cancelled` apart from `crashed` in telemetry so the difference is visible
  without reading logs (#4423, 2026-09-12).
