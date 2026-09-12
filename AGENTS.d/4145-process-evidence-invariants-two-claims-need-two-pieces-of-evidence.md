# Process-evidence invariants: two claims need two pieces of evidence (issue #4145)

A conversion pass was declared dead twice on evidence that could not have
shown it either way. It had in fact run for 88 minutes and converted 8
patches.

It was launched with its stdout redirected:

```sh
setsid --fork bash -c 'exec bash convpass.sh 3194 3210 ...' > "$T/convpassW.log" 2>&1
```

`convpassW.log` stayed at 0 bytes and was read as "it produced nothing and
died." But the script does not log to stdout — every line went to
`$T/convpass.log` via `LOG=`, which showed the pass finishing normally:
`converted=8 held=4 skipped=1`. The liveness check that preceded the verdict
was worse: `pgrep -f 'convpass.sh 3194'` returned the two PIDs of the shell
running that very `pgrep`, so the pattern matched its own caller. The check
reports "alive" whenever it is run and is therefore incapable of reporting
"dead" — a confident and completely wrong conclusion in both directions
within the same iteration.

- **A redirect you added is not evidence about output you did not route
  there.** Before treating an empty capture file as a result, confirm the
  reader's source is the place the program actually writes
  (`capture_routed`). A program with its own log destination leaves a
  wrapper's capture empty on every run, success and failure alike — useless
  precisely when it looks most damning (`unrouted_capture_finding`). Where a
  wrapper needs the output, read the program's own log (`LOG=... convpass.sh`)
  rather than redirecting a stream it never uses.
- **A process-liveness check must not be able to match itself.** `pgrep -f
  PATTERN` scans full command lines, including the command line of the shell
  invoking it, so any pattern drawn from the arguments just used matches the
  caller (`LivenessCheck::Pattern`). Check by PID, recorded at launch, and
  verify with `kill -0` (`LivenessCheck::Pid`); where a pattern is
  unavoidable, exclude the caller and require the match to be the program, not
  a shell whose argv quotes it. A pattern check whose only matches are its own
  caller is `Liveness::SelfReferential` (`self_referential_finding`) — the
  read-only twin of `pkill -f` killing its own invoking shell, and more
  dangerous, because killing yourself is loud and a false "alive" is silent.
- **"Produced no output" and "is not running" are two claims needing two
  pieces of evidence.** Neither implies the other: a finished run produces no
  new output and is not running; a wedged run produces no new output and is
  running; a run logging elsewhere produces no output *in your file* and is
  perfectly healthy. Terminal lines exist to separate these (#4094) — read the
  log the program actually writes, and look for its terminal line.

Checkable in `autospec_core::process_evidence` (`capture_routed`,
`OutputDestination`, `EvidenceSource`, `unrouted_capture_finding`,
`LivenessCheck`, `ProcessMatch`, `Liveness`, `liveness_verdict`,
`self_referential_finding`, `Observation`, `Diagnosis`, `diagnose`,
`diagnosis_finding` — pure in-memory, no I/O, no clock, so the shell
`convpass.sh` launcher and its status tooling can adopt them as the single
source of truth). Tests:
`crates/autospec-core/tests/process_evidence.rs`, including the regression
that reconstructs the incident end-to-end: the empty stdout capture read as
"died", the `pgrep -f 'convpass.sh 3194'` that returned its own caller's PIDs
(`3142082 3142084`), and the terminal line `converted=8 held=4 skipped=1` the
reader missed.
