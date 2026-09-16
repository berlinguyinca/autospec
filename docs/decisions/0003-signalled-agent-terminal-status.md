# ADR 0003 — A signalled agent is a terminal `SIGNALLED` run, never a baseline

- **Date:** 2026-09-15
- **Status:** Accepted
- **Satisfies:** issue #4651 acceptance criteria 2, 3 and the in-repo half of 1 and 4;
  corrected invariants 1, 2 and 3 of the same issue; the liveness policy already
  declared in `crates/autospec-core/src/agent_watchdog.rs` (#4258) invariants 3 and 4;
  the baseline contract of #4644.

## Context

`iw-87` ran 3602 s and was terminated mid-edit. Its record:

```
agent_secs=3602   agent_rc=143   agent_limit=25200
changed_files=12  build_rc=0  fmt_rc=1  test_rc=101
agent.out  EMPTY (no report written)
```

`143 = 128 + 15` is SIGTERM. The runner bounds itself with
`timeout -k 60 "$LIMIT" pi …`, `LIMIT=25200`, and a `timeout` expiry exits `124`,
which the runner already maps to `TIMEOUT-NO-OUTPUT`. So the runner's own limit did
not fire: a *different* supervisor sent the signal. The issue's follow-up identified
it — the runner's own 45-minute stall watchdog (`STALL_MIN=45` in the fleet runner
script), which decides from the mtime of `$SC/.sessions` and `$OUT/agent.out`. Both
of those go quiet during one long model generation, so a working agent reads as a
stalled one. Its sibling `agent-watchdog.sh` documents the same signal as invalid and
uses 450 minutes.

Three facts decide the design:

1. **The exit code already distinguishes the two terminations.** `124` is
   "my own `timeout` fired"; `128 + N` is "something signalled me." Nothing else in
   the record does. The in-repo reader parses neither: `AgentReport`
   (`execution/status_triage.rs`) parses `status`, `build_rc`, `test_rc`, `fmt_rc` and
   `fmt_files`, and silently drops `agent_rc` — even though the fleet writes it on
   every record (`status=NO-OUTPUT agent_rc=143 agent_secs=2761 changed_files=0`), and
   even though `run_lifecycle::STALL_KILL_RC = 143` and `Terminal::is_stall_killed()`
   already exist. The distinction is recorded and then discarded one layer up.

