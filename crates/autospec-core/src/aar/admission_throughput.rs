//! The admission throughput probe: instrument the decision before tuning
//! the threshold (issue #4412).
//!
//! The gateway admits a (re-)registering worker on a throughput probe: a
//! ~512-token prompt to the worker's completions path, a prefill rate
//! measured off it, and a floor the rate must clear. Two defects, found while
//! trying to verify the probe fires:
//!
//! 1. **A fixed probe prompt can be served from cache.** The probe sent the
//!    same prompt every time, and `llama-server` keeps a prompt cache, so the
//!    second and subsequent probes against a worker can hit it and report a
//!    prefill rate that reflects a cache lookup rather than computation. A
//!    CPU-bound worker would then clear a floor it should fail.
//! 2. **The decision is silent on every path.** After deploying the probe,
//!    two workers registered with `201` and the gateway logged zero
//!    unverified admissions — so admission apparently succeeded outright
//!    rather than taking the bypass. Minutes later, a direct 512-token
//!    request to the same worker timed out at 140s, consistent with ~6
//!    tok/s prefill (512 tokens ≈ 85s plus queueing): a probe bounded well
//!    below that should have timed out and taken the bypass. The gateway
//!    logs nothing on successful admission, so "it worked" and "it never
//!    ran" look identical from outside. That absence is why three fixes
//!    were needed instead of one.
//!
//! The invariants this module makes checkable:
//!
//! 1. **The probe prompt must be unique per probe.** A constant prompt can
//!    be served from the server's prompt cache, and a rate measured off a
//!    cache lookup is not a rate of work. A nonce prefix makes every probe
//!    fresh work: any benchmark of prefill has this property, and the
//!    shipped one lost it by using a constant ([`probe_prompt`],
//!    [`prompt_shape`]).
//! 2. **The admission throughput decision renders a line on every path** —
//!    measured or not-measured, the rate when measured, the floor, and
//!    which branch admission took. A check whose *non-firing* is
//!    indistinguishable from its *absence* cannot be debugged, only guessed
//!    at; a bypass must be visible in the log, not implied by the silence
//!    around a `201` ([`decide_admission`], [`AdmissionDecision::line`]).
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! runs the probe, measures the rate, and calls these with the observed
//! values; the line [`AdmissionDecision::line`] returns is what the gateway
//! logs on every path.

use serde::{Deserialize, Serialize};

/// The per-probe prefix every probe prompt carries.
///
/// The prompt cache keys on prompt text: two probes that send the same
/// prompt let the second one be served from the cache, and its "prefill
/// rate" is then the rate of a cache lookup. A per-probe nonce at the
/// *front* of the prompt makes the prefix diverge on the first token, so
/// every probe is fresh work.
pub fn nonce_prefix(nonce: &str) -> String {
    format!("admission-throughput-probe {nonce}")
}

/// Build the probe prompt for one probe: the nonce prefix over the fixed
/// body.
///
/// Fails closed on a blank nonce or body: a blank nonce leaves the prompt
/// constant across probes — exactly the defect this exists to remove — and
/// a blank body measures nothing at all.
pub fn probe_prompt(nonce: &str, body: &str) -> Result<String, String> {
    if nonce.trim().is_empty() {
        return Err(
            "the probe nonce must not be blank: a blank nonce leaves the prompt \
             constant across probes, which the prompt cache can serve"
                .to_string(),
        );
    }
    if body.trim().is_empty() {
        return Err(
            "the probe prompt body must not be blank: a blank prompt measures \
             nothing"
                .to_string(),
        );
    }
    Ok(format!("{} {body}", nonce_prefix(nonce)))
}

/// How the probe prompt varied across the probes that ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptShape {
    /// Every probe sent its own prompt: each measurement is of real work.
    UniquePerProbe,
    /// Two probes shared a prompt: the later one could be served from the
    /// server's prompt cache, and its rate measures a cache lookup, not
    /// computation.
    Reused,
}

/// Classify the prompts actually sent across the probes that ran.
///
/// The reuse defect needs two identical prompts, so an empty or
/// single-probe set has nothing to compare: it is [`PromptShape::UniquePerProbe`].
pub fn prompt_shape(prompts: &[&str]) -> PromptShape {
    for (i, prompt) in prompts.iter().enumerate() {
        if prompts[i + 1..].contains(prompt) {
            return PromptShape::Reused;
        }
    }
    PromptShape::UniquePerProbe
}

