//! Per-repository fleet health (issue #4068).
//!
//! The four acceptance scenarios, in the order the incident produced them:
//!
//! * a repository with open issues, staged specs and zero agents for longer
//!   than the interval reports `stalled`, even though every component the old
//!   status line reported — scheduler script, worker pool, queue file — was
//!   present;
//! * `stalled` and `idle` never render the same line, so a pipeline that
//!   stopped is not read as a pipeline with nothing to do;
//! * a dispatcher declared operator-driven reports its own state and never
//!   stalls: the absence of a schedule is a recorded decision;
//! * the report covers every repository the fleet serves, including the one
//!   with no scheduler entry on this machine, and an unobserved repository is
//!   never rendered as idle.

use std::time::Duration;

use autospec_core::fleet_health::{
    assess, DispatchMode, FleetHealthPolicy, FleetReport, RepoHealth, RepoObservation, RepoState,
    DEFAULT_STALL_AFTER,
};

fn hours(n: u64) -> Duration {
    Duration::from_secs(n * 3_600)
}

fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

/// A repository at work: agents running, work in the queue.
fn healthy_repo() -> RepoState {
    RepoState {
        repo: "autospec".to_string(),
        dispatch: DispatchMode::Scheduled,
        declared_in: "cron: */10 dispatch-gw.sh".to_string(),
        observation: Some(RepoObservation {
            open_issues: 251,
            staged_specs: 15,
            agents_in_flight: 2,
            last_dispatch: Some(minutes(4)),
        }),
    }
}

/// The incident's repository: open issues, staged specs, nobody running, and
/// no dispatch since before the weekend.
fn stalled_repo() -> RepoState {
    RepoState {
        repo: "inferweave-gateway".to_string(),
        dispatch: DispatchMode::Undeclared,
        declared_in: String::new(),
        observation: Some(RepoObservation {
            open_issues: 15,
            staged_specs: 3,
            agents_in_flight: 0,
            last_dispatch: Some(hours(72)),
        }),
    }
}

/// A repository with open issues but no staged spec: genuinely nothing an
/// agent could be handed.
fn idle_repo() -> RepoState {
    RepoState {
        repo: "docs-site".to_string(),
        dispatch: DispatchMode::Scheduled,
        declared_in: "cron: */30 dispatch-docs-site.sh".to_string(),
        observation: Some(RepoObservation {
            open_issues: 7,
            staged_specs: 0,
            agents_in_flight: 0,
            last_dispatch: Some(hours(30)),
        }),
    }
}

