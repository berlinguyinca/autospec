//! Bounded liveness for a pool of generation workers (issue #4372).
//!
//! The invariants this module exists to enforce:
//!
//! 1. A liveness check must exercise the path the client uses. `GET /health`
//!    is served by the HTTP layer and answers while generation is stuck, so a
//!    constant-cost liveness check is blind to the failure it exists to catch.
//! 2. A detector that fires and is ignored is worse than no detector. Its
//!    warnings make the system look observed while nothing acts on them.
//! 3. "Busy is not dead" is a bound, not a verdict. A deadline miss is
//!    inconclusive on its own (a busy worker's probe queues behind production
//!    traffic), but a miss contradicted by an idle GPU, or a miss that persists
//!    beyond a bound when no independent signal is available, is a wedge.
//!    Decide; do not keep forever.
//! 4. Rotation is mitigation, not a cure. It restores service, but it must be
//!    reported as mitigation and its recurrence must be tracked, or the
//!    underlying fault becomes invisible behind the remedy.
//!
//! The incident: all five `qwen3.8-flash-next` workers answered `GET /health`
//! with 200 while answering a generation request with 000. The completion
//! probe timed out 237 times and every warning was `probe timed out; leaving
//! worker in the pool (busy is not dead)`. The wedged worker had the GPU at
//! 0%, half its VRAM free, and three of four slots idle. The detector fired
//! 237 times and nothing acted, and it was the presence of those warnings that
//! hid the wedge.
//!
//! This module builds on [`inferweave`](super::inferweave) rather than
//! restating it: `classify_probe` still says a *single* deadline miss is
//! inconclusive (the starvation argument, preserved), and the bound added
//! here is what makes a *persisted* or *contradicted* miss decisive instead of
//! a perpetual keep.
//!
//! Every function here is pure. Callers perform I/O (probe the worker, sample
//! the GPU, rotate a worker) with the verdicts and findings these functions
//! return.

use super::inferweave::{
    classify_probe, DeadReason, LivenessProbe, PoolAction, ProbeCheck, ProbeSignal, ProbeVerdict,
};

/// The default bound on how long "busy is not dead" may persist before a
/// deadline miss with no independent signal is decided a wedge, in seconds.
///
/// Deliberately much larger than the probe deadline (5s): a genuinely busy
/// worker's probe queues behind production traffic and times out, but a queue
/// that drains between requests still completes the probe. Only a stuck worker
/// goes longer than the bound without a single completed probe.
pub const DEFAULT_INCONCLUSIVE_BOUND_SECS: u64 = 600;

// ── Invariant 1: the liveness check must exercise the client's path ─────────

/// Whether a probe check exercises the path the client uses (generation) or
/// only the control plane that answers while generation is stuck.
///
/// A completion runs the model, so it exercises the client's path. A health or
/// model-list check reads metadata served by the HTTP layer, which answers 200
/// while generation is wedged.
pub fn exercises_client_path(check: &ProbeCheck) -> bool {
    matches!(check, ProbeCheck::Completion { .. })
}

/// The blind spot of a liveness check that does not exercise the client's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blindspot {
    /// The check is constant-cost and never runs the model: it answers while
    /// generation is stuck, which is exactly the failure the check exists to
    /// catch.
    ControlPlaneOnly,
}

impl Blindspot {
    pub fn line(&self) -> String {
        match self {
            Blindspot::ControlPlaneOnly => {
                "liveness check does not exercise the client's path: it answers \
                 while generation is stuck"
                    .to_string()
            }
        }
    }
}

/// The blind spot of a check that gates a pool of generation workers, if any.
///
/// `None` means the check exercises the client's path and is not blind to the
/// wedge. `Some(ControlPlaneOnly)` means the check is constant-cost and will
/// answer while generation is stuck — the incident, where every liveness check
/// in the fleet was a `GET /health`.
pub fn liveness_blindspot(gating: &ProbeCheck) -> Option<Blindspot> {
    if exercises_client_path(gating) {
        None
    } else {
        Some(Blindspot::ControlPlaneOnly)
    }
}