/// What the probe produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProbeMeasurement {
    /// The probe completed and the prefill rate was measured.
    Measured { prefill_tok_s: f64 },
    /// The probe produced no measurement — a timeout, a refused
    /// connection, an error status. The reason names which, so the log
    /// line says why there is no rate.
    NotMeasured { reason: String },
}

impl ProbeMeasurement {
    /// A completed probe.
    ///
    /// Fails closed on a rate that is not a positive finite number: that is
    /// not a measurement and must not be read as one. Zero and negative
    /// rates would fail the floor and refuse a healthy worker; a NaN
    /// compares false against every floor and would refuse the fastest
    /// worker of all.
    pub fn measured(prefill_tok_s: f64) -> Result<Self, String> {
        if !(prefill_tok_s > 0.0) {
            return Err(format!(
                "a measured prefill rate must be a positive finite number, got {prefill_tok_s}"
            ));
        }
        Ok(Self::Measured { prefill_tok_s })
    }

    /// A probe that produced no measurement.
    ///
    /// The reason is mandatory: a not-measured with no reason is a silence,
    /// and silence is what this module removes.
    pub fn not_measured(reason: &str) -> Result<Self, String> {
        if reason.trim().is_empty() {
            return Err(
                "a not-measured probe must name its reason: a silence is not a line".to_string(),
            );
        }
        Ok(Self::NotMeasured {
            reason: reason.to_string(),
        })
    }
}

/// Which branch the admission took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmissionBranch {
    /// Measured, and the rate cleared the floor: admitted on the
    /// measurement.
    Verified,
    /// Measured, and the rate failed the floor: refused.
    Refused,
    /// Not measured: the check produced no rate, and the admission
    /// proceeded without it. The bypass is a branch like any other and is
    /// named as one — a bypass that is not logged is a bypass that cannot
    /// be debugged.
    Bypassed,
}

impl AdmissionBranch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Refused => "refused",
            Self::Bypassed => "bypassed",
        }
    }
}

/// The admission throughput decision, and the line it renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdmissionDecision {
    /// The worker the probe ran against.
    pub worker: String,
    /// What the probe produced.
    pub measurement: ProbeMeasurement,
    /// The floor the measured rate must clear for a verified admission, in
    /// tokens/second.
    pub floor: f64,
    /// Which branch the admission took.
    pub branch: AdmissionBranch,
}

impl AdmissionDecision {
    /// The log line for the decision.
    ///
    /// Every branch renders one: measured or not-measured, the rate when
    /// measured, the floor, and the branch. This is the instrument the
    /// issue asks for — "it worked" and "it never ran" must look
    /// different in the gateway log, and so must a verified admission and
    /// a bypass.
    pub fn line(&self) -> String {
        let branch = match self.branch {
            AdmissionBranch::Verified => "admitted, verified against the floor",
            AdmissionBranch::Refused => "refused, below the floor",
            AdmissionBranch::Bypassed => "admitted via bypass, unverified",
        };
        match &self.measurement {
            ProbeMeasurement::Measured { prefill_tok_s } => format!(
                "admission throughput: worker {} measured {:.1} tok/s (floor {:.1} tok/s): {}",
                self.worker, prefill_tok_s, self.floor, branch
            ),
            ProbeMeasurement::NotMeasured { reason } => format!(
                "admission throughput: worker {} not measured ({}; floor {:.1} tok/s): {}",
                self.worker, reason, self.floor, branch
            ),
        }
    }
}

/// Decide the admission: compare the measurement to the floor and name the
/// branch.
///
/// A not-measured probe takes the bypass — the existing behaviour, the
/// branch that was silent — but takes it as a named branch with a rendered
/// line, not a silence. A measured rate at or above the floor is verified;
/// below it, refused.
pub fn decide_admission(
    worker: &str,
    measurement: &ProbeMeasurement,
    floor: f64,
) -> Result<AdmissionDecision, String> {
    if !(floor > 0.0) {
        return Err(format!(
            "the admission floor must be a positive finite number, got {floor}"
        ));
    }
    let branch = match measurement {
        ProbeMeasurement::Measured { prefill_tok_s } => {
            if *prefill_tok_s >= floor {
                AdmissionBranch::Verified
            } else {
                AdmissionBranch::Refused
            }
        }
        ProbeMeasurement::NotMeasured { .. } => AdmissionBranch::Bypassed,
    };
    Ok(AdmissionDecision {
        worker: worker.to_string(),
        measurement: measurement.clone(),
        floor,
        branch,
    })
}
