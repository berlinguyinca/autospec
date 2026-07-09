# Autospec Observatory Outbox

Autospec emits structured observatory events to a local durable outbox before any
network upload is attempted. The MVP helper is `scripts/autospec-observatory-events.sh`.

## Files

- `.autospec/observatory/outbox/<run-id>.jsonl` — append-only JSONL events, ordered
  by the per-run `sequence` field.
- `.autospec/observatory/checkpoints.json` — per-run checkpoint with
  `last_sequence`, `next_sequence`, upload status, retry count, backoff seconds,
  and `next_retry_at`.

## Offline-safe operation

Set `AUTOSPEC_OBSERVATORY_OFFLINE=1` to force local-only behavior. `flush` reports
`STATUS:offline`, leaves events queued in the outbox, and exits zero so autospec
implementation work is not blocked by observatory availability.

Without `AUTOSPEC_OBSERVATORY_URL`, or when the configured endpoint cannot be
reached, `flush` records `upload_status=queued` with exponential retry backoff in
`checkpoints.json`. Upload is best-effort; raw logs are not uploaded by default.

## Smoke test

```bash
AUTOSPEC_OBSERVATORY_OFFLINE=1 bash tests/observatory-outbox.bats
```

The smoke test uses real temporary `.autospec/observatory` files and verifies that
`RunStarted` and `WorkerHeartbeat` serialize with monotonically increasing
`sequence` values.
