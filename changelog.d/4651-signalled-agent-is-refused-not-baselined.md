## convert: a signalled agent is refused on its own status, not baselined (#4651)

`iw-87` ran 3602 s and died mid-edit with `agent_rc=143`, 12 files
half-applied and an empty report. The reader of the run record parsed
`status`, `build_rc`, `test_rc`, `fmt_rc` and dropped `agent_rc` — the one
field that says how the process ended — so a killed agent triaged as a
repairable formatting failure: the pass formatted 2000 lines of half-applied
change and offered it to the conversion queue.

- `SIGNALLED` joins the run-status vocabulary as an **emitted** status, so an
  unlisted name cannot fall through to the green arm; adding it forces every
  consumer to route it (the exhaustive `triage` match, the vocabulary audit
  lists, `FAILED_RUN_STATUSES`) rather than silently stop matching.
- `AgentReport` reads `agent_rc` and `signal`, and a new
  `execution::status_triage::signal` module decodes the exit code: `124` is
  the runner's own bounding `timeout` (a known sender, still `TIMEOUT`),
  `128 + N` is a signal from somewhere that has not said who it is. A
  `Signalled` label, a named `signal=`, or a signalling exit code each decide
  it, and the termination outranks every stage field beside it — a killed run
  reached no stage.
- `triage` answers with a new terminal `TriageDecision::Signalled`: not held
  (a hold asserts a property of the submission a killed agent never
  established), not re-dispatched (the retry walks into the same supervisor),
  not gated (a full gate spent on a tree whose agent is gone). Its refusal
  line carries the attribution the record owes the reader: which signal, and
  that the runner's own timeout did not fire.
- `dispatch_guard::classify_report` replaces the label-only classifier at the
  one call site. A signalled artifact is a failed run whatever its label
  claims, so it is archived — preserved, never deleted — instead of held as
  convertible while the conversion pass refuses it, which left the dispatch
  slot blocked on a run that could never produce a verdict.

`UNKNOWN-NO-BASELINE` is unchanged and stays the converter's own job: the
point is that it is a claim about the *baseline*, and a killed agent never
had one. A record naming no termination classifies exactly as before.

The 45-minute stall watchdog itself lives in the fleet runner script on the
cluster host and is unchanged here; `docs/decisions/0003-signalled-agent-
terminal-status.md` records which of the two disagreeing supervisors the
repository declares authoritative, and why the liveness signal it forbids
(`agent_watchdog.rs` invariant 4) is the one that fired.
