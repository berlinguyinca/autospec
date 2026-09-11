//! Before trusting a configuration change, send the workload through it
//! (issue #4282).
//!
//! The incident: `--ubatch-size` was raised 512 → 2048 on a worker to attack
//! prefill latency. Every signal a monitor collects said healthy — the Slurm
//! job was `RUNNING`, the worker log had **zero** error lines, `GET /health`
//! returned 200, `GET /slots` reported 4 slots, the gateway registration
//! returned 201 Created (the admission probe, which *includes* a one-token
//! completion, passed) — and yet a one-token completion from outside
//! returned http 000 after 90 s, and a 12k-token prompt timed out at 420 s,
//! twice. The worker was registered and therefore eligible to receive
//! **user traffic**; the change intended to fix a latency problem would
//! have made it worse.
//!
//! The rule: **before trusting a configuration change, send the workload
//! through it** — not a health endpoint, not a registration, not an absence
//! of errors in a log. The actual shape of request the change was made for.
//! This is the same rule as "liveness is output, never process state"
//! (#4259): a check that answers from a different code path than the
//! failure is not evidence about the failure.
//!
//! The invariants this module makes checkable:
//!
//! 1. **A health endpoint answers from a different code path than the
//!    workload.** [`ServingEvidence::class`] sorts every monitor-collectable
//!    check: [`EvidenceClass::ControlPlane`] checks (job state, log scan,
//!    `/health`, `/slots`, registration) answer from the listener or the
//!    bookkeeping, never from inference. [`trust_config_change`] trusts a
//!    configuration change only on a completed
//!    [`EvidenceClass::ExternalWorkload`] request of the intended shape;
//!    every other check, however many of them pass, is rendered as
//!    insufficient, not as evidence.
//! 2. **A passing probe is only as good as its resemblance to real
//!    traffic.** [`probe_soundness`] adjudicates the admission probe
//!    against an identical external request: a probe that passes while the
//!    external request fails is [`ProbeSoundness::Unsound`] — served from a
//!    warm path or bounded differently, either way not measuring "this
//!    worker can serve". A probe that has never been checked against the
//!    outside is [`ProbeSoundness::Unverified`], fail-closed: it is never
//!    read as sound. [`registration_standing`] turns the soundness into
//!    the registration's standing, and [`RegistrationStanding::UnsoundLive`]
//!    names the worst state: the registration exists, so the worker is
//!    eligible for user traffic, so a probe that can pass while the thing
//!    it gates cannot serve is worse than no probe — because it produces
//!    a registration.
//! 3. **Stage a config change on one instance and send it real work before
//!    it can receive any.** [`traffic_gate`] separates "up" from
//!    "eligible for user traffic": a worker that registers immediately on
//!    startup while its configuration change is unverified has no window
//!    between the two ([`TrafficGate::RegisteredUnverified`]). Fast
//!    registration and unvalidated config are a bad combination; the gate
//!    holds until the intended workload has completed.
//! 4. **Measure the baseline on an idle system before concluding that a
//!    parameter is the bottleneck.** The 794 tok/s prefill figure that made
//!    `--ubatch-size` look like the lever was measured with three of four
//!    slots busy; an idle worker at the *unchanged* setting measures 2434
//!    tok/s. [`baseline_attribution`] decides what a
//!    [`Baseline`] number describes: a measurement under contention
//!    describes the load, not the parameter, and
//!    [`evaluate_bottleneck_claim`] refuses to support a parameter claim
//!    from one.
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! runs the checks and sends the requests; this module decides what the
//! observed signals mean for trusting a configuration change.

use serde::{Deserialize, Serialize};

// --- Invariant 1: the check's code path decides what it is evidence of ---

/// The code path a check answers from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceClass {
    /// Answers from the listener or the bookkeeping — job state, log scan,
    /// `/health`, `/slots`, gateway registration. It says a component is
    /// up; on a change that affects inference it says nothing about whether
    /// inference completes.
    ControlPlane,
    /// A workload-shaped request from the privileged inside vantage — the
    /// gateway's admission probe. It exercises inference, but from the
    /// position that real traffic does not come from.
    PrivilegedWorkload,
    /// A workload-shaped request from outside, from the position user
    /// traffic comes from. The only class that is evidence a worker can
    /// serve.
    ExternalWorkload,
}

