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
//! A fifth concern lives in the same files, because it was born in them
//! (#3961): two issues filed hours apart from independent symptoms both
//! landed in the same dispatch-queue file and collided on dispatch, and
//! nothing at filing time compared them. Issues are filed by symptom and
//! implemented by file, so the comparison has to happen on the *file*, not
//! the subject:
//!
//! * **An issue records its predicted write surface** ([`IssueWriteSurface`])
//!   — the paths or modules its fix expects to touch, read from the issue
//!   body's `## Files touched` section.
//! * **Filing checks that surface against every open issue**
//!   ([`FilingOverlapCheck`]) and reports the shared entries, because the
//!   fix at filing time is one sentence — "related to #N; both touch the
//!   dispatch queue; serialise" — and after dispatch it costs a GPU run.
//! * **Overlapping issues are serialised, not run in parallel**
//!   ([`DispatchWaves`]): an issue joins the first wave its surface is free
//!   in; an issue with no declared surface joins no wave but its own.
//! * **A sibling that merges in flight is named on re-stage**
//!   ([`SiblingLanding`]): the re-staged spec says what landed and where,
//!   and instructs the agent to extend rather than re-implement, because its
//!   base snapshot is the world before the sibling.
//!
//! A sixth concern is the lifecycle of the entries in that queue (#3911). A
//! produced patch is not finished work: it is a patch waiting to be
//! converted, and a dispatcher that treats `produced` as terminal holds the
//! entry's slot forever — a queue that fills with such entries then looks
//! exactly like a queue with no work, and the stall is silent. The answer is
//! state the refresher cannot wipe:
//!
//! * **Each queue entry carries a lifecycle state** ([`EntryState`],
//!   [`LifecycleLedger`]): `queued` is the default, `produced` is
//!   mid-lifecycle, and `converted` is the only terminal state. The ledger is
//!   consumer-owned and the refresher never writes it, so a refresh cannot
//!   erase where the work is.
//! * **`produced` re-enters as conversion, not re-dispatch**
//!   ([`DispatchAction`]): the action for a produced entry is to convert the
//!   patch that already exists, and a held entry, when its reason clears,
//!   resumes from the state it was held in rather than starting over.
//! * **A tick that dispatches nothing reports every skip with a reason**
//!   ([`DispatchTick`], [`SkipReason`]): which entries are held and why, and
//!   which are converted and should leave the queue. Produced-but-unconverted
//!   is directly queryable ([`LifecycleLedger::produced_but_unconverted`]).
//! * **Dispatch is bounded by attempts, not by evidence of success** (#4451):
//!   the guard "already produced a patch" treats an *absent* patch as "not
//!   yet tried", so an issue that fails before producing one is redispatched
//!   forever — eight issues consumed 83 GPU dispatches. The ledger instead
//!   counts each fresh dispatch per entry, flags the entry in flight until an
//!   outcome is recorded, and holds an entry that reaches
//!   [`DEFAULT_MAX_DISPATCH_ATTEMPTS`] dispatches without a patch — with the
//!   count and the reason — instead of spending another run on it
//!   ([`SkipReason::InFlight`], [`SkipReason::AttemptBoundExceeded`]).
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

/// Fresh dispatches an entry may consume without producing a patch before it
/// is held rather than redispatched (#4451). The bound is on attempts, never
/// on the presence of a patch file: an issue dispatched N times with no patch
/// is a defect to be reported, not "not yet tried".
pub const DEFAULT_MAX_DISPATCH_ATTEMPTS: u64 = 3;

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
    /// An admitted issue (open, carrying the eligibility label) is not in the
    /// queue, so filed work will never run while the dispatcher idles over it.
    AdmittedNotSchedulable,
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
            Self::AdmittedNotSchedulable => "ADMITTED_NOT_SCHEDULABLE",
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

/// A reconciliation between the *admitted* issues and the *schedulable* ones
/// (the queue's entries).
///
/// The admitted set — the open issues carrying the eligibility label — is
/// authoritative: the label is the contract, and an open labelled issue is
/// schedulable with no second, hand-maintained step. The queue is a derived
/// copy of that set. When the copy lags the tracker, an admitted issue that is
/// not in the queue is *admitted-but-unschedulable*: filed work that will never
/// run, invisible because the queue looks like a complete, fresh artifact.
///
/// This reconciliation names that divergence. The count of
/// admitted-but-unschedulable issues is the defect: zero is the expected answer,
/// and any other number is a failure. The reverse direction (queue entries that
/// are no longer admitted — closed or label removed) is reported for cleanup
/// but is not the defect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulingReconciliation {
    /// Admitted but not in the queue: filed work that will never run. The
    /// count of these is the defect (nonzero is a failure).
    pub admitted_not_schedulable: Vec<u64>,
    /// In the queue but no longer admitted (closed or label removed): stale
    /// entries to clean up. Reported, not the defect.
    pub schedulable_not_admitted: Vec<u64>,
    /// Admitted and in the queue: the work that will actually run.
    pub schedulable: Vec<u64>,
}

fn join_numbers(numbers: &[u64]) -> String {
    numbers
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl SchedulingReconciliation {
    /// Reconcile the admitted set against the schedulable set (the queue's
    /// issue numbers). Both are deduplicated and reported in ascending order.
    pub fn new(
        admitted: impl IntoIterator<Item = u64>,
        schedulable: impl IntoIterator<Item = u64>,
    ) -> Self {
        let admitted: BTreeSet<u64> = admitted.into_iter().collect();
        let schedulable: BTreeSet<u64> = schedulable.into_iter().collect();
        Self {
            admitted_not_schedulable: admitted.difference(&schedulable).copied().collect(),
            schedulable_not_admitted: schedulable.difference(&admitted).copied().collect(),
            schedulable: admitted.intersection(&schedulable).copied().collect(),
        }
    }

    /// The defect count: admitted issues that are not schedulable. Zero is the
    /// expected answer; any other number is a failure.
    pub fn admitted_not_schedulable_count(&self) -> usize {
        self.admitted_not_schedulable.len()
    }

    /// Zero is the expected answer, any other number is a defect.
    pub fn is_defect(&self) -> bool {
        !self.admitted_not_schedulable.is_empty()
    }

    /// The one-line report, shaped for the periodic reconciliation and dispatch
    /// logs. It states the count and names the issues.
    pub fn line(&self) -> String {
        if self.is_defect() {
            let numbers = join_numbers(&self.admitted_not_schedulable);
            return format!(
                "RECONCILE DEFECT: {} admitted issue(s) are not schedulable: {}; the queue lags the tracker — refresh-queue must repopulate it",
                self.admitted_not_schedulable_count(),
                numbers
            );
        }
        if self.schedulable_not_admitted.is_empty() {
            return "RECONCILE clean: every admitted issue is schedulable".to_string();
        }
        let count = self.schedulable_not_admitted.len();
        let noun = if count == 1 { "entry" } else { "entries" };
        let stale = join_numbers(&self.schedulable_not_admitted);
        format!("RECONCILE clean with {count} stale queue {noun} to clean up: {stale}")
    }
}

/// The predicted write surface of one issue: the paths or modules its fix
/// is expected to touch, recorded at filing time (#3961).
///
/// Issues are filed by symptom, so two genuinely independent symptoms can
/// still name the same file — "produced patches hold their slot" and
/// "filing does not schedule" share no keywords worth searching, but both
/// touched the dispatch-queue file. A declared surface is what makes the
/// comparison mechanical at filing time, and it is the input the dispatcher
/// needs to serialise overlapping work instead of running it in parallel and
/// holding the loser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueWriteSurface {
    /// The issue that declares this surface.
    pub issue: u64,
    /// Normalised entries: trimmed, empty entries dropped, de-duplicated,
    /// sorted. A trailing `/` marks a directory and covers every path
    /// beneath it; anything else matches exactly.
    pub paths: BTreeSet<String>,
}

/// One entry of a write surface declared as a directory (`trailing /`).
fn is_directory_entry(entry: &str) -> bool {
    entry.ends_with('/')
}

/// Whether `candidate` lies strictly beneath the directory `entry` declares.
fn beneath(entry: &str, candidate: &str) -> bool {
    let Some(dir) = entry.strip_suffix('/') else {
        return false;
    };
    candidate.starts_with(dir) && candidate.len() > dir.len()
}

impl IssueWriteSurface {
    pub fn new(issue: u64, paths: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self {
            issue,
            paths: normalise_names(paths),
        }
    }

