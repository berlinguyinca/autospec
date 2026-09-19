//! Liveness and reaping of a fleet of long-running agents (issue #4579).
//!
//! The incident: a triage rule reaped "an agent past a 20-minute budget with
//! zero bytes in `agent.out`". It worked while the base copy was broken —
//! the agents it named were genuinely wedged — and it worked for the wrong
//! reason: `agent.out` is written when the agent *finishes*, so it was zero
//! for the healthy fleet too, and the stall merely made the two states
//! indistinguishable. After the copy fix, every healthy agent matched the
//! rule; applied a day later it would have reaped the entire fleet.
//!
//! The regression tests run in the configuration the bug required: the
//! eight healthy agents past the copy, all with `agent.out` at zero. The
//! controls: the same eight agents under the replacement rule, and the
//! four invariants (completion artifacts are not liveness signals; prefer
//! signals that advance; validate on a known-healthy subject before acting;
//! re-validate the triage when the fault it was built under is fixed).

use std::time::Duration;

use autospec_core::agent_reap::{
    action_blockers, may_act, zero_byte_rule_fires, ActionBlocker, AgentObservation, HealthyTrial,
    KeepReason, LivenessSignal, Phase, PhaseBudgets, ReapRule, ReapVerdict, RuleProvenance,
    SignalQuality, Validation, ValidationState, WriteTiming,
};

/// The incident's sample: eight agents past the copy, all identical —
/// `agent.out` at zero, the Slurm log at 267 bytes, ages from 8:27 down to
/// 6:02. Every one healthy and doing real work (`as-4534` was running `pi`
/// against the LLM with `rustc` compiling at 110% CPU).
fn healthy_fleet() -> Vec<AgentObservation> {
    let ages: [u64; 8] = [
        8 * 3600 + 27 * 60, // 8:27
        8 * 3600 + 5 * 60,
        7 * 3600 + 41 * 60,
        7 * 3600 + 12 * 60,
        6 * 3600 + 55 * 60,
        6 * 3600 + 30 * 60,
        6 * 3600 + 14 * 60,
        6 * 3600 + 2 * 60, // 6:02
    ];
    ages.iter()
        .enumerate()
        .map(|(i, age)| AgentObservation {
            name: format!("as-{}", 4534 + i),
            phase: Phase::Agent,
            age: Duration::from_secs(*age),
            // The Slurm log grew (36B -> 267B) as each agent cleared the
            // copy and has been moving since: not quiet.
            slurm_log_quiet: Duration::from_secs(5 * 60),
            // The incident's signal: zero for every healthy agent, exactly
            // as for a wedged one.
            agent_out_bytes: 0,
        })
        .collect()
}

/// The `agent.out` signal the incident's rule was built on: written when
/// the agent finishes.
fn agent_out() -> LivenessSignal {
    LivenessSignal {
        name: "agent.out".to_string(),
        timing: WriteTiming::OnCompletion,
        quality: SignalQuality::Completion,
    }
}

/// The Slurm log: grows while the agent works (36B -> 267B across the copy).
fn slurm_log() -> LivenessSignal {
    LivenessSignal {
        name: "slurm.log".to_string(),
        timing: WriteTiming::WhileWorking,
        quality: SignalQuality::Advancing,
    }
}

/// The explicit heartbeat with a phase name and a timestamp: the best
/// signal the incident names.
fn heartbeat() -> LivenessSignal {
    LivenessSignal {
        name: "heartbeat".to_string(),
        timing: WriteTiming::WhileWorking,
        quality: SignalQuality::Heartbeat,
    }
}

/// The incident's validation record: the rule was checked against the two
/// wedged agents only (both `tar` processes at 0% CPU, scratch growing 0 KB
/// per 60 s) — never against a subject verified healthy.
fn stuck_only_validation() -> Validation {
    Validation {
        healthy: Vec::new(),
        stuck: 2,
    }
}

/// The incident's provenance: written during the outage, the fault is now
/// fixed, and nothing re-validated the rule after.
fn outage_provenance() -> RuleProvenance {
    RuleProvenance {
        written_during_outage: true,
        fault_fixed: true,
        revalidated_after_fix: false,
    }
}

