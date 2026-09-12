//! `refresh-queue` is a first-class scheduled component, not a session habit
//! (#4320).
//!
//! The refresh step — the one that regenerates `queue.txt` from the live
//! tracker — is the first hop of the filing-to-dispatch pipeline, and it was
//! being run from a session: a cron entry on the operator's laptop that
//! nobody was watching, a token that could have sat on cluster-shared
//! storage, and a queue file whose `mtime` nothing checked. The queue walk
//! would then look at 132 entries, find zero eligible, and say nothing.
//! Nothing is the problem: with 118 entries already produced and 14 in
//! flight, silence was being read as "no work" instead of "the refresh is
//! not running."
//!
//! This module keeps four invariants, each with a named primitive:
//!
//! 1. **The schedule manifest declares every step on the critical path —
//!    either scheduled or explicitly manual.** [`ScheduleManifest`]
//!    enumerates the critical-path steps with a [`ScheduleMode`] each, and
//!    [`ScheduleManifest::coverage_audit`] names any critical-path step that
//!    is undeclared (`CoverageFinding::UndeclaredCriticalStep`), any step
//!    declared twice (`DuplicateStep`), and any manual step with a zero
//!    staleness limit that would alarm on every check (`ZeroLimit`).
//!    [`ScheduleManifest::from_topology`] derives the manifest from a
//!    [`PipelineTopology`]: a `SessionScoped` step becomes
//!    [`ScheduleMode::MonitoredManual`] with a default limit, so "a human
//!    runs it" stops meaning "silence is not a defect" and starts meaning
//!    "staleness is alarmed."
//!
//! 2. **A queue walk that finds zero eligible entries must explain why.**
//!    [`WalkSummary::from_tick`] partitions a [`DispatchTick`] into the five
//!    numbers — walked / in flight / already produced / over dispatch bound /
//!    eligible — and [`WalkSummary::line`] renders the summary. The summary is *required*
//!    ([`WalkSummary::required`]) exactly when the queue is non-empty and
//!    nothing is eligible: that is the case that previously produced silence.
//!    [`WalkSummary::reconciles`] asserts the partition covers the queue
//!    exactly, so a summary whose numbers do not add up is a defect, not a
//!    rounding.
//!
//! 3. **Artifacts must be fresh, and `mtime` is the filesystem's fact, not
//!    the producer's claim.** [`assess_artifact`] compares the artifact's
//!    `mtime` against the now-epoch and the limit derived from the producer's
//!    [`ScheduleMode`] ([`ScheduleMode::max_stale_secs`]: scheduled →
//!    `interval_secs × max_intervals`, manual → the declared limit), and
//!    yields [`ArtifactStaleness::Stale`] or [`ArtifactStaleness::Missing`]
//!    — the alarm — when the producer has not run recently. An artifact that
//!    the filesystem says is old is stale, no matter what the producer's log
//!    says it did.
//!
//! 4. **A schedule entry with missing credentials is rejected, and the
//!    rejection says where the credentials live.** [`admit_schedule`]
//!    admits a [`PipelineStep`] for unattended scheduling only when its
//!    credential is present on a host that is permitted to hold it; otherwise
//!    it yields [`SchedulingVerdict::Rejected`] carrying a [`CredentialGap`]
//!    whose [`CredentialGap::line`] names the step, the requirement, and the
//!    explicit answer: credentials live in the operator's private home on the
//!    authenticated host (`GH_TOKEN` or `~/.config/gh/hosts.yml`), never on
//!    cluster-shared storage.
//!
//! The types here are pure and report-only: the caller (the CLI `schedule`
//! subcommand, the `tick` walk summary) decides exit codes and rendering
//! beyond the named `line()` text.

use crate::dispatch_pipeline::{
    CredentialRequirement, DispatchTick, HostKind, PipelineStep, PipelineTopology, QueueFile,
    StepSchedule, DEFAULT_INTERVAL_SECS, DEFAULT_MAX_STALE_INTERVALS,
};
use serde::{Deserialize, Serialize};

