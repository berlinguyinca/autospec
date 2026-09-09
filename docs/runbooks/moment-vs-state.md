# Moment vs state (issue #3973)

A single observation of an append-only or asynchronous source describes a
**moment**, not a **state**. "No result line yet" is not "no result will
come"; "no dispatch in the tail" is not "dispatch is broken"; "the log is
quiet right now" is not "the job died". In one session of six confident
diagnoses, each of these mistakes was made, and one of them killed a job
that had actually opened 86 PRs (it had written a terminal completion
marker, which nobody checked).

The invariant: **state claims require two independent observations or a
terminal record — never a single sample, and never log recency alone.**

## `scripts/lib/autospec-log-state.sh`

Source it (`. scripts/lib/autospec-log-state.sh`), then:

### Writer side

| Helper | Contract |
|---|---|
| `autospec_heartbeat <logfile> [text]` | Append one heartbeat line (`autospec-heartbeat <UTC ts> pid=<pid> [text]`). `0` on success, `1` on I/O error, `2` on usage. |
| `autospec_terminal <logfile> [status]` | Append the terminal marker line `######## <status> ########` (default status `complete`; e.g. `failed rc=7` on error). The marker is a **record**: once written, the step's running phase is over regardless of what the log does afterwards. `0`/`1`/`2` as above. |
| `autospec_run_step <logfile> [--heartbeat-every SECS] [--] <cmd> [args...]` | Run `<cmd>` while a heartbeat is appended every `SECS` (default 30). On exit, append `######## complete ########` (or `######## failed rc=N ########`) and propagate the command's exit code. The heartbeat loop reaps itself if the calling shell dies, so a crashed step cannot keep lying in its own log. |

### Reader side

| Helper | Contract |
|---|---|
| `autospec_log_state <logfile> [--marker RE] [--gap SECS]` | Report `running` (rc `0`), `complete` (rc `3`), or `stalled` (rc `4`). `complete`: a line matches the terminal marker (default RE `^######## .+ ########$`) — a terminal record, one observation suffices. Otherwise the log is sampled (sha256) **twice, `gap` seconds apart** (default 2): different → `running`, identical → `stalled`. A freshly written but quiet log is `stalled`, not `running` — recency is not evidence of motion. rc `1`: log missing/unreadable; rc `2`: usage (including an invalid marker RE). |
| `autospec_destructive_guard <logfile> [--marker RE] [--gap SECS]` | The gate for **stop / re-dispatch / archive**. Returns `0` (allowed) only on a terminal marker or on the two-sample stall verdict; returns `1` (refusing) while running, and fails closed on any other error (missing log included). |
| `autospec_log_yield <logfile> <pattern>` | A yield/throughput characterisation is a **count**, computed over the **whole** file: prints `grep -c <pattern>` over `<logfile>`. Never an estimate from the visible tail. rc `0` (even at count 0), `1` unreadable, `2` usage. |

## Where each of the six diagnoses maps

| Diagnosis (the moment) | State evidence required |
|---|---|
| "no result line yet" → "no result will come" | `autospec_log_state`: marker, or two samples apart in time |
| "no dispatch in the tail" → "dispatch is broken" | `autospec_log_yield <log> <dispatch-pattern>`: the count, not the tail |
| "log is quiet now" → "job died" (killed a job that had opened 86 PRs) | `autospec_destructive_guard`: terminal marker or two agreeing samples; the killed job *had* a marker |
| "one sample says X" → "X is the state" | second sample, `gap` seconds later; disagreement reopens the question |
| "recency says running" | rejected by construction — `autospec_log_state` never reads mtime |
| "I am sure" | the guard refuses; certainty is not an observation |

## Tests

`tests/autospec-log-state.bats` covers the populated #3793 case: a status
check against a log **mid-write** must report `running`, a freshly written
but quiet log must report `stalled` (not running — the recency trap), a
terminal marker (complete or failed) is final, the destructive guard
refuses while running and fails closed on a missing log, and yield counts
the whole file even when the visible tail contains zero matches.

## Usage in a pipeline step

```bash
. scripts/lib/autospec-log-state.sh

# The step (writer): heartbeats + terminal marker, exit code preserved.
autospec_run_step "$work/topup.log" --heartbeat-every 30 -- \
    my-dispatcher --resume >> "$work/topup.log" 2>&1

# A status query (reader): never trusts one sample.
case "$(autospec_log_state "$work/topup.log" --gap 5)" in
    running)  echo "still working";;
    complete) echo "done — proceed";;
    stalled)  echo "no marker, two samples agree: escalate";;
esac

# Before stop / re-dispatch / archive:
autospec_destructive_guard "$work/topup.log" --gap 5 || exit 1

# Before characterising yield:
opened=$(autospec_log_yield "$work/topup.log" 'PR: https')
```
