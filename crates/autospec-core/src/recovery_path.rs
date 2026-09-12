//! Recovery paths and readiness waits (issue #4399).
//!
//! The incident: a worker probed a model server's `/props` endpoint
//! **once**, 10 seconds after start, and treated the failure as permanent:
//!
//! ```sh
//! CTX_PER_SLOT=$(curl -s --max-time 10 "$ENDPOINT/props" ...)
//! ```
//!
//! `/props` answers `503 {"error":"Loading model"}` until the weights are
//! resident, and `qwen3.8-flash-next` is a 105 GiB mmap from a network
//! filesystem that had not finished loading after **1h44m**. The probe
//! always failed, the endpoint file was never written, and four workers
//! sat holding **8 GPUs** — fully loaded and completely invisible to the
//! gateway.
//!
//! The code carried this comment:
//!
//! ```text
//! REGISTER FAILED http=$code -- regsweep will retry within 5 minutes
//! ```
//!
//! That reassurance is **false on this path**: `regsweep` reconciles *from*
//! the endpoint files — and the endpoint file is only written once the
//! probe succeeds. The recovery mechanism consumes the artifact that the
//! failing step was supposed to produce.
//!
//! So the failure is not "a retry was missing". It is that **the retry loop
//! and the failing step share a dependency, so the retry can never fire for
//! the case that actually needs it.** The reconciler covers the easy failure
//! (gateway briefly down) and is structurally incapable of covering the hard
//! one (model still loading).
//!
//! The primitives here make the invariants checkable:
//!
//! 1. **Waiting for a slow dependency is a wait, not a probe**
//!    ([`readiness_verdict`]). Any check against a resource with a startup
//!    phase must poll to a deadline, and the deadline must be scaled to the
//!    resource — 10 seconds for something that takes an hour is not a
//!    conservative choice, it is a guaranteed failure.
//! 2. **A recovery path must not depend on an artifact produced by the step
//!    it recovers** ([`recovery_verdict`]). Write down, for every
//!    reconciler, what it reads and which failures can prevent that input
//!    from existing. If the answer includes the failure being recovered,
//!    the recovery is decorative.
//! 3. **A comment asserting that something else will retry is a claim that
//!    must be verified** ([`claim_verdict`]). It shaped the behaviour of
//!    everyone reading the code — it is the reason nobody looked — and it
//!    was wrong.
//! 4. **A spec that says "register on startup" must state the readiness
//!    policy** ([`RegistrationSpec`]): what happens when the thing being
//!    registered is not ready yet, how long readiness may take (with the
//!    measured worst case, not a guess), and who retries — naming the input
//!    that retry depends on.

use std::time::Duration;

/// A step with a startup phase that writes its artifacts only if its own
/// readiness check succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Stable identifier (`"worker"`).
    pub id: String,
    /// Artifacts this step produces. Each exists only after the step's own
    /// readiness check succeeded — the endpoint file exists only if the
    /// probe did.
    pub produces: Vec<String>,
    /// Measured worst case of the startup phase this step depends on
    /// (measured, not guessed): 1h44m for a 105 GiB mmap from a network
    /// filesystem.
    pub readiness_worst_case: Duration,
}

/// A check against a resource that has a startup phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessProbe {
    /// The resource being checked (the model server's `/props`).
    pub resource: String,
    /// How many times the check ran. One is a probe in the bad sense: a
    /// single measurement of a resource that is still starting up.
    pub checks: u32,
    /// The total wait budgeted before the failure is treated as permanent.
    pub deadline: Duration,
}

/// What a readiness check against a resource with a startup phase is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessVerdict {
    /// The check is a wait: it polls to a deadline, and the deadline is
    /// scaled past the resource's measured worst case.
    Waits {
        resource: String,
        checks: u32,
        deadline: Duration,
        worst_case: Duration,
    },
    /// A single check of a resource with a startup phase. Waiting for a
    /// slow dependency is a wait, not a probe: one measurement cannot
    /// separate "still loading" from "failed".
    OneShot {
        resource: String,
        worst_case: Duration,
    },
    /// A poll whose deadline falls inside the measured worst case. The
    /// failure is guaranteed, not likely: the deadline expires while the
    /// resource is still in its startup phase.
    GuaranteedFailure {
        resource: String,
        checks: u32,
        deadline: Duration,
        worst_case: Duration,
    },
}

impl ReadinessVerdict {
    /// One-line rendering for a review, a closeout, or a spec.
    pub fn line(&self) -> String {
        match self {
            Self::Waits {
                resource,
                checks,
                deadline,
                worst_case,
            } => format!(
                "OK: '{resource}' is polled {checks}x to a {deadline:?} deadline, scaled past its {worst_case:?} measured worst case — the check is a wait"
            ),
            Self::OneShot { resource, worst_case } => format!(
                "FAIL: '{resource}' has a startup phase measured at {worst_case:?} but is checked once — waiting for a slow dependency is a wait, not a probe; poll to a deadline scaled to the resource"
            ),
            Self::GuaranteedFailure {
                resource,
                checks,
                deadline,
                worst_case,
            } => format!(
                "FAIL: '{resource}' is polled {checks}x but the {deadline:?} deadline falls inside its {worst_case:?} measured worst case — a guaranteed failure, not a conservative choice"
            ),
        }
    }
}

