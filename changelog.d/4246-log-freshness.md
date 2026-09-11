### Added

- `autospec_core::log_freshness`: a log's last line is only "now" if its
  mtime says so (issue #4246). Encodes the incident's rules as checkable
  primitives: `SweepHit` (a match cannot exist without the file's mtime),
  `freshness`/`hit_line` (every error-sweep match renders with its file's
  age alongside, plainly flagged `STALE` when the file is older than the
  investigation window), `sweep_verdict` (a sweep whose every match comes
  from a stale file is evidence about the past, not the present),
  `outage_grounding` (state beats history whenever state is queryable — a
  log-grounded outage claim is refused when the 15-second live query
  exists), `authoritative_log` (two logs for one component: the fresh one
  is the present; the stale error-only log is not), and
  `error_log_standing`/`monument_finding` (a stale error-only log with
  neither per-line timestamps nor rotation is a `Monument` — a permanent
  record of one bad afternoon). Regression tests reconstruct the incident:
  the three-day-old `cron-regsweep.log` (191 bytes, two error lines, mtime
  Sep 7 09:45), the two-minute-old `regsweep.log` (last line `sweep done:
  pool=10 gateway=10`), and the gateway reporting 10 registered workers.
