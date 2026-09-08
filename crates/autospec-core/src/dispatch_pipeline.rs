//! Filing-to-dispatch liveness (#3800).
//!
//! A queue that is only ever repopulated from inside an agent session is a
//! queue that silently stops moving when the session does. The filing chain is
//! `file a labelled issue -> refresh the queue artifact -> the consumer reads
//! the artifact and dispatches an agent`, and the middle hop is normally a
//! deployment script run from cron. When that script runs only inside an
//! ephemeral session, filed issues never reach dispatch and — worse — nothing
//! can tell the difference between "nothing was filed" and "nobody looked".
//! `queue.txt` frozen since 10:30 while the consumer reports `no work` every
//! ten minutes is the failure this module exists to make impossible.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **Queue population is a scheduled step like any other** ([`StepSchedule`],
//!    [`TopologyViolation::ProducerNotScheduled`]). A step that repopulates an
//!    artifact somebody else consumes must have an interval and a durable host,
//!    and it has its own failure signal — its log. A step with no log has no
//!    failure signal, only silence.
//! 2. **An unrefreshed queue is an error, not a steady state**
//!    ([`DispatchOutcome::Hold`]). A consumer may treat the artifact as
//!    authoritative only while its `refreshed-at` stamp is inside the
//!    threshold. Past it the answer is a named failure, never `no work`.
//!    "No new issues were filed" ([`DispatchOutcome::Idle`]) is only sayable
//!    about a *fresh* artifact.
//! 3. **Every hop between filing and dispatch emits liveness**
//!    ([`LivenessLedger`], [`LivenessVerdict`]). The stamp on the artifact is
//!    one beat; each other hop beats into the ledger. A beat stamped in the
//!    future is a clock fault ([`LivenessVerdict::ClockRewind`]), not
//!    permanent freshness.
//! 4. **Credential-holding steps are named as such**
//!    ([`CredentialRequirement`], [`PipelineTopology::credential_steps`],
//!    [`TopologyViolation::CredentialOnSharedStorage`]). Each step declares
//!    which host it runs on, so the topology states whether the host that
//!    holds credentials is the durable one or an agent session that dies, and
//!    forbids a credential ever landing on cluster-shared storage.
//!
//! Everything here is pure and testable: no I/O, no clock, no subprocess. The
//! caller supplies `now` and the artifact it read; [`DispatchPipeline`] decides.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Name of the queue artifact the refresh step writes and the consumer reads.
pub const QUEUE_ARTIFACT: &str = "queue.txt";

/// Interval assumed for a step that declares no schedule of its own.
pub const DEFAULT_INTERVAL_SECS: u64 = 600;

/// How many missed intervals a consumer tolerates before the artifact stops
/// being authoritative.
pub const DEFAULT_MAX_STALE_INTERVALS: u64 = 3;

/// Header the refresher writes with the epoch seconds of the last refresh. A
/// queue file without it cannot be told apart from a frozen one, so it is
/// refused rather than read.
pub const STAMP_HEADER: &str = "# refreshed-at:";

/// Header naming the step that wrote the artifact.
pub const REFRESHER_HEADER: &str = "# refreshed-by:";

/// Machine-readable identity of a liveness or topology failure, so a cron
/// wrapper can branch on the code and a human reads the name of the culprit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FailureCode {
    /// The consumer found no artifact at all.
    QueueMissing,
    /// The artifact exists but carries no `refreshed-at` stamp.
    QueueUnstamped,
    /// The stamp is older than `max_intervals` intervals.
    StampNotRefreshed,
    /// The stamp is in the future; the clock moved and freshness is meaningless.
    ClockRewind,
    /// A step has never emitted a liveness beat.
    StepNeverBeat,
    /// A step stopped emitting beats.
    StepSilent,
    /// A credential-holding step is scheduled on an ephemeral host.
    CredentialHostNotDurable,
    /// A credential-holding step runs on cluster-shared storage.
    CredentialOnSharedStorage,
    /// The producer of a consumed artifact only runs from a session.
    ProducerNotScheduled,
    /// The producer of a consumed artifact runs on a host that dies.
    ProducerHostNotDurable,
    /// An artifact is consumed but produced by no step.
    UnknownProducer,
    /// A step has no log, so its failure would be silent.
    StepHasNoLog,
}

