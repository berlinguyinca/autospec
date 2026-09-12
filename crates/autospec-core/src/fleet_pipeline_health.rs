//! Fleet pipeline health (issue #4068).
//!
//! A pipeline is a queue, a dispatcher, and the scheduler that runs the
//! dispatcher. The third is easy to omit because it is not code — it is a
//! line in a crontab — and its absence produces no error, no log line, and
//! no failed run. The gateway pipeline had 15 open issues, 45 staged specs
//! and zero agents: the dispatcher existed and worked, the specs were
//! staged, the workers had slots, and nothing anywhere reported that
//! nothing runs the dispatcher.
//!
//! A dispatcher that crashes leaves a stack trace; a dispatcher that is
//! never invoked leaves nothing at all, and every component health check
//! keeps passing. The remedy is to measure the outcome, not the
//! components: three numbers per repository — open issues, staged specs,
//! agents running — plus the time since the last dispatch. The combination
//! `open > 0, staged > 0, running = 0` sustained past a threshold is the
//! whole condition for "stalled".
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **The report covers every repository the fleet serves**, not only
//!    the one with a scheduler: open issues, staged specs, agents in
//!    flight, and time since the last dispatch ([`RepoReport::line`],
//!    [`FleetHealthReport::line`]).
//! 2. **Stalled is a distinct state from "no work available" and must not
//!    render the same** ([`PipelineState::Stalled`] vs
//!    [`PipelineState::Idle`]). Stalled requires dispatchable work
//!    (open > 0 and staged > 0) AND zero agents sustained past the
//!    configurable threshold — a never-dispatched pipeline with waiting
//!    work is stalled, not "unknown".
//! 3. **An operator-driven dispatcher is a declared decision, not an
//!    omission.** Where dispatching is intended to be operator-driven
//!    rather than scheduled, that is declared ([`DispatcherMode::OperatorDriven`])
//!    and the repository reports `operator-driven`, never `stalled` — the
//!    recorded decision is indistinguishable from an accident unless it
//!    is said.

/// Default stall interval: a repository with staged, dispatchable work and
/// zero agents is reported stalled once the last dispatch is older than
/// this, or if it has never dispatched at all.
pub const DEFAULT_STALL_THRESHOLD_SECS: u64 = 3600;

/// Whether the repository's dispatcher is scheduled or intentionally
/// operator-driven.
///
/// The distinction is a recorded decision, not an inference: a dispatcher
/// that is "simply not on a schedule" is an omission indistinguishable
/// from the incident, and one that is operator-driven by decision must
/// be able to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatcherMode {
    /// The dispatcher is expected to run on a schedule (cron, monitor,
    /// top-up loop). A long silence with waiting work is a defect the
    /// report detects.
    Scheduled,
    /// The dispatcher is intentionally operator-driven. "Not on a
    /// schedule" is the recorded decision, and the repository is never
    /// reported stalled on account of it.
    OperatorDriven,
}

/// One repository's four numbers, read at one moment.
///
/// These are the outcome numbers the report must carry for every
/// repository the fleet serves — the state of the pipeline, not the
/// health of its components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoPipeline {
    /// Issues open and awaiting work.
    pub open_issues: u32,
    /// Specs staged and ready to dispatch.
    pub staged_specs: u32,
    /// Agents currently running against this repository.
    pub agents_running: u32,
    /// Seconds since the last dispatch, or `None` if the dispatcher has
    /// never dispatched. `None` is not "unknown" to be defaulted away —
    /// a dispatcher that has never run is exactly the incident, and it is
    /// read as stalled wherever dispatchable work is waiting.
    pub secs_since_last_dispatch: Option<u64>,
}

/// The repository's pipeline state, each rendering differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    /// Agents are running. The pipeline is moving.
    Healthy,
    /// Dispatchable work is present and the last dispatch is within the
    /// stall window — not yet stale; the next sweep re-checks. Distinct
    /// from stalled (work is waiting, the silence is young) and from idle
    /// (work exists).
    Settling,
    /// Dispatchable work has been waiting with zero agents for longer
    /// than the stall interval (or the dispatcher has never run). The
    /// state the report exists to surface: every component is healthy,
    /// nothing is dispatching.
    Stalled,
    /// No dispatchable work: nothing is open and staged. The pipeline is
    /// quiet because there is nothing to do — a different fact from
    /// stalled, and it renders as one.
    Idle,
    /// The dispatcher is declared operator-driven. The silence is a
    /// recorded decision, never a detected stall.
    OperatorDriven,
}

/// The stall decision for one repository: the numbers, the declared
/// dispatcher mode, the threshold that was applied, and the state that
/// follows from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoReport {
    /// The repository the report is about.
    pub repo: String,
    pub pipeline: RepoPipeline,
    pub mode: DispatcherMode,
    pub state: PipelineState,
    /// The stall interval applied, in seconds.
    pub stall_threshold_secs: u64,
}