/// One step on the critical path and how it is kept alive.
///
/// A step is on the critical path when an agent dispatch depends on an
/// artifact it produces: the refresh step produces `queue.txt`, and a
/// dispatch that reads a stale `queue.txt` is dispatching from a world that
/// no longer exists (#3800).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriticalPathStep {
    /// The step's name, matching [`PipelineStep::name`].
    pub name: String,
    /// The artifact this step produces, if any. Staleness alarms attach to
    /// the artifact, not the step, because the artifact is what the consumer
    /// reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// How the step is kept alive.
    pub mode: ScheduleMode,
}

/// How a critical-path step is kept alive.
///
/// This is the middle ground [`StepSchedule`] lacked: `SessionScoped` meant
/// "nothing runs it unless a session starts it," which the pipeline read as
/// "silence is not a defect." [`ScheduleMode::MonitoredManual`] is the
/// explicit alternative: a human or session runs it, but the artifact's
/// staleness is still alarmed, so an unrun manual step cannot stay invisible
/// indefinitely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleMode {
    /// A durable scheduler (cron, systemd timer) runs it every
    /// `interval_secs`.
    Scheduled { interval_secs: u64 },
    /// A human or session runs it; staleness is alarmed after
    /// `max_stale_secs`.
    MonitoredManual { max_stale_secs: u64 },
}

impl ScheduleMode {
    /// The staleness limit in seconds, given the fleet's
    /// [`DEFAULT_MAX_STALE_INTERVALS`]-style multiplier.
    ///
    /// Scheduled steps alarm after `interval_secs × max_intervals`: the
    /// producer is expected to have run at least once per interval, and
    /// `max_intervals` missed intervals is the alarm. Manual steps alarm
    /// after their declared limit, directly.
    pub fn max_stale_secs(self, max_intervals: u64) -> u64 {
        match self {
            Self::Scheduled { interval_secs } => interval_secs.saturating_mul(max_intervals),
            Self::MonitoredManual { max_stale_secs } => max_stale_secs,
        }
    }

    /// Whether a durable scheduler runs this step, as opposed to a human or
    /// session.
    pub fn is_scheduled(self) -> bool {
        matches!(self, Self::Scheduled { .. })
    }

    pub fn as_str(self) -> String {
        match self {
            Self::Scheduled { interval_secs } => format!("scheduled, every {interval_secs}s"),
            Self::MonitoredManual { max_stale_secs } => {
                format!("manual, alarmed after {max_stale_secs}s")
            }
        }
    }
}

/// The declaration that every critical-path step is kept alive — by a
/// scheduler or explicitly by a human — with the staleness alarm that
/// catches it when it is not (#4320).
///
/// A manifest whose [`ScheduleManifest::coverage_audit`] is non-empty is
/// declaring a pipeline the operator does not actually run: an undeclared
/// critical step has no alarm at all, which is the silence that hid the
/// unrefreshed queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScheduleManifest {
    /// The declared steps, in critical-path order.
    pub steps: Vec<CriticalPathStep>,
    /// The steps on the critical path: the names an agent dispatch depends
    /// on. Every name here must appear in `steps`; that is what
    /// [`ScheduleManifest::coverage_audit`] checks.
    pub critical_path: Vec<String>,
}

impl ScheduleManifest {
    /// An empty manifest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Derive the manifest from a [`PipelineTopology`].
    ///
    /// Every step becomes a critical-path step. A `Scheduled` step keeps its
    /// interval; a `SessionScoped` step becomes
    /// [`ScheduleMode::MonitoredManual`] with the default limit
    /// (`DEFAULT_INTERVAL_SECS × DEFAULT_MAX_STALE_INTERVALS`), so the
    /// "silence is not a defect" reading of session-scoped steps is replaced
    /// by an explicit staleness alarm. The step's `produces` artifact carries
    /// over, because that is what the staleness check attaches to. The
    /// critical path is every step name: deriving from a topology that has
    /// already passed [`PipelineTopology::audit`] means every step is
    /// load-bearing.
    pub fn from_topology(topology: &PipelineTopology) -> Self {
        let steps = topology
            .steps()
            .iter()
            .map(|step| CriticalPathStep {
                name: step.name.clone(),
                artifact: step.produces.clone(),
                mode: match step.schedule {
                    StepSchedule::Scheduled { interval_secs } => {
                        ScheduleMode::Scheduled { interval_secs }
                    }
                    StepSchedule::SessionScoped => ScheduleMode::MonitoredManual {
                        max_stale_secs: DEFAULT_INTERVAL_SECS * DEFAULT_MAX_STALE_INTERVALS,
                    },
                },
            })
            .collect();
        let critical_path = topology
            .steps()
            .iter()
            .map(|step| step.name.clone())
            .collect();
        Self {
            steps,
            critical_path,
        }
    }

