# A liveness signal must be written while the subject works, and a reaping rule must be validated on a known-healthy subject before it acts (issue #4579)

While the base copy was broken, a triage rule was built: an agent past a
20-minute budget with **zero bytes in `agent.out`** is stuck and gets
reaped. It worked — the agents it identified were genuinely wedged,
verified independently (both `tar` processes at 0% CPU, scratch directory
growing 0 KB per 60 s). It worked for the wrong reason: `agent.out` is
written when the agent *finishes*, not as it works. It was zero for stalled
agents and zero for working agents alike; the stall just made that
indistinguishable.

Now that the copy is fixed, the same rule is actively dangerous. Sampled
just after the fix, eight agents past the copy — all eight identical:
`slurm=267B`, `agent.out=0B`, ages from 8:27 down to 6:02. Every one is
healthy and doing real work; `as-4534` is running `pi` against the LLM with
`rustc` compiling at 110% CPU. An agent legitimately spends its whole
multi-hour run with `agent.out` at zero. **Applied after the fix, the rule
would have reaped the entire fleet at the 20-minute mark.** No agent was
wrongly reaped — the rule was only ever applied while the fault was live —
but it would have been on the next application, with no step anywhere
prompting a re-check.

- **A liveness signal must be something the subject writes *while* working,
  not when it finishes.** A completion artifact answers "is it done", never
  "is it alive", and the two are only confusable when nothing is alive —
  which is exactly the situation in which the signal gets chosen.
  `WriteTiming::OnCompletion` is refused by
  `LivenessSignal::usable_for_liveness`; the rule's decision never reads
  `agent.out` at all — the field is recorded for the evidence record and
  ignored by the verdict, the same shape as process state in
  `progress_contract`.
- **Prefer a signal that advances.** The Slurm log moved 36B → 267B as the
  agent cleared the copy; a heartbeat with a phase name and a timestamp
  (`phase=copy`, `phase=agent`, `phase=gate`) answers both "alive" and
  "where" without inspecting the process tree on a compute node.
  `SignalQuality::rank` makes the preference checkable: heartbeat >
  advancing log > completion artifact.
- **The reaping condition is "the Slurm log has not grown in N minutes AND
  the agent is past its phase budget"** — both, not either. The phase is
  published so the budget can differ per phase: the copy is minutes, the
  agent run is hours, and one number cannot serve both (`Phase`,
  `PhaseBudgets`; an unknown published phase has no budget and cannot be
  reaped on one, `Phase::parse` fails closed). Either condition alone reaps
  the fleet: a healthy agent spends hours past a flat budget with `agent.out`
  at zero, and it goes quiet between log lines.
- **A reaping rule must be validated against a known-healthy subject before
  it is allowed to act.** Validation on stuck subjects only is
  `ValidationState::StuckOnly` — every candidate signal looks identical
  there — and a rule that fires on a known-healthy subject is defective by
  definition (`ValidationState::FiresOnHealthy`). This is the same defect
  already filed as #4555 ("a health signal's first use must not be a
  deletion"): the invariant was filed, and then a rule was written that
  violates it, because the signal was right there and the fleet was on fire.
- **When the underlying fault is fixed, re-validate the triage built during
  it.** Rules written under an outage encode the outage's conditions; the
  incident's rule survived the fix and silently inverted from useful to
  destructive. `RuleProvenance` carries the three facts (written during the
  outage, fault fixed, re-validated) and flags the un-revalidated survivor;
  `action_blockers` collects every invariant the rule still fails, and
  `may_act` is empty-blockers-only.

Checkable in `autospec_core::agent_reap` (`WriteTiming`, `SignalQuality`,
`LivenessSignal`, `Phase`, `PhaseBudgets`, `AgentObservation`,
`zero_byte_rule_fires`, `ReapRule`, `ReapVerdict`, `HealthyTrial`,
`Validation`, `ValidationState`, `RuleProvenance`, `ActionBlocker`,
`action_blockers`, `may_act`). Tests:
`crates/autospec-core/tests/agent_reap.rs`, including the regression that
reconstructs the incident end to end: the eight healthy agents past the copy
(all `agent.out=0B`) fire the zero-byte rule and are kept by the replacement
rule; the incident's rule is blocked on all three counts (completion
artifact, stuck-only validation, stale from outage); and the post-fix rule
(heartbeat signal, validated on healthy, re-validated after the fix) acts —
and reaps only the wedged agent.