// ── The incident, end to end ────────────────────────────────────────────────

#[test]
fn incident_zero_byte_rule_fires_on_the_entire_healthy_fleet() {
    let budget = Duration::from_secs(20 * 60);
    for obs in healthy_fleet() {
        assert!(
            zero_byte_rule_fires(&obs, budget),
            "the old rule fires on {obs:?} — this is why it would have reaped the fleet"
        );
    }
}

#[test]
fn incident_new_rule_keeps_the_entire_healthy_fleet() {
    let rule = ReapRule::with_defaults();
    for obs in healthy_fleet() {
        assert_eq!(
            rule.decision(&obs),
            ReapVerdict::Keep {
                reason: KeepReason::LogAdvancing
            },
            "the new rule must keep {obs:?}"
        );
    }
}

#[test]
fn incident_rule_is_blocked_on_all_three_counts() {
    // The incident's rule: completion-artifact signal, stuck-only
    // validation, outage provenance never re-validated. It may not act.
    let blockers = action_blockers(&agent_out(), &stuck_only_validation(), &outage_provenance());
    assert!(
        blockers.contains(&ActionBlocker::CompletionArtifactSignal {
            signal: "agent.out".to_string()
        }),
        "invariant 1: a completion artifact is not a liveness signal: {blockers:?}"
    );
    assert!(
        blockers.contains(&ActionBlocker::NotValidatedOnHealthy),
        "invariant 3: stuck-only validation may not act: {blockers:?}"
    );
    assert!(
        blockers.contains(&ActionBlocker::StaleFromOutage),
        "invariant 4: {blockers:?}"
    );
    assert!(!may_act(
        &agent_out(),
        &stuck_only_validation(),
        &outage_provenance()
    ));
}

#[test]
fn post_fix_rule_is_actionable_and_reaps_only_the_wedged() {
    let signal = heartbeat();
    let validation = Validation {
        healthy: vec![
            HealthyTrial {
                subject: "as-4534".to_string(),
                fired: false,
            },
            HealthyTrial {
                subject: "as-4535".to_string(),
                fired: false,
            },
        ],
        stuck: 2,
    };
    let provenance = RuleProvenance {
        written_during_outage: true,
        fault_fixed: true,
        revalidated_after_fix: true,
    };
    assert!(
        may_act(&signal, &validation, &provenance),
        "the post-fix rule clears all four invariants"
    );

    let rule = ReapRule::with_defaults();

    // The entire healthy fleet: kept.
    for obs in healthy_fleet() {
        assert!(!rule.decision(&obs).is_reap());
    }

    // The wedged agent: Slurm log silent past the quiet bound, past the
    // copy budget. Reaped.
    let wedged = AgentObservation {
        name: "as-4542".to_string(),
        phase: Phase::Copy,
        age: Duration::from_secs(40 * 60),
        slurm_log_quiet: Duration::from_secs(30 * 60),
        agent_out_bytes: 0,
    };
    assert!(rule.decision(&wedged).is_reap());
}

// ── The decision: Slurm log quiet AND past the phase budget ────────────────

#[test]
fn wedged_agent_is_reaped() {
    let rule = ReapRule::with_defaults();
    let obs = AgentObservation {
        name: "as-4542".to_string(),
        phase: Phase::Copy,
        age: Duration::from_secs(40 * 60),
        slurm_log_quiet: Duration::from_secs(30 * 60),
        agent_out_bytes: 0,
    };
    match rule.decision(&obs) {
        ReapVerdict::Reap { silent, budget } => {
            assert_eq!(silent, Duration::from_secs(30 * 60));
            assert_eq!(budget, rule.budgets.copy);
        }
        other => panic!("expected reap, got {other:?}"),
    }
}