    /// Record the surface declared in the issue body's `## Files touched`
    /// section, the issue-quality contract's one-path-per-line grammar.
    /// `None` when the section is absent or declares no safe repo-relative
    /// path — an undeclared surface is preserved as undeclared, never
    /// guessed at.
    pub fn from_body(issue: u64, body: &str) -> Option<Self> {
        let mut in_section = false;
        let mut paths = BTreeSet::new();
        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("## ") {
                in_section = trimmed.trim_end() == "## Files touched";
                continue;
            }
            if !in_section || trimmed.is_empty() {
                continue;
            }
            let entry = trimmed
                .trim_start_matches('-')
                .trim()
                .trim_matches('`')
                .trim();
            if entry.is_empty()
                || entry.starts_with('/')
                || entry.contains(' ')
                || entry
                    .split('/')
                    .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                continue;
            }
            paths.insert(entry.to_string());
        }
        (!paths.is_empty()).then_some(Self { issue, paths })
    }

    /// The entries both surfaces claim for the same file or directory, sorted.
    /// Empty when the two issues can merge without a conflict. When one side
    /// declares a directory covering a path the other declares, the broader
    /// declaration is the shared entry named.
    pub fn shared_entries(&self, other: &IssueWriteSurface) -> Vec<String> {
        let mut shared = BTreeSet::new();
        for a in &self.paths {
            for b in &other.paths {
                let entry = if a == b || (is_directory_entry(a) && beneath(a, b)) {
                    Some(a)
                } else if is_directory_entry(b) && beneath(b, a) {
                    Some(b)
                } else {
                    None
                };
                if let Some(entry) = entry {
                    shared.insert(entry.clone());
                }
            }
        }
        shared.into_iter().collect()
    }

    /// Whether the two issues write the same file or directory.
    pub fn overlaps_with(&self, other: &IssueWriteSurface) -> bool {
        !self.shared_entries(other).is_empty()
    }
}

/// One overlap found by [`FilingOverlapCheck`]: which open issue, and which
/// surface entries collide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceOverlap {
    pub issue: u64,
    pub shared: Vec<String>,
}

/// The result of checking a newly filed issue's predicted write surface
/// against the open issues' surfaces (#3961, invariant 1).
///
/// The habit this replaces is searching the backlog for the same *topic*;
/// that would not catch two independent symptoms sharing a file. The check
/// is mechanical: every open surface is compared against the new one, and
/// every shared entry is reported at filing time — the moment the fix is one
/// sentence rather than a GPU run plus a design-level merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilingOverlapCheck {
    /// The issue being filed.
    pub issue: u64,
    /// Open issues that share at least one surface entry, with the shared
    /// entries named.
    pub overlaps: Vec<SurfaceOverlap>,
}

impl FilingOverlapCheck {
    /// Check `candidate` against the open issues' surfaces. The candidate's
    /// own surface, if it appears in `open`, is excluded.
    pub fn check(candidate: &IssueWriteSurface, open: &[IssueWriteSurface]) -> Self {
        let overlaps = open
            .iter()
            .filter(|other| other.issue != candidate.issue)
            .filter_map(|other| {
                let shared = candidate.shared_entries(other);
                (!shared.is_empty()).then_some(SurfaceOverlap {
                    issue: other.issue,
                    shared,
                })
            })
            .collect();
        Self {
            issue: candidate.issue,
            overlaps,
        }
    }

    /// No open issue shares a surface entry: the issue can be filed without a
    /// serialisation note.
    pub fn clean(&self) -> bool {
        self.overlaps.is_empty()
    }

    /// The lines the filer sees at filing time: one per overlap, naming the
    /// sibling and the shared entries, plus the serialise-or-merge instruction.
    pub fn lines(&self) -> Vec<String> {
        if self.overlaps.is_empty() {
            return vec![format!(
                "WRITE-SURFACE clean: #{} declares a surface no open issue shares",
                self.issue
            )];
        }
        self.overlaps
            .iter()
            .map(|overlap| {
                format!(
                    "WRITE-SURFACE OVERLAP: #{} shares {} with open issue #{} — serialise or merge, and name the sibling on the later issue",
                    self.issue,
                    overlap.shared.join(", "),
                    overlap.issue
                )
            })
            .collect()
    }
}

/// The dispatcher's concurrency plan for the queue's entries (#3961,
/// invariant 3).
///
/// Every wave is a set of issues whose declared surfaces are pairwise
/// disjoint, so the waves may run in parallel; within and across waves an
/// issue runs no sooner than the first wave its surface is free. Two issues
/// that share a write surface therefore never run concurrently: serialising
/// costs latency, colliding costs a dispatch plus a merge a supervisor should
/// not be doing in someone else's interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchWaves {
    pub waves: Vec<Vec<u64>>,
}

impl DispatchWaves {
    /// Plan `entries` (in queue order) into waves using `surfaces`.
    ///
    /// An issue with no declared surface cannot be proven disjoint from
    /// anything, so it takes a wave to itself rather than riding with
    /// neighbours whose conflict it would silently cause.
    pub fn plan(entries: impl IntoIterator<Item = u64>, surfaces: &[IssueWriteSurface]) -> Self {
        let declared: BTreeMap<u64, &IssueWriteSurface> = surfaces
            .iter()
            .map(|surface| (surface.issue, surface))
            .collect();
        let disjoint = |candidate: &IssueWriteSurface, member: u64| match declared.get(&member) {
            Some(other) => !candidate.overlaps_with(other),
            // A wave member with no declared surface cannot be proven
            // disjoint, so it overlaps everything by default.
            None => false,
        };
        let mut waves: Vec<Vec<u64>> = Vec::new();
        for entry in entries {
            let wave = match declared.get(&entry) {
                Some(candidate) => waves
                    .iter()
                    .position(|wave| wave.iter().all(|member| disjoint(candidate, *member)))
                    .unwrap_or(waves.len()),
                None => waves.len(),
            };
            if waves.len() == wave {
                waves.push(Vec::new());
            }
            waves[wave].push(entry);
        }
        Self { waves }
    }