impl FailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QueueMissing => "QUEUE_MISSING",
            Self::QueueUnstamped => "QUEUE_UNSTAMPED",
            Self::StampNotRefreshed => "STAMP_NOT_REFRESHED",
            Self::ClockRewind => "CLOCK_REWIND",
            Self::StepNeverBeat => "STEP_NEVER_BEAT",
            Self::StepSilent => "STEP_SILENT",
            Self::CredentialHostNotDurable => "CREDENTIAL_HOST_NOT_DURABLE",
            Self::CredentialOnSharedStorage => "CREDENTIAL_ON_SHARED_STORAGE",
            Self::ProducerNotScheduled => "PRODUCER_NOT_SCHEDULED",
            Self::ProducerHostNotDurable => "PRODUCER_HOST_NOT_DURABLE",
            Self::UnknownProducer => "UNKNOWN_PRODUCER",
            Self::StepHasNoLog => "STEP_HAS_NO_LOG",
        }
    }
}

/// One named failure: which step, on which artifact, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessFailure {
    pub code: FailureCode,
    /// The step at fault, named ("refresh-queue") — never "the queue".
    pub step: String,
    /// The artifact involved, when the failure is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    pub message: String,
}

impl LivenessFailure {
    pub fn line(&self) -> String {
        match &self.artifact {
            Some(artifact) => format!(
                "LIVENESS FAILURE [{}]: step {step} -> {artifact}: {message}",
                self.code.as_str(),
                step = self.step,
                message = self.message
            ),
            None => format!(
                "LIVENESS FAILURE [{}]: step {step}: {message}",
                self.code.as_str(),
                step = self.step,
                message = self.message
            ),
        }
    }
}

/// Where a step runs, which decides what it may hold and whether it survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKind {
    /// A durable host with its own scheduler and a private authenticated home.
    Authenticated,
    /// Cluster-shared storage or compute. No credential may land here.
    SharedCluster,
    /// An interactive agent session's scratch directory: dies with the session.
    EphemeralSession,
}

impl HostKind {
    /// Whether a credential may be stored where this host can read it.
    pub fn allows_credentials(self) -> bool {
        !matches!(self, Self::SharedCluster)
    }

    /// Whether the host outlives a login session, i.e. can run a cron step.
    pub fn is_durable(self) -> bool {
        !matches!(self, Self::EphemeralSession)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authenticated => "authenticated",
            Self::SharedCluster => "shared-cluster",
            Self::EphemeralSession => "ephemeral-session",
        }
    }
}

/// What a step needs an authenticated host for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialRequirement {
    /// No credential: safe to schedule anywhere.
    None,
    /// A `gh` session / GitHub token. Renamed explicitly so the wire spelling
    /// matches `as_str`: kebab-casing the variant would yield `git-hub-token`.
    #[serde(rename = "gh-token")]
    GitHubToken,
}

impl CredentialRequirement {
    pub fn requires_credential(self) -> bool {
        !matches!(self, Self::None)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::GitHubToken => "gh-token",
        }
    }
}

/// How a step gets run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepSchedule {
    /// A durable scheduler (cron, systemd timer) runs it every `interval_secs`.
    Scheduled { interval_secs: u64 },
    /// Nothing runs it unless a human or agent session starts it.
    SessionScoped,
}

impl StepSchedule {
    pub fn interval_secs(self) -> Option<u64> {
        match self {
            Self::Scheduled { interval_secs } => Some(interval_secs),
            Self::SessionScoped => None,
        }
    }

    pub fn as_str(self) -> String {
        match self {
            Self::Scheduled { interval_secs } => format!("every {interval_secs}s"),
            Self::SessionScoped => "session-scoped".to_string(),
        }
    }
}