    /// The declared step named `name`, if any.
    pub fn step(&self, name: &str) -> Option<&CriticalPathStep> {
        self.steps.iter().find(|step| step.name == name)
    }

    /// The step declared to produce `artifact`, if any.
    pub fn producer_of(&self, artifact: &str) -> Option<&CriticalPathStep> {
        self.steps
            .iter()
            .find(|step| step.artifact.as_deref().is_some_and(|a| a == artifact))
    }

    /// The staleness limit for `artifact` in seconds, derived from its
    /// producer's [`ScheduleMode`], if a producer is declared.
    pub fn max_stale_secs_for(&self, artifact: &str, max_intervals: u64) -> Option<u64> {
        self.producer_of(artifact)
            .map(|step| step.mode.max_stale_secs(max_intervals))
    }

    /// The audit of the manifest against its declared critical path.
    ///
    /// Finds, in order: each step name declared more than once
    /// ([`CoverageFinding::DuplicateStep`], once per duplicate name); each
    /// critical-path name that no declared step carries
    /// ([`CoverageFinding::UndeclaredCriticalStep`], once per name); and each
    /// manual step with a zero staleness limit
    /// ([`CoverageFinding::ZeroLimit`]), which would alarm on every check.
    /// An empty result is a healthy manifest: every critical-path step is
    /// declared exactly once and alarmed with a positive limit.
    pub fn coverage_audit(&self) -> Vec<CoverageFinding> {
        let mut findings = Vec::new();
        // Duplicates: names declared more than once, reported once per name.
        let mut seen: Vec<&str> = Vec::new();
        for step in &self.steps {
            if seen.iter().any(|name| *name == step.name) {
                findings.push(CoverageFinding::DuplicateStep {
                    step: step.name.clone(),
                });
            } else {
                seen.push(&step.name);
            }
        }
        // Undeclared critical-path steps: names on the critical path no
        // declared step carries.
        for name in &self.critical_path {
            if self.step(name).is_none() {
                findings.push(CoverageFinding::UndeclaredCriticalStep { step: name.clone() });
            }
        }
        // Zero-limit manual steps: a limit of zero alarms on every check.
        for step in &self.steps {
            if matches!(
                step.mode,
                ScheduleMode::MonitoredManual { max_stale_secs: 0 }
            ) {
                findings.push(CoverageFinding::ZeroLimit {
                    step: step.name.clone(),
                });
            }
        }
        findings
    }

    /// Whether the manifest covers the critical path with no defects.
    pub fn healthy(&self) -> bool {
        self.coverage_audit().is_empty()
    }

    /// The per-line report: one line per finding, in audit order.
    pub fn lines(&self) -> Vec<String> {
        self.coverage_audit()
            .iter()
            .map(CoverageFinding::line)
            .collect()
    }
}

/// A defect in a [`ScheduleManifest`]'s coverage of the critical path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CoverageFinding {
    /// A critical-path step is not declared in the manifest at all: it has
    /// no scheduler and no staleness alarm, so an unrun step stays invisible.
    UndeclaredCriticalStep { step: String },
    /// A step is declared more than once: the manifest does not say which
    /// declaration is authoritative.
    DuplicateStep { step: String },
    /// A manual step allows zero seconds of staleness: it would alarm on
    /// every check, which is how the alarm gets ignored.
    ZeroLimit { step: String },
}

impl CoverageFinding {
    /// The step name the finding refers to.
    pub fn step(&self) -> &str {
        match self {
            Self::UndeclaredCriticalStep { step }
            | Self::DuplicateStep { step }
            | Self::ZeroLimit { step } => step,
        }
    }

    /// The report text for the finding.
    pub fn line(&self) -> String {
        match self {
            Self::UndeclaredCriticalStep { step } => format!(
                "SCHEDULE GAP [UNDECLARED_CRITICAL_STEP]: {step} is on the critical path but is not declared in the manifest: schedule it or declare it explicitly manual"
            ),
            Self::DuplicateStep { step } => format!(
                "SCHEDULE DEFECT [DUPLICATE_STEP]: {step} is declared more than once in the manifest"
            ),
            Self::ZeroLimit { step } => format!(
                "SCHEDULE DEFECT [ZERO_LIMIT]: {step} allows zero seconds of staleness: it will alarm on every check"
            ),
        }
    }
}

