# Failure signatures over a rolling window

A fleet of agent runs fails in two different ways. One run failing once is
noise. The same run failing sixty times is a systemic problem, and the only
thing that separates the two is a count next to a denominator.

`autospec doctor failures` produces that count. It groups the fleet's failures
into **signatures**, counts each signature over a rolling window of the most
recent runs, and flags every signature that repeats often enough to stop being
an incident and start being a property of the system.

```console
$ autospec doctor failures
62 of 190 runs failed (32.6%) in the last 190 runs of .autospec/runs; 1 systemic signature(s) above the 5.0% threshold.
  count   share  status     signature
     62   32.6%  SYSTEMIC   gcc -c <path>/main.c:<n>: error: cannot allocate <n> bytes
      3    1.6%  below      exit: argument list too long
      2    1.1%  below      <no output>
```

## What counts as a run

A run is one directory under the runs root (`.autospec/runs` by default,
overridable with `--runs-dir`). The runner writes two things into it:

| File | Purpose |
|---|---|
| `status.json` | `{"status": "failed"}`, `{"status": "completed"}`, or `{"exit_code": 137}` |
| `status` | A plain-text alternative: `failed`, `killed`, `timeout`, `cancelled`, `oom`, `completed`, … |
| `stderr.log` | The run's stderr. `stderr.txt`, `stderr`, the first `*.err`, or the first `*.log` are also read. |

Runs are ordered by directory mtime, newest first, and the window takes the
most recent `--last` of them (200 by default). The window is **rolling**: a
signature that burned through the fleet last week and has not appeared in the
current window is not systemic today.

## What a signature is

The signature is the **last meaningful line of stderr**, normalised so that one
defect always produces one signature no matter where it struck:

| Raw | Normalised |
|---|---|
| `/scratch/gw-as-3735-22766006/repo/src/main.c:42:10` | `<path>/main.c:<n>:<n>` |
| `0x7ffd12345678`, `3f2a8b1c-0d9e-…` | `<addr>`, `<uuid>` |
| `2026-07-29T04:00:00` | `<date>T04:00:00` |
| `cannot allocate 1048576 bytes` | `cannot allocate <n> bytes` |

Directory components collapse to `<path>` while the final filename survives —
the file that crashed is signal, the scratch directory it was checked out into
is not. Numbers, addresses, UUIDs and timestamps are all variance around the
same defect, so they collapse to placeholders.

Slurm's own chatter never becomes a signature. Lines starting with `slurm`,
`srun:`, `sbatch:`, `scontrol:`, `sacct:`, `mpibind`, `pmix:` and anything from
`slurmstepd` or `slurm_load_jobs` are skipped when looking for the last
meaningful line: the scheduler announcing that it cancelled a job tells you
nothing about why the run failed.

## Three buckets that must not disappear

A run that produced nothing, or died before it could report, is exactly the
kind of run a naive report drops — and it is the kind worth knowing about. All
three are counted, get their own signature, and can cross the threshold on
their own:

- **`<no output>`** — the run failed and left no readable stderr, or only
  scheduler noise.
- **`<no status file>`** — the run never wrote a usable status file, so nobody
  knows whether it finished. `failed_runs` counts these as not-successful;
  they are never silently counted as successes.
- **`<timeout, no output>`** — the run was killed by a walltime/budget timeout
  (status `timeout`/`timed_out`, exit code 124 or 281) and left no readable
  stderr. Kept distinct from `<no output>`: a silent timeout means the agent's
  budget was too small for the job, while a silent crash means the code is
  broken — the frontier reacts to these differently (grow the budget versus
  fix the crash), so they must not share a bucket (issue #3690). A timeout that
  *did* write a meaningful stderr line reports that line instead.

Both stay in the denominator. Twenty runs of which four were silent reports
`4 of 20 runs failed (20.0%)`, not `0 of 16`.

## Threshold

The default threshold is **5% of the runs in the window**, strictly greater
than. A signature at exactly the threshold is not systemic; one above it is.
`--threshold-percent` moves it (a value in `(0, 100]`).

```console
$ autospec doctor failures --last 500 --threshold-percent 2 --top 20 --json
```

## Machine contract

| Exit code | Meaning |
|---|---|
| `0` | The window is readable and no signature crossed the threshold |
| `1` | At least one systemic signature — the command surfaced it automatically |
| `2` | Bad arguments, or the runs on disk could not be read |

`--json` emits the same report with the denominator explicit:

```json
{
  "runs_dir": ".autospec/runs",
  "window": 200,
  "runs": 190,
  "failed_runs": 62,
  "unsigned_runs": 2,
  "threshold_percent": 5.0,
  "systemic": true,
  "signatures": [
    { "signature": "gcc -c <path>/main.c:<n>: error: cannot allocate <n> bytes",
      "count": 62, "share_percent": 32.63, "systemic": true }
  ],
  "truncated": 0
}
```

`runs` is the denominator every `share_percent` divides by; `unsigned_runs` is
how many of those runs landed in one of the two buckets above; `truncated`
counts signatures beyond `--top`.

## Where this runs in the fleet

The Slurm fleet runner (`autospec-fleet`) is external to this repository, so
the command reads the run-directory convention above rather than talking to
`sacct` directly. Point it at whatever directory the launcher records runs in:

```bash
autospec doctor failures --runs-dir /srv/autospec/runs --json
```

Exit code 1 makes it usable as a gate: a monitor that runs it after each batch
learns about a systemic failure the same way a human reads the table — by
counting.