/// One hop between an issue being filed and an agent being dispatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineStep {
    pub name: String,
    pub host: HostKind,
    pub credential: CredentialRequirement,
    pub schedule: StepSchedule,
    /// The artifact this step rewrites, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produces: Option<String>,
    /// Artifacts this step reads.
    #[serde(default)]
    pub consumes: Vec<String>,
    /// Where the step writes its own log. Without one a failure leaves nothing
    /// behind, which is how #3800 stayed invisible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
}

/// Declared violations of the filing-to-dispatch topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TopologyViolation {
    /// Invariant 4: a credential must never be placed on cluster-shared storage.
    CredentialOnSharedStorage { step: String },
    /// Invariant 4: an unattended credential step whose only authenticated host
    /// is a session stops authenticating the moment nobody is logged in.
    CredentialHostNotDurable { step: String },
    /// Invariant 1: the artifact another step consumes is repopulated only when
    /// a session happens to run.
    ProducerNotScheduled { step: String, artifact: String },
    /// Invariant 1: the producer runs on a host that dies with its session.
    ProducerHostNotDurable { step: String, artifact: String },
    /// Invariant 1: an artifact is consumed but produced by no step at all.
    UnknownProducer { artifact: String },
    /// Invariant 1 (failure signal): the step cannot report its own failure.
    NoLog { step: String },
}

impl TopologyViolation {
    pub fn code(&self) -> FailureCode {
        match self {
            Self::CredentialOnSharedStorage { .. } => FailureCode::CredentialOnSharedStorage,
            Self::CredentialHostNotDurable { .. } => FailureCode::CredentialHostNotDurable,
            Self::ProducerNotScheduled { .. } => FailureCode::ProducerNotScheduled,
            Self::ProducerHostNotDurable { .. } => FailureCode::ProducerHostNotDurable,
            Self::UnknownProducer { .. } => FailureCode::UnknownProducer,
            Self::NoLog { .. } => FailureCode::StepHasNoLog,
        }
    }

    pub fn step(&self) -> Option<&str> {
        match self {
            Self::CredentialOnSharedStorage { step }
            | Self::CredentialHostNotDurable { step }
            | Self::NoLog { step } => Some(step),
            Self::ProducerNotScheduled { step, .. } | Self::ProducerHostNotDurable { step, .. } => {
                Some(step)
            }
            Self::UnknownProducer { .. } => None,
        }
    }

    pub fn line(&self) -> String {
        match self {
            Self::CredentialOnSharedStorage { step } => format!(
                "TOPOLOGY DEFECT [CREDENTIAL_ON_SHARED_STORAGE]: step {step} holds a credential on cluster-shared storage"
            ),
            Self::CredentialHostNotDurable { step } => format!(
                "TOPOLOGY DEFECT [CREDENTIAL_HOST_NOT_DURABLE]: step {step} runs unattended on a host that dies with its session"
            ),
            Self::ProducerNotScheduled { step, artifact } => format!(
                "TOPOLOGY DEFECT [PRODUCER_NOT_SCHEDULED]: {artifact} is consumed but repopulated only by {step} running from a session"
            ),
            Self::ProducerHostNotDurable { step, artifact } => format!(
                "TOPOLOGY DEFECT [PRODUCER_HOST_NOT_DURABLE]: {step} produces {artifact} on a host that dies with its session"
            ),
            Self::UnknownProducer { artifact } => format!(
                "TOPOLOGY DEFECT [UNKNOWN_PRODUCER]: {artifact} is consumed but no step produces it"
            ),
            Self::NoLog { step } => format!(
                "TOPOLOGY DEFECT [STEP_HAS_NO_LOG]: step {step} writes no log, so its failure would be silent"
            ),
        }
    }