/// One check a monitor can collect about a (re)started worker, or one
/// request an operator has sent it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServingEvidence {
    /// Slurm job state: `RUNNING` or not.
    JobState { running: bool },
    /// A scan of the worker log: how many error lines it contains.
    LogScan { error_lines: u64 },
    /// `GET /health`: the HTTP status it returned.
    HealthEndpoint { status: u16 },
    /// `GET /slots`: the free slots it reported.
    SlotsReported { free_slots: u64 },
    /// Gateway registration: the HTTP status it returned (201 Created).
    Registration { status: u16 },
    /// The one-token completion inside the gateway's admission probe.
    AdmissionProbe { completed: bool },
    /// A real request sent from outside, in a named shape (e.g.
    /// `"12k-token prompt"`). The shape is the resemblance to the traffic
    /// the change was made for.
    WorkloadRequest { shape: String, completed: bool },
}

impl ServingEvidence {
    /// The code path this check answers from (invariant 1).
    pub fn class(&self) -> EvidenceClass {
        match self {
            Self::JobState { .. }
            | Self::LogScan { .. }
            | Self::HealthEndpoint { .. }
            | Self::SlotsReported { .. }
            | Self::Registration { .. } => EvidenceClass::ControlPlane,
            Self::AdmissionProbe { .. } => EvidenceClass::PrivilegedWorkload,
            Self::WorkloadRequest { .. } => EvidenceClass::ExternalWorkload,
        }
    }

    /// The check and its observed outcome, for the report table.
    pub fn line(&self) -> String {
        match self {
            Self::JobState { running } => format!(
                "slurm job state: {}",
                word(*running, "RUNNING", "not running")
            ),
            Self::LogScan { error_lines } => format!("worker log: {error_lines} error line(s)"),
            Self::HealthEndpoint { status } => format!("GET /health: {status}"),
            Self::SlotsReported { free_slots } => format!("GET /slots: {free_slots} slots"),
            Self::Registration { status } => format!("gateway registration: {status}"),
            Self::AdmissionProbe { completed } => format!(
                "admission probe completion: {}",
                word(*completed, "passed", "failed")
            ),
            Self::WorkloadRequest { shape, completed } => format!(
                "external {shape} request: {}",
                word(*completed, "completed", "did not complete")
            ),
        }
    }
}

/// The word a boolean renders as in a report line.
fn word<'a>(value: bool, when_true: &'a str, when_false: &'a str) -> &'a str {
    if value {
        when_true
    } else {
        when_false
    }
}

/// Whether a configuration change has earned trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigTrust {
    /// A completed external workload request of the intended shape.
    Trusted,
    /// No completed external request of the intended shape. The insufficient
    /// signals are rendered so the report shows the whole "every monitor
    /// signal said healthy" table next to the verdict that refused them.
    Untrusted { insufficient: Vec<String> },
}

/// Trust a configuration change only on the workload it was made for
/// (invariant 1). `intended_shape` is the shape of request the change was
/// made for; a completed request of any other shape is resemblance-failure,
/// not evidence.
pub fn trust_config_change(intended_shape: &str, evidences: &[ServingEvidence]) -> ConfigTrust {
    let trusted = evidences.iter().any(|e| match e {
        ServingEvidence::WorkloadRequest { shape, completed } => {
            *completed && shape == intended_shape
        }
        _ => false,
    });
    if trusted {
        ConfigTrust::Trusted
    } else {
        ConfigTrust::Untrusted {
            insufficient: evidences.iter().map(ServingEvidence::line).collect(),
        }
    }
}

/// The full report: every collected signal, then the verdict (invariant 1).
pub fn serving_report(intended_shape: &str, evidences: &[ServingEvidence]) -> String {
    let mut lines: Vec<String> = evidences.iter().map(ServingEvidence::line).collect();
    match trust_config_change(intended_shape, evidences) {
        ConfigTrust::Trusted => lines.push(format!(
            "verdict: trusted — an external {intended_shape} request completed"
        )),
        ConfigTrust::Untrusted { .. } => lines.push(format!(
            "verdict: untrusted — no external {intended_shape} request completed; \
             every other signal answers from a different code path"
        )),
    }
    lines.join("\n")
}

// --- Invariant 2: a probe is only as good as its resemblance to traffic --

/// What the identical external request did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalOutcome {
    /// The external request completed.
    Completed,
    /// The external request failed (timeout, http 000, reset).
    Failed,
}

/// What the admission probe proves about the worker it gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeSoundness {
    /// The probe passed and the identical external request completed: the
    /// probe resembles the traffic it gates.
    Sound,
    /// The probe passed while the identical external request failed: the
    /// probe was either served from a warm path or bounded differently.
    /// Whatever it measured, it was not "this worker can serve".
    Unsound,
    /// The probe itself failed. Fail-closed: a probe that failed proves
    /// nothing and gates nothing.
    Rejected,
    /// The probe has never been checked against the outside. Fail-closed:
    /// unadjudicated is never read as sound.
    Unverified,
}