/// Invariant 1: waiting for a slow dependency is a wait, not a probe.
///
/// A check against a resource with a startup phase must poll to a deadline
/// scaled past the measured worst case. One check is never a verdict about
/// a resource that is still loading; a deadline inside the worst case is a
/// retry that was designed to lose.
pub fn readiness_verdict(probe: &ReadinessProbe, worst_case: Duration) -> ReadinessVerdict {
    if probe.checks <= 1 && worst_case != Duration::ZERO {
        return ReadinessVerdict::OneShot {
            resource: probe.resource.clone(),
            worst_case,
        };
    }
    if probe.deadline <= worst_case {
        return ReadinessVerdict::GuaranteedFailure {
            resource: probe.resource.clone(),
            checks: probe.checks,
            deadline: probe.deadline,
            worst_case,
        };
    }
    ReadinessVerdict::Waits {
        resource: probe.resource.clone(),
        checks: probe.checks,
        deadline: probe.deadline,
        worst_case,
    }
}

/// A recovery path: a reconciler, sweep, or retry loop meant to rescue a
/// step that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// The recovery's name (`"regsweep"`).
    pub name: String,
    /// The step the recovery is meant to rescue.
    pub recovers: String,
    /// The inputs the recovery reads (endpoint files, job records, logs).
    pub reads: Vec<String>,
}

/// Whether a recovery path can fire for the case that actually needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryVerdict {
    /// The recovery reads an artifact produced by the very step it
    /// recovers. That artifact exists only if the step succeeded, so the
    /// retry can never fire for the case that actually needs it: the
    /// recovery is decorative.
    Decorative {
        recovery: String,
        step: String,
        /// The shared input — the artifact the failing step was supposed
        /// to produce.
        artifact: String,
    },
    /// Every input the recovery reads exists even when the step it
    /// recovers fails: no failure the recovery covers can prevent its own
    /// input from existing.
    Covers {
        recovery: String,
        step: String,
        /// Inputs whose independence from the failure was checked.
        independent_inputs: usize,
    },
    /// The recovery names a step that is not in the model. Fail-closed:
    /// what it reads, and whether those inputs survive the failure, cannot
    /// be checked — that is not `Covers`.
    Unverifiable { recovery: String, step: String },
}

impl RecoveryVerdict {
    /// One-line rendering for a review, a closeout, or a spec.
    pub fn line(&self) -> String {
        match self {
            Self::Decorative {
                recovery,
                step,
                artifact,
            } => format!(
                "FAIL: '{recovery}' recovers '{step}' but reads '{artifact}' — an artifact '{step}' produces, which exists only if '{step}' succeeded; the retry can never fire for the case that actually needs it, so the recovery is decorative"
            ),
            Self::Covers {
                recovery,
                step,
                independent_inputs,
            } => format!(
                "OK: '{recovery}' recovers '{step}' from {independent_inputs} input(s) that exist even when '{step}' fails — the retry can fire for the case that needs it"
            ),
            Self::Unverifiable { recovery, step } => format!(
                "FAIL: '{recovery}' recovers '{step}', which is not in the step model — what it reads, and whether those inputs survive the failure, cannot be checked; fail-closed, this is not coverage"
            ),
        }
    }
}

/// Invariant 2: a recovery path must not depend on an artifact produced by
/// the step it recovers.
///
/// For every reconciler, this answers what it reads and whether the
/// failure being recovered can prevent that input from existing. If the
/// recovered step produces one of the recovery's inputs, the retry loop and
/// the failing step share a dependency — the reconciler covers the easy
/// failure and is structurally incapable of covering the hard one.
pub fn recovery_verdict(recovery: &Recovery, steps: &[Step]) -> RecoveryVerdict {
    let Some(step) = steps.iter().find(|s| s.id == recovery.recovers) else {
        return RecoveryVerdict::Unverifiable {
            recovery: recovery.name.clone(),
            step: recovery.recovers.clone(),
        };
    };
    if let Some(artifact) = recovery
        .reads
        .iter()
        .find(|input| step.produces.contains(input))
    {
        return RecoveryVerdict::Decorative {
            recovery: recovery.name.clone(),
            step: step.id.clone(),
            artifact: artifact.clone(),
        };
    }
    RecoveryVerdict::Covers {
        recovery: recovery.name.clone(),
        step: step.id.clone(),
        independent_inputs: recovery.reads.len(),
    }
}

/// A comment in the code asserting that something else will retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryClaim {
    /// The comment's text (`"REGISTER FAILED http=$code -- regsweep will
    /// retry within 5 minutes"`).
    pub text: String,
    /// The recovery the claim names (`"regsweep"`).
    pub names: String,
}