    pub fn failure(&self) -> LivenessFailure {
        LivenessFailure {
            code: self.code(),
            step: self.step().unwrap_or("pipeline").to_string(),
            artifact: match self {
                Self::ProducerNotScheduled { artifact, .. }
                | Self::ProducerHostNotDurable { artifact, .. }
                | Self::UnknownProducer { artifact } => Some(artifact.clone()),
                _ => None,
            },
            message: self.line(),
        }
    }
}

/// The declared filing-to-dispatch chain and what it implies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineTopology {
    steps: Vec<PipelineStep>,
}

impl PipelineTopology {
    pub fn new(steps: Vec<PipelineStep>) -> Self {
        Self { steps }
    }

    pub fn steps(&self) -> &[PipelineStep] {
        &self.steps
    }

    pub fn step(&self, name: &str) -> Option<&PipelineStep> {
        self.steps.iter().find(|step| step.name == name)
    }

    /// Invariant 4: the steps that need an authenticated host, named.
    pub fn credential_steps(&self) -> Vec<&PipelineStep> {
        self.steps
            .iter()
            .filter(|step| step.credential.requires_credential())
            .collect()
    }

    pub fn producer_of(&self, artifact: &str) -> Option<&PipelineStep> {
        self.steps
            .iter()
            .find(|step| step.produces.as_deref() == Some(artifact))
    }

    /// Artifacts read by at least one step, in declaration order of first use.
    pub fn consumed_artifacts(&self) -> BTreeSet<String> {
        self.steps
            .iter()
            .flat_map(|step| step.consumes.iter().cloned())
            .collect()
    }

    /// The deployed filing-to-dispatch chain, stated as it *should* be: every
    /// hop scheduled, the queue repopulated from cron on the authenticated
    /// host, and no credential on the shared cluster.
    pub fn reference() -> Self {
        Self::new(vec![
            PipelineStep {
                name: "file-issue".to_string(),
                host: HostKind::Authenticated,
                credential: CredentialRequirement::GitHubToken,
                schedule: StepSchedule::SessionScoped,
                produces: None,
                consumes: Vec::new(),
                log: Some("~/.autospec/logs/file-issue.log".to_string()),
            },
            PipelineStep {
                name: "refresh-queue".to_string(),
                host: HostKind::Authenticated,
                credential: CredentialRequirement::GitHubToken,
                schedule: StepSchedule::Scheduled {
                    interval_secs: DEFAULT_INTERVAL_SECS,
                },
                produces: Some(QUEUE_ARTIFACT.to_string()),
                // Its input is the labelled-issue list GitHub itself serves, so
                // no in-topology producer is declared for it; the step's own
                // `github-token` credential is what says where it reads from.
                consumes: Vec::new(),
                log: Some("~/.autospec/logs/refresh-queue.log".to_string()),
            },
            PipelineStep {
                name: "topup".to_string(),
                host: HostKind::Authenticated,
                credential: CredentialRequirement::GitHubToken,
                schedule: StepSchedule::Scheduled {
                    interval_secs: DEFAULT_INTERVAL_SECS,
                },
                produces: Some("dispatch-request".to_string()),
                consumes: vec![QUEUE_ARTIFACT.to_string()],
                log: Some("~/.autospec/logs/topup.log".to_string()),
            },
            PipelineStep {
                name: "dispatch-agent".to_string(),
                host: HostKind::SharedCluster,
                credential: CredentialRequirement::None,
                schedule: StepSchedule::Scheduled {
                    interval_secs: DEFAULT_INTERVAL_SECS,
                },
                produces: None,
                consumes: vec!["dispatch-request".to_string()],
                log: Some("~/.autospec/logs/dispatch-agent.log".to_string()),
            },
        ])
    }

    /// Every declared invariant this topology breaks.
    pub fn audit(&self) -> Vec<TopologyViolation> {
        let mut violations = Vec::new();
        for step in &self.steps {
            violations.extend(Self::credential_defects(step));
            violations.extend(self.producer_defects(step));
            if step.log.is_none() {
                violations.push(TopologyViolation::NoLog {
                    step: step.name.clone(),
                });
            }
        }
        for artifact in self.consumed_artifacts() {
            if self.producer_of(&artifact).is_none() {
                violations.push(TopologyViolation::UnknownProducer { artifact });
            }
        }
        violations
    }

