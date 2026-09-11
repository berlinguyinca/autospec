### Added

- `autospec-core::circuit_breaker` — per-worker outcome state and a
  circuit breaker for the gateway, for the invariant that a detector must
  write to state a decision can read: the worker deadlocked, `/health`
  kept returning 200, and the gateway logged the probe timeout 237 times
  while routing to it for hours, because the signals had nowhere to go.
  Liveness is the last successful completion of real work — there is no
  method that records a health response (`WorkerOutcome::last_success`,
  `is_live`); probe timeouts feed the same consecutive-failure counters
  the routing decision reads (`probe_timeout`); "busy is not dead" gets a
  bound next to the rule (`BreakerConfig::stuck_timeout`) and the
  discriminator is progress — a token since the last check — not
  wall-clock, so a two-hour request emitting tokens never opens
  (`WorkerOutcome::stuck`); a failure against healthy same-model peers
  opens at the smaller peer threshold, not the absolute one
  (`Fleet::tick`); the open circuit is reported with its age and reopen
  time rather than a silently dropped worker that stays registered
  (`RoutingDecision::Excluded`, `Fleet::routing_report`); and the report
  is per worker — one line each with that worker's own counters — so a
  fleet that looked healthy in aggregate while one model sat at 0%
  success cannot hide it again (`Fleet::summary_lines`). Half-open
  admits exactly one probe: success closes, failure re-opens with a
  doubled back-off capped at `backoff_max`, and
  `WorkerOutcome::snapshot` / `restore` keep "how long has this been
  bad" across a restart instead of resetting the picture to optimistic
  (#4378, 2026-09-11).