    /// The entries that run in the same wave as `entry`: the set the
    /// dispatcher may launch concurrently with it.
    pub fn concurrent_with(&self, entry: u64) -> Vec<u64> {
        self.waves
            .iter()
            .find(|wave| wave.contains(&entry))
            .map(|wave| {
                wave.iter()
                    .copied()
                    .filter(|member| *member != entry)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// What a merged sibling left behind, recorded when the sibling lands while
/// the issue is still in flight (#3961, invariant 4).
///
/// The re-dispatched agent's base snapshot is the world before the sibling,
/// so without a note it re-derives the same conflict — parallel types for the
/// same subsystem, the same `use` list and doc-table rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiblingLanding {
    /// The sibling that merged.
    pub issue: u64,
    /// What it introduced (types, subcommands, failure codes), named.
    pub introduced: BTreeSet<String>,
    /// The files it changed: where the re-stage extends rather than
    /// re-implements.
    pub touched: BTreeSet<String>,
}

/// Trim, drop empties, de-duplicate and sort a list of names or paths.
fn normalise_names(items: impl IntoIterator<Item = impl AsRef<str>>) -> BTreeSet<String> {
    items
        .into_iter()
        .map(|item| item.as_ref().trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

impl SiblingLanding {
    pub fn new(
        issue: u64,
        introduced: impl IntoIterator<Item = impl AsRef<str>>,
        touched: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        Self {
            issue,
            introduced: normalise_names(introduced),
            touched: normalise_names(touched),
        }
    }

    /// The re-stage note appended to the spec the agent reads: what landed,
    /// where, and the instruction to extend rather than re-implement.
    pub fn note(&self, target: u64) -> String {
        format!(
            "SIBLING LANDED while #{} was in flight: #{} merged, introducing {} in {}. Extend what it added — do not re-implement it or shadow it with parallel types.",
            target,
            self.issue,
            self.introduced.iter().cloned().collect::<Vec<_>>().join(", "),
            self.touched.iter().cloned().collect::<Vec<_>>().join(", ")
        )
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

    /// The dispatcher's verdict when it also knows the *admitted* set — the
    /// open issues carrying the eligibility label, which the label makes
    /// schedulable with no second, hand-maintained step.
    ///
    /// A fresh, empty queue reads as [`DispatchOutcome::Idle`]: "no new issues
    /// were filed." That sentence is a lie while admitted issues sit outside
    /// the queue — the dispatcher then holds free capacity over work that was
    /// filed and labelled, and the idleness is silent rather than a reported
    /// fault. An idle dispatcher with admitted-but-unschedulable work is a
    /// fault, not a quiet state: free capacity and eligible issues should be
    /// loud.
    ///
    /// This layers [`SchedulingReconciliation`] on [`Self::authorize_queue`]: a
    /// fresh-empty queue (free capacity) with admitted-but-unschedulable issues
    /// is a [`DispatchOutcome::Hold`] naming the refresh step, not an [`Idle`].
    /// A queue that is already non-empty (proceeding) or already held is left
    /// as-is — the reconciliation remains the general defect detector either
    /// way.
    pub fn authorize_queue_with_admission(
        &self,
        queue: Option<&QueueFile>,
        admitted: impl IntoIterator<Item = u64>,
        now: u64,
    ) -> DispatchOutcome {
        let base = self.authorize_queue(queue, now);
        if !matches!(base, DispatchOutcome::Idle { .. }) {
            return base;
        }
        let reconciliation = SchedulingReconciliation::new(
            admitted,
            queue.map(|queue| queue.entries.clone()).unwrap_or_default(),
        );
        if !reconciliation.is_defect() {
            return base;
        }
        let step = self
            .queue_producer()
            .map(|step| step.name.clone())
            .unwrap_or_else(|| "refresh-queue".to_string());
        let numbers: Vec<String> = reconciliation
            .admitted_not_schedulable
            .iter()
            .map(u64::to_string)
            .collect();
        Self::hold(
            FailureCode::AdmittedNotSchedulable,
            step.clone(),
            format!(
                "{} admitted issue(s) are not in the queue: {}; free capacity over filed work — the queue lags the tracker and {} must repopulate it",
                reconciliation.admitted_not_schedulable_count(),
                numbers.join(", "),
                step
            ),
        )
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

// ── Entry lifecycle (#3911) ────────────────────────────────────────────────

/// Where one queue entry is in its lifecycle.
///
/// `produced` is a state mid-lifecycle, not a terminal one (#3911): the
/// patch exists and the remaining work is to convert it. A dispatcher that
/// treats `produced` as done holds the entry's slot forever, and a queue that
/// fills with such entries looks exactly like a queue with no work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryState {
    /// The default: filed and schedulable, no patch yet.
    Queued,
    /// An agent produced a patch; the remaining work is conversion.
    Produced,
    /// The only terminal state: the patch became a PR/commit. The entry
    /// should leave the queue; a tick reports it instead of dispatching it.
    Converted,
}

impl EntryState {
    /// `converted` is the only state the lifecycle ends in.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Converted)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Produced => "produced",
            Self::Converted => "converted",
        }
    }
}

/// One entry's lifecycle record: its state, the reason it is held (when it
/// is), and when the record was last stamped.
///
/// A hold is a flag on the state, not a state of its own: it preserves what
/// the entry was while held, so when the reason clears the entry resumes from
/// exactly where it stopped — a released `produced` entry re-enters as a
/// conversion, not as a fresh dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRecord {
    pub state: EntryState,
    /// Why the entry is held; `None` when it is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_reason: Option<String>,
    /// Epoch seconds of the last accepted stamp.
    pub recorded_at: u64,
    /// Dispatches that ended without a patch, since the last reset (a
    /// `produced` / `converted` stamp, or an explicit `release`). The
    /// dispatch bound (#4451) is enforced on this count, never on the
    /// presence of a patch file.
    #[serde(default)]
    pub attempts: u64,
    /// The entry is in flight since this instant: a fresh dispatch was
    /// recorded and no outcome (`produced` / `failed`) since. `None` when
    /// nothing is running for it — "no patch" is only "untried" or "kept
    /// failing" then, never "still going" (#4451).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<u64>,
}

/// The consumer-owned durable record of where each queue entry is in its
/// lifecycle (#3911).
///
/// The queue artifact is rewritten by the refresher from the tracker's label
/// set on every pass; it cannot carry per-entry state without the refresher
/// wiping it. The ledger lives on the consumer's side and the refresher never
/// touches it, so state survives refreshes. An entry the ledger has never
/// seen is [`EntryState::Queued`]: absence of a record is the default, not a
/// hole.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleLedger {
    records: BTreeMap<u64, EntryRecord>,
}

impl LifecycleLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// The effective state: the stamped state, or [`EntryState::Queued`] for
    /// an entry the ledger has never seen.
    pub fn state_of(&self, issue: u64) -> EntryState {
        self.records
            .get(&issue)
            .map(|record| record.state)
            .unwrap_or(EntryState::Queued)
    }

    /// The raw record, when one exists.
    pub fn record_of(&self, issue: u64) -> Option<&EntryRecord> {
        self.records.get(&issue)
    }

    /// Stamp a state. Monotonic in `at`: a stamp older than the record's
    /// current one is refused, so a stale "converted" cannot rewind a newer
    /// record (ties are accepted — re-stamping is idempotent). Stamping a
    /// state clears any hold on the entry: an explicit stamp supersedes the
    /// hold. It also resets the dispatch-attempt count and clears the
    /// in-flight flag: a stamped outcome (a produced patch, a conversion)
    /// is evidence the no-patch loop has broken (#4451).
    pub fn record(&mut self, issue: u64, state: EntryState, at: u64) -> bool {
        if let Some(existing) = self.records.get(&issue) {
            if at < existing.recorded_at {
                return false;
            }
        }
        self.records.insert(
            issue,
            EntryRecord {
                state,
                held_reason: None,
                recorded_at: at,
                attempts: 0,
                dispatched_at: None,
            },
        );
        true
    }

    /// Record a fresh dispatch for the entry (#4451): flag it in flight
    /// until an outcome is recorded. The attempt count is deliberately NOT
    /// advanced here — a dispatch that is still running has not yet failed,
    /// and the count only advances when a run ends without a patch
    /// ([`Self::record_failed`]). The state and any hold are preserved.
    /// Refused when the stamp is older than the record's current one: a
    /// stale dispatch cannot rewind the record.
    pub fn record_attempt(&mut self, issue: u64, at: u64) -> bool {
        let existing = self.records.get(&issue).cloned().unwrap_or(EntryRecord {
            state: EntryState::Queued,
            held_reason: None,
            recorded_at: 0,
            attempts: 0,
            dispatched_at: None,
        });
        if at < existing.recorded_at {
            return false;
        }
        self.records.insert(
            issue,
            EntryRecord {
                state: existing.state,
                held_reason: existing.held_reason,
                recorded_at: at,
                attempts: existing.attempts,
                dispatched_at: Some(at),
            },
        );
        true
    }

    /// Record that a run ended without a patch (#4451): advance the attempt
    /// count and clear the in-flight flag. The state and any hold are
    /// preserved — a failed run is not evidence of success, the entry stays
    /// `queued`, and the bound is what stops the redispatch. The count is on
    /// dispatches that ended with no patch, so a run that never reports an
    /// outcome neither advances the count nor is redispatched: it sits in
    /// flight, named on every tick. Refused for terminal entries and for
    /// stamps older than the record's current one. Returns the new attempt
    /// count.
    pub fn record_failed(&mut self, issue: u64, at: u64) -> Option<u64> {
        if self.state_of(issue).is_terminal() {
            return None;
        }
        let existing = self.records.get(&issue).cloned().unwrap_or(EntryRecord {
            state: EntryState::Queued,
            held_reason: None,
            recorded_at: 0,
            attempts: 0,
            dispatched_at: None,
        });
        if at < existing.recorded_at {
            return None;
        }
        let attempts = existing.attempts.saturating_add(1);
        self.records.insert(
            issue,
            EntryRecord {
                state: existing.state,
                held_reason: existing.held_reason,
                recorded_at: at,
                attempts,
                dispatched_at: None,
            },
        );
        Some(attempts)
    }

    /// The attempt count for one entry: fresh dispatches recorded since the
    /// last reset. Zero for an entry the ledger has never seen.
    pub fn attempts_of(&self, issue: u64) -> u64 {
        self.records
            .get(&issue)
            .map(|record| record.attempts)
            .unwrap_or(0)
    }

    /// When the entry's current run was dispatched, if it is in flight:
    /// a fresh dispatch was recorded and no outcome since. `None` when
    /// nothing is running for it.
    pub fn in_flight_since(&self, issue: u64) -> Option<u64> {
        self.records
            .get(&issue)
            .and_then(|record| record.dispatched_at)
    }

    /// Hold a non-terminal entry with the reason. The state is preserved — a
    /// held `produced` entry is still a `produced` entry — and the hold is
    /// what the next tick reports as the skip reason. An entry the ledger has
    /// never seen is created as `queued` and held. The attempt count and the
    /// in-flight flag are preserved: holding an entry does not erase its
    /// failures. Refused when the reason is empty, the entry is terminal, or
    /// the stamp is older than the record.
    pub fn hold(&mut self, issue: u64, reason: &str, at: u64) -> bool {
        if reason.trim().is_empty() {
            return false;
        }
        let state = self.state_of(issue);
        if state.is_terminal() {
            return false;
        }
        if let Some(existing) = self.records.get(&issue) {
            if at < existing.recorded_at {
                return false;
            }
        }
        let attempts = self.attempts_of(issue);
        let dispatched_at = self.in_flight_since(issue);
        self.records.insert(
            issue,
            EntryRecord {
                state,
                held_reason: Some(reason.trim().to_string()),
                recorded_at: at,
                attempts,
                dispatched_at,
            },
        );
        true
    }

    /// Clear a hold. The state is preserved, so a released `produced` entry
    /// re-enters the next tick as a conversion, not a fresh dispatch.
    /// Release is also the operator's explicit re-arm after triage (#4451):
    /// it resets the attempt count and clears the in-flight flag, so a
    /// released entry gets a full bound of fresh dispatches again. An entry
    /// held over the dispatch bound re-arms only this way — the bound is on
    /// attempts, and triage is what earns a new budget. Idempotent on an
    /// entry that is not held. Refused when there is no record to release or
    /// the entry is terminal.
    pub fn release(&mut self, issue: u64, at: u64) -> bool {
        let Some(existing) = self.records.get(&issue) else {
            return false;
        };
        if existing.state.is_terminal() {
            return false;
        }
        if at < existing.recorded_at {
            return false;
        }
        let mut record = existing.clone();
        record.held_reason = None;
        record.recorded_at = at;
        record.attempts = 0;
        record.dispatched_at = None;
        self.records.insert(issue, record);
        true
    }

    /// The entries that produced a patch that has not been converted: the
    /// directly queryable answer to "what is sitting unconverted" (#3911).
    /// Held entries count — the hold is a reason they are unconverted, not an
    /// escape from the question.
    pub fn produced_but_unconverted(&self) -> Vec<u64> {
        self.records
            .iter()
            .filter(|(_, record)| record.state == EntryState::Produced)
            .map(|(issue, _)| *issue)
            .collect()
    }

    /// How many entries are sitting on a produced, unconverted patch.
    pub fn unconverted_count(&self) -> usize {
        self.produced_but_unconverted().len()
    }

    /// Every issue the ledger has a record for, ascending.
    pub fn issues(&self) -> impl Iterator<Item = &u64> {
        self.records.keys()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| error.to_string())
    }
}

/// What a tick does with one dispatched entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DispatchAction {
    /// No patch yet: run a fresh agent.
    Dispatch,
    /// A patch already exists: convert it. Re-running the agent for work that
    /// already produced output is the #3911 failure.
    Convert,
}