impl RepoReport {
    /// Classify the repository under its declared dispatcher mode and
    /// stall threshold. Construction is the only way to get a report, so
    /// the state always agrees with the numbers it renders.
    pub fn new(
        repo: impl Into<String>,
        pipeline: &RepoPipeline,
        mode: DispatcherMode,
        stall_threshold_secs: u64,
    ) -> Self {
        Self {
            repo: repo.into(),
            pipeline: *pipeline,
            mode,
            state: classify(pipeline, mode, stall_threshold_secs),
            stall_threshold_secs,
        }
    }

    /// The one-line report for this repository. The numbers come first,
    /// the state last; each state renders differently, and stalled never
    /// renders the same as idle.
    pub fn line(&self) -> String {
        format!(
            "{}: open={} staged={} agents={} last_dispatch={} -> {}",
            self.repo,
            self.pipeline.open_issues,
            self.pipeline.staged_specs,
            self.pipeline.agents_running,
            self.last_dispatch_fragment(),
            self.state_fragment()
        )
    }

    fn last_dispatch_fragment(&self) -> String {
        match self.pipeline.secs_since_last_dispatch {
            None => "never".to_string(),
            Some(secs) => format!("{age} ago", age = format_age(secs)),
        }
    }

    fn state_fragment(&self) -> String {
        match self.state {
            PipelineState::Healthy => "healthy".to_string(),
            PipelineState::Settling => format!(
                "settling (last dispatch within the {} window)",
                format_age(self.stall_threshold_secs)
            ),
            PipelineState::Stalled => {
                let silence = self
                    .pipeline
                    .secs_since_last_dispatch
                    .map(format_age)
                    .unwrap_or_else(|| "never".to_string());
                format!(
                    "stalled (no agents for {silence}; {} open, {} staged)",
                    self.pipeline.open_issues, self.pipeline.staged_specs
                )
            }
            PipelineState::Idle => "idle (no dispatchable work)".to_string(),
            PipelineState::OperatorDriven => {
                "operator-driven (dispatch is a recorded decision, not a schedule)".to_string()
            }
        }
    }
}

/// Classify a repository's pipeline state.
///
/// The decision order is deliberate:
///
/// - an operator-driven dispatcher reports its decision first — it is
///   never stalled, whatever the numbers say;
/// - running agents mean the pipeline is moving, regardless of backlog;
/// - stalled requires *both* dispatchable work (`open > 0` and
///   `staged > 0`) and silence: no agents, and the last dispatch older
///   than the threshold — or never. Work with a recent dispatch is
///   settling, not stalled, and re-checked by the next sweep;
/// - everything else is idle: the queue is quiet because there is
///   nothing to dispatch.
pub fn classify(
    pipeline: &RepoPipeline,
    mode: DispatcherMode,
    stall_threshold_secs: u64,
) -> PipelineState {
    if mode == DispatcherMode::OperatorDriven {
        return PipelineState::OperatorDriven;
    }
    if pipeline.agents_running > 0 {
        return PipelineState::Healthy;
    }
    let dispatchable_work = pipeline.open_issues > 0 && pipeline.staged_specs > 0;
    if !dispatchable_work {
        return PipelineState::Idle;
    }
    match pipeline.secs_since_last_dispatch {
        // Never dispatched, with work waiting: the incident, exactly.
        None => PipelineState::Stalled,
        // "Longer than the configurable interval": at the threshold the
        // silence is still within the window.
        Some(secs) if secs > stall_threshold_secs => PipelineState::Stalled,
        Some(_) => PipelineState::Settling,
    }
}

/// A fleet-wide health report: one [`RepoReport`] per repository the
/// fleet serves, so a pipeline with no scheduler cannot be absent from
/// the picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetHealthReport {
    repos: Vec<RepoReport>,
}

impl FleetHealthReport {
    /// Build the report over the given per-repository reports.
    pub fn new(repos: Vec<RepoReport>) -> Self {
        Self { repos }
    }

    /// The reports, in the order given.
    pub fn repos(&self) -> &[RepoReport] {
        &self.repos
    }

    /// The repositories reported stalled.
    pub fn stalled(&self) -> Vec<&RepoReport> {
        self.repos
            .iter()
            .filter(|r| r.state == PipelineState::Stalled)
            .collect()
    }