/// The partition of one queue walk into its four numbers (#4320).
///
/// A walk over a non-empty queue that finds zero eligible entries used to
/// produce no output: the operator could not tell "nothing is eligible
/// because everything is in flight or already produced" from "the refresh
/// is not running and the queue is a fossil." The summary makes the former
/// visible, so the latter stops hiding in the silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WalkSummary {
    /// The total number of entries the walk looked at.
    pub walked: usize,
    /// Entries skipped because they are held, or dispatched with no outcome
    /// recorded yet: the work is blocked, not missing.
    pub in_flight: usize,
    /// Entries the walk will not dispatch fresh: a patch already exists
    /// (`convert`), or the work is terminal (`converted`).
    pub already_produced: usize,
    /// Entries held over the dispatch bound: dispatched repeatedly without
    /// a patch (#4451). Blocked, not missing — and not the same as a plain
    /// hold, so they get their own number.
    pub over_bound: usize,
    /// Entries the walk will dispatch fresh.
    pub eligible: usize,
}

impl WalkSummary {
    /// Build the summary from a [`DispatchTick`] over `queue`.
    ///
    /// `walked` is the queue length; `in_flight` is the tick's held skips
    /// plus its in-flight skips; `already_produced` is the tick's convert
    /// dispatches plus its converted skips (both are "a patch exists, do
    /// not re-run the agent"); `over_bound` is the tick's dispatch-bound
    /// skips (#4451); `eligible` is the tick's fresh dispatches. The
    /// partition is exact — every queue entry is in exactly one bucket — so
    /// [`WalkSummary::reconciles`] holds by construction.
    pub fn from_tick(queue: &QueueFile, tick: &DispatchTick) -> Self {
        Self {
            walked: queue.entries.len(),
            in_flight: tick.held_count() + tick.in_flight_count(),
            already_produced: tick.convert_count() + tick.converted_count(),
            over_bound: tick.over_bound_count(),
            eligible: tick.fresh_count(),
        }
    }

    /// The summary line, e.g. `walked 132 entries, 14 in flight, 118 already
    /// produced, 0 over dispatch bound, 0 eligible`.
    ///
    /// The line is emitted on every human-facing walk, not only when
    /// `eligible == 0`: the numbers are the context that makes the zero
    /// legible, and emitting only the zero case would make the zero case
    /// unexplainable on the runs where the explanation was visible.
    pub fn line(&self) -> String {
        format!(
            "walked {} entries, {} in flight, {} already produced, {} over dispatch bound, {} eligible",
            self.walked, self.in_flight, self.already_produced, self.over_bound, self.eligible
        )
    }

    /// Whether the summary is *required*: the queue is non-empty and nothing
    /// is eligible. This is the exact case that previously produced silence
    /// — a non-empty walk with no output — so the summary must be present
    /// when `required()` is true. An empty queue is not required: there is
    /// nothing to explain.
    pub fn required(&self) -> bool {
        self.eligible == 0 && self.walked > 0
    }

    /// Whether the partition covers the queue exactly. A summary whose
    /// numbers do not add up is a counting defect, not a rounding, and must
    /// be treated as one.
    pub fn reconciles(&self) -> bool {
        self.walked == self.in_flight + self.already_produced + self.over_bound + self.eligible
    }
}

/// The freshness of an artifact, measured from the filesystem, not the
/// producer's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ArtifactStaleness {
    /// The artifact exists and its `mtime` is within the limit.
    Fresh { age_secs: u64 },
    /// The artifact exists but its `mtime` is older than the limit: the
    /// producer has not run recently, no matter what its log says.
    Stale { age_secs: u64, max_secs: u64 },
    /// The artifact does not exist: the producer has never run (or its
    /// output was lost).
    Missing,
}

impl ArtifactStaleness {
    /// Whether the artifact is not fresh.
    pub fn is_stale(&self) -> bool {
        !matches!(self, Self::Fresh { .. })
    }

