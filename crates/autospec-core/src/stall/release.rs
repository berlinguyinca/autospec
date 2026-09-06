//! Stall release policy: the note, the attempt counter, and escalation.
//!
//! When an attempt is killed, three things must happen in this order: the work
//! is captured, the issue records what the attempt proved, and only then is the
//! issue handed on — re-queued onto a different model, or escalated. The
//! previous behaviour was a silent requeue with no note and no count, which lost
//! both the evidence and the fact that the attempt had happened at all.
//!
//! The deeper point is what a stall *means*. One stall is a fact about a model
//! and a quantization. A stall that survives rotation onto a different
//! architecture family is a fact about the **spec**: the task cannot be completed
//! as written, and retrying it onto a third model produces a third stall.

use super::attempts::{
    AttemptHistory, AttemptOutcome, AttemptRecord, ModelChoice, ModelRoster, Rotation,
};
use super::lease::LeasePolicy;
use super::liveness::Liveness;
pub use super::note::{SpecRepairReport, StallNote};
use super::partial_work::{Artifact, PartialWork, WorkProduced};

/// Label applied when an issue stops being retried by the implementer path.
pub const LABEL_ATTEMPTS_EXHAUSTED: &str = "stalled-attempts-exhausted";
/// Label applied when the issue goes to the spec-repair path.
pub const LABEL_SPEC_REPAIR: &str = "spec-repair";

/// Why an attempt was considered stalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallReason {
    /// The lease elapsed with no observed progress.
    LeaseExpired,
    /// Transcript and output both flat for the stall window.
    NoLiveness,
    /// The child process exited without a result.
    WorkerExited,
    /// The supervisor cancelled the attempt.
    Cancelled,
    /// The runner reported a model-level error.
    ModelError,
}

impl StallReason {
    pub fn label(&self) -> &'static str {
        match self {
            StallReason::LeaseExpired => "lease expired",
            StallReason::NoLiveness => "no liveness signal",
            StallReason::WorkerExited => "worker exited",
            StallReason::Cancelled => "cancelled",
            StallReason::ModelError => "model error",
        }
    }
}

/// Policy knobs for the release path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StallPolicy {
    pub lease: LeasePolicy,
    /// Flat-transcript seconds before a quiet run is declared stalled.
    pub stall_secs: u64,
    /// Attempts allowed per issue before escalation.
    pub max_attempts: u32,
    /// Transcript tail captured per attempt.
    pub transcript_tail_bytes: usize,
}

impl Default for StallPolicy {
    fn default() -> Self {
        Self {
            lease: LeasePolicy::default(),
            stall_secs: default_stall_secs(),
            max_attempts: default_max_attempts(),
            transcript_tail_bytes: default_transcript_tail_bytes(),
        }
    }
}

impl StallPolicy {
    pub fn from_env() -> Self {
        let stall_secs = env_u64("AUTOSPEC_ISSUE_STALL_SECS", default_stall_secs());
        let transcript_tail_bytes = env_u64("AUTOSPEC_STALL_TRANSCRIPT_TAIL_BYTES", 64 * 1_024)
            .min(usize::MAX as u64) as usize;
        let max_attempts = env_u64("AUTOSPEC_STALL_MAX_ATTEMPTS", default_max_attempts() as u64)
            .max(1)
            .min(u32::MAX as u64) as u32;
        Self {
            lease: LeasePolicy::from_env(),
            stall_secs,
            max_attempts,
            transcript_tail_bytes,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.lease.validate()?;
        if self.stall_secs == 0 {
            return Err("stall_secs must be > 0".to_string());
        }
        if self.max_attempts == 0 {
            return Err("max_attempts must be >= 1".to_string());
        }
        if self.transcript_tail_bytes == 0 {
            return Err("transcript_tail_bytes must be > 0".to_string());
        }
        Ok(())
    }
}

fn default_stall_secs() -> u64 {
    1_800
}

fn default_max_attempts() -> u32 {
    2
}

fn default_transcript_tail_bytes() -> usize {
    64 * 1_024
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(fallback)
}

/// Why rotation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationReason {
    /// The per-issue attempt limit was reached with models still untried.
    AttemptLimit { max_attempts: u32 },
    /// Every configured model already stalled.
    RosterExhausted { attempted: usize },
    /// Only one model is configured and it stalled twice: there is nothing to
    /// rotate to, and a third attempt on it re-learns nothing.
    RotationUnavailable,
    /// No model roster is configured at all.
    NoModels,
}

