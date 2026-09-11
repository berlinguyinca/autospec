//! Bounded agent calls and the watchdog that reaps the ones that escape
//! them (issue #4258).
//!
//! The fleet's numbers pin the fixtures: the model call was bounded by
//! `timeout 2700 pi --print ...` — 45 minutes, SIGTERM only — and 19 of 21
//! agents ran past it, one by 6 hours 50 minutes, each holding a GPU slot.
//! The fixed wrapper is `timeout -k 60 2700 pi --print ...`: SIGTERM at
//! 2700s, SIGKILL 60s later. The reaper fires at `limit + 30m` on
//! supervisor bookkeeping, kills at most 5 per sweep, and logs a summary
//! line every run, even an idle one.

use std::time::Duration;

use autospec_core::agent_watchdog::{
    inspect_timeout_command, liveness, verify_call, watchdog_sweep, AgentObservation, Escalation,
    EscalationError, Liveness, ProcessState, ProgressArtifact, SupervisorVerdict, TimeoutCommand,
    WatchdogPolicy,
};

/// The fleet's limit: 45 minutes.
fn limit() -> Duration {
    Duration::from_secs(2700)
}

/// The fixed wrapper: SIGTERM at 2700s, SIGKILL 60s later.
fn esc() -> Escalation {
    Escalation::new(limit(), Duration::from_secs(60)).unwrap()
}

/// The reaper's policy as the fleet deployed it: 30-minute detection
/// margin, 5 kills per sweep.
fn policy() -> WatchdogPolicy {
    WatchdogPolicy::default_for_limit(limit())
}

/// One agent as the supervisor sees it.
fn agent(id: &str, in_call: bool, call_elapsed: Duration, artifact: bool) -> AgentObservation {
    AgentObservation {
        id: id.to_string(),
        in_call,
        call_elapsed,
        artifact: ProgressArtifact { present: artifact },
    }
}