    /// The fleet report, one line per repository.
    pub fn line(&self) -> String {
        self.repos
            .iter()
            .map(RepoReport::line)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Render an age in seconds as a short human fragment: `45s`, `38m`,
/// `5h04m`.
pub fn format_age(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(open: u32, staged: u32, agents: u32, last: Option<u64>) -> RepoPipeline {
        RepoPipeline {
            open_issues: open,
            staged_specs: staged,
            agents_running: agents,
            secs_since_last_dispatch: last,
        }
    }

    #[test]
    fn work_and_no_agents_past_threshold_is_stalled() {
        let report = RepoReport::new(
            "inferweave-gateway",
            &pipeline(15, 45, 0, Some(2 * 3600)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        );
        assert_eq!(report.state, PipelineState::Stalled);
        assert!(report.line().contains("stalled"));
    }

    #[test]
    fn no_staged_work_is_idle_never_stalled() {
        let report = RepoReport::new(
            "inferweave-gateway",
            &pipeline(15, 0, 0, Some(2 * 3600)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        );
        assert_eq!(report.state, PipelineState::Idle);
        assert!(report.line().contains("idle"));

        let stalled = RepoReport::new(
            "inferweave-gateway",
            &pipeline(15, 45, 0, Some(2 * 3600)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        );
        assert_ne!(
            report.line(),
            stalled.line(),
            "stalled must not render the same as idle"
        );
    }

    #[test]
    fn running_agents_are_healthy() {
        let report = RepoReport::new(
            "autospec",
            &pipeline(8, 9, 6, Some(240)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        );
        assert_eq!(report.state, PipelineState::Healthy);
        assert!(report.line().contains("healthy"));
    }

    #[test]
    fn operator_driven_never_reports_stalled() {
        let report = RepoReport::new(
            "manual",
            &pipeline(15, 45, 0, Some(48 * 3600)),
            DispatcherMode::OperatorDriven,
            DEFAULT_STALL_THRESHOLD_SECS,
        );
        assert_eq!(report.state, PipelineState::OperatorDriven);
        assert!(!report.line().contains("stalled"));
        assert!(report.line().contains("operator-driven"));
    }

    #[test]
    fn a_never_dispatched_pipeline_with_waiting_work_is_stalled() {
        let report = RepoReport::new(
            "inferweave-gateway",
            &pipeline(15, 45, 0, None),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        );
        assert_eq!(report.state, PipelineState::Stalled);
        assert!(
            report.line().contains("never"),
            "the never-dispatched fact must be visible: {}",
            report.line()
        );
    }

    #[test]
    fn silence_at_the_threshold_is_still_settling() {
        let at = RepoReport::new(
            "r",
            &pipeline(3, 2, 0, Some(3600)),
            DispatcherMode::Scheduled,
            3600,
        );
        assert_eq!(
            at.state,
            PipelineState::Settling,
            "longer than, not at or beyond"
        );

        let over = RepoReport::new(
            "r",
            &pipeline(3, 2, 0, Some(3601)),
            DispatcherMode::Scheduled,
            3600,
        );
        assert_eq!(over.state, PipelineState::Stalled);
    }

    #[test]
    fn recent_dispatch_with_waiting_work_is_settling() {
        let report = RepoReport::new(
            "r",
            &pipeline(3, 2, 0, Some(300)),
            DispatcherMode::Scheduled,
            3600,
        );
        assert_eq!(report.state, PipelineState::Settling);
    }

    #[test]
    fn staged_specs_without_open_issues_are_not_stalled() {
        // The issue's condition is `open > 0, staged > 0, running = 0`;
        // a staged spec whose issue is no longer open is not the
        // dispatchable work the report detects.
        let report = RepoReport::new(
            "r",
            &pipeline(0, 5, 0, Some(2 * 3600)),
            DispatcherMode::Scheduled,
            DEFAULT_STALL_THRESHOLD_SECS,
        );
        assert_eq!(report.state, PipelineState::Idle);
    }

    #[test]
    fn the_fleet_report_covers_every_repository() {
        let fleet = FleetHealthReport::new(vec![
            RepoReport::new(
                "autospec",
                &pipeline(8, 9, 22, Some(600)),
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
                "manual",
                &pipeline(4, 2, 0, None),
                DispatcherMode::OperatorDriven,
                DEFAULT_STALL_THRESHOLD_SECS,
            ),
        ]);

        let line = fleet.line();
        assert!(line.contains("autospec:"));
        assert!(line.contains("inferweave-gateway:"));
        assert!(line.contains("manual:"));
        let stalled = fleet.stalled();
        assert_eq!(stalled.len(), 1);
        assert_eq!(stalled[0].repo, "inferweave-gateway");
    }

    #[test]
    fn ages_render_compactly() {
        assert_eq!(format_age(45), "45s");
        assert_eq!(format_age(38 * 60), "38m");
        assert_eq!(format_age(5 * 3600 + 4 * 60), "5h04m");
    }
}