// ── Invariant 3: "busy is not dead" is a bound, not a verdict ───────────────

/// An out-of-band observation of the worker's GPU, independent of the probe
/// that timed out.
///
/// The probe's own latency cannot tell a wedged worker from a busy one: on a
/// busy worker the probe queues behind production traffic and times out. The
/// GPU can. This is the *conclusion* of the out-of-band observation (e.g. "the
/// GPU was at 0% across six samples"), not a single sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndependentSignal {
    /// The GPU is at or near 0%: no generation is running. A "busy" worker
    /// whose GPU is idle is not loaded — it is wedged.
    Idle,
    /// The GPU is actively generating: consistent with a genuinely busy worker
    /// whose probe is queued behind production traffic (the starvation
    /// argument). The bound does not evict an active-GPU worker.
    Active,
}

/// The verdict of a liveness assessment that applies a bound to "busy is not
/// dead".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundedVerdict {
    /// The worker answered: alive.
    Alive,
    /// A definitive failure: refused, reset, error status, or identity
    /// mismatch.
    Dead(DeadReason),
    /// A deadline miss that is neither contradicted by an idle GPU nor
    /// persisted beyond the bound: "busy is not dead". Keep.
    Busy,
    /// A deadline miss contradicted by an idle GPU: a "busy" worker whose GPU
    /// is idle is wedged, not loaded. Evict.
    WedgedIdleGpu,
    /// A deadline miss that persisted beyond the bound with no independent
    /// signal: busy for longer than the bound is dead. Evict.
    WedgedBeyondBound,
}

impl BoundedVerdict {
    /// Only a decisive verdict evicts. `Busy` keeps the worker, preserving the
    /// starvation argument for a single or active-GPU miss.
    pub fn pool_action(&self) -> PoolAction {
        match self {
            BoundedVerdict::Alive | BoundedVerdict::Busy => PoolAction::Keep,
            BoundedVerdict::Dead(_)
            | BoundedVerdict::WedgedIdleGpu
            | BoundedVerdict::WedgedBeyondBound => PoolAction::Evict,
        }
    }

    /// True when the verdict is a death verdict that must act (evict/rotate).
    /// This is the action a fired detector is owed: a wedge verdict is not
    /// logged and dropped.
    pub fn decisive(&self) -> bool {
        !matches!(self, BoundedVerdict::Alive | BoundedVerdict::Busy)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            BoundedVerdict::Alive => "alive",
            BoundedVerdict::Dead(reason) => reason.as_str(),
            BoundedVerdict::Busy => "busy",
            BoundedVerdict::WedgedIdleGpu => "wedged-idle-gpu",
            BoundedVerdict::WedgedBeyondBound => "wedged-beyond-bound",
        }
    }
}

/// Decide keep/evict for a worker, applying a bound to "busy is not dead".
///
/// `streak_start` is when the *current* consecutive deadline-miss streak began
/// (or `None` if the worker is currently answering); the streak resets on any
/// answer, so it measures how long the worker has gone without completing a
/// probe, not how long it has been probed.
///
/// A definitive signal and a live answer classify exactly as
/// [`classify_probe`] does. A deadline miss is distinguished using the
/// independent GPU signal when present and the bound when it is not:
///
/// * an **idle** GPU contradicts "busy" and is decisive on its own;
/// * an **active** GPU proves the worker is doing real work, so the probe is
///   queued (the starvation argument) and the worker is kept no matter how
///   long the streak;
/// * with **no** signal, a miss within the bound is kept ("busy is not dead")
///   and a miss that has persisted beyond it is decided a wedge, rather than
///   kept forever.
pub fn bound_decision(
    probe: &LivenessProbe,
    latest: &ProbeSignal,
    streak_start: Option<u64>,
    now: u64,
    independent: Option<IndependentSignal>,
    bound_secs: u64,
) -> BoundedVerdict {
    match classify_probe(probe, latest) {
        ProbeVerdict::Dead { reason } => return BoundedVerdict::Dead(reason),
        ProbeVerdict::Alive => return BoundedVerdict::Alive,
        ProbeVerdict::Inconclusive => {}
    }

    match independent {
        Some(IndependentSignal::Idle) => BoundedVerdict::WedgedIdleGpu,
        Some(IndependentSignal::Active) => BoundedVerdict::Busy,
        None => match streak_start {
            Some(start) if now.saturating_sub(start) >= bound_secs => {
                BoundedVerdict::WedgedBeyondBound
            }
            _ => BoundedVerdict::Busy,
        },
    }
}