impl EscalationReason {
    pub fn sentence(&self) -> String {
        match self {
            EscalationReason::AttemptLimit { max_attempts } => {
                format!("hit the attempt limit of {max_attempts}")
            }
            EscalationReason::RosterExhausted { attempted } => {
                format!("every configured model has already stalled ({attempted})")
            }
            EscalationReason::RotationUnavailable => {
                "rotation unavailable: only one model is configured and it stalled twice"
                    .to_string()
            }
            EscalationReason::NoModels => {
                "no model roster is configured, so no rotation target exists".to_string()
            }
        }
    }
}

impl std::fmt::Display for EscalationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sentence())
    }
}

/// The decision for the attempt that just ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseDecision {
    /// The attempt produced a diff. Nothing is recorded; it goes to review.
    Completed { attempt: u32 },
    /// Re-queue with a recorded stall note, preferably on another model.
    Requeue {
        attempt: u32,
        next_model: ModelChoice,
        changed_family: bool,
        rotation_note: String,
    },
    /// Stop re-queueing and hand the issue to spec repair.
    Escalate {
        attempt: u32,
        reason: EscalationReason,
        report: SpecRepairReport,
    },
}

impl ReleaseDecision {
    pub fn attempt(&self) -> u32 {
        match self {
            ReleaseDecision::Completed { attempt }
            | ReleaseDecision::Requeue { attempt, .. }
            | ReleaseDecision::Escalate { attempt, .. } => *attempt,
        }
    }

    pub fn is_escalation(&self) -> bool {
        matches!(self, ReleaseDecision::Escalate { .. })
    }
}

/// Everything the release path needs to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasePlan {
    pub issue: u64,
    pub decision: ReleaseDecision,
    /// The stall note to record, or `None` for a clean completion.
    pub note: Option<StallNote>,
    /// Captured partial work to attach or hand to the next attempt.
    pub artifacts: Vec<Artifact>,
    pub work: WorkProduced,
}

impl ReleasePlan {
    pub fn note_markdown(&self) -> Option<String> {
        self.note.as_ref().map(StallNote::markdown)
    }
}

/// Decide the release for the attempt that just ended.
///
/// `history` must already contain that attempt as its last record; the decision
/// reads the outcome and the models tried so far. Pure: no tracker, no worktree,
/// no clock.
#[allow(clippy::too_many_arguments)]
pub fn plan_release(
    issue: u64,
    policy: &StallPolicy,
    roster: &ModelRoster,
    history: &AttemptHistory,
    work: &PartialWork,
    reason: StallReason,
    liveness: Liveness,
) -> ReleasePlan {
    let Some(record) = history.last() else {
        return ReleasePlan {
            issue,
            decision: ReleaseDecision::Completed { attempt: 0 },
            note: None,
            artifacts: Vec::new(),
            work: WorkProduced::None,
        };
    };

    let attempt = record.attempt;
    let produced = work.work_produced();

    // A finished attempt produces a diff for review, not a stall note.
    if record.outcome == AttemptOutcome::Completed {
        return ReleasePlan {
            issue,
            decision: ReleaseDecision::Completed { attempt },
            note: None,
            artifacts: Vec::new(),
            work: produced,
        };
    }

    let mut note = StallNote::from_record(
        issue,
        record,
        policy.max_attempts,
        produced,
        reason,
        liveness,
    );
    let artifacts = work.artifacts(attempt);
    let attempted = history.attempted_model_ids();

    // The attempt limit is checked before rotation: a limit that only fires once
    // the roster runs out is not a limit.
    if history.len() as u32 >= policy.max_attempts {
        let why = escalation_reason(roster, history, policy.max_attempts);
        note.next_step = Some(format!("escalate to spec repair: {}", why.sentence()));
        return escalate(issue, attempt, why, history, &artifacts, note, produced);
    }

    match roster.select_next(&attempted) {
        Rotation::Next {
            model,
            changed_family,
        } => requeue(
            issue,
            attempt,
            model.clone(),
            changed_family,
            rotation_note(&model, changed_family, &attempted),
            note,
            artifacts,
            produced,
        ),
        // The single-model case: retry it once, and say plainly that there is
        // nothing to rotate to.
        Rotation::SameModelRetry { model } => requeue(
            issue,
            attempt,
            model.clone(),
            false,
            format!(
                "rotation unavailable: {} is the only model configured. This is the single retry \
                 allowed before escalation.",
                model.label()
            ),
            note,
            artifacts,
            produced,
        ),
        rotation @ (Rotation::Exhausted | Rotation::NoModels) => {
            match single_retry_without_roster(rotation, history, record) {
                Some((model, degrade_note)) => requeue(
                    issue,
                    attempt,
                    model,
                    false,
                    degrade_note,
                    note,
                    artifacts,
                    produced,
                ),
                None => escalate(
                    issue,
                    attempt,
                    escalation_reason(roster, history, policy.max_attempts),
                    history,
                    &artifacts,
                    note,
                    produced,
                ),
            }
        }
    }
}

