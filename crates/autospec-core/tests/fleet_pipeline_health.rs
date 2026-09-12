//! Integration tests for fleet pipeline health (issue #4068).
//!
//! The acceptance criteria, each a test:
//!
//! 1. A repository with open issues, staged specs and no agents reports
//!    stalled.
//! 2. One with no staged work reports idle.
//! 3. One with agents running reports healthy.
//! 4. A dispatcher declared operator-driven does not report stalled.
//!
//! Plus the incident itself, reconstructed: the gateway pipeline with 15
//! open issues, 45 staged specs and zero agents on a dispatcher that
//! appears in no crontab — and the fleet report that would have surfaced
//! it.

use autospec_core::fleet_pipeline_health::{
    classify, format_age, DispatcherMode, FleetHealthReport, PipelineState, RepoPipeline,
    RepoReport, DEFAULT_STALL_THRESHOLD_SECS,
};

fn pipeline(open: u32, staged: u32, agents: u32, last: Option<u64>) -> RepoPipeline {
    RepoPipeline {
        open_issues: open,
        staged_specs: staged,
        agents_running: agents,
        secs_since_last_dispatch: last,
    }
}

#[test]
fn ac1_work_and_zero_agents_past_the_interval_reports_stalled() {
    let report = RepoReport::new(
        "inferweave-gateway",
        &pipeline(15, 45, 0, Some(2 * 3600)),
        DispatcherMode::Scheduled,
        DEFAULT_STALL_THRESHOLD_SECS,
    );

    assert_eq!(report.state, PipelineState::Stalled);
    assert!(report.line().contains("stalled"), "line: {}", report.line());
}

#[test]
fn ac2_no_staged_work_reports_idle() {
    let report = RepoReport::new(
        "inferweave-gateway",
        &pipeline(15, 0, 0, Some(2 * 3600)),
        DispatcherMode::Scheduled,
        DEFAULT_STALL_THRESHOLD_SECS,
    );

    assert_eq!(report.state, PipelineState::Idle);
    assert!(report.line().contains("idle"));
}

#[test]
fn ac3_agents_running_reports_healthy() {
    let report = RepoReport::new(
        "autospec",
        &pipeline(8, 9, 22, Some(600)),
        DispatcherMode::Scheduled,
        DEFAULT_STALL_THRESHOLD_SECS,
    );

    assert_eq!(report.state, PipelineState::Healthy);
    assert!(report.line().contains("healthy"));
}

#[test]
fn ac4_operator_driven_does_not_report_stalled() {
    let report = RepoReport::new(
        "manual",
        &pipeline(15, 45, 0, Some(72 * 3600)),
        DispatcherMode::OperatorDriven,
        DEFAULT_STALL_THRESHOLD_SECS,
    );

    assert_eq!(report.state, PipelineState::OperatorDriven);
    assert!(
        !report.line().contains("stalled"),
        "line: {}",
        report.line()
    );
}

#[test]
fn stalled_and_idle_must_not_render_the_same() {
    let stalled = RepoReport::new(
        "inferweave-gateway",
        &pipeline(15, 45, 0, Some(2 * 3600)),
        DispatcherMode::Scheduled,
        DEFAULT_STALL_THRESHOLD_SECS,
    );
    let idle = RepoReport::new(
        "inferweave-gateway",
        &pipeline(15, 0, 0, Some(2 * 3600)),
        DispatcherMode::Scheduled,
        DEFAULT_STALL_THRESHOLD_SECS,
    );

    assert_ne!(stalled.line(), idle.line());
}

#[test]
fn the_incident_is_reconstructed_and_surfaced() {
    // Three repositories are served by the fleet. Only one has a scheduler.
    let fleet = FleetHealthReport::new(vec![
        RepoReport::new(
            "autospec",
            &pipeline(6, 7, 22, Some(600)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        ),
        RepoReport::new(
            "inferweave-gateway",
            &pipeline(15, 45, 0, Some(5 * 3600 + 1800)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        ),
        RepoReport::new(
            "inferweave",
            &pipeline(3, 0, 2, Some(1200)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        ),
    ]);

    // The report covers every repository the fleet serves, not only the
    // one with a scheduler.
    let line = fleet.line();
    for repo in ["autospec", "inferweave-gateway", "inferweave"] {
        assert!(line.contains(repo), "missing {repo} in: {line}");
    }

    // The gateway pipeline — 15 open, 45 staged, zero agents — is the
    // stalled one, and it is named in the report.
    let stalled = fleet.stalled();
    assert_eq!(stalled.len(), 1);
    assert_eq!(stalled[0].repo, "inferweave-gateway");
    assert!(line.contains("15 open, 45 staged"));
    assert!(line.contains("stalled (no agents for 5h30m"));
}

#[test]
fn the_classify_primitive_is_the_single_decision() {
    assert_eq!(
        classify(
            &pipeline(15, 45, 0, Some(2 * 3600)),
            DispatcherMode::Scheduled,
            3600
        ),
        PipelineState::Stalled
    );
    assert_eq!(
        classify(
            &pipeline(15, 0, 0, Some(2 * 3600)),
            DispatcherMode::Scheduled,
            3600
        ),
        PipelineState::Idle
    );
    assert_eq!(
        classify(
            &pipeline(15, 45, 0, Some(600)),
            DispatcherMode::Scheduled,
            3600
        ),
        PipelineState::Settling
    );
    assert_eq!(
        classify(&pipeline(0, 0, 0, None), DispatcherMode::Scheduled, 3600),
        PipelineState::Idle
    );
    // Never dispatched with work waiting is stalled, not "unknown".
    assert_eq!(
        classify(&pipeline(1, 1, 0, None), DispatcherMode::Scheduled, 3600),
        PipelineState::Stalled
    );
    // "Longer than the interval", not "at or beyond".
    assert_eq!(
        classify(
            &pipeline(1, 1, 0, Some(3600)),
            DispatcherMode::Scheduled,
            3600
        ),
        PipelineState::Settling
    );
    // Operator-driven wins over any numbers.
    assert_eq!(
        classify(
            &pipeline(15, 45, 0, None),
            DispatcherMode::OperatorDriven,
            3600
        ),
        PipelineState::OperatorDriven
    );
}

#[test]
fn the_threshold_is_configurable() {
    let short = RepoReport::new(
        "r",
        &pipeline(1, 1, 0, Some(2000)),
        DispatcherMode::Scheduled,
        1800,
    );
    assert_eq!(short.state, PipelineState::Stalled);

    let long = RepoReport::new(
        "r",
        &pipeline(1, 1, 0, Some(2000)),
        DispatcherMode::Scheduled,
        7200,
    );
    assert_eq!(long.state, PipelineState::Settling);
}

#[test]
fn ages_render_compactly() {
    assert_eq!(format_age(45), "45s");
    assert_eq!(format_age(38 * 60), "38m");
    assert_eq!(format_age(5 * 3600 + 1800), "5h30m");
}