    /// The report text for the staleness.
    pub fn line(&self) -> String {
        match self {
            Self::Fresh { age_secs } => format!("artifact fresh: {age_secs}s old"),
            Self::Stale {
                age_secs,
                max_secs,
            } => format!(
                "STALE ARTIFACT: mtime {age_secs}s old, limit {max_secs}s — the producer is not running"
            ),
            Self::Missing => "STALE ARTIFACT: artifact missing — the producer is not running"
                .to_string(),
        }
    }
}

/// Assess the freshness of an artifact whose `mtime` is `mtime` (or that is
/// missing, when `None`), as of epoch `now`, against the staleness limit
/// `max_secs` derived from the producer's [`ScheduleMode`].
///
/// The `mtime` is the filesystem's fact: an artifact the filesystem says is
/// old is stale, no matter what the producer's log claims. A `mtime` in the
/// future (clock skew) is treated as fresh with age zero: a skew is a
/// configuration problem, not evidence the producer is dead.
pub fn assess_artifact(mtime: Option<u64>, now: u64, max_secs: u64) -> ArtifactStaleness {
    match mtime {
        None => ArtifactStaleness::Missing,
        Some(m) if m >= now => ArtifactStaleness::Fresh { age_secs: 0 },
        Some(m) => {
            let age = now - m;
            if age <= max_secs {
                ArtifactStaleness::Fresh { age_secs: age }
            } else {
                ArtifactStaleness::Stale {
                    age_secs: age,
                    max_secs,
                }
            }
        }
    }
}

/// A schedule entry that was refused admission, with the reason and the
/// explicit answer about where the credential belongs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialGap {
    /// The step that was refused.
    pub step: String,
    /// The credential the step requires.
    pub requirement: CredentialRequirement,
    /// The host the step would run unattended on.
    pub host: HostKind,
    /// Whether the credential is actually present on that host.
    pub present: bool,
}

impl CredentialGap {
    /// The report text for the gap.
    ///
    /// Every line ends with the explicit answer the issue required:
    /// credentials live in the operator's private home on the authenticated
    /// host (`GH_TOKEN` or `~/.config/gh/hosts.yml`), never on
    /// cluster-shared storage.
    pub fn line(&self) -> String {
        let req = self.requirement.as_str();
        if self.host == HostKind::SharedCluster {
            if self.present {
                format!(
                    "SCHEDULE REJECTED [CREDENTIAL_ON_SHARED_HOST]: step {} holds a {} on cluster-shared storage: it belongs in the operator's private home on the authenticated host",
                    self.step, req
                )
            } else {
                format!(
                    "SCHEDULE REJECTED [CREDENTIAL_HOST_CANNOT_HOLD]: step {} requires a {} on a host that must not hold credentials: provision it in the operator's private home on the authenticated host",
                    self.step, req
                )
            }
        } else {
            format!(
                "SCHEDULE REJECTED [CREDENTIAL_ABSENT]: step {} requires a {} that is not present on its host: credentials live in the operator's private home on the authenticated host (GH_TOKEN or ~/.config/gh/hosts.yml)",
                self.step, req
            )
        }
    }
}

/// The admission decision for scheduling one step unattended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "kebab-case")]
pub enum SchedulingVerdict {
    /// The step may run unattended: it needs no credential, or its
    /// credential is present on a host that is permitted to hold it.
    Admitted,
    /// The step may not run unattended, with the gap.
    Rejected { gap: CredentialGap },
}

impl SchedulingVerdict {
    /// Whether the step was admitted.
    pub fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }
}