    fn credential_defects(step: &PipelineStep) -> Vec<TopologyViolation> {
        if !step.credential.requires_credential() {
            return Vec::new();
        }
        let mut defects = Vec::new();
        if !step.host.allows_credentials() {
            defects.push(TopologyViolation::CredentialOnSharedStorage {
                step: step.name.clone(),
            });
        }
        // An attended step is fine on a session host; an unattended one has no
        // one to authenticate when the session is gone.
        if matches!(step.schedule, StepSchedule::Scheduled { .. }) && !step.host.is_durable() {
            defects.push(TopologyViolation::CredentialHostNotDurable {
                step: step.name.clone(),
            });
        }
        defects
    }

    fn producer_defects(&self, step: &PipelineStep) -> Vec<TopologyViolation> {
        let Some(artifact) = step.produces.as_deref() else {
            return Vec::new();
        };
        // Only an artifact somebody actually waits on makes the producer a
        // scheduled step.
        if !self.consumed_artifacts().contains(artifact) {
            return Vec::new();
        }
        let mut defects = Vec::new();
        if step.schedule.interval_secs().is_none() {
            defects.push(TopologyViolation::ProducerNotScheduled {
                step: step.name.clone(),
                artifact: artifact.to_string(),
            });
        }
        if !step.host.is_durable() {
            defects.push(TopologyViolation::ProducerHostNotDurable {
                step: step.name.clone(),
                artifact: artifact.to_string(),
            });
        }
        defects
    }
}

/// How much staleness a consumer tolerates, expressed in the producer's own
/// intervals rather than in wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessPolicy {
    /// Interval assumed for a step that declares none.
    pub interval_secs: u64,
    /// Intervals a hop may miss before it is a named failure.
    pub max_intervals: u64,
}

impl Default for FreshnessPolicy {
    fn default() -> Self {
        Self {
            interval_secs: DEFAULT_INTERVAL_SECS,
            max_intervals: DEFAULT_MAX_STALE_INTERVALS,
        }
    }
}

impl FreshnessPolicy {
    pub fn new(interval_secs: u64, max_intervals: u64) -> Option<Self> {
        if interval_secs == 0 || max_intervals == 0 {
            return None;
        }
        Some(Self {
            interval_secs,
            max_intervals,
        })
    }

    /// The producer's declared interval wins; `interval_secs` covers steps and
    /// artifacts that declare none.
    pub fn interval_for(&self, step: Option<&PipelineStep>) -> u64 {
        step.and_then(|step| step.schedule.interval_secs())
            .unwrap_or(self.interval_secs)
    }

    pub fn threshold_for(&self, step: Option<&PipelineStep>) -> u64 {
        self.interval_for(step).saturating_mul(self.max_intervals)
    }

    /// Whole intervals of silence, for a message that says "4 intervals".
    pub fn missed_intervals(&self, age_secs: u64, step: Option<&PipelineStep>) -> u64 {
        age_secs.saturating_div(self.interval_for(step))
    }
}

/// Liveness of one hop, derived from its last beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LivenessVerdict {
    /// Beat inside the threshold.
    Live { age_secs: u64 },
    /// No beat for more than `max_intervals` intervals.
    Silent {
        age_secs: u64,
        missed_intervals: u64,
        max_intervals: u64,
    },
    /// The hop has never beaten at all.
    NeverBeat,
    /// The last beat is in the future: the clock moved, freshness is undefined.
    ClockRewind { ahead_secs: u64 },
    /// A session-scoped step beats only while a session runs, so silence is not
    /// a defect here. What flags a session-scoped *producer* is the topology
    /// audit (`PRODUCER_NOT_SCHEDULED`), not its missing heartbeat.
    EventDriven { last_beat_age_secs: Option<u64> },
}