#[test]
fn quiet_log_within_phase_budget_is_kept() {
    let rule = ReapRule::with_defaults();
    // Log quiet 30 minutes — past the 15-minute quiet bound — but only an
    // hour into a 12-hour agent run: silence the phase can absorb.
    let obs = AgentObservation {
        name: "as-4536".to_string(),
        phase: Phase::Agent,
        age: Duration::from_secs(60 * 60),
        slurm_log_quiet: Duration::from_secs(30 * 60),
        agent_out_bytes: 0,
    };
    assert_eq!(
        rule.decision(&obs),
        ReapVerdict::Keep {
            reason: KeepReason::WithinPhaseBudget
        }
    );
}

#[test]
fn advancing_log_past_phase_budget_is_kept() {
    let rule = ReapRule::with_defaults();
    // 14 hours in — past the 12-hour agent budget — but the Slurm log grew
    // five minutes ago: the subject writes while it works. One condition
    // alone must not reap.
    let obs = AgentObservation {
        name: "as-4534".to_string(),
        phase: Phase::Agent,
        age: Duration::from_secs(14 * 3600),
        slurm_log_quiet: Duration::from_secs(5 * 60),
        agent_out_bytes: 0,
    };
    assert_eq!(
        rule.decision(&obs),
        ReapVerdict::Keep {
            reason: KeepReason::LogAdvancing
        }
    );
}

#[test]
fn one_quiet_window_two_budgets() {
    // The same observation, two published phases: reaped under the copy
    // budget (minutes), kept under the agent budget (hours). One number
    // cannot serve both.
    let rule = ReapRule::with_defaults();
    let obs = AgentObservation {
        name: "as-4543".to_string(),
        phase: Phase::Copy,
        age: Duration::from_secs(40 * 60),
        slurm_log_quiet: Duration::from_secs(30 * 60),
        agent_out_bytes: 0,
    };
    assert!(rule.decision(&obs).is_reap());

    let same_agent_phase = AgentObservation {
        name: obs.name.clone(),
        phase: Phase::Agent,
        ..obs
    };
    assert!(!rule.decision(&same_agent_phase).is_reap());
}

#[test]
fn unknown_published_phase_fails_closed() {
    // A phase the runner did not publish (or misspelled) has no budget:
    // the agent cannot be reaped on a budget it has no name for.
    assert_eq!(Phase::parse("copy"), Some(Phase::Copy));
    assert_eq!(Phase::parse("agent"), Some(Phase::Agent));
    assert_eq!(Phase::parse("gate"), Some(Phase::Gate));
    assert_eq!(Phase::parse("rebalance"), None);
    assert_eq!(Phase::parse(""), None);
}

#[test]
fn zero_quiet_bound_and_zero_phase_budget_are_refused() {
    assert!(matches!(
        ReapRule::new(Duration::ZERO, PhaseBudgets::default()),
        Err(autospec_core::agent_reap::RuleError::ZeroQuietBound)
    ));
    let budgets = PhaseBudgets {
        agent: Duration::ZERO,
        ..PhaseBudgets::default()
    };
    assert!(matches!(
        ReapRule::new(Duration::from_secs(900), budgets),
        Err(autospec_core::agent_reap::RuleError::ZeroPhaseBudget {
            phase: Phase::Agent
        })
    ));
}

// ── Invariants 1 and 2: the signal ─────────────────────────────────────────

#[test]
fn completion_artifact_is_not_a_liveness_signal() {
    assert!(!agent_out().usable_for_liveness());
    assert!(agent_out().finding().is_some());

    assert!(slurm_log().usable_for_liveness());
    assert!(slurm_log().finding().is_none());

    assert!(heartbeat().usable_for_liveness());
    assert!(heartbeat().finding().is_none());
}

#[test]
fn a_heartbeat_beats_an_advancing_log_which_beats_a_completion_artifact() {
    assert!(heartbeat().quality.rank() > slurm_log().quality.rank());
    assert!(slurm_log().quality.rank() > agent_out().quality.rank());
}

// ── Invariant 3: validate on a known-healthy subject before acting ─────────

#[test]
fn stuck_only_validation_may_not_act() {
    // The incident's record: two wedged agents, verified independently.
    // Every candidate signal looks identical there, so the record proves
    // nothing about the healthy fleet.
    let v = stuck_only_validation();
    assert_eq!(v.state(), ValidationState::StuckOnly { stuck: 2 });
    assert!(!v.may_act());
}