/// Admit or refuse a [`PipelineStep`] for unattended scheduling.
///
/// Admission requires the step's credential, if any, to be *present* on a
/// host that is *permitted* to hold credentials. The two failure modes are
/// distinct and both refused: a credential on shared storage is a leak
/// ([`CredentialGap`] with `present = true` on a [`HostKind::SharedCluster`]);
/// a required credential that is absent is a schedule that will fail the
/// first unattended run ([`CredentialGap`] with `present = false`).
///
/// A step with [`CredentialRequirement::None`] is admitted: it has nothing
/// to place and nothing to miss.
pub fn admit_schedule(step: &PipelineStep, credential_present: bool) -> SchedulingVerdict {
    if !step.credential.requires_credential() {
        return SchedulingVerdict::Admitted;
    }
    if !step.host.allows_credentials() {
        return SchedulingVerdict::Rejected {
            gap: CredentialGap {
                step: step.name.clone(),
                requirement: step.credential,
                host: step.host,
                present: credential_present,
            },
        };
    }
    if credential_present {
        SchedulingVerdict::Admitted
    } else {
        SchedulingVerdict::Rejected {
            gap: CredentialGap {
                step: step.name.clone(),
                requirement: step.credential,
                host: step.host,
                present: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_from_topology_maps_scheduled_and_session_scoped() {
        let topology = PipelineTopology::new(vec![
            PipelineStep {
                schedule: StepSchedule::Scheduled { interval_secs: 600 },
                ..scheduled("refresh-queue")
            },
            PipelineStep {
                schedule: StepSchedule::SessionScoped,
                ..scheduled("top-up")
            },
        ]);
        let manifest = ScheduleManifest::from_topology(&topology);
        assert_eq!(manifest.steps.len(), 2);
        assert!(matches!(
            manifest.step("refresh-queue").unwrap().mode,
            ScheduleMode::Scheduled { interval_secs: 600 }
        ));
        assert!(matches!(
            manifest.step("top-up").unwrap().mode,
            ScheduleMode::MonitoredManual {
                max_stale_secs: 1800
            }
        ));
        assert!(manifest.healthy());
        assert!(manifest
            .critical_path
            .contains(&"refresh-queue".to_string()));
    }

    fn scheduled(name: &str) -> PipelineStep {
        PipelineStep {
            name: name.to_string(),
            host: HostKind::Authenticated,
            credential: CredentialRequirement::None,
            schedule: StepSchedule::SessionScoped,
            produces: None,
            consumes: Vec::new(),
            log: None,
        }
    }

    #[test]
    fn coverage_audit_names_undeclared_duplicate_and_zero_limit() {
        let manifest = ScheduleManifest {
            steps: vec![
                CriticalPathStep {
                    name: "refresh-queue".to_string(),
                    artifact: Some("queue.txt".to_string()),
                    mode: ScheduleMode::Scheduled { interval_secs: 600 },
                },
                CriticalPathStep {
                    name: "refresh-queue".to_string(),
                    artifact: None,
                    mode: ScheduleMode::MonitoredManual { max_stale_secs: 0 },
                },
            ],
            critical_path: vec!["refresh-queue".to_string(), "top-up".to_string()],
        };
        let findings = manifest.coverage_audit();
        assert_eq!(findings.len(), 3);
        assert!(findings.iter().any(
            |f| matches!(f, CoverageFinding::DuplicateStep { step } if step == "refresh-queue")
        ));
        assert!(findings.iter().any(
            |f| matches!(f, CoverageFinding::UndeclaredCriticalStep { step } if step == "top-up")
        ));
        assert!(findings
            .iter()
            .any(|f| matches!(f, CoverageFinding::ZeroLimit { step } if step == "refresh-queue")));
        assert!(!manifest.healthy());
    }

    #[test]
    fn walk_summary_from_tick_partitions_and_reconciles() {
        use crate::dispatch_pipeline::{EntryState, LifecycleLedger};
        const NOW: u64 = 1_800_000_000;
        let queue = QueueFile {
            entries: vec![1, 2, 3, 4, 5],
            refreshed_at: Some(NOW - 60),
            refreshed_by: Some("test".to_string()),
        };
        let mut ledger = LifecycleLedger::new();
        ledger.hold(1, "in flight", NOW);
        ledger.record(2, EntryState::Produced, NOW);
        ledger.record(3, EntryState::Converted, NOW);
        // 4 and 5 stay queued → eligible.
        let tick = DispatchTick::run(&queue, &ledger);
        let summary = WalkSummary::from_tick(&queue, &tick);
        assert_eq!(summary.walked, 5);
        assert_eq!(summary.in_flight, 1);
        assert_eq!(summary.already_produced, 2);
        assert_eq!(summary.over_bound, 0);
        assert_eq!(summary.eligible, 2);
        assert!(summary.reconciles());
        assert!(!summary.required());
        assert_eq!(
            summary.line(),
            "walked 5 entries, 1 in flight, 2 already produced, 0 over dispatch bound, 2 eligible"
        );
    }

    #[test]
    fn walk_summary_required_on_zero_eligible_nonempty() {
        use crate::dispatch_pipeline::{EntryState, LifecycleLedger};
        const NOW: u64 = 1_800_000_000;
        let queue = QueueFile {
            entries: vec![1, 2],
            refreshed_at: Some(NOW - 60),
            refreshed_by: Some("test".to_string()),
        };
        let mut ledger = LifecycleLedger::new();
        ledger.hold(1, "in flight", NOW);
        ledger.record(2, EntryState::Produced, NOW);
        let tick = DispatchTick::run(&queue, &ledger);
        let summary = WalkSummary::from_tick(&queue, &tick);
        // produced is dispatched as convert, so it is not eligible — but it
        // is already_produced, so eligible is 0 and the queue is non-empty.
        assert_eq!(summary.eligible, 0);
        assert_eq!(summary.already_produced, 1);
        assert_eq!(summary.in_flight, 1);
        assert!(summary.required());
    }

    #[test]
    fn walk_summary_empty_queue_not_required() {
        use crate::dispatch_pipeline::LifecycleLedger;
        let queue = QueueFile {
            entries: Vec::new(),
            refreshed_at: None,
            refreshed_by: None,
        };
        let ledger = LifecycleLedger::new();
        let tick = DispatchTick::run(&queue, &ledger);
        let summary = WalkSummary::from_tick(&queue, &tick);
        assert!(!summary.required());
        assert!(summary.reconciles());
    }

    #[test]
    fn assess_artifact_fresh_stale_missing_and_future_mtime() {
        let fresh = assess_artifact(Some(1_800_000_000 - 100), 1_800_000_000, 600);
        assert_eq!(fresh, ArtifactStaleness::Fresh { age_secs: 100 });
        let stale = assess_artifact(Some(1_800_000_000 - 700), 1_800_000_000, 600);
        assert_eq!(
            stale,
            ArtifactStaleness::Stale {
                age_secs: 700,
                max_secs: 600
            }
        );
        assert_eq!(
            assess_artifact(None, 1_800_000_000, 600),
            ArtifactStaleness::Missing
        );
        // Future mtime (clock skew): fresh with age zero, not an error.
        assert_eq!(
            assess_artifact(Some(1_800_000_100), 1_800_000_000, 600),
            ArtifactStaleness::Fresh { age_secs: 0 }
        );
        assert!(stale.is_stale());
        assert!(!fresh.is_stale());
    }

    #[test]
    fn admit_schedule_admits_no_credential() {
        let step = scheduled("refresh-queue");
        assert_eq!(admit_schedule(&step, false), SchedulingVerdict::Admitted);
        assert_eq!(admit_schedule(&step, true), SchedulingVerdict::Admitted);
    }

    #[test]
    fn admit_schedule_rejects_credential_on_shared_host() {
        let step = PipelineStep {
            host: HostKind::SharedCluster,
            credential: CredentialRequirement::GitHubToken,
            ..scheduled("refresh-queue")
        };
        // Present on shared storage: a leak.
        let verdict = admit_schedule(&step, true);
        assert!(!verdict.is_admitted());
        if let SchedulingVerdict::Rejected { gap } = &verdict {
            assert!(gap.present);
            assert!(gap.line().contains("CREDENTIAL_ON_SHARED_HOST"));
            assert!(gap.line().contains("refresh-queue"));
        } else {
            panic!("expected rejection");
        }
        // Absent on shared storage: the host cannot hold it at all.
        let verdict = admit_schedule(&step, false);
        if let SchedulingVerdict::Rejected { gap } = &verdict {
            assert!(!gap.present);
            assert!(gap.line().contains("CREDENTIAL_HOST_CANNOT_HOLD"));
        } else {
            panic!("expected rejection");
        }
    }

    #[test]
    fn admit_schedule_rejects_absent_credential_on_authenticated_host() {
        let step = PipelineStep {
            credential: CredentialRequirement::GitHubToken,
            ..scheduled("refresh-queue")
        };
        assert_eq!(admit_schedule(&step, true), SchedulingVerdict::Admitted);
        let verdict = admit_schedule(&step, false);
        if let SchedulingVerdict::Rejected { gap } = &verdict {
            assert!(gap.line().contains("CREDENTIAL_ABSENT"));
            assert!(gap.line().contains("GH_TOKEN or ~/.config/gh/hosts.yml"));
        } else {
            panic!("expected rejection");
        }
    }
}