2. **The misclassification does real work, not just cosmetic work.** Fed `iw-87`'s
   record, `triage` sees `fmt_rc=1` with `build_rc=0` and returns
   `TriageDecision::FormatAndRecheck`: the pass formats a 2 000-line half-applied
   patch and re-checks it, and if the format happens to land clean the patch becomes a
   conversion candidate. A run whose agent was killed mid-sentence is indistinguishable
   from a run that finished and left tidy work. `UNKNOWN-NO-BASELINE` is worse: it is a
   claim *about the baseline* (#4644), and a killed agent never reached a baseline or a
   gate at all.

3. **The repository already declares which liveness signal is valid.**
   `agent_watchdog.rs` (#4258) invariant 4: *"A working process can also show an empty
   redirect target, so 'the log is empty' is never a hang signal — and
   `AgentObservation` deliberately carries no output-size field."* Its detector is the
   supervisor's own bookkeeping, not a file mtime. So the codebase already says which of
   the two disagreeing watchdogs is wrong; the 45-minute mtime watchdog is the defect.

The issue lists `execution/verification.rs` and `execution/mod.rs` as the files
touched. That is not where the decision lives: `verification.rs` scores a *patch
profile*, and by the time it runs the pass has already decided to trust the run. The
decision point is the triage, and the vocabulary that gates it.

## Decision

### 1. `SIGNALLED` joins the run-status vocabulary as an emitted name

`config/run-status-vocabulary.tsv` gains `SIGNALLED` — *"the agent process was
terminated by a signal the runner did not send itself"* — backed by
`Status::Signalled`. It is **emitted**, not gate-only: the runner writes it, and the
conversion pass, the dispatch guard and the runner verdict all have to route it.

It must be a vocabulary name rather than a local string because the vocabulary is the
only thing that makes an unlisted name a compile error. A status the vocabulary does
not declare resolves to `None`, and `None` in `triage` falls through to
`GateLocally { AgentGreen }` — an unknown label is treated as success. Declaring
`SIGNALLED` is what turns "a killed agent" from a silent fall-through into a routed
case.

### 2. The reader parses the termination, and the termination outranks the label

`AgentReport` gains `signal` and `agent_rc`, both parsed leniently to key presence
(the harness may add fields) but strictly to value. A new pure helper,
`execution/status_triage/signal.rs`, maps a recorded exit code onto
`Termination::{OwnTimeout, Signalled { signal }, Plain}`:

- `rc = 124` → `OwnTimeout`. The runner's own bounding `timeout` fired; the sender is
  known and this is `TIMEOUT`, which already re-dispatches.
- `rc = 128 + N` → `Signalled { signal }` for the signals a supervisor actually sends
  (`SIGINT`, `SIGABRT`, `SIGKILL`, `SIGTERM`), naming the signal rather than a number.
- anything else → `Plain`.

In `triage`, a report is signalled when its label canonicalises to `Signalled`, **or**
it recorded a `signal`, **or** its `agent_rc` decodes to a signal. That rule sits with
the timeout rules, above fmt and build: a run that was killed never reached a stage, so
an `fmt_rc` in the same file is debris, not a verdict. The signal outranking the label
is the same precedent the module already applies to `test_rc` (#4206: a record that
says `VERIFIED` and carries `test_rc=101` is triaged on the code).

Ordering is what fixes `iw-87`: `fmt_rc=1` no longer reaches `FormatAndRecheck`,
because the termination is read first.

**One ordering deliberately does not go the other way.** A `TIMEOUT` label still
outranks the signal, because it is the runner *asserting* its own limit fired and
an assertion outranks an inference. The exit code cannot settle the question in
either direction: `timeout` itself exits `124`, but a harness that reports the
child's death-signal writes `143` for the very same event. So a signalling code
with no claim beside it is refused as *unattributed* — the honest reading — while
a `TIMEOUT` label is honored as what the runner said, and `dispatch_guard` mirrors
the same precedence so an archived artifact is never mislabelled as a kill the
runner did not make. This is also why acceptance criterion 3 asks for an explicit
"did my own timeout fire" field: the ambiguity is in the code, and no amount of
reading it harder removes it.

### 3. Triage answers with a new terminal variant: `TriageDecision::Signalled`

Not `GateLocally` (the pass would spend a full gate on a tree whose agent is gone), not
`Hold` (a hold asserts a deterministic property of the *submission*; a killed agent
establishes none), not `Redispatch` (the same stall setting would kill the retry). It
is a refusal: no verdict about the patch exists, and none can be conjured from the
record. The variant carries the signal name so the refusal line can say who ended the
run and whether the runner's own timeout fired — the record's own attribution, which is
corrected invariant 1.

`dispatch_guard::FAILED_RUN_STATUSES` gains `Signalled` alongside `Timeout` and
`TimeoutNoOutput`: a signalled artifact is permanently unconvertible, so the guard
archives it and frees the dispatch slot instead of holding forever. It stays *not*
failing in `runner_verdict::is_failing`, which is the same judgement `NoOutput` and
`Timeout` already make: "nothing was measured" is not "the patch is broken," so the
artifact is never written to `changes.rejected-*.patch` where a real negative lives.

### 4. Which watchdog is authoritative is stated, not implied

`agent_watchdog.rs` (#4258) owns the liveness policy and already forbids the file-mtime
inference. `SIGNALLED` is the vocabulary name for what happens when the *other* kind of
supervisor wins, and its refusal line says so. `run_lifecycle::STALL_KILL_RC` stays the
single definition of the stall-kill exit code; `signal.rs` reads it rather than
restating `143`.

## What is deliberately not done here

- **The 45-minute watchdog is not removed in this repository.** It lives in the fleet
  runner script on the cluster host. The in-repo change makes its victims legible and
  terminal; the fleet change (drop `STALL_MIN` to a heartbeat/last-progress signal, or
  raise it to the sibling's 450-minute value and say why) is tracked by #4651 and is an
  operator action on the host.
- **No re-dispatch on a signal.** A killed agent's retry hits the same supervisor. The
  pass refuses and says why; re-dispatch stays an operator decision.
- **`NO-OUTPUT` keeps its current route** when no signal is recorded. Reports written
  before this change carry `status=NO-OUTPUT` and no `agent_rc`; they still raise for
  review. Only records that actually name a termination change behaviour.
- **No new status for "killed after a long run" vs "killed early."** `agent_secs` is
  already in the record; encoding a second axis into the status name would multiply the
  vocabulary without adding a routing decision.

## Consequences

- Adding a status to the vocabulary forces every consumer to route it: the exhaustive
  match in `triage`, the vocabulary audit lists in the tests, and `FAILED_RUN_STATUSES`.
  That is the point of the vocabulary (#4206), and it is why this is the cheap half of
  the fix.
- Half-applied patches from killed agents stop entering the conversion queue as
  candidates. They are archived with the named signal, which is the first time those
  three lost runs would have been diagnosable from the artifact alone.
- `run_status.rs` and `execution/status_triage.rs` were both over the file-size ratchet
  and could not absorb the change. The evidence-and-verdict audit moves to
  `run_status/evidence.rs`, and the hold/decision rendering moves to
  `execution/status_triage/render.rs`; both re-export from their parent, so no call site
  changes.