impl LivenessVerdict {
    pub fn failed(self) -> bool {
        !matches!(self, Self::Live { .. } | Self::EventDriven { .. })
    }

    pub fn line(self, step: &str) -> String {
        match self {
            Self::Live { age_secs } => format!("hop {step}: LIVE last beat {age_secs}s ago"),
            Self::Silent {
                age_secs,
                missed_intervals,
                max_intervals,
            } => format!(
                "hop {step}: SILENT no beat in {missed_intervals} intervals (threshold {max_intervals}, last beat {age_secs}s ago)"
            ),
            Self::NeverBeat => {
                format!("hop {step}: NEVER BEAT no liveness stamp recorded for this hop")
            }
            Self::ClockRewind { ahead_secs } => format!(
                "hop {step}: CLOCK REWIND last beat is {ahead_secs}s in the future; freshness is not evaluable"
            ),
            Self::EventDriven {
                last_beat_age_secs: Some(age_secs),
            } => format!(
                "hop {step}: EVENT-DRIVEN beat expected only while a session runs (last beat {age_secs}s ago)"
            ),
            Self::EventDriven {
                last_beat_age_secs: None,
            } => format!(
                "hop {step}: EVENT-DRIVEN beat expected only while a session runs (never beaten)"
            ),
        }
    }

    pub fn failure(self, step: &str) -> Option<LivenessFailure> {
        let (code, message) = match self {
            Self::Live { .. } => return None,
            Self::Silent { .. } => (FailureCode::StepSilent, self.line(step)),
            Self::NeverBeat => (FailureCode::StepNeverBeat, self.line(step)),
            Self::ClockRewind { .. } => (FailureCode::ClockRewind, self.line(step)),
            Self::EventDriven { .. } => return None,
        };
        Some(LivenessFailure {
            code,
            step: step.to_string(),
            artifact: None,
            message,
        })
    }
}

/// Last liveness beat per hop, durable between cron invocations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessLedger {
    beats: BTreeMap<String, u64>,
}

impl LivenessLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `step` was alive at `at` (epoch seconds). A beat never
    /// moves backwards, so a rewound clock cannot erase evidence of life.
    pub fn record(&mut self, step: &str, at: u64) {
        let current = self.beats.get(step).copied().unwrap_or(0);
        self.beats.insert(step.to_string(), current.max(at));
    }

    pub fn last_beat(&self, step: &str) -> Option<u64> {
        self.beats.get(step).copied()
    }

    pub fn steps(&self) -> impl Iterator<Item = &str> {
        self.beats.keys().map(String::as_str)
    }

    pub fn assess(
        &self,
        step: &PipelineStep,
        policy: &FreshnessPolicy,
        now: u64,
    ) -> LivenessVerdict {
        let beaten = self.last_beat(&step.name);
        if matches!(step.schedule, StepSchedule::SessionScoped) {
            return LivenessVerdict::EventDriven {
                last_beat_age_secs: beaten.map(|beat| now.saturating_sub(beat)),
            };
        }
        let Some(beaten) = beaten else {
            return LivenessVerdict::NeverBeat;
        };
        if beaten > now {
            return LivenessVerdict::ClockRewind {
                ahead_secs: beaten - now,
            };
        }
        let age_secs = now - beaten;
        if age_secs > policy.threshold_for(Some(step)) {
            return LivenessVerdict::Silent {
                age_secs,
                missed_intervals: policy.missed_intervals(age_secs, Some(step)),
                max_intervals: policy.max_intervals,
            };
        }
        LivenessVerdict::Live { age_secs }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| error.to_string())
    }
}

/// The queue artifact as read from disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueFile {
    /// Issue numbers listed for dispatch.
    pub entries: Vec<u64>,
    /// Epoch seconds from the `refreshed-at` header; `None` when unstamped.
    pub refreshed_at: Option<u64>,
    /// The step named by the writer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_by: Option<String>,
}