/// The served list of the fleet as it stood at the time: every repository the
/// fleet is meant to serve, including `inferweave-gateway`, which had no
/// scheduler entry on the machine the status line was read from.
fn served() -> Vec<String> {
    ["autospec", "inferweave-gateway", "docs-site"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn states() -> Vec<RepoState> {
    vec![healthy_repo(), stalled_repo(), idle_repo()]
}

// --- Invariant 1: the report is per repository, over the served list -------

#[test]
fn every_served_repository_gets_an_entry_even_the_one_with_no_scheduler() {
    let report = FleetReport::build(&served(), &states(), &FleetHealthPolicy::default());

    assert_eq!(report.repos.len(), 3);
    let names: Vec<&str> = report.repos.iter().map(|r| r.repo.as_str()).collect();
    assert_eq!(names, vec!["autospec", "inferweave-gateway", "docs-site"]);
}

#[test]
fn a_served_repository_with_no_observation_is_unobserved_never_idle() {
    // The gateway was reached by no probe at all. Rendering that as "nothing to
    // do" is how three days went unnoticed.
    let report = FleetReport::build(
        &served(),
        &[healthy_repo(), idle_repo()],
        &FleetHealthPolicy::default(),
    );

    let missing = report
        .repos
        .iter()
        .find(|r| r.repo == "inferweave-gateway")
        .expect("every served repository has an entry");
    assert_eq!(missing.health, RepoHealth::Unobserved);
    assert_eq!(missing.health.label(), "UNOBSERVED");
    assert!(missing.health.needs_attention());
    assert!(missing
        .line()
        .contains("never observed: no verdict possible"));
    assert!(!missing.line().contains("nothing to do"));
    assert_eq!(report.unobserved().len(), 1);
}

#[test]
fn an_observed_repository_on_no_served_list_is_reported_not_dropped() {
    let mut extra = stalled_repo();
    extra.repo = "forgotten-repo".to_string();
    let report = FleetReport::build(
        &served(),
        &[states(), vec![extra]].concat(),
        &FleetHealthPolicy::default(),
    );

    assert_eq!(report.unserved, vec!["forgotten-repo".to_string()]);
    assert!(report.summary_line().contains("on no served list"));
    // It is reported, not silently assessed into the covered set.
    assert_eq!(report.repos.len(), 3);
}

// --- Invariant 2: stalled is not idle, and the interval is configurable ----

#[test]
fn work_present_no_agents_and_silent_past_the_interval_is_stalled() {
    let status = assess(&stalled_repo(), &FleetHealthPolicy::default());

    assert_eq!(status.health, RepoHealth::Stalled);
    assert!(status.health.is_stalled());
    assert!(status.health.needs_attention());
    assert_eq!(status.health.label(), "STALLED");
}

#[test]
fn stalled_and_idle_never_render_the_same_line() {
    let stalled = assess(&stalled_repo(), &FleetHealthPolicy::default()).line();
    let idle = assess(&idle_repo(), &FleetHealthPolicy::default()).line();

    assert_ne!(stalled, idle);
    assert!(stalled.contains("STALLED"));
    assert!(idle.contains("IDLE"));
    assert!(idle.contains("nothing to do"));
    assert!(!stalled.contains("nothing to do"));
    // Each line carries the observation it was drawn from.
    assert!(stalled.contains("staged=3"));
    assert!(idle.contains("staged=0"));
}

#[test]
fn a_quiet_repository_inside_the_window_is_not_stalled() {
    // Staged work, no agents, and a dispatch 90 s ago: the dispatcher is
    // alive, its workers are simply full.
    let mut state = stalled_repo();
    state.observation.as_mut().unwrap().last_dispatch = Some(minutes(20));

    let status = assess(
        &state,
        &FleetHealthPolicy {
            stall_after: hours(2),
        },
    );
    assert_eq!(status.health, RepoHealth::DispatchedRecently);
    assert!(!status.health.is_stalled());
}

#[test]
fn the_stall_interval_is_a_parameter_not_a_constant() {
    let mut state = stalled_repo();
    state.observation.as_mut().unwrap().last_dispatch = Some(hours(3));

    // The same observation: stalled under a 2 h window, not stalled under a
    // 4 h one, stalled under a zero window.
    assert_eq!(
        assess(
            &state,
            &FleetHealthPolicy {
                stall_after: hours(2)
            }
        )
        .health,
        RepoHealth::Stalled
    );
    assert_eq!(
        assess(
            &state,
            &FleetHealthPolicy {
                stall_after: hours(4)
            }
        )
        .health,
        RepoHealth::DispatchedRecently
    );
    assert_eq!(
        assess(
            &state,
            &FleetHealthPolicy {
                stall_after: Duration::ZERO
            }
        )
        .health,
        RepoHealth::Stalled
    );
}

#[test]
fn the_default_interval_is_exposed_for_callers_to_report() {
    assert_eq!(
        FleetHealthPolicy::default().stall_after,
        DEFAULT_STALL_AFTER
    );
    assert!(DEFAULT_STALL_AFTER >= hours(1));
}

#[test]
fn staged_work_with_no_dispatch_ever_is_stalled_at_once() {
    let mut state = stalled_repo();
    state.observation.as_mut().unwrap().last_dispatch = None;

    let status = assess(
        &state,
        &FleetHealthPolicy {
            stall_after: hours(6),
        },
    );
    assert_eq!(status.health, RepoHealth::Stalled);
    assert!(status.line().contains("no dispatch ever recorded"));
}

#[test]
fn open_issues_without_a_staged_spec_are_idle_not_stalled() {
    // Open issues are not dispatchable: nothing can be handed to an agent, so
    // the gap is upstream of the dispatcher, and the line has to say so.
    let status = assess(&idle_repo(), &FleetHealthPolicy::default());

    assert_eq!(status.health, RepoHealth::Idle);
    assert!(!status.health.needs_attention());
    assert!(status.line().contains("no staged specs"));
}

// --- Invariant 3: operator-driven is declared, or it is not ---------------

#[test]
fn a_declared_operator_driven_dispatcher_never_reports_stalled() {
    let mut state = stalled_repo();
    state.dispatch = DispatchMode::OperatorDriven;
    state.declared_in = "runbook: gateway dispatch is manual".to_string();

    let status = assess(
        &state,
        &FleetHealthPolicy {
            stall_after: minutes(1),
        },
    );

    assert_eq!(status.health, RepoHealth::AwaitingOperator);
    assert!(!status.health.is_stalled());
    assert!(status.line().contains("declared"));
    // The work waiting for a human is still visible, not swallowed.
    assert!(status.line().contains("staged=3"));
}

#[test]
fn an_operator_driven_claim_with_no_record_of_the_claim_is_undeclared() {
    // A bare assertion in nobody's notes is not a decision on file, so it
    // cannot excuse a three-day gap.
    let mut state = stalled_repo();
    state.dispatch = DispatchMode::OperatorDriven;
    state.declared_in = "   ".to_string();

    assert_eq!(state.effective_mode(), DispatchMode::Undeclared);
    assert_eq!(
        assess(&state, &FleetHealthPolicy::default()).health,
        RepoHealth::Stalled
    );
}

#[test]
fn an_undeclared_dispatch_mode_is_listed_even_when_the_repo_is_healthy() {
    // The gateway's real defect: nobody had said which way it was driven, so
    // its silence meant nothing either way.
    let status_repo = RepoState {
        dispatch: DispatchMode::Undeclared,
        declared_in: String::new(),
        ..healthy_repo()
    };

    let status = assess(&status_repo, &FleetHealthPolicy::default());
    assert_eq!(status.health, RepoHealth::Healthy);
    assert!(status.line().contains("dispatch mode undeclared"));

    let served_one = vec![status_repo.repo.clone()];
    let report = FleetReport::build(&served_one, &[status_repo], &FleetHealthPolicy::default());
    assert_eq!(report.undeclared_dispatchers().len(), 1);
}

#[test]
fn dispatch_mode_tokens_parse_and_round_trip() {
    assert_eq!(
        DispatchMode::parse("operator-driven"),
        DispatchMode::OperatorDriven
    );
    assert_eq!(DispatchMode::parse("CRON"), DispatchMode::Scheduled);
    assert_eq!(DispatchMode::parse(""), DispatchMode::Undeclared);
    assert_eq!(DispatchMode::parse("maybe-async"), DispatchMode::Undeclared);
    for mode in [
        DispatchMode::Scheduled,
        DispatchMode::OperatorDriven,
        DispatchMode::Undeclared,
    ] {
        assert_eq!(DispatchMode::parse(mode.as_str()), mode);
    }
}

// --- Invariant 4: outcome first, observation beside it --------------------

#[test]
fn a_repo_with_agents_running_is_healthy_however_long_since_it_dispatched() {
    let mut state = stalled_repo();
    state.observation.as_mut().unwrap().agents_in_flight = 1;

    let status = assess(
        &state,
        &FleetHealthPolicy {
            stall_after: minutes(1),
        },
    );
    assert_eq!(status.health, RepoHealth::Healthy);
    assert!(!status.health.needs_attention());
}

#[test]
fn the_summary_line_leads_with_the_stall_count_and_never_calls_a_stall_idle() {
    let report = FleetReport::build(&served(), &states(), &FleetHealthPolicy::default());
    let summary = report.summary_line();

    assert_eq!(report.stalled().len(), 1);
    assert_eq!(report.stalled()[0].repo, "inferweave-gateway");
    assert!(summary.starts_with("3 repos in "));
    // The stalled count is the first thing on the line.
    assert!(summary.contains("1 stalled"));
    assert!(summary.find("1 stalled") < summary.find("1 idle"));
    assert!(!summary.contains("nothing to do"));
}

#[test]
fn an_all_idle_fleet_summarizes_as_idle() {
    let report = FleetReport::build(
        &["docs-site".to_string()],
        &[idle_repo()],
        &FleetHealthPolicy::default(),
    );

    assert!(report.stalled().is_empty());
    assert!(report.summary_line().contains("1 idle"));
}

#[test]
fn the_rendered_report_has_one_line_per_repo_plus_the_summary() {
    let report = FleetReport::build(&served(), &states(), &FleetHealthPolicy::default());
    let rendered = report.render();
    let lines: Vec<&str> = rendered.trim_end().lines().collect();

    assert_eq!(lines.len(), 4);
    assert_eq!(
        lines[1],
        assess(&stalled_repo(), &FleetHealthPolicy::default()).line()
    );
    assert_eq!(lines[3], report.summary_line());
}