impl DispatchAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Convert => "convert",
        }
    }
}

/// One entry a tick decided to dispatch, with the action it takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchedEntry {
    pub issue: u64,
    /// The state the decision was made from.
    pub state: EntryState,
    pub action: DispatchAction,
}

/// Why a tick skipped one entry.
///
/// A tick that dispatches nothing over a non-empty queue is a stall signal,
/// and the signal is only useful when it names the cause per entry: a skip
/// with no reason is the silent version of the bug it reports (#3911).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SkipReason {
    /// The entry is held; the recorded reason travels with the skip.
    Held {
        reason: String,
        /// The state the entry was held in: it is preserved, not erased.
        state: EntryState,
    },
    /// The entry is terminal: the converted work should leave the queue, and
    /// the tick names it instead of dispatching it again.
    Converted,
    /// The entry is in flight: a fresh dispatch was recorded and no outcome
    /// since. The tick waits for the result rather than redispatching a run
    /// that is still going (#4451).
    InFlight { dispatched_at: u64 },
    /// The entry was dispatched `attempts` times without producing a patch
    /// and `bound` is the cap it has reached. It is held — with the count
    /// and the reason — not redispatched (#4451).
    AttemptBoundExceeded { attempts: u64, bound: u64 },
}

impl SkipReason {
    /// The state the skip refers to.
    pub fn state(&self) -> EntryState {
        match self {
            Self::Held { state, .. } => *state,
            Self::Converted => EntryState::Converted,
            // Both are fresh-dispatch states: attempts only accumulate on
            // queued entries, and a stamped outcome resets them.
            Self::InFlight { .. } | Self::AttemptBoundExceeded { .. } => EntryState::Queued,
        }
    }

    /// The per-entry report text, after the issue number.
    pub fn line(&self) -> String {
        match self {
            Self::Held { reason, state } => format!("held [{}]: {reason}", state.as_str()),
            Self::Converted => "converted [terminal]: should leave the queue".to_string(),
            Self::InFlight { dispatched_at } => format!(
                "in flight: dispatched at {dispatched_at}, no outcome recorded — not redispatched"
            ),
            Self::AttemptBoundExceeded { attempts, bound } => format!(
                "held [queued]: dispatch bound — {attempts} dispatches with no patch (bound {bound}); held for triage, not redispatched"
            ),
        }
    }
}

/// One entry a tick skipped, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub issue: u64,
    pub reason: SkipReason,
}

/// One dispatch tick over the queue (#3911): the join of the queue artifact
/// (which entries exist) and the lifecycle ledger (where each entry is in its
/// lifecycle).
///
/// The tick is pure and only reports; it mutates nothing. The caller marks
/// entries `produced` / `converted` / held as the work happens, and the next
/// tick reflects that. Wave planning ([`DispatchWaves`]) is the caller's
/// concern over the dispatched set: the tick decides *which* entries act on,
/// the waves decide which of them act concurrently.
#[derive(Debug)]
pub struct DispatchTick {
    dispatched: Vec<DispatchedEntry>,
    skipped: Vec<SkippedEntry>,
}

impl DispatchTick {
    /// Join the queue with the lifecycle ledger, in queue order, enforcing
    /// the default dispatch bound ([`DEFAULT_MAX_DISPATCH_ATTEMPTS`]).
    pub fn run(queue: &QueueFile, lifecycle: &LifecycleLedger) -> Self {
        Self::run_bounded(queue, lifecycle, DEFAULT_MAX_DISPATCH_ATTEMPTS)
    }

    /// Join the queue with the lifecycle ledger, in queue order, with an
    /// explicit dispatch bound (#4451).
    ///
    /// Per entry: `converted` → skipped as terminal; attempts at or over the
    /// bound → skipped over the dispatch bound, carrying the count and the
    /// bound; held → skipped, carrying the hold reason and the state it was
    /// held in; a recorded fresh dispatch with no outcome → skipped in
    /// flight; `produced` → dispatched with [`DispatchAction::Convert`];
    /// `queued` (the default) → dispatched with [`DispatchAction::Dispatch`].
    ///
    /// The bound is checked before the hold: an entry held over the bound
    /// keeps reporting as over the bound on every run, so the skip is never
    /// silently re-labelled a plain hold. The tick is pure and only reports;
    /// [`Self::apply_to_ledger`] is what makes its decisions durable.
    pub fn run_bounded(queue: &QueueFile, lifecycle: &LifecycleLedger, bound: u64) -> Self {
        let mut dispatched = Vec::new();
        let mut skipped = Vec::new();
        for &issue in &queue.entries {
            let state = lifecycle.state_of(issue);
            if state == EntryState::Converted {
                skipped.push(SkippedEntry {
                    issue,
                    reason: SkipReason::Converted,
                });
                continue;
            }
            let record = lifecycle.record_of(issue);
            if let Some(record) = record {
                if record.attempts >= bound {
                    skipped.push(SkippedEntry {
                        issue,
                        reason: SkipReason::AttemptBoundExceeded {
                            attempts: record.attempts,
                            bound,
                        },
                    });
                    continue;
                }
                if let Some(reason) = &record.held_reason {
                    skipped.push(SkippedEntry {
                        issue,
                        reason: SkipReason::Held {
                            reason: reason.clone(),
                            state,
                        },
                    });
                    continue;
                }
                if let Some(dispatched_at) = record.dispatched_at {
                    skipped.push(SkippedEntry {
                        issue,
                        reason: SkipReason::InFlight { dispatched_at },
                    });
                    continue;
                }
            }
            dispatched.push(DispatchedEntry {
                issue,
                state,
                action: if state == EntryState::Produced {
                    DispatchAction::Convert
                } else {
                    DispatchAction::Dispatch
                },
            });
        }
        Self {
            dispatched,
            skipped,
        }
    }

