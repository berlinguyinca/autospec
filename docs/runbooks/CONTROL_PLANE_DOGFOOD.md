# Control Plane Dogfood Runbook

This runbook proves the MVP control-plane dogfood criterion from
`docs/specs/2026-07-08-autospec-sovereign-control-plane-design.md`: a run against
`berlinguyinca/autospec` produces both a run timeline and a cost report through
the local observatory event flow.

The dogfood path is intentionally safe by default:

- companion repositories are bootstrapped with `scripts/autospec-control-plane.sh bootstrap --dry-run`;
- autospec/observatory events are written to the durable local outbox before upload;
- offline replay is the default so the smoke test never needs GitHub, Postgres, or a running web UI;
- online replay is opt-in with `--online --observatory-url URL`.

## Artifacts

Each run writes an artifact directory, defaulting to
`.autospec/control-plane-dogfood/<run-id>/`, with these files:

| Artifact | Purpose |
| --- | --- |
| `companion-bootstrap.txt` | Dry-run scaffold for `autospec-governance` and `autospec-observatory`. |
| `outbox.jsonl` | Replayed copy of `.autospec/observatory/outbox/<run-id>.jsonl`. |
| `replay.log` | Offline/online replay status from `scripts/autospec-observatory-events.sh flush`. |
| `timeline.json` | Ordered run timeline derived from outbox events. |
| `cost-report.json` | Cost/duration/outcome report derived from outbox events. |
| `manifest.json` | Run metadata and paths to all evidence artifacts. |

The two operator-facing proof artifacts are `timeline.json` and
`cost-report.json`. Preserve both when attaching evidence to an issue, PR, or
observatory run record.

## Offline smoke

Run the focused smoke test:

```bash
bash tests/control-plane-dogfood.bats
```

Run the dogfood script directly:

```bash
bash scripts/dogfood-control-plane.sh \
  --offline \
  --run-id control-plane-dogfood-local \
  --output-dir .autospec/control-plane-dogfood/control-plane-dogfood-local
```

Expected terminal output includes:

```text
STATUS:offline run_id=control-plane-dogfood-local pending_events=5
replay_mode=full
timeline_artifact=.autospec/control-plane-dogfood/control-plane-dogfood-local/timeline.json
cost_artifact=.autospec/control-plane-dogfood/control-plane-dogfood-local/cost-report.json
```

Validate the data shape:

```bash
jq -e '.run_id == "control-plane-dogfood-local" and (.events | length) >= 5' \
  .autospec/control-plane-dogfood/control-plane-dogfood-local/timeline.json
jq -e '.run_id == "control-plane-dogfood-local" and .total_events >= 5 and .cost_events >= 1' \
  .autospec/control-plane-dogfood/control-plane-dogfood-local/cost-report.json
```

## Offline replay-only check

Use replay-only mode when an outbox already exists and you only need to rebuild
report artifacts after changing report logic:

```bash
bash scripts/dogfood-control-plane.sh \
  --offline \
  --replay-only \
  --run-id control-plane-dogfood-local \
  --output-dir .autospec/control-plane-dogfood/control-plane-dogfood-local
```

This reuses `.autospec/observatory/outbox/control-plane-dogfood-local.jsonl`,
runs the same flush/replay step, and rewrites `timeline.json`,
`cost-report.json`, and `manifest.json`.

## Online operator verification

After the observatory stack is running locally, replay the same flow online:

```bash
AUTOSPEC_OBSERVATORY_URL=http://127.0.0.1:3000 \
  bash scripts/dogfood-control-plane.sh \
    --online \
    --run-id control-plane-dogfood-online \
    --output-dir .autospec/control-plane-dogfood/control-plane-dogfood-online
```

If upload succeeds, `replay.log` contains `STATUS:uploaded`. If the service is
not reachable, the helper leaves the outbox intact and records `STATUS:queued`,
which is still valid evidence that autospec continued working while the
observatory was unavailable.

Operator verification for the full local stack should record:

1. the command used to start `autospec-observatory`;
2. `manifest.json` from this script;
3. the web/API view or export that corresponds to `timeline.json`;
4. the web/API view or export that corresponds to `cost-report.json`;
5. the `replay.log` upload status.

## Full repository validation

Run the repository gate after changing the dogfood script or docs:

```bash
autospec validate
```

The focused dogfood smoke remains:

```bash
bash tests/control-plane-dogfood.bats
```

## Future-version boundary

This runbook checks only MVP data-shape compatibility: companion bootstrap,
offline/online outbox replay, timeline evidence, and cost evidence. Production
daemon migration, SaaS tenancy, richer analytics, and future-version control
plane capabilities remain outside this dogfood script's scope.
