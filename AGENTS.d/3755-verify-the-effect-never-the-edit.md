# Verify the effect, never the edit (issue #3755)

Across one session the supervising agent caused nine outages — and four of
them were repairs for earlier problems it had caused itself. The
agent-caused holds in the same window were routine and cheap; the
expensive failures were self-inflicted, and every one of the nine was
caught the same way — by observing the **effect** of the change, never by
re-reading the edit: grepping the logs for what actually killed the jobs,
running the topup and seeing `sbatch` reject the wrap, drawing from the
selector five times and getting nothing, checking whether the lints existed
on `main`. Re-reading the change would have caught none of them, because
each edit was correct in the sense its author intended.

The control is neither capability — it raises the *rate of changes made*,
and each change is an opportunity to break a running system; every
incident was taken confidently and for good reason — nor care: two of the
nine recurred three and four times each despite being known, documented,
and filed. The control is a mechanical verification step:

- **After any change to a running system, verify the effect, not the
  edit.** Run the thing once and read its output — one command, and it
  caught nine incidents (`ProbeKind::Effect` vs `ProbeKind::Reread`,
  `ship_verdict`). A verification plan that only re-reads the edit is the
  insidious shape: it *looks* like verification (`REREAD_ONLY`), and a
  change to a running system with no probe answering the 60-second
  question ships only against an idle system (`NO_EFFECT_PROBE`,
  `ShipVerdict::WaitForIdle`).
- **A verification that returns nothing is a failure, not a pass.** Item
  5's symptom was an empty result set, which reads as "no problems
  found" (`run_probe` -> `ProbeOutcome::NoSignal`, `outcome_line`). A
  probe that cannot name the signal a healthy system produces cannot read
  its output as a pass — fail-closed (`PROBE_NO_SIGNAL`).
- **State the blast radius before editing shared state.** Who reads this
  path, what do they do if it changes mid-read, what makes the change
  exclusive with respect to them (#3732) — each missing question is its
  own finding (`BlastRadius`, `blast_radius_findings`), and "nobody reads
  this" is a claim, not a default.
- **Never raise a bar ad hoc.** A gate matches the repository's
  definition or it is a different gate (#3753): one that adds strictness
  the repository never ran can reject what the repository accepts, and one
  that drops it can accept what the repository rejects — both directions
  are findings (`GateApplication`, `ad_hoc_gate_findings`).
- **Prefer the change you can verify cheaply** over the better change you
  cannot. The verifiability filter runs *before* the quality ranking: the
  higher-value change with no in-window effect probe is never chosen
  (`choose_change`). Item 5 was a genuine improvement to load distribution
  that broke a working system under load; the ranking it replaced was
  worse and total.

The uncomfortable version, worth stating plainly: an autonomous supervisor
with write access to a running fleet will damage it, and the damage will
come disguised as maintenance. Budget for that. The useful question in a
design review is not "is this change correct" but "how will I know within
sixty seconds if it is not" (`VERIFY_WINDOW_SECONDS`,
`sixty_second_answer`) — and any change that cannot answer it should wait
for a moment when the system is idle. A design review that accepts a
change whose answer is the wait statement has moved the change to an idle
window; it has not waved it through.

Checkable in `autospec_core::effect_verification` (`VERIFY_WINDOW_SECONDS`,
`ProbeKind`, `Probe::answers_within_window`, `Target`, `Change`,
`ship_verdict`, `ShipVerdict`, `plan_findings`, `sixty_second_answer`,
`run_probe`, `ProbeOutcome`, `outcome_line`, `BlastRadius`,
`blast_radius_findings`, `GateApplication`, `ad_hoc_gate_findings`,
`choose_change` — pure in-memory, so the supervisor scripts can adopt them
as the single source of truth). Tests:
`crates/autospec-core/tests/effect_verification.rs`, including the
regression that reconstructs the incident's session: the re-read-only plan
that looks verified and waits, the five empty selector draws that are all
failures, the `sbatch` rejection that is the `Failed` outcome, and the
`clippy -D warnings` gate that adds a flag the repository's CI never ran.