#[test]
fn no_trials_at_all_may_not_act() {
    let v = Validation {
        healthy: Vec::new(),
        stuck: 0,
    };
    assert_eq!(v.state(), ValidationState::Untested);
    assert!(!v.may_act());
}

#[test]
fn a_rule_that_fires_on_a_known_healthy_subject_is_defective() {
    // Had the incident's rule been applied after the copy fix, this is
    // what its validation record would have shown.
    let v = Validation {
        healthy: vec![
            HealthyTrial {
                subject: "as-4534".to_string(),
                fired: true,
            },
            HealthyTrial {
                subject: "as-4535".to_string(),
                fired: false,
            },
        ],
        stuck: 2,
    };
    assert_eq!(
        v.state(),
        ValidationState::FiresOnHealthy {
            subject: "as-4534".to_string()
        }
    );
    assert!(!v.may_act());
}

#[test]
fn healthy_trials_without_a_fire_validate_the_rule() {
    let v = Validation {
        healthy: vec![HealthyTrial {
            subject: "as-4534".to_string(),
            fired: false,
        }],
        stuck: 1,
    };
    assert_eq!(
        v.state(),
        ValidationState::Validated {
            healthy: 1,
            stuck: 1
        }
    );
    assert!(v.may_act());
}

// ── Invariant 4: re-validate when the fault is fixed ───────────────────────

#[test]
fn an_outage_rule_the_fix_outlived_is_stale() {
    let p = outage_provenance();
    assert!(p.finding().is_some());
}

#[test]
fn revalidation_clears_the_staleness() {
    let p = RuleProvenance {
        revalidated_after_fix: true,
        ..outage_provenance()
    };
    assert!(p.finding().is_none());
}

#[test]
fn a_rule_not_written_during_an_outage_is_never_stale() {
    let p = RuleProvenance {
        written_during_outage: false,
        fault_fixed: true,
        revalidated_after_fix: false,
    };
    assert!(p.finding().is_none());
}

#[test]
fn an_unfixed_fault_needs_no_revalidation_yet() {
    // While the fault is still live, the rule is still encoding the
    // conditions it was written under: staleness only starts when the
    // fault is fixed.
    let p = RuleProvenance {
        written_during_outage: true,
        fault_fixed: false,
        revalidated_after_fix: false,
    };
    assert!(p.finding().is_none());
}

// ── Rendering ───────────────────────────────────────────────────────────────

#[test]
fn verdict_lines_carry_the_numbers_they_rest_on() {
    let rule = ReapRule::with_defaults();

    let healthy = AgentObservation {
        name: "as-4534".to_string(),
        phase: Phase::Agent,
        age: Duration::from_secs(8 * 3600 + 27 * 60),
        slurm_log_quiet: Duration::from_secs(5 * 60),
        agent_out_bytes: 0,
    };
    let line = rule.decision(&healthy).line(&healthy);
    assert!(line.contains("as-4534"), "{line}");
    assert!(line.contains("phase=agent"), "{line}");
    assert!(line.contains("8:27:00"), "{line}");
    assert!(line.contains("keep: log advancing"), "{line}");

    let wedged = AgentObservation {
        name: "as-4542".to_string(),
        phase: Phase::Copy,
        age: Duration::from_secs(40 * 60),
        slurm_log_quiet: Duration::from_secs(30 * 60),
        agent_out_bytes: 0,
    };
    let line = rule.decision(&wedged).line(&wedged);
    assert!(line.contains("REAP"), "{line}");
    assert!(line.contains("phase=copy"), "{line}");
    assert!(line.contains("30:00"), "{line}");

    // The rule line names every phase budget: one number cannot serve
    // both, so the rule shows it does not.
    let rule_line = rule.line();
    assert!(rule_line.contains("copy 20:00"), "{rule_line}");
    assert!(rule_line.contains("agent 12:00:00"), "{rule_line}");
    assert!(rule_line.contains("gate 30:00"), "{rule_line}");
}