/// Adjudicate the admission probe against an identical external request
/// (invariant 2). `external` is `None` when the request has not been sent
/// yet.
pub fn probe_soundness(probe_completed: bool, external: Option<ExternalOutcome>) -> ProbeSoundness {
    match (probe_completed, external) {
        (true, Some(ExternalOutcome::Completed)) => ProbeSoundness::Sound,
        (true, Some(ExternalOutcome::Failed)) => ProbeSoundness::Unsound,
        (true, None) => ProbeSoundness::Unverified,
        (false, _) => ProbeSoundness::Rejected,
    }
}

/// The standing of a registration the probe produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationStanding {
    /// Produced by a sound probe: the registration is evidence the worker
    /// can serve.
    Valid,
    /// Produced by a probe that passes while the identical external request
    /// failed. The registration exists, so the worker is eligible for user
    /// traffic while it cannot serve — the worst state, because a probe
    /// that can pass while the thing it gates cannot serve is worse than
    /// no probe: it produces a registration.
    UnsoundLive,
    /// The probe failed: the registration should not exist.
    Void,
    /// The probe passed but has never been checked from outside: hold the
    /// registration as unproven until an external request adjudicates it.
    Held,
}

/// The standing of the registration a probe produced (invariant 2).
pub fn registration_standing(soundness: ProbeSoundness) -> RegistrationStanding {
    match soundness {
        ProbeSoundness::Sound => RegistrationStanding::Valid,
        ProbeSoundness::Unsound => RegistrationStanding::UnsoundLive,
        ProbeSoundness::Rejected => RegistrationStanding::Void,
        ProbeSoundness::Unverified => RegistrationStanding::Held,
    }
}

// --- Invariant 3: no window between "up" and "eligible for traffic" ------

/// Whether "up" and "eligible for user traffic" are separated by a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrafficGate {
    /// The worker cannot receive user traffic until the intended workload
    /// has completed on it: "up" and "eligible" are separate states.
    VerifiedBeforeTraffic,
    /// The worker registers immediately on startup and its configuration
    /// change is unverified: there is no window between "up" and "eligible
    /// for user traffic". Fast registration and unvalidated config are a
    /// bad combination.
    RegisteredUnverified,
}

/// Separate "up" from "eligible for user traffic" (invariant 3). A worker
/// that does not register immediately still has a staging window even
/// unverified; one that registers immediately does not.
pub fn traffic_gate(registers_immediately: bool, workload_verified: bool) -> TrafficGate {
    if registers_immediately && !workload_verified {
        TrafficGate::RegisteredUnverified
    } else {
        TrafficGate::VerifiedBeforeTraffic
    }
}

// --- Invariant 4: a number taken under load describes the load -----------

/// A rate measured under a known load state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    /// The measured rate, in the unit the parameter was argued about
    /// (e.g. prefill tokens/second).
    pub rate: f64,
    /// Slots busy at the time of the measurement.
    pub busy_slots: u32,
    /// Total slots on the system.
    pub total_slots: u32,
}

/// What a baseline number describes (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaselineAttribution {
    /// Measured with nothing else on the system: the number describes the
    /// parameter.
    Parameter,
    /// Measured with other slots busy: the number describes the load, not
    /// the parameter.
    Load {
        /// Slots that were busy at the time of the measurement.
        busy_slots: u32,
        /// Total slots on the system.
        total_slots: u32,
    },
}

/// Decide what a [`Baseline`] number describes (invariant 4).
pub fn baseline_attribution(baseline: &Baseline) -> BaselineAttribution {
    if baseline.busy_slots == 0 {
        BaselineAttribution::Parameter
    } else {
        BaselineAttribution::Load {
            busy_slots: baseline.busy_slots,
            total_slots: baseline.total_slots,
        }
    }
}

/// Whether a baseline supports the claim that a parameter is the
/// bottleneck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottleneckClaim {
    /// The baseline is idle: the number may be used to argue the parameter
    /// is (or is not) the bottleneck.
    Supported,
    /// The baseline was taken under contention: the number describes the
    /// load, not the parameter, and is not evidence about it.
    Unsupported { reason: String },
}

/// Refuse a parameter claim built on a number taken under load
/// (invariant 4).
pub fn evaluate_bottleneck_claim(parameter: &str, baseline: &Baseline) -> BottleneckClaim {
    match baseline_attribution(baseline) {
        BaselineAttribution::Parameter => BottleneckClaim::Supported,
        BaselineAttribution::Load {
            busy_slots,
            total_slots,
        } => BottleneckClaim::Unsupported {
            reason: format!(
                "{parameter} was argued from {rate} measured with {busy_slots}/{total_slots} \
                 slots busy; a number taken under load describes the load, not the parameter",
                rate = baseline.rate
            ),
        },
    }
}