    /// The mutating half of the tick (#4451): record this tick's decisions in
    /// the ledger so they survive the dispatcher process. Every fresh
    /// dispatch advances the entry's attempt count and flags it in flight;
    /// every entry skipped over the dispatch bound is held with the count
    /// and the reason, so the next tick — and every human looking at the
    /// ledger — sees it as "repeatedly failed", not silently re-queued.
    /// Returns the number of ledger records written, so a caller with zero
    /// changes can skip the write.
    pub fn apply_to_ledger(&self, lifecycle: &mut LifecycleLedger, at: u64) -> usize {
        let mut written = 0;
        for entry in &self.dispatched {
            if entry.action == DispatchAction::Dispatch && lifecycle.record_attempt(entry.issue, at)
            {
                written += 1;
            }
        }
        for entry in &self.skipped {
            if let SkipReason::AttemptBoundExceeded { attempts, bound } = entry.reason {
                let reason = format!(
                    "dispatch bound: {attempts} dispatches with no patch (bound {bound}) — held for triage, not redispatched until released"
                );
                if lifecycle.hold(entry.issue, &reason, at) {
                    written += 1;
                }
            }
        }
        written
    }

    pub fn dispatched(&self) -> &[DispatchedEntry] {
        &self.dispatched
    }

    pub fn skipped(&self) -> &[SkippedEntry] {
        &self.skipped
    }

    /// A tick that dispatches nothing over a non-empty queue is a stall, not
    /// an idle state: the caller exits non-zero and reads the skip reasons.
    pub fn dispatched_anything(&self) -> bool {
        !self.dispatched.is_empty()
    }

    /// The dispatched entries that run a fresh agent.
    pub fn fresh_count(&self) -> usize {
        self.dispatched
            .iter()
            .filter(|entry| entry.action == DispatchAction::Dispatch)
            .count()
    }

    /// The dispatched entries that convert a patch that already exists.
    pub fn convert_count(&self) -> usize {
        self.dispatched
            .iter()
            .filter(|entry| entry.action == DispatchAction::Convert)
            .count()
    }

    /// The skipped entries that are held.
    pub fn held_count(&self) -> usize {
        self.skipped
            .iter()
            .filter(|entry| matches!(entry.reason, SkipReason::Held { .. }))
            .count()
    }

    /// The skipped entries that are terminal and should leave the queue.
    pub fn converted_count(&self) -> usize {
        self.skipped
            .iter()
            .filter(|entry| matches!(entry.reason, SkipReason::Converted))
            .count()
    }

    /// The skipped entries that are in flight: dispatched, outcome pending.
    pub fn in_flight_count(&self) -> usize {
        self.skipped
            .iter()
            .filter(|entry| matches!(entry.reason, SkipReason::InFlight { .. }))
            .count()
    }

    /// The skipped entries that exceeded the dispatch bound (#4451). The run
    /// must report this count — a silent skip reproduces the invisibility
    /// the bound exists to remove.
    pub fn over_bound_count(&self) -> usize {
        self.skipped
            .iter()
            .filter(|entry| matches!(entry.reason, SkipReason::AttemptBoundExceeded { .. }))
            .count()
    }

    /// The per-entry report: one summary line, then one line per entry.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![self.summary_line()];
        lines.extend(self.dispatched.iter().map(|entry| {
            format!(
                "#{issue} {} [{}]",
                entry.action.as_str(),
                entry.state.as_str(),
                issue = entry.issue
            )
        }));
        lines.extend(
            self.skipped
                .iter()
                .map(|entry| format!("#{issue} {}", entry.reason.line(), issue = entry.issue)),
        );
        lines
    }

    /// The non-zero skip categories, in fixed order. They partition every
    /// skip reason, so a non-empty skip set always names at least one.
    fn skip_categories(&self) -> String {
        let mut categories = Vec::new();
        if self.held_count() > 0 {
            categories.push(format!("{} held", self.held_count()));
        }
        if self.converted_count() > 0 {
            categories.push(format!("{} converted", self.converted_count()));
        }
        if self.in_flight_count() > 0 {
            categories.push(format!("{} in flight", self.in_flight_count()));
        }
        if self.over_bound_count() > 0 {
            categories.push(format!("{} over dispatch bound", self.over_bound_count()));
        }
        categories.join(", ")
    }

    fn summary_line(&self) -> String {
        if self.dispatched.is_empty() && self.skipped.is_empty() {
            return "dispatch tick: queue empty — nothing to dispatch".to_string();
        }
        if self.dispatched.is_empty() {
            return format!(
                "dispatch tick: nothing dispatched — {} entries skipped ({})",
                self.skipped.len(),
                self.skip_categories(),
            );
        }
        // The over-bound and in-flight counts ride in the summary even when
        // other entries dispatched: a silent skip reproduces the
        // invisibility the dispatch bound exists to remove (#4451).
        let notable: Vec<String> = [
            (
                self.in_flight_count() > 0,
                format!("{} in flight", self.in_flight_count()),
            ),
            (
                self.over_bound_count() > 0,
                format!("{} over dispatch bound", self.over_bound_count()),
            ),
        ]
        .into_iter()
        .filter(|(present, _)| *present)
        .map(|(_, text)| text)
        .collect();
        let mut line = format!(
            "dispatch tick: {} dispatched ({} fresh, {} convert), {} skipped",
            self.dispatched.len(),
            self.fresh_count(),
            self.convert_count(),
            self.skipped.len(),
        );
        if !notable.is_empty() {
            line.push_str(&format!(" ({})", notable.join(", ")));
        }
        line
    }

    pub fn to_json(&self) -> String {
        #[derive(Serialize)]
        struct TickJson<'a> {
            dispatched: &'a [DispatchedEntry],
            skipped: &'a [SkippedEntry],
        }
        serde_json::to_string_pretty(&TickJson {
            dispatched: &self.dispatched,
            skipped: &self.skipped,
        })
        .unwrap_or_else(|_| "{}".to_string())
    }
}

// ── Write-surface overlap (#3961) ───────────────────────────────────────────

#[cfg(test)]
mod write_surface_tests {
    use super::*;

    fn body_with_files(files: &[&str]) -> String {
        let mut body = String::from("## Goal\nFile the fix.\n\n## Files touched\n");
        for file in files {
            body.push_str(&format!("- `{file}`\n"));
        }
        body.push_str("\n## Acceptance criteria\n- [ ] done\n");
        body
    }

    #[test]
    fn surface_normalizes_trims_dedupes_and_sorts() {
        let surface = IssueWriteSurface::new(
            3927,
            [
                " crates/autospec-core/src/dispatch_pipeline.rs ",
                "docs/cli-reference.md",
                "docs/cli-reference.md",
                "  ",
                "crates/autospec-core/src/queue.rs",
            ],
        );
        assert_eq!(surface.paths, {
            let mut set = BTreeSet::new();
            set.insert("crates/autospec-core/src/dispatch_pipeline.rs".to_string());
            set.insert("crates/autospec-core/src/queue.rs".to_string());
            set.insert("docs/cli-reference.md".to_string());
            set
        });
    }

    #[test]
    fn directory_entry_matches_descendants_not_siblings() {
        let directory = IssueWriteSurface::new(1, ["crates/autospec-core/src/"]);
        let descendant = IssueWriteSurface::new(2, ["crates/autospec-core/src/queue.rs"]);
        let sibling = IssueWriteSurface::new(3, ["crates/autospec-cli/src/main.rs"]);
        let crammed = IssueWriteSurface::new(4, ["crates/autospec-core-src/"]);

        assert!(directory.overlaps_with(&descendant));
        assert!(descendant.overlaps_with(&directory));
        assert!(!directory.overlaps_with(&sibling));
        // A near-miss prefix (`-src/` vs `src/`) is a different directory.
        assert!(!directory.overlaps_with(&crammed));
        assert_eq!(
            directory.shared_entries(&descendant),
            vec!["crates/autospec-core/src/"]
        );
    }