fn mins(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

// --- Invariant 1: every timeout must be escalated -------------------------

#[test]
fn zero_grace_is_rejected_bare_timeout_is_not_a_control() {
    let err = Escalation::new(limit(), Duration::ZERO).unwrap_err();
    assert_eq!(err, EscalationError::NoEscalation);
    assert_eq!(err.as_str(), "no-escalation");
}

#[test]
fn escalation_deadline_is_limit_plus_grace() {
    assert_eq!(esc().hard_deadline(), Duration::from_secs(2760));
}

#[test]
fn rendered_command_escalates_to_sigkill() {
    let cmd = esc().render_command("pi --print --provider hive --model qwen3.8-27b-q8");
    assert_eq!(
        cmd,
        "timeout -k 60 2700 pi --print --provider hive --model qwen3.8-27b-q8"
    );
}

#[test]
fn original_defective_wrapper_is_bare() {
    // The command the fleet actually ran.
    let cmd = inspect_timeout_command(
        "timeout 2700 pi --print --provider hive --model qwen3.8-27b-q8 \
         --thinking-level high -p \"$PROMPT\" 2>&1 | tee $LOG_FILE",
    );
    assert_eq!(cmd, TimeoutCommand::Bare { limit: limit() });
    assert!(!cmd.is_control());
}

#[test]
fn fixed_wrapper_is_escalating() {
    let cmd = inspect_timeout_command("timeout -k 60 2700 pi --print --provider hive");
    assert_eq!(
        cmd,
        TimeoutCommand::Escalating {
            limit: limit(),
            grace: Duration::from_secs(60)
        }
    );
    assert!(cmd.is_control());
}

#[test]
fn kill_after_long_form_is_escalating() {
    let cmd = inspect_timeout_command("timeout --kill-after=60 2700 pi --print");
    assert_eq!(
        cmd,
        TimeoutCommand::Escalating {
            limit: limit(),
            grace: Duration::from_secs(60)
        }
    );
}

#[test]
fn unbounded_call_has_no_timeout_wrapper() {
    assert_eq!(
        inspect_timeout_command("pi --print --provider hive"),
        TimeoutCommand::Unbounded
    );
    assert_eq!(
        inspect_timeout_command("bash agent.sh"),
        TimeoutCommand::Unbounded
    );
}

#[test]
fn other_options_and_duration_suffixes_do_not_mislead_the_inspector() {
    // Unknown options are skipped (their values are not the limit); GNU
    // duration suffixes parse.
    assert_eq!(
        inspect_timeout_command("timeout -s TERM 45m pi --print"),
        TimeoutCommand::Bare { limit: limit() }
    );
    assert_eq!(
        inspect_timeout_command("timeout -k 1m 45m pi --print"),
        TimeoutCommand::Escalating {
            limit: limit(),
            grace: Duration::from_secs(60)
        }
    );
}

#[test]
fn rendered_command_round_trips_through_the_inspector() {
    let cmd = esc().render_command("pi --print --provider hive");
    assert_eq!(
        inspect_timeout_command(&cmd),
        TimeoutCommand::Escalating {
            limit: limit(),
            grace: Duration::from_secs(60)
        }
    );
}

// --- Invariant 2: the supervisor asserts the child died -------------------

#[test]
fn six_hours_fifty_minutes_past_the_limit_is_a_child_that_outlived_the_bound() {
    // The observed worst case: limit 45m, child gone after 6h50m.
    let verdict = verify_call(&esc(), Duration::from_secs(6 * 3600 + 50 * 60), true);
    assert_eq!(verdict, SupervisorVerdict::ChildOutlivedBound);
}

#[test]
fn child_gone_exactly_at_the_hard_deadline_is_escalated_not_a_violation() {
    // 2700 + 60 = 2760: the SIGKILL landed on the last possible tick.
    assert_eq!(
        verify_call(&esc(), Duration::from_secs(2760), true),
        SupervisorVerdict::Escalated
    );
    // One tick past the deadline: the signal never landed.
    assert_eq!(
        verify_call(&esc(), Duration::from_secs(2761), true),
        SupervisorVerdict::ChildOutlivedBound
    );
}

#[test]
fn call_returning_at_or_before_the_limit_is_in_time() {
    assert_eq!(
        verify_call(&esc(), Duration::from_secs(600), true),
        SupervisorVerdict::InTime
    );
    assert_eq!(
        verify_call(&esc(), limit(), true),
        SupervisorVerdict::InTime
    );
    // One second past the limit, still inside the grace window: the
    // escalation did its job.
    assert_eq!(
        verify_call(&esc(), limit() + Duration::from_secs(1), true),
        SupervisorVerdict::Escalated
    );
}

#[test]
fn a_child_not_yet_observed_gone_is_not_verified_even_if_long_gone() {
    // Sending the signal is not killing: at 46 minutes the supervisor has
    // not regained control, the verdict is NotVerified, and at 7 hours it
    // is still NotVerified — the child has never been *observed* dead.
    assert_eq!(
        verify_call(&esc(), mins(46), false),
        SupervisorVerdict::NotVerified
    );
    assert_eq!(
        verify_call(&esc(), Duration::from_secs(7 * 3600), false),
        SupervisorVerdict::NotVerified
    );
}

// --- Invariant 3: liveness is output, never process state -----------------

#[test]
fn a_running_job_with_no_artifact_is_stalled() {
    // "The job is RUNNING" was true for all 19 hung agents.
    assert_eq!(
        liveness(ProcessState::Running, &ProgressArtifact { present: false }),
        Liveness::Stalled
    );
}

#[test]
fn a_running_job_with_an_artifact_is_alive() {
    // build.log is written immediately after the model call returns: its
    // presence says the call completed.
    assert_eq!(
        liveness(ProcessState::Running, &ProgressArtifact { present: true }),
        Liveness::Alive
    );
}

#[test]
fn process_state_is_ignored_in_both_directions() {
    // Exited is not liveness either: a process that exited without the
    // artifact never produced the progress.
    assert_eq!(
        liveness(ProcessState::Exited, &ProgressArtifact { present: false }),
        Liveness::Stalled
    );
    assert_eq!(
        liveness(ProcessState::Exited, &ProgressArtifact { present: true }),
        Liveness::Alive
    );
}

// --- Invariant 4: the detector cannot be fooled by buffering --------------

#[test]
fn a_young_in_call_agent_with_empty_output_is_healthy() {
    // 0 bytes in the redirect target is what a *working* process looks
    // like while output is buffered — it is not a hang signal, and
    // AgentObservation carries no output-size field for the detector to
    // be fooled by.
    let sweep = watchdog_sweep(&[agent("a1", true, mins(44), false)], &policy());
    assert_eq!(sweep.checked, 1);
    assert_eq!(sweep.healthy, 1);
    assert!(sweep.killed.is_empty());
}

#[test]
fn past_the_limit_but_inside_the_margin_is_not_yet_hung() {
    // limit + 29m: the escalation and a slow return still have room to
    // land before the reaper acts.
    let sweep = watchdog_sweep(&[agent("a1", true, mins(74), false)], &policy());
    assert_eq!(sweep.healthy, 1);
    assert!(sweep.killed.is_empty());
}

#[test]
fn still_inside_the_call_past_limit_plus_margin_with_no_artifact_is_hung() {
    // limit + 31m, no build.log: the call should have been SIGKILLed at
    // limit + 1m; it was not.
    let sweep = watchdog_sweep(&[agent("a1", true, mins(76), false)], &policy());
    assert_eq!(sweep.killed, vec!["a1".to_string()]);
}

#[test]
fn at_exactly_limit_plus_margin_the_detector_holds() {
    // The detector is strict: 45m + 30m exactly is not yet past the bound.
    let sweep = watchdog_sweep(&[agent("a1", true, mins(75), false)], &policy());
    assert_eq!(sweep.healthy, 1);
    assert!(sweep.killed.is_empty());
}

#[test]
fn an_agent_past_the_bound_but_with_an_artifact_is_in_the_build_phase_not_hung() {
    // build.log exists: the call returned; the agent is building. The
    // build takes legitimately long and is not inside the call.
    let sweep = watchdog_sweep(&[agent("a1", true, mins(120), true)], &policy());
    assert_eq!(sweep.healthy, 1);
    assert!(sweep.killed.is_empty());
}

#[test]
fn a_returned_call_is_never_killed_it_is_reported_as_a_supervisor_violation() {
    // The 6h50m call, once the wrapper finally regains control: the child
    // is gone, so the reaper has nothing to kill — but the supervisor
    // failed to verify the child died at limit + grace, and the sweep
    // names it.
    let sweep = watchdog_sweep(
        &[agent(
            "a1",
            false,
            Duration::from_secs(6 * 3600 + 50 * 60),
            false,
        )],
        &policy(),
    );
    assert!(sweep.killed.is_empty());
    assert_eq!(sweep.supervisor_violations, vec!["a1".to_string()]);
    // A violation is not healthy, but it is not hung either.
    assert_eq!(sweep.checked, 1);
    assert_eq!(sweep.healthy, 0);
}

// --- Invariant 5: the reaper caps its blast radius and always speaks ------

#[test]
fn first_real_sweep_caps_kills_at_five_and_defers_the_rest() {
    // The fleet's first sweep: 21 agents observed, 13 healthy (8 in the
    // build phase with build.log, 5 young in-call), 8 hung — and only 5
    // die. If the detection is ever wrong, 5 is recoverable; 22 is not.
    let mut agents: Vec<AgentObservation> = Vec::new();
    for i in 1..=8 {
        // In the build phase: call returned, artifact present.
        agents.push(agent(
            &format!("build-{i}"),
            true,
            mins(90 + i as u64),
            true,
        ));
    }
    for i in 1..=5 {
        // Young in-call: buffered output, well inside the limit.
        agents.push(agent(&format!("young-{i}"), true, mins(5 * i), false));
    }
    for i in 1..=8 {
        // Hung: inside the call 3+ hours, no artifact, 0 bytes.
        agents.push(agent(
            &format!("hung-{i}"),
            true,
            mins(150 + i as u64),
            false,
        ));
    }
    assert_eq!(agents.len(), 21);

    let sweep = watchdog_sweep(&agents, &policy());
    assert_eq!(sweep.checked, 21);
    assert_eq!(sweep.healthy, 13);
    // Sorted-id order, capped at 5: hung-1..hung-5 die, hung-6..8 wait.
    assert_eq!(
        sweep.killed,
        (1..=5).map(|i| format!("hung-{i}")).collect::<Vec<_>>()
    );
    assert_eq!(
        sweep.deferred,
        (6..=8).map(|i| format!("hung-{i}")).collect::<Vec<_>>()
    );
    assert!(sweep.supervisor_violations.is_empty());
    assert_eq!(
        sweep.summary_line(),
        "checked=21 healthy=13 hung_killed=5 hung_deferred=3"
    );
}

#[test]
fn an_idle_sweep_still_renders_its_summary_line() {
    // The watchdog logs one line every run, even when it did nothing — so
    // a dead watchdog is visible.
    let sweep = watchdog_sweep(&[agent("a1", true, mins(3), false)], &policy());
    assert_eq!(sweep.summary_line(), "checked=1 healthy=1 hung_killed=0");
}

#[test]
fn the_sweep_counts_reconcile() {
    // healthy + killed + deferred + violations == checked, always.
    let agents = vec![
        agent("young", true, mins(10), false),
        agent("building", true, mins(100), true),
        agent("h1", true, mins(200), false),
        agent("h2", true, mins(210), false),
        agent("late-return", false, Duration::from_secs(7 * 3600), false),
    ];
    let sweep = watchdog_sweep(&agents, &policy());
    let accounted = sweep.healthy
        + sweep.killed.len()
        + sweep.deferred.len()
        + sweep.supervisor_violations.len();
    assert_eq!(accounted, sweep.checked);
    assert_eq!(sweep.healthy, 2);
    assert_eq!(sweep.killed.len(), 2);
}

#[test]
fn zero_cap_defers_everything_and_kills_nothing() {
    // A misconfigured (or deliberately quiet) reaper reports instead of
    // acting: all 3 hung, none killed.
    let p = WatchdogPolicy {
        kill_cap: 0,
        ..policy()
    };
    let agents = vec![
        agent("a", true, mins(200), false),
        agent("b", true, mins(210), false),
        agent("c", true, mins(220), false),
    ];
    let sweep = watchdog_sweep(&agents, &p);
    assert!(sweep.killed.is_empty());
    assert_eq!(sweep.deferred.len(), 3);
    assert_eq!(
        sweep.summary_line(),
        "checked=3 healthy=0 hung_killed=0 hung_deferred=3"
    );
}

#[test]
fn the_killed_set_is_deterministic_in_the_input_not_its_order() {
    let base = [
        agent("h3", true, mins(180), false),
        agent("h1", true, mins(170), false),
        agent("h2", true, mins(190), false),
    ];
    let mut reversed = base.to_vec();
    reversed.reverse();
    let first = watchdog_sweep(&base, &policy());
    let second = watchdog_sweep(&reversed, &policy());
    assert_eq!(first.killed, second.killed);
    assert_eq!(first.killed, vec!["h1", "h2", "h3"]);
}