// ── Invariants 1 + 3 combined: a sound liveness policy gates on a bounded
// completion ─────────────────────────────────────────────────────────────────

/// A liveness policy: the check that gates the pool, and whether its verdict
/// is bounded.
///
/// A sound policy gates on a completion (which exercises the client's path,
/// invariant 1) whose verdict is bounded (a persisted or GPU-contradicted miss
/// escalates to a wedge, invariant 3).
///
/// This is the post-incident resolution and it supersedes the pre-incident
/// premise that the gating check be constant-cost (see
/// [`LivenessProbe::validate`](super::inferweave::LivenessProbe::validate)).
/// That premise passed validation and looked correct, and it was blind: the
/// check answered while generation was wedged. The resolution is not to make
/// the constant-cost check more frequent but to gate on a *bounded* completion
/// — it sees the wedge (invariant 1) and a single miss is still "busy is not
/// dead" while a persisted or GPU-contradicted miss is a wedge (invariant 3),
/// which is what preserves the starvation argument. `findings` reports which
/// half of that requirement is missing.
#[derive(Debug, Clone, PartialEq)]
pub struct LivenessPolicy {
    /// The check whose verdict gates pool membership.
    pub gating: ProbeCheck,
    /// Whether the gating verdict is bounded: a persisted or GPU-contradicted
    /// miss escalates to a wedge instead of a perpetual keep.
    pub bounded: bool,
}

/// Why a liveness policy is unsound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessPolicyFinding {
    /// The gating check does not exercise the client's path: it answers while
    /// generation is stuck.
    BlindGatingCheck,
    /// The gating check exercises the client's path but its verdict is
    /// unbounded: a single miss never escalates, so a wedged worker is kept
    /// forever (the starvation argument applied without a bound).
    UnboundedCompletion,
}

impl LivenessPolicyFinding {
    pub fn line(&self) -> String {
        match self {
            LivenessPolicyFinding::BlindGatingCheck => {
                "the gating check does not exercise the client's path".to_string()
            }
            LivenessPolicyFinding::UnboundedCompletion => {
                "the completion's verdict is unbounded: a wedged worker is kept \
                 forever"
                    .to_string()
            }
        }
    }
}

impl LivenessPolicy {
    /// The findings of the policy. A sound policy (a bounded completion) has
    /// none.
    pub fn findings(&self) -> Vec<LivenessPolicyFinding> {
        let mut out = Vec::new();
        if !exercises_client_path(&self.gating) {
            out.push(LivenessPolicyFinding::BlindGatingCheck);
        }
        if exercises_client_path(&self.gating) && !self.bounded {
            out.push(LivenessPolicyFinding::UnboundedCompletion);
        }
        out
    }
}

// ── Invariant 2: a detector that fires must act ─────────────────────────────

/// A record of a detector's firings and the actions they led to.
///
/// A detector that fires is ignored is worse than no detector: its warnings
/// make the system look observed while nothing acts. The incident detector
/// logged 237 "probe timed out" warnings — every one `flash-next` — and took
/// zero actions, and it was the presence of those warnings that hid the wedge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorLedger {
    /// The detector's name (e.g. "probe-timed-out").
    pub name: String,
    /// How many times the detector fired.
    pub firings: u64,
    /// How many firings led to an action (evict, rotate, alert).
    pub actions: u64,
}