    #[test]
    fn from_body_records_the_files_touched_section() {
        let body = body_with_files(&[
            "crates/autospec-core/src/dispatch_pipeline.rs",
            "docs/cli-reference.md",
        ]);
        let surface = IssueWriteSurface::from_body(3961, &body).expect("section present");
        assert_eq!(surface.issue, 3961);
        assert_eq!(surface.paths.len(), 2);
        assert!(surface
            .paths
            .contains("crates/autospec-core/src/dispatch_pipeline.rs"));
    }

    #[test]
    fn from_body_rejects_unsafe_paths_and_missing_sections() {
        let absolute = body_with_files(&["/etc/passwd"]);
        let dotdot = body_with_files(&["crates/../secrets"]);
        let prose = body_with_files(&["the dispatch queue file"]);
        assert!(IssueWriteSurface::from_body(1, &absolute).is_none());
        assert!(IssueWriteSurface::from_body(1, &dotdot).is_none());
        assert!(IssueWriteSurface::from_body(1, &prose).is_none());
        // No section, no surface: preserved as undeclared, never guessed at.
        assert!(IssueWriteSurface::from_body(1, "## Goal\nNo surface section here.\n").is_none());
    }

    #[test]
    fn filing_check_reports_overlap_naming_the_sibling_and_the_file() {
        // The #3911/#3793 shape: two issues with disjoint subjects that both
        // declare the dispatch-queue file.
        let new_issue = IssueWriteSurface::from_body(
            3961,
            &body_with_files(&["crates/autospec-core/src/dispatch_pipeline.rs"]),
        )
        .unwrap();
        let open = vec![
            IssueWriteSurface::from_body(
                3927,
                &body_with_files(&["crates/autospec-core/src/dispatch_pipeline.rs"]),
            )
            .unwrap(),
            IssueWriteSurface::from_body(
                3911,
                &body_with_files(&["crates/autospec-core/src/queue.rs"]),
            )
            .unwrap(),
        ];

        let check = FilingOverlapCheck::check(&new_issue, &open);
        assert!(!check.clean());
        assert_eq!(
            check.overlaps,
            vec![SurfaceOverlap {
                issue: 3927,
                shared: vec!["crates/autospec-core/src/dispatch_pipeline.rs".to_string()],
            }]
        );
        let line = &check.lines()[0];
        assert!(line.contains("WRITE-SURFACE OVERLAP"));
        assert!(line.contains("#3927"), "{line}");
        assert!(line.contains("dispatch_pipeline.rs"), "{line}");
        assert!(line.contains("serialise"), "{line}");
    }

    #[test]
    fn filing_check_excludes_the_candidate_itself_and_reports_clean() {
        let surface = IssueWriteSurface::from_body(
            12,
            &body_with_files(&["crates/autospec-core/src/queue.rs"]),
        )
        .unwrap();
        let open = vec![surface.clone(), surface.clone()];

        assert!(FilingOverlapCheck::check(&surface, &open).clean());
        let lines = FilingOverlapCheck::check(&surface, &open).lines();
        assert_eq!(
            lines,
            vec!["WRITE-SURFACE clean: #12 declares a surface no open issue shares"]
        );
    }

    #[test]
    fn planner_serialises_issues_sharing_a_surface() {
        // Two issues declaring the same file must never share a wave.
        let surfaces = vec![
            IssueWriteSurface::new(10, ["crates/autospec-core/src/queue.rs"]),
            IssueWriteSurface::new(11, ["crates/autospec-core/src/queue.rs"]),
        ];
        let plan = DispatchWaves::plan([10, 11], &surfaces);

        assert_eq!(plan.waves, vec![vec![10], vec![11]]);
        assert!(
            plan.concurrent_with(10).is_empty(),
            "#11 shares #10's file and must not run alongside it"
        );
    }

    #[test]
    fn planner_runs_disjoint_surfaces_in_one_wave() {
        let surfaces = vec![
            IssueWriteSurface::new(10, ["crates/autospec-core/src/queue.rs"]),
            IssueWriteSurface::new(11, ["docs/cli-reference.md"]),
        ];
        let plan = DispatchWaves::plan([10, 11], &surfaces);

        assert_eq!(plan.waves, vec![vec![10, 11]]);
        assert_eq!(plan.concurrent_with(10), vec![11]);
    }

    #[test]
    fn planner_lets_a_third_issue_join_the_earliest_free_wave() {
        let surfaces = vec![
            IssueWriteSurface::new(10, ["a.rs"]),
            IssueWriteSurface::new(11, ["a.rs"]),
            IssueWriteSurface::new(12, ["b.rs"]),
        ];
        // 12 is disjoint from 10 as well, so it joins the first wave rather
        // than waiting behind 11 for no reason.
        let plan = DispatchWaves::plan([10, 11, 12], &surfaces);

        assert_eq!(plan.waves, vec![vec![10, 12], vec![11]]);
    }

    #[test]
    fn planner_serialises_undeclared_surfaces_into_their_own_wave() {
        // No declared surface is not a clean surface: it cannot be proven
        // disjoint, so it rides alone.
        let surfaces = vec![IssueWriteSurface::new(10, ["a.rs"])];
        let plan = DispatchWaves::plan([10, 11, 12], &surfaces);

        assert_eq!(plan.waves, vec![vec![10], vec![11], vec![12]]);
    }

    #[test]
    fn sibling_landing_note_names_what_landed_and_instructs_to_extend() {
        let landing = SiblingLanding::new(
            3927,
            [
                "FailureCode::AdmittedNotSchedulable",
                "autospec dispatch reconcile",
            ],
            [
                "crates/autospec-core/src/dispatch_pipeline.rs",
                "docs/cli-reference.md",
            ],
        );
        let note = landing.note(3911);

        assert!(note.contains("#3911"), "{note}");
        assert!(note.contains("#3927"), "{note}");
        assert!(
            note.contains("FailureCode::AdmittedNotSchedulable"),
            "{note}"
        );
        assert!(note.contains("dispatch_pipeline.rs"), "{note}");
        assert!(note.contains("Extend"), "{note}");
        assert!(note.contains("do not re-implement"), "{note}");
    }

