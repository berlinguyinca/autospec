# Never report a cause you have not established, and log every destructive action with its origin (issue #4423)

Five workers were cancelled simultaneously and the gateway recorded all five
as **crashed** — `{"level":"ERROR","msg":"worker crashed (no walltime deadline
known)","job_id":"23026707"}` — while `sacct` said `23026707|CANCELLED by
298907|0:0|00:09:11`. uid 298907 is `gw`, our own account; our automation
cancelled them nine minutes into their life across four models. The message
even admits the gap ("no walltime deadline known") and then asserts the
stronger claim anyway. A cancelled worker and a crashed worker call for
opposite responses — one is the fleet doing something deliberate, the other is
a fault to investigate — and recording both as ERROR/crashed inflated the
error count to 28 with no real faults behind it, so the log could support
neither decision.

Attribution was impossible from the logs kept: `agent-watchdog` logged
`hung_killed=0` across the window, `reconcile-cron.sh` carried `--no-trim`,
`slurm_canceler.sh` was request-driven and last acted Sep 1,
`reconcile-workers.sh` had no `LOG=` set, and `hold-loop.sh` contained no
`scancel`. Every script that could cancel a worker could do so with no record,
so a five-worker loss was unattributable until an unrelated error scan.

- **A vanished worker is classified from scheduler terminal evidence, never
  from absence.** `sacct -j <id> --format=State,ExitCode` answers
  `CANCELLED` vs `COMPLETED` vs `FAILED` definitively and is one call away
  (`classify`). Where it is unavailable the message says "worker disappeared;
  cause unknown" (`Disposition::Unknown`), never naming a cause that has not
  been established. The incident message asserted `crashed` while admitting it
  held no walltime deadline — the exact failure the fail-closed `Unknown`
  prevents.
- **Every cancellation logs who/what/why to a single file before the call.**
  A `CancelNotice` carries `origin` (who), `job_id` (what) and `reason` (why),
  and its `log_line` is written to the shared cancellation file before
  `scancel` (`reconcile-workers.sh` with no `LOG=` was the unattributable
  cancel site). The destructive action is recorded before it is issued, never
  after.
- **Telemetry distinguishes cancelled from crashed.** `DispositionCounters`
  keeps `cancelled` apart from `crashed` (and `completed` / `unknown`) and
  renders them on one line, so a deliberate fleet action and a fault are
  different numbers without reading logs — the 28-error crash count with no
  faults behind it is the shape this counter makes impossible.

Checkable in `autospec_core::worker_disposition` (`classify`, `Disposition`,
`CancelNotice`, `log_line`, `DispositionCounters`, `record`, `line` — pure
in-memory, so the gateway's admit/drain paths and the fleet scripts can adopt
it as the single source of truth). Tests:
`crates/autospec-core/tests/worker_disposition.rs`, including the regression
that reconstructs the incident end-to-end: a `CANCELLED by 298907` row
classified as a cancellation, not a crash; a vanished worker with no evidence
reported "cause unknown", not crashed; and counters that keep the two apart.
