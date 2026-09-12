# Unfed passes and idle passes (issue #4296)

`convpass.sh` took its work positionally, and run with no arguments it
printed `######## convpass: converted=0 held=0 skipped=0 ########` —
byte-identical to a healthy pass that examined every patch and found
nothing to do — while `convselect.sh` reported four waiting candidates.
The failure is stable and quiet: a pass that reports `converted=0` every
run looks like a backlog that happens to be empty, and the longer it runs
the more normal it looks. The pass runs under the instruction "log one
line even when there is nothing to convert, so a silently broken loop is
distinguishable from an idle one", and the one line it logs is the line
that cannot tell the two apart. The sibling file already had the fix —
`iwconv.sh` says `nothing to convert (no issues given)` — the same
divergence as #3741: an invariant learned in one file did not reach the
file next to it.

- **"I was given no work" and "there is no work" print different lines.**
  Any loop whose body may execute zero times has an explicit empty-input
  branch before the summary (`unfed_line` vs `examined_line`). A pair of
  summary lines that are byte-identical for the two states is a finding
  (`identical_summary_finding`) — that identity is the incident, and it
  defeats the very instruction the one log line exists to satisfy.
- **Selection and execution are not separately invocable without a
  default.** `convpass.sh` with no arguments runs `convselect.sh` itself
  or refuses — the refusal names the selector, because a diagnostic that
  does not name the remedy is half a policy
  (`plan_invocation`, `BareDefault`, `refuse_line`). Requiring a caller
  to remember `convpass.sh $(convselect.sh)` guarantees that one day
  someone runs it bare and reads the result as good news. A bare `Run`
  with zero candidates under a refuse policy is a finding
  (`bare_invocation_finding`); an empty result from a selector that did
  run is a true idle, not one — the pass knew it had been fed.
- **An exit-trap summary must reflect the exit path.** The incident's
  trap printed `TERMINATED rc=0 (converted=0 held=0 skipped=0)` directly
  after the guard line — a diagnostic that a later handler overwrites
  with the thing it was correcting is not a fix. A run that ended on a
  guard names the guard and never re-states counters that were never
  populated (`trap_line`, `trap_line_findings`).
- **A counter of zero is not evidence of work performed.** Report the
  size of the input the pass was handed — `examined=N` alongside
  `converted`/`held`/`skipped` — so an unfed run is self-evident
  (`missing_examined_finding`), and counters that exceed the input are a
  state that cannot exist (`PassCounters::reconciles`).

Checkable in `autospec_core::unfed_pass` (`unfed_line`, `examined_line`,
`identical_summary_finding`, `plan_invocation`, `BareDefault`,
`refuse_line`, `bare_invocation_finding`, `ExitPath`, `trap_line`,
`trap_line_findings`, `PassCounters`, `missing_examined_finding` — pure
in-memory, so the shell `convpass.sh` / `iwconv.sh` pair can adopt them
as the single source of truth). Tests:
`crates/autospec-core/tests/unfed_pass.rs`, including the regression that
reconstructs the incident end-to-end: the byte-identical summary line,
the trap that re-states unpopulated counters after the guard, and the
four candidates (`4065 4251 4257 4282`) that sat behind `converted=0
held=0 skipped=0`.