    #[test]
    fn populated_case_two_issues_one_file_are_overlapping_and_serialised() {
        // The populated case from #3793: the queue already holds 242 entries,
        // two of them — #123 and #237 — declare the same file, and a third
        // issue (#243) is being filed against that file now. The filing check
        // reports both siblings, and the plan never places the same-file
        // issues in one wave while the disjoint issues run in parallel.
        let queue: Vec<u64> = (1..=242).collect();
        let same_file = "crates/autospec-core/src/dispatch_pipeline.rs";
        let mut surfaces: Vec<IssueWriteSurface> = (1..=242)
            .map(|n| IssueWriteSurface::new(n, [format!("crates/autospec-core/src/mod-{n}.rs")]))
            .collect();
        surfaces[122] = IssueWriteSurface::new(123, [same_file]);
        surfaces[236] = IssueWriteSurface::new(237, [same_file]);

        let filing = IssueWriteSurface::new(243, [same_file]);
        let check = FilingOverlapCheck::check(&filing, &surfaces);
        assert_eq!(
            check.overlaps,
            vec![
                SurfaceOverlap {
                    issue: 123,
                    shared: vec![same_file.to_string()],
                },
                SurfaceOverlap {
                    issue: 237,
                    shared: vec![same_file.to_string()],
                },
            ]
        );
        assert_eq!(check.lines().len(), 2);

        let plan = DispatchWaves::plan(queue.iter().copied(), &surfaces);
        let wave_of = |entry: u64| plan.waves.iter().position(|wave| wave.contains(&entry));
        assert_eq!(wave_of(123), Some(0));
        assert!(
            wave_of(237) > Some(0),
            "#237 shares #123's file and must land in a later wave"
        );
        assert!(!plan.waves[0].contains(&237));
        // The disjoint issues still ride wave zero: serialising the same-file
        // pair must not serialise the whole queue.
        assert!(plan.waves[0].contains(&1));
        assert!(plan.waves[0].contains(&242));
        // And the newly filed issue overlaps both siblings, so wherever it
        // plans it shares no wave with either of them.
        let extended = std::iter::once(243).chain(queue.iter().copied());
        let surfaces = std::iter::once(&filing)
            .chain(surfaces.iter())
            .cloned()
            .collect::<Vec<_>>();
        let replan = DispatchWaves::plan(extended, &surfaces);
        let wave_243 = replan
            .waves
            .iter()
            .position(|wave| wave.contains(&243))
            .expect("#243 is planned");
        assert!(!replan.waves[wave_243].contains(&123));
        assert!(!replan.waves[wave_243].contains(&237));
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    fn queue_with(entries: &[u64]) -> QueueFile {
        QueueFile {
            entries: entries.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn unknown_entries_default_to_queued() {
        let ledger = LifecycleLedger::new();
        assert_eq!(ledger.state_of(101), EntryState::Queued);
        let tick = DispatchTick::run(&queue_with(&[101]), &ledger);
        assert_eq!(tick.dispatched().len(), 1);
        assert_eq!(tick.dispatched()[0].action, DispatchAction::Dispatch);
        assert_eq!(tick.dispatched()[0].state, EntryState::Queued);
        assert!(tick.skipped().is_empty());
    }

    #[test]
    fn produced_entries_dispatch_as_convert_not_fresh() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(102, EntryState::Produced, 100));
        let tick = DispatchTick::run(&queue_with(&[102]), &ledger);
        assert_eq!(tick.dispatched().len(), 1);
        assert_eq!(tick.dispatched()[0].action, DispatchAction::Convert);
        assert_eq!(tick.dispatched()[0].state, EntryState::Produced);
    }

    #[test]
    fn converted_entries_are_skipped_and_named() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(110, EntryState::Converted, 100));
        let tick = DispatchTick::run(&queue_with(&[110]), &ledger);
        assert!(!tick.dispatched_anything());
        assert_eq!(tick.skipped().len(), 1);
        assert_eq!(tick.skipped()[0].reason, SkipReason::Converted);
        assert!(tick
            .lines()
            .iter()
            .any(|line| line.contains("should leave the queue")));
    }

    #[test]
    fn mixed_tick_reports_every_entry() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(102, EntryState::Produced, 100));
        assert!(ledger.record(108, EntryState::Produced, 90));
        assert!(ledger.hold(108, "conversion blocked: branch dirty", 110));
        assert!(ledger.record(110, EntryState::Converted, 120));
        let tick = DispatchTick::run(&queue_with(&[101, 102, 108, 110]), &ledger);

        assert_eq!(tick.fresh_count(), 1);
        assert_eq!(tick.convert_count(), 1);
        assert_eq!(tick.held_count(), 1);
        assert_eq!(tick.converted_count(), 1);

        let lines = tick.lines();
        assert_eq!(
            lines[0],
            "dispatch tick: 2 dispatched (1 fresh, 1 convert), 2 skipped"
        );
        assert!(lines.iter().any(|line| line == "#101 dispatch [queued]"));
        assert!(lines.iter().any(|line| line == "#102 convert [produced]"));
        assert!(lines
            .iter()
            .any(|line| line == "#108 held [produced]: conversion blocked: branch dirty"));
        assert!(lines
            .iter()
            .any(|line| line == "#110 converted [terminal]: should leave the queue"));
    }

    #[test]
    fn stall_tick_reports_every_skip_with_reason() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(102, EntryState::Produced, 100));
        assert!(ledger.hold(102, "convert gate red", 110));
        assert!(ledger.record(110, EntryState::Converted, 120));
        let tick = DispatchTick::run(&queue_with(&[102, 110]), &ledger);

        assert!(!tick.dispatched_anything());
        assert_eq!(tick.skipped().len(), 2);
        let lines = tick.lines();
        assert_eq!(
            lines[0],
            "dispatch tick: nothing dispatched — 2 entries skipped (1 held, 1 converted)"
        );
        assert!(lines
            .iter()
            .any(|line| line == "#102 held [produced]: convert gate red"));
    }

    #[test]
    fn empty_queue_tick_is_quiet_and_clean() {
        let ledger = LifecycleLedger::new();
        let tick = DispatchTick::run(&queue_with(&[]), &ledger);
        assert!(!tick.dispatched_anything());
        assert!(tick.skipped().is_empty());
        assert_eq!(
            tick.lines(),
            vec!["dispatch tick: queue empty — nothing to dispatch"]
        );
    }

    #[test]
    fn produced_but_unconverted_lists_and_counts() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(101, EntryState::Queued, 10));
        assert!(ledger.record(102, EntryState::Produced, 20));
        assert!(ledger.record(103, EntryState::Produced, 30));
        assert!(ledger.record(110, EntryState::Converted, 40));
        // a held produced entry still counts: the hold is a reason it is
        // unconverted, not an escape from the question
        assert!(ledger.hold(102, "convert gate red", 50));

        assert_eq!(ledger.produced_but_unconverted(), vec![102, 103]);
        assert_eq!(ledger.unconverted_count(), 2);
    }

    #[test]
    fn hold_preserves_state_and_release_reenters_as_convert() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(108, EntryState::Produced, 100));
        assert!(ledger.hold(108, "branch dirty", 110));
        assert_eq!(ledger.state_of(108), EntryState::Produced);
        assert_eq!(
            ledger
                .record_of(108)
                .and_then(|record| record.held_reason.as_deref()),
            Some("branch dirty")
        );

        // held: skipped, carrying the reason
        let tick = DispatchTick::run(&queue_with(&[108]), &ledger);
        assert!(!tick.dispatched_anything());
        assert!(matches!(
            tick.skipped()[0].reason,
            SkipReason::Held { ref reason, .. } if reason == "branch dirty"
        ));

        // the reason clears: the entry resumes as conversion, not fresh
        // dispatch
        assert!(ledger.release(108, 120));
        let tick = DispatchTick::run(&queue_with(&[108]), &ledger);
        assert_eq!(tick.dispatched().len(), 1);
        assert_eq!(tick.dispatched()[0].action, DispatchAction::Convert);
        assert_eq!(ledger.state_of(108), EntryState::Produced);

        // release is idempotent on an entry that is not held
        assert!(ledger.release(108, 130));
        // but refuses entries with no record at all
        assert!(!ledger.release(999, 140));
    }

    #[test]
    fn hold_refuses_empty_reasons_and_terminal_entries() {
        let mut ledger = LifecycleLedger::new();
        assert!(!ledger.hold(101, "   ", 100));
        assert!(!ledger.hold(101, "", 100));
        assert!(ledger.record(110, EntryState::Converted, 100));
        assert!(!ledger.hold(110, "too late", 110));
        assert!(!ledger.release(110, 120));
    }

    #[test]
    fn stamps_are_monotonic_and_the_ledger_round_trips() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(101, EntryState::Produced, 200));
        // an older stamp is refused: a stale "converted" cannot rewind the
        // record
        assert!(!ledger.record(101, EntryState::Converted, 100));
        assert_eq!(ledger.state_of(101), EntryState::Produced);
        // a tie is accepted: re-stamping is idempotent
        assert!(ledger.record(101, EntryState::Converted, 200));
        assert_eq!(ledger.state_of(101), EntryState::Converted);

        // an explicit stamp clears a hold on the entry
        let mut held = LifecycleLedger::new();
        assert!(held.record(102, EntryState::Produced, 100));
        assert!(held.hold(102, "gate red", 110));
        assert!(held.record(102, EntryState::Produced, 120));
        assert!(held
            .record_of(102)
            .and_then(|record| record.held_reason.as_ref())
            .is_none());

        // the ledger round-trips through its durable form
        let text = held.to_json();
        let back = LifecycleLedger::from_json(&text).expect("ledger json parses");
        assert_eq!(held, back);
        let empty = LifecycleLedger::from_json(r#"{"records":{}}"#).expect("empty ledger parses");
        assert!(empty.is_empty());
    }
}

// ── Dispatch bound (#4451) ────────────────────────────────────────────────

#[cfg(test)]
mod dispatch_bound_tests {
    use super::*;

    const BOUND: u64 = 3;

    fn queue_with(entries: &[u64]) -> QueueFile {
        QueueFile {
            entries: entries.to_vec(),
            ..Default::default()
        }
    }

    /// One dispatch cycle: run the tick over the ledger, then make the
    /// tick's decisions durable.
    fn dispatch_once(ledger: &mut LifecycleLedger, queue: &QueueFile, at: u64) -> DispatchTick {
        let tick = DispatchTick::run_bounded(queue, ledger, BOUND);
        tick.apply_to_ledger(ledger, at);
        tick
    }