impl QueueFile {
    /// Tolerant read: unknown comment lines and non-numeric lines are skipped,
    /// a missing stamp is preserved as `None` (never guessed at).
    pub fn parse(text: &str) -> Self {
        let mut file = Self::default();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix(STAMP_HEADER) {
                if let Ok(stamped) = rest.trim().parse::<u64>() {
                    file.refreshed_at = Some(stamped);
                }
            } else if let Some(rest) = trimmed.strip_prefix(REFRESHER_HEADER) {
                let writer = rest.trim();
                if !writer.is_empty() {
                    file.refreshed_by = Some(writer.to_string());
                }
            } else if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            } else if let Ok(number) = trimmed.parse::<u64>() {
                file.entries.push(number);
            }
        }
        file
    }

    /// What the refresher calls before renaming the artifact into place.
    pub fn stamp(&mut self, at: u64, by: &str) {
        self.refreshed_at = Some(at);
        self.refreshed_by = Some(by.to_string());
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        if let Some(at) = self.refreshed_at {
            out.push_str(&format!("{STAMP_HEADER} {at}\n"));
        }
        if let Some(by) = &self.refreshed_by {
            out.push_str(&format!("{REFRESHER_HEADER} {by}\n"));
        }
        for entry in &self.entries {
            out.push_str(&format!("{entry}\n"));
        }
        out
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// What a consumer may conclude from the artifact it just read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DispatchOutcome {
    /// The artifact is fresh and has work.
    Proceed { entries: usize, age_secs: u64 },
    /// The artifact is fresh and empty: no new issues were filed. Only a
    /// *fresh* artifact earns this sentence.
    Idle { age_secs: u64 },
    /// The artifact is not authoritative. The failure names the step at fault;
    /// "no work" is not an available answer here.
    Hold { failure: LivenessFailure },
}

impl DispatchOutcome {
    pub fn held(&self) -> bool {
        matches!(self, Self::Hold { .. })
    }

    pub fn failure(&self) -> Option<&LivenessFailure> {
        match self {
            Self::Hold { failure } => Some(failure),
            _ => None,
        }
    }

    pub fn line(&self) -> String {
        match self {
            Self::Proceed { entries, age_secs } => {
                format!("DISPATCH queue ready: {entries} entries, refreshed {age_secs}s ago")
            }
            Self::Idle { age_secs } => {
                format!("DISPATCH queue idle: no new issues were filed, refreshed {age_secs}s ago")
            }
            Self::Hold { failure } => failure.line(),
        }
    }
}

/// One hop's line in the filing-to-dispatch report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HopStatus {
    pub step: String,
    pub line: String,
    pub failed: bool,
    /// The verdict behind `line`, so a wrapper can branch on the code instead
    /// of parsing its own name out of the message.
    pub verdict: LivenessVerdict,
}

/// The whole picture: topology, per-hop liveness, and what the consumer may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineReport {
    pub outcome: DispatchOutcome,
    pub hops: Vec<HopStatus>,
    pub topology_violations: Vec<TopologyViolation>,
}

impl PipelineReport {
    pub fn held(&self) -> bool {
        self.outcome.held()
    }

    /// True when the report is clean: not held, every hop live, topology sound.
    pub fn healthy(&self) -> bool {
        !self.outcome.held()
            && !self.hops.iter().any(|hop| hop.failed)
            && self.topology_violations.is_empty()
    }

    pub fn failures(&self) -> Vec<LivenessFailure> {
        let mut failures: Vec<LivenessFailure> =
            self.outcome.failure().cloned().into_iter().collect();
        failures.extend(
            self.hops
                .iter()
                .filter_map(|hop| hop.verdict.failure(&hop.step)),
        );
        failures.extend(
            self.topology_violations
                .iter()
                .map(TopologyViolation::failure),
        );
        failures
    }

