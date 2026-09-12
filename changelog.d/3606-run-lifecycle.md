### Added

- `autospec-core::run_lifecycle` — primitives for the invariant that a
  run is visible in every state it was in: the status record is
  written at start (`status=RUNNING` with the endpoint and model) and
  updated on every transition, so a killed run leaves a record of
  where it was; a run with no status record is FAILED in every tally
  (the denominator is the dispatched runs, never the status files that
  exist); liveness is a periodic heartbeat line with a timestamp and a
  work counter, flushed unbuffered, so a progressing run is
  distinguishable from a stopped one without inspecting anything
  outside the run's own output (a running record with no line at all
  is `LIVENESS_UNRECORDED`); an endpoint unreachable at start costs
  seconds, not a scheduled slot (`audit_dispatch`, `classify_loss`);
  a lost connection is retried against a different pool member from a
  fresh read of the endpoint directory (`reselect`,
  `same_endpoint_retries`); and the session transcript is copied into
  the shared output directory whenever the run is about to be recorded
  no-output, stall-killed or non-zero (`transcript_policy`,
  `transcript_verdict`), with `finish_reason` and the final
  response's token counts as fields in the status file and the four
  no-output causes separable from it (`classify_no_output`,
  `NoOutputCause::retry_worthy`) (#3606, 2026-09-11).
