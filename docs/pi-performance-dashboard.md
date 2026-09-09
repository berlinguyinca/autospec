# pi-performance-dashboard.sh

Operator-facing dashboard for the local-model fleet (issue #3327). It replays a
fixed execution ledger (`--ledger <jsonl>`) and an optional in-flight live
record (`--live <json>`), then prints the live card, the historical summary,
and the routing advice. The record shapes, the nearest-rank percentile rule,
and the Wilson success lower bound stay in lockstep with
`autospec_core::aar::dashboard` (unit tests: `tests/aar_dashboard.rs`,
shell tests: `tests/pi-performance-dashboard.bats`).

## Options

| Option | Default | Meaning |
|---|---|---|
| `--ledger <jsonl>` | required | Completed-issue ledger, one JSON record per line. |
| `--live <json>` | none | One in-flight work item (all 14 keys required). |
| `--window-hours <n>` | `24` | Throughput window for `successful_issues_per_hour`. |
| `--min-samples <n>` | `20` | Below this, routing advice stays on the static profile. |
| `--static-profile <name>` | fleet default | Profile kept while no benchmark profile is eligible. |
| `--model-family <family>` | `qwen3.8` | Locked family; the advice never reports another family. |
| `--now <epoch-seconds>` | current time | "Now" for liveness; pass a fixed value for reproducible output. |
| `--liveness-threshold-minutes <n>` | `5` | Silence up to this still counts as progress (issue #3723). |

## Liveness (issue #3723)

A keystone run must let a supervisor distinguish "working" from "stuck" in one
command. The live record therefore carries two timestamps (epoch ms):
`started_ms` (run start) and `last_heartbeat_ms` (last unbuffered heartbeat
line). The live card then answers:

- `liveness: progressing` — the heartbeat is no older than
  `--liveness-threshold-minutes` (the boundary counts as progress).
- `liveness: no output for N minutes` — `N` is the floor of the silent
  milliseconds over 60 000.
- `elapsed_ratio: <x>` — the run's elapsed time as a multiple of the ledger's
  mean completed-issue duration (`mean_ms` in the history section), rounded to
  two decimals; `n/a` until the ledger has history.

The same rule lives in `autospec_core::aar::dashboard` as `Liveness::assess`
with `DEFAULT_LIVENESS_THRESHOLD_MS`, plus `elapsed_ratio` and
`HistorySummary::mean_duration_ms` — the shell and the core must not drift.

## Exit codes

`0` ok; `1` bad arguments or invalid ledger/live record (fail-closed, nothing
is printed); `2` `jq` missing.