/// Why a detector's record is a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorFinding {
    /// The detector fired but took no action: its warnings made the system
    /// look observed while nothing acted.
    Ignored {
        /// How many times the detector fired without acting.
        firings: u64,
    },
}

impl DetectorFinding {
    pub fn line(&self) -> String {
        match self {
            DetectorFinding::Ignored { firings } => {
                format!(
                    "{firings} detection(s) produced zero actions: firing without \
                     action is worse than no detector"
                )
            }
        }
    }
}

impl DetectorLedger {
    /// A detector that has fired but taken no action is ignored.
    pub fn ignored(&self) -> bool {
        self.firings > 0 && self.actions == 0
    }

    /// The finding, when the detector is ignored.
    pub fn finding(&self) -> Option<DetectorFinding> {
        if self.ignored() {
            Some(DetectorFinding::Ignored {
                firings: self.firings,
            })
        } else {
            None
        }
    }

    /// The operator line: firings, actions, and the finding when ignored.
    pub fn line(&self) -> String {
        match self.finding() {
            Some(finding) => format!(
                "detector {:?}: {} firing(s), 0 action(s) — {}",
                self.name,
                self.firings,
                finding.line()
            ),
            None => format!(
                "detector {:?}: {} firing(s), {} action(s)",
                self.name, self.firings, self.actions
            ),
        }
    }
}

// ── Invariant 4: rotation is mitigation, and its recurrence is tracked ─────

/// How a rotation is reported to the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MitigationClaim {
    /// Service was restored by rotation; the fault is not diagnosed.
    Mitigation,
    /// The rotation is claimed to have fixed the fault.
    Fix,
}

/// A rotation report: how often the pool was rotated to restore service, how
/// often the fault recurred, and how the rotation is reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationReport {
    /// How many times the pool was rotated to restore service.
    pub rotations: u64,
    /// How many of the rotated workers (or their replacements) wedged again —
    /// the recurrence of the undiagnosed fault.
    pub recurrence: u64,
    /// How the rotation is reported to the operator.
    pub claim: MitigationClaim,
}

/// Why a rotation report is unsound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MitigationFinding {
    /// A rotation was reported as a fix. Rotation restores service; it does
    /// not diagnose or fix the fault, and claiming a fix hides it.
    ReportedAsFix,
    /// The pool was rotated more than once but the recurrence is 0: the fault
    /// recurred without being tracked, so it is invisible behind the remedy.
    RecurrenceUntracked {
        /// How many times the pool was rotated.
        rotations: u64,
    },
}

impl MitigationFinding {
    pub fn line(&self) -> String {
        match self {
            MitigationFinding::ReportedAsFix => {
                "rotation is mitigation, not a fix: claiming a cure hides the \
                 fault"
                    .to_string()
            }
            MitigationFinding::RecurrenceUntracked { rotations } => {
                format!(
                    "the pool was rotated {rotations} times but recurrence is 0: \
                     the fault is invisible behind the remedy"
                )
            }
        }
    }
}

impl RotationReport {
    /// The findings of the report. A sound report (claimed as mitigation,
    /// recurrence tracked) has none.
    pub fn findings(&self) -> Vec<MitigationFinding> {
        let mut out = Vec::new();
        if self.claim == MitigationClaim::Fix {
            out.push(MitigationFinding::ReportedAsFix);
        }
        // If the pool had to be rotated more than once, the fault recurred at
        // least once; a recurrence of 0 means it was not tracked.
        if self.rotations >= 2 && self.recurrence == 0 {
            out.push(MitigationFinding::RecurrenceUntracked {
                rotations: self.rotations,
            });
        }
        out
    }

    /// The operator line. States the truth: rotation is mitigation, not a
    /// cure, and reports the recurrence.
    pub fn line(&self) -> String {
        format!(
            "rotation is mitigation, not a cure — {} rotation(s), recurrence {}; \
             the fault is undiagnosed",
            self.rotations, self.recurrence
        )
    }
}
