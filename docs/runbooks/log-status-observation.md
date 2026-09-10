# Log-status observation guards (issue #3973)

Every confident diagnosis in the issue read a system **in motion** as a
system **at rest**: a log mid-write, a process table between samples, a
spec staged asynchronously. A single observation of an append-only or
asynchronous source describes a *moment*; a conclusion describes a
*state*. The failure is not carelessness — each observation was real and
correctly read. The missing step is distinguishing **not yet** from
**not ever**, and nothing in a single observation carries that
distinction. It recurred six times in one session *while actively being
noticed and corrected*, which is the signature of something that cannot
be fixed by intending to be careful — so it gets a control.

The cost is asymmetric: a premature "it is broken" invites destructive
intervention (a 21-hour, 86-PR conversion job was killed on two log
lines and had to be restarted); a premature "it is fine" merely delays
discovery. So the bias produced by paying attention points at the more
expensive error, and the guards below point the other way.

## The invariants

1. **Never conclude from a single sample of an append-only or
   asynchronous source.** Sample twice with a gap, or wait for a terminal
   marker. A pass that writes `######## complete ########` can be *asked*
   whether it is finished; a tail cannot.
2. **Every long-running step emits a terminal marker and a heartbeat.**
   Then "no output yet" is distinguishable from "stopped", and no reader
   has to infer it.
3. **Destructive actions require a second, independent observation.**
   Stopping a job, re-dispatching an issue, or archiving output is gated
   on a terminal marker or two observations — never on one glance at a
   log.
4. **Quantify before characterising.** "Low yield" is a number, and the
   number was available: a count over the whole log said 86. A judgement
   expressible as a count is computed, not estimated from the visible
   tail.
5. **Report the observation and the inference separately.** "The tail
   shows no dispatch since 13:10" is a fact; "the dispatcher is broken"
   is a claim. Keeping them apart in the report makes the gap visible to
   the reader — and to the author.

## `scripts/lib/autospec-log-status.sh`

Source it (`. scripts/lib/autospec-log-status.sh`), then:

| Helper | Contract |
|---|---|
| `autospec_log_status <logfile> [terminal_marker] [gap_seconds]` | Verdict from markers and two samples, never from recency alone. A terminal marker (default: `######## complete ########`) is final by definition, so one observation of it suffices → `complete`. Without a marker the log is sampled, the reader waits `<gap_seconds>` (default 2), and samples again: the log advanced, or the marker appeared in the meantime → `running`; two identical samples → `stalled`. `stalled` is only ever produced by two independent observations. The observation evidence (both samples: sizes, digests, timestamps) goes to stderr under `obs:`; the inference is labelled `inference:` on stderr, and the bare verdict word goes to stdout. Returns 0 with a verdict, 1 when the log is missing or unreadable, 2 on usage. |
| `autospec_log_gate <logfile> [terminal_marker] [gap_seconds]` | Destructive-action gate (stop, re-dispatch, archive). Allows (0) only on `complete` or on a `stalled` verdict that already carries two independent observations; refuses (1) on `running` — a live system is never acted destructively on on one glance at its log. Passes the status observation lines through. Returns 2 on usage, 1 when the log is unreadable. |
| `autospec_log_count <logfile> <pattern>` | A computed count (fixed-string, whole log) of `<pattern>`. A judgement expressible as a number is computed, not estimated from the visible tail. Prints the count — a computed 0 is a result, so it returns 0 too; 1 when the log is unreadable, 2 on usage. |
| `autospec_log_terminal <logfile> [marker]` | Append the terminal completion marker (default: `######## complete ########`) so a reader can ask the log whether the pass is finished instead of inferring it from silence. |
| `autospec_log_heartbeat <logfile> [step]` | Append a heartbeat line (`heartbeat: <step> <UTC timestamp>`), so "no output yet" stays distinguishable from "stopped" between writes. |

Status tooling that needs a running / complete / stalled verdict calls
`autospec_log_status` (or `autospec_log_gate` when the verdict authorises
a destructive step). It does not derive state from log mtime or tail
recency: those describe when the log was last touched, not whether the
pass is still advancing.

## Heartbeat liveness (issue #3995)

A periodic process (a cron or loop pass) is **live by its heartbeat, never
by its log's mtime**. Two incidents pinned this: a top-up cron was read as
dead because the check looked at `cron-topup.log` while the loop writes to
`topup.log` (an empty, never-touched file), and a repaired reg-sweep was
read as broken because its log ended in a syntax error while its
heartbeat was fresh. Both misdiagnoses are impossible when liveness is a
function of the heartbeat line and the check proves it is reading the
process's own log.

The Rust side lives in `autospec-core`'s `heartbeat` module
(`crates/autospec-core/src/heartbeat.rs`):

- **Write** — `write_heartbeat(log, step, now, outcome)` appends
  `heartbeat: <step> <RFC3339Z> [<outcome>]`. Every pass appends one,
  *including a pass that did nothing* (outcome `nothing to do`). The
  shell counterpart is `autospec_log_heartbeat` above; the line shape is
  the same, so the two writers are interchangeable for a reader.
- **Assess** — `assess_liveness(step, lines, interval, now)` returns
  `Live` / `Stale` / `NoRecord`. Stale means "no heartbeat since T" —
  `Liveness::describe()` never says "dead", because silence past the
  stale threshold is a claim about heartbeats, not about the process.
- **Path check** — `LogHealthCheck::verdict(observed_log, lines, now)`
  first compares the path it was given with the process's declared log.
  A mismatch is `BrokenCheck { expected, observed }`: the check is
  broken, and says nothing about the process. The empty-`cron-topup.log`
  incident is exactly this shape.

Tests: `crates/autospec-core/tests/heartbeat.rs` — ran-and-did-nothing is
`Live`, not-run is `Stale` (not "dead"), wrong log path is `BrokenCheck`
(not a process verdict).

## Tests

`tests/autospec-log-status.bats` covers the populated #3793 case — a
status check against a mid-write log must report `running`, not
`stalled` — in both directions a single sample gets wrong: a log that is
advancing during the gap, and a log whose terminal marker is written
within the gap (read as `complete`, not `stalled`). It also pins the
observation/inference split in the report, the destructive-action gate
(refusing while running, allowing on a marker or a double-observed
verdict), and the whole-log count behind the 86-PRs-behind-two-hold-lines
case.

Related: issue #3939 (a record read as history when it holds only
current state — the same moment/state confusion, in storage rather than
in observation), issue #3963 (a precondition asserted rather than
measured), issue #3967 (a control that sampled instead of holding).
