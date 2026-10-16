### Added

- `autospec-core::agent_reap` — a liveness signal must be written while the
  subject works, and a reaping rule must be validated on a known-healthy
  subject before it acts. The incident's triage rule reaped "an agent past
  a 20-minute budget with zero bytes in `agent.out`": `agent.out` is written
  when the agent finishes, so it was zero for the healthy fleet too, and
  after the base-copy fix every healthy agent matched the rule — applied a
  day later it would have reaped the entire fleet. The zero-bytes test is
  replaced by "the Slurm log has not grown in N minutes AND the agent is
  past its phase budget", with the phase published (`Phase` — `copy`,
  `agent`, `gate`) so the budget differs per phase (copy minutes, agent
  hours, `PhaseBudgets`; an unknown phase fails closed); a completion
  artifact is refused as a liveness signal (`WriteTiming`,
  `LivenessSignal::usable_for_liveness`), signals are ranked by what they
  carry while the subject works (`SignalQuality::rank`: heartbeat >
  advancing log > completion artifact); a rule validated only on stuck
  subjects may not act (`ValidationState::StuckOnly`), a rule that fires on
  a known-healthy subject is defective (`FiresOnHealthy`); and a rule
  written during an outage must be re-validated after the fault is fixed
  (`RuleProvenance`). The old rule is kept as `zero_byte_rule_fires` —
  named, never called by the decision — so the regression tests can show it
  firing on the entire healthy fleet (#4579, 2026-09-13).