/// Whether a retry claim holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimVerdict {
    /// The named recovery exists and its inputs survive the failure: the
    /// retry fires for the case that needs it.
    Verified { recovery: String, step: String },
    /// The named recovery is decorative — it reads an artifact the failing
    /// step produces. The claim shaped the behaviour of everyone reading
    /// the code; it was false on the path that needed it.
    False {
        recovery: String,
        step: String,
        artifact: String,
    },
    /// No recovery by that name exists: the claim points at nothing.
    NamesNothing { recovery: String },
    /// The named recovery exists, but what it reads cannot be checked
    /// against the step model. Fail-closed: unverified is not verified.
    Unverifiable { recovery: String, step: String },
}

impl ClaimVerdict {
    /// One-line rendering for a review, a closeout, or a spec.
    pub fn line(&self) -> String {
        match self {
            Self::Verified { recovery, step } => format!(
                "OK: the claim names '{recovery}', which actually recovers '{step}' from inputs that survive the failure"
            ),
            Self::False {
                recovery,
                step,
                artifact,
            } => format!(
                "FAIL: the claim names '{recovery}', but it recovers '{step}' from '{artifact}' — an artifact the failing step produces; the retry it promises can never fire for the case that needs it"
            ),
            Self::NamesNothing { recovery } => format!(
                "FAIL: the claim names '{recovery}', which does not exist — a retry that cannot be pointed at cannot be relied on"
            ),
            Self::Unverifiable { recovery, step } => format!(
                "FAIL: the claim names '{recovery}', but what it reads to recover '{step}' is not in the step model — the claim is unverified, not verified"
            ),
        }
    }
}

/// Invariant 3: a comment asserting that something else will retry is a
/// claim that must be verified.
///
/// The verification is the check the comment makes possible: does the
/// named recovery exist, and do its inputs survive the failure the claim
/// reassures about? A claim is trusted because it is reassuring — that is
/// the reason nobody looked.
pub fn claim_verdict(claim: &RetryClaim, recoveries: &[Recovery], steps: &[Step]) -> ClaimVerdict {
    let Some(recovery) = recoveries.iter().find(|r| r.name == claim.names) else {
        return ClaimVerdict::NamesNothing {
            recovery: claim.names.clone(),
        };
    };
    match recovery_verdict(recovery, steps) {
        RecoveryVerdict::Decorative { step, artifact, .. } => ClaimVerdict::False {
            recovery: recovery.name.clone(),
            step,
            artifact,
        },
        RecoveryVerdict::Unverifiable { .. } => ClaimVerdict::Unverifiable {
            recovery: recovery.name.clone(),
            step: recovery.recovers.clone(),
        },
        RecoveryVerdict::Covers { .. } => ClaimVerdict::Verified {
            recovery: recovery.name.clone(),
            step: recovery.recovers.clone(),
        },
    }
}

/// A readiness bound backed by a measurement, not a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredWorstCase {
    /// The measured worst case of the startup phase.
    pub value: Duration,
    /// Where it was measured: host, resource, date, command. A bound with
    /// no source is a guess wearing a number.
    pub source: String,
}

impl MeasuredWorstCase {
    /// Construct a bound. The source is mandatory: the measured worst
    /// case is what the spec owes — a guess is the choice that became 10
    /// seconds.
    pub fn new(value: Duration, source: &str) -> Option<Self> {
        if source.trim().is_empty() {
            return None;
        }
        Some(Self {
            value,
            source: source.to_string(),
        })
    }
}

/// Who retries, naming the input that retry depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryOwner {
    /// The recovery that retries (`"regsweep"`).
    pub owner: String,
    /// The input the retry depends on. Naming it is what makes the claim
    /// checkable — and it is what made the incident visible: the endpoint
    /// file, which exists only after the probe succeeded.
    pub input: String,
}

/// The "register on startup" part of a spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistrationSpec {
    /// What happens when the thing being registered is not ready yet.
    pub not_ready: Option<String>,
    /// How long readiness may take — the measured worst case, not a
    /// guess.
    pub readiness: Option<MeasuredWorstCase>,
    /// Who retries, naming the input that retry depends on.
    pub retry: Option<RetryOwner>,
}

impl RegistrationSpec {
    /// Invariant 4: a spec that says "register on startup" must state
    /// what happens when the thing being registered is not ready yet, how
    /// long readiness may take (with the measured worst case, not a
    /// guess), and who retries — naming the input that retry depends on.
    /// One finding per missing part.
    pub fn findings(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self
            .not_ready
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(_) => {}
            None => out.push(
                "the spec does not state what happens when the thing being registered is not ready yet"
                    .into(),
            ),
        }
        match &self.readiness {
            None => out.push("the spec does not state how long readiness may take".into()),
            Some(m) if m.source.trim().is_empty() => out.push(
                "the readiness bound is a guess, not a measured worst case — it names no source"
                    .into(),
            ),
            Some(_) => {}
        }
        match &self.retry {
            None => out.push("the spec does not say who retries".into()),
            Some(r) if r.owner.trim().is_empty() || r.input.trim().is_empty() => {
                out.push("the retry owner does not name the input that retry depends on".into())
            }
            Some(_) => {}
        }
        out
    }

    /// Whether the spec states the readiness policy in full.
    pub fn is_complete(&self) -> bool {
        self.findings().is_empty()
    }
}