    pub fn lines(&self) -> Vec<String> {
        let mut lines = self
            .topology_violations
            .iter()
            .map(TopologyViolation::line)
            .collect::<Vec<_>>();
        lines.extend(self.hops.iter().map(|hop| hop.line.clone()));
        lines.push(self.outcome.line());
        lines
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// The filing-to-dispatch chain: topology, liveness, and the consumer's
/// staleness policy, evaluated together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchPipeline {
    pub topology: PipelineTopology,
    pub liveness: LivenessLedger,
    pub policy: FreshnessPolicy,
}

impl DispatchPipeline {
    pub fn new(
        topology: PipelineTopology,
        liveness: LivenessLedger,
        policy: FreshnessPolicy,
    ) -> Self {
        Self {
            topology,
            liveness,
            policy,
        }
    }

    /// The step that must keep the queue alive, named for failure messages.
    pub fn queue_producer(&self) -> Option<&PipelineStep> {
        self.topology.producer_of(QUEUE_ARTIFACT)
    }

    /// Invariant 2: decide what the consumer may conclude from `queue`.
    ///
    /// `queue` is `None` when the artifact does not exist. A fresh artifact may
    /// be reported idle; anything else is a named failure, never `no work`.
    pub fn authorize_queue(&self, queue: Option<&QueueFile>, now: u64) -> DispatchOutcome {
        let step = self.queue_producer();
        let producer = step
            .map(|step| step.name.clone())
            .unwrap_or_else(|| format!("{QUEUE_ARTIFACT} producer (undeclared in topology)"));

        let Some(queue) = queue else {
            return Self::hold(
                FailureCode::QueueMissing,
                producer,
                format!("no {QUEUE_ARTIFACT} found; it was never written or was removed, so there is nothing to trust"),
            );
        };

        let Some(stamped) = queue.refreshed_at else {
            return Self::hold(
                FailureCode::QueueUnstamped,
                producer,
                format!("{QUEUE_ARTIFACT} carries no {STAMP_HEADER} header; a queue without a stamp cannot be told apart from a frozen one"),
            );
        };

        if stamped > now {
            return Self::hold(
                FailureCode::ClockRewind,
                producer,
                format!("{QUEUE_ARTIFACT} is stamped {}s in the future (stamped {stamped}, now {now}); the clock moved and staleness cannot be evaluated", stamped - now),
            );
        }

        let age_secs = now - stamped;
        let threshold_secs = self.policy.threshold_for(step);
        if age_secs > threshold_secs {
            let missed = self.policy.missed_intervals(age_secs, step);
            let message = format!(
                "{producer} has not refreshed {QUEUE_ARTIFACT} for {missed} intervals (last stamp {stamped}, {age_secs}s old, threshold {threshold_secs}s); refusing to read a stale queue as 'no work'"
            );
            return Self::hold(FailureCode::StampNotRefreshed, producer, message);
        }

        if queue.entries.is_empty() {
            return DispatchOutcome::Idle { age_secs };
        }
        DispatchOutcome::Proceed {
            entries: queue.entries.len(),
            age_secs,
        }
    }

    /// A refusal that names the producer and the artifact it failed on.
    fn hold(code: FailureCode, step: String, message: String) -> DispatchOutcome {
        DispatchOutcome::Hold {
            failure: LivenessFailure {
                code,
                step,
                artifact: Some(QUEUE_ARTIFACT.to_string()),
                message,
            },
        }
    }

    /// Invariant 3: one line per hop, filing through dispatch.
    pub fn hop_statuses(&self, now: u64) -> Vec<HopStatus> {
        self.topology
            .steps()
            .iter()
            .map(|step| {
                let verdict = self.liveness.assess(step, &self.policy, now);
                HopStatus {
                    step: step.name.clone(),
                    line: verdict.line(&step.name),
                    failed: verdict.failed(),
                    verdict,
                }
            })
            .collect()
    }

    /// Full report: topology audit, hop liveness, consumer verdict.
    pub fn report(&self, queue: Option<&QueueFile>, now: u64) -> PipelineReport {
        PipelineReport {
            outcome: self.authorize_queue(queue, now),
            hops: self.hop_statuses(now),
            topology_violations: self.topology.audit(),
        }
    }
}