/// Requeue the issue onto `model`, recording the rotation sentence as the note's
/// next step so the handoff is self-contained.
#[allow(clippy::too_many_arguments)]
fn requeue(
    issue: u64,
    attempt: u32,
    model: ModelChoice,
    changed_family: bool,
    rotation_note: String,
    mut note: StallNote,
    artifacts: Vec<Artifact>,
    produced: WorkProduced,
) -> ReleasePlan {
    note.next_step = Some(rotation_note.clone());
    ReleasePlan {
        issue,
        decision: ReleaseDecision::Requeue {
            attempt,
            next_model: model,
            changed_family,
            rotation_note,
        },
        note: Some(note),
        artifacts,
        work: produced,
    }
}

/// With no roster at all the stalled model is still the only thing that could run
/// the task: degrade to one retry on it rather than refusing to act.
fn single_retry_without_roster(
    rotation: Rotation,
    history: &AttemptHistory,
    record: &AttemptRecord,
) -> Option<(ModelChoice, String)> {
    if !matches!(rotation, Rotation::NoModels) || history.len() >= 2 {
        return None;
    }
    let model = record.model.clone();
    if model.id.trim().is_empty() {
        return None;
    }
    let degrade_note = format!(
        "rotation unavailable: no model roster is configured, so {} gets the single retry before \
         escalation.",
        model.label()
    );
    Some((model, degrade_note))
}

/// Why rotation stopped. Rotation availability is judged before the attempt
/// limit: with one model configured, the honest reason a run stops is that there
/// was nothing to rotate to, not that a counter ran out.
fn escalation_reason(
    roster: &ModelRoster,
    history: &AttemptHistory,
    max_attempts: u32,
) -> EscalationReason {
    if roster.distinct_model_count() <= 1 && history.len() >= 2 {
        return EscalationReason::RotationUnavailable;
    }
    if history.len() as u32 >= max_attempts {
        return EscalationReason::AttemptLimit { max_attempts };
    }
    if roster.is_empty() {
        return EscalationReason::NoModels;
    }
    EscalationReason::RosterExhausted {
        attempted: history.len(),
    }
}

fn escalate(
    issue: u64,
    attempt: u32,
    reason: EscalationReason,
    history: &AttemptHistory,
    artifacts: &[Artifact],
    note: StallNote,
    produced: WorkProduced,
) -> ReleasePlan {
    let report = SpecRepairReport {
        issue,
        reason,
        attempts: history.records().to_vec(),
        artifact_paths: Vec::new(),
        escalation_sentence: history.escalation_sentence(),
    };
    ReleasePlan {
        issue,
        decision: ReleaseDecision::Escalate {
            attempt,
            reason,
            report,
        },
        note: Some(note),
        artifacts: artifacts.to_vec(),
        work: produced,
    }
}

fn rotation_note(model: &ModelChoice, changed_family: bool, attempted: &[String]) -> String {
    let tried = if attempted.is_empty() {
        "none".to_string()
    } else {
        attempted.join(", ")
    };
    if changed_family {
        format!(
            "next attempt: {next} (different family; already stalled on: {tried})",
            next = model.label()
        )
    } else {
        format!(
            "next attempt: {next} (same family; already stalled on: {tried})",
            next = model.label()
        )
    }
}