    #[test]
    fn fresh_dispatches_flag_in_flight_without_advancing_the_count() {
        let mut ledger = LifecycleLedger::new();
        let queue = queue_with(&[101, 102]);

        let tick = dispatch_once(&mut ledger, &queue, 100);
        assert_eq!(tick.fresh_count(), 2);
        // A running dispatch has not failed yet: only a no-patch outcome
        // advances the count.
        assert_eq!(ledger.attempts_of(101), 0);
        assert_eq!(ledger.in_flight_since(101), Some(100));

        // The next tick must not redispatch a run that is still going: the
        // "no patch because in flight" state is explicit, not inferred from
        // the absence of a file.
        let next = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert_eq!(next.in_flight_count(), 2);
        assert!(next.dispatched().is_empty());
        assert_eq!(
            next.skipped()[0].reason,
            SkipReason::InFlight { dispatched_at: 100 }
        );
        assert_eq!(
            next.lines()[0],
            "dispatch tick: nothing dispatched — 2 entries skipped (2 in flight)"
        );
        // Nothing new to record: a second pass writes nothing, and a running
        // dispatch has not failed, so the count has not advanced.
        assert_eq!(next.apply_to_ledger(&mut ledger, 110), 0);
        assert_eq!(ledger.attempts_of(101), 0);
    }

    #[test]
    fn failed_outcomes_advance_the_count_and_clear_in_flight() {
        let mut ledger = LifecycleLedger::new();
        let queue = queue_with(&[101]);

        for (attempt, at) in [100, 110, 120].into_iter().enumerate() {
            let tick = dispatch_once(&mut ledger, &queue, at);
            assert_eq!(tick.fresh_count(), 1, "attempt {attempt} dispatches");
            // While in flight, a second tick must not redispatch the run.
            let mid = DispatchTick::run_bounded(&queue, &ledger, BOUND);
            assert_eq!(
                mid.skipped()[0].reason,
                SkipReason::InFlight { dispatched_at: at }
            );
            // The run ends without a patch.
            let count = ledger.record_failed(101, at + 5).expect("failed recorded");
            assert_eq!(count, (attempt + 1) as u64);
            assert_eq!(ledger.in_flight_since(101), None);
        }

        // Three dispatches, no patch: the fourth is refused and the entry is
        // held, with the count and the reason, not redispatched.
        let held_tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert!(held_tick.dispatched().is_empty());
        assert_eq!(
            held_tick.skipped()[0].reason,
            SkipReason::AttemptBoundExceeded {
                attempts: 3,
                bound: BOUND
            }
        );
        assert_eq!(
            held_tick.lines()[0],
            "dispatch tick: nothing dispatched — 1 entries skipped (1 over dispatch bound)"
        );
        assert_eq!(held_tick.apply_to_ledger(&mut ledger, 200), 1);
        let reason = ledger
            .record_of(101)
            .and_then(|record| record.held_reason.as_deref())
            .expect("the hold is durable");
        assert!(reason.contains("3 dispatches with no patch"), "{reason}");
        assert!(reason.contains("bound 3"), "{reason}");

        // The bound is checked before the hold: every later run keeps
        // reporting the skip as over the bound, never as a plain hold.
        let again = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert_eq!(
            again.skipped()[0].reason,
            SkipReason::AttemptBoundExceeded {
                attempts: 3,
                bound: BOUND
            }
        );
        assert_eq!(again.over_bound_count(), 1);
    }

    #[test]
    fn release_rearms_a_bound_held_entry_with_a_fresh_budget() {
        let mut ledger = LifecycleLedger::new();
        let queue = queue_with(&[101]);
        for at in [100, 110, 120] {
            let _ = dispatch_once(&mut ledger, &queue, at);
            ledger.record_failed(101, at + 5).expect("failed recorded");
        }
        let bound_tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert_eq!(bound_tick.over_bound_count(), 1);
        bound_tick.apply_to_ledger(&mut ledger, 200);

        // Triage: the operator releases the hold. The entry gets a fresh
        // budget, not a continuation of the old count.
        assert!(ledger.release(101, 210));
        assert_eq!(ledger.attempts_of(101), 0);
        let rearmed = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert_eq!(rearmed.fresh_count(), 1);

        // Without the release, the bound holds: releasing is the only path
        // back to dispatch.
        let mut stuck = LifecycleLedger::new();
        for at in [100, 110, 120] {
            let _ = dispatch_once(&mut stuck, &queue, at);
            stuck.record_failed(101, at + 5).expect("failed recorded");
        }
        let stuck_tick = DispatchTick::run_bounded(&queue, &stuck, BOUND);
        stuck_tick.apply_to_ledger(&mut stuck, 200);
        let still_bound = DispatchTick::run_bounded(&queue, &stuck, BOUND);
        assert_eq!(
            still_bound.over_bound_count(),
            1,
            "without release the bound holds"
        );
    }

    #[test]
    fn a_produced_stamp_resets_attempts_and_clears_in_flight() {
        let mut ledger = LifecycleLedger::new();
        let queue = queue_with(&[101]);
        dispatch_once(&mut ledger, &queue, 100);
        ledger.record_failed(101, 110).expect("failed recorded");
        assert_eq!(ledger.attempts_of(101), 1);

        // The patch lands: the no-patch loop is broken and the count is
        // evidence no longer needed.
        assert!(ledger.record(101, EntryState::Produced, 120));
        assert_eq!(ledger.attempts_of(101), 0);
        assert_eq!(ledger.in_flight_since(101), None);
        let tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert_eq!(tick.convert_count(), 1);
        assert!(tick.skipped().is_empty());
    }

    #[test]
    fn over_bound_is_reported_even_when_other_entries_dispatch() {
        let mut ledger = LifecycleLedger::new();
        let queue = queue_with(&[101, 102]);
        for at in [100, 110, 120] {
            ledger.record_failed(101, at).expect("failed recorded");
        }

        let tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert_eq!(tick.fresh_count(), 1, "102 is still dispatched");
        assert_eq!(tick.over_bound_count(), 1);
        assert_eq!(
            tick.lines()[0],
            "dispatch tick: 1 dispatched (1 fresh, 0 convert), 1 skipped (1 over dispatch bound)"
        );
    }

    #[test]
    fn hold_preserves_the_attempt_count_and_in_flight_flag() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record_attempt(101, 100));
        assert_eq!(ledger.record_failed(101, 105), Some(1));
        assert!(ledger.record_attempt(101, 110));
        assert_eq!(ledger.record_failed(101, 115), Some(2));

        // A manual hold preserves the failures; the in-flight flag too.
        assert!(ledger.record_attempt(101, 120));
        assert!(ledger.hold(101, "waiting on dependency", 125));
        assert_eq!(ledger.attempts_of(101), 2);
        assert_eq!(ledger.in_flight_since(101), Some(120));

        // A failed outcome advances the count and clears the flag.
        assert_eq!(ledger.record_failed(101, 130), Some(3));
        assert_eq!(ledger.in_flight_since(101), None);
    }

    #[test]
    fn failed_is_refused_for_terminal_and_stale_stamps() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record(110, EntryState::Converted, 100));
        assert_eq!(ledger.record_failed(110, 110), None);

        let mut stale = LifecycleLedger::new();
        assert!(stale.record_attempt(101, 200));
        assert_eq!(stale.record_failed(101, 100), None, "stale stamp refused");
        assert_eq!(stale.attempts_of(101), 0);
    }

    #[test]
    fn attempt_fields_survive_the_durable_form_and_absent_fields_default() {
        let mut ledger = LifecycleLedger::new();
        assert!(ledger.record_attempt(101, 100));
        assert_eq!(ledger.record_failed(101, 105), Some(1));
        assert!(ledger.record_attempt(101, 110));
        assert!(ledger.hold(
            101,
            "dispatch bound: 1 dispatches with no patch (bound 3)",
            120
        ));

        let text = ledger.to_json();
        let back = LifecycleLedger::from_json(&text).expect("ledger json parses");
        assert_eq!(ledger, back);
        assert_eq!(back.attempts_of(101), 1);
        assert_eq!(back.in_flight_since(101), Some(110));

        // A ledger written before #4451 has no attempt fields at all: it
        // parses, and the entry reads as untried, never as failed.
        let old =
            LifecycleLedger::from_json(r#"{"records":{"101":{"state":"queued","recorded_at":5}}}"#)
                .expect("pre-4451 ledger parses");
        assert_eq!(old.attempts_of(101), 0);
        assert_eq!(old.in_flight_since(101), None);
        let tick = DispatchTick::run_bounded(&queue_with(&[101]), &old, BOUND);
        assert_eq!(tick.fresh_count(), 1);
    }
}
