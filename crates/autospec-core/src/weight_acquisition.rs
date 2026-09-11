//! Verify runtime support before acquiring weights (issue #4355).
//!
//! The incident: asked to integrate a newly released model (DeepSeek V4.1), the
//! obvious sequence — find the weights, download, deploy — costs 508 GB and
//! hours before discovering the runtime cannot read the file. The runtime
//! (llama.cpp) had exactly one reference to the `deepseek_v41` architecture — an
//! *open* conversion PR — so it could not load it yet. The fleet already carries
//! that scar: GLM sits undeployed behind `unknown model architecture:
//! 'glm5next'` (#299) — weights acquired, staged, unusable pending upstream
//! support.
//!
//! The ordering that avoids it puts the expensive step last and lets each cheap
//! step stop the process: (1) does the runtime support the architecture — check
//! the upstream project, not the model card; (2) does a policy-compliant quant
//! exist from a trusted publisher — a quant from an unrecognised account is a
//! supply-chain choice, not a download detail; (3) does placement have room for
//! weights + full KV — only now is size a useful question; (4) then download.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **Verify runtime support before acquiring weights.** [`decide`] walks the
//!    gates in [`GATE_ORDER`] and never reaches the expensive download gate
//!    while a cheaper gate has not passed.
//! 2. **Publisher trust is part of the model decision.** A quant is admissible
//!    only when its level meets the policy floor *and* its publisher is trusted
//!    ([`Quant::rejections`]).
//! 3. **"Acquired but unservable" is a distinct state.**
//!    [`DeploymentState::AcquiredUnservable`] names the GLM scar.
//! 4. **A negative from a tool that cannot produce a positive is not a
//!    finding.** A [`Probe`] is validated only after it has produced a positive
//!    on a known-good input; before that its negatives mean "the method does
//!    not work here", and [`decide`] renders them as
//!    [`AcquisitionDecision::SupportUnverified`] rather than a stop.
//!
//! Everything here is pure in-memory state — no I/O, no clock, no subprocess.

use std::collections::BTreeSet;

/// The four gates in the order they must run (invariant 1). The expensive gate
/// ([`AcquisitionGate::Download`]) is last; every gate before it is cheap and
/// can stop the process without reaching the download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AcquisitionGate {
    /// Gate 1: does the runtime load this architecture? (invariant 1)
    RuntimeSupport,
    /// Gate 2: does a policy-compliant quant from a trusted publisher exist?
    /// (invariant 2)
    QuantPolicy,
    /// Gate 3: does placement have room for weights + full KV?
    PlacementRoom,
    /// The download. Expensive — hundreds of gigabytes and hours. Always last.
    Download,
}

impl AcquisitionGate {
    /// Only the download is expensive; every gate before it is a cheap check
    /// that can stop the process (invariant 1).
    pub fn is_expensive(self) -> bool {
        matches!(self, AcquisitionGate::Download)
    }
}

/// The canonical gate order: cheap checks first, the download last.
pub const GATE_ORDER: [AcquisitionGate; 4] = [
    AcquisitionGate::RuntimeSupport,
    AcquisitionGate::QuantPolicy,
    AcquisitionGate::PlacementRoom,
    AcquisitionGate::Download,
];

/// The runtime's answer to "can it load this architecture?" (gate 1) and the
/// source that produced it; its validity (invariant 4) gates the negative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSupport {
    /// Whether the runtime loads the architecture, as reported by the source.
    pub supported: bool,
    /// Was the source validated against a positive control (invariant 4)?
    pub source_valid: bool,
    /// The source that produced the answer (e.g. `upstream tracker`,
    /// `binary inspection`).
    pub source: String,
    /// For `supported == false`: the upstream reference (PR/issue) that would
    /// add support, if one was found.
    pub reference: Option<String>,
}

/// The quant policy: the floor on quant level (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantPolicy {
    pub min_level: u32,
}

impl Default for QuantPolicy {
    /// The fleet's floor: Q6 (issue #4355).
    fn default() -> Self {
        QuantPolicy { min_level: 6 }
    }
}

/// The publishers the fleet trusts for serving weights (invariant 2): a fleet
/// decision owned by whoever owns the supply-chain risk, not a file property.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedPublishers {
    names: BTreeSet<String>,
}

impl TrustedPublishers {
    pub fn new(names: Vec<String>) -> Self {
        Self {
            names: names.into_iter().collect(),
        }
    }

    pub fn trusts(&self, publisher: &str) -> bool {
        self.names.contains(publisher)
    }
}

/// A candidate quant: its name, level, publisher, and weight size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quant {
    pub name: String,
    /// The quantization level (2 = Q2, 4 = Q4, 6 = Q6, 8 = Q8).
    pub level: u32,
    /// The publisher account the quant was downloaded from.
    pub publisher: String,
    pub weights_mib: u64,
}

impl Quant {
    /// Every reason this quant is not admissible (invariant 2): empty when its
    /// level meets the policy floor *and* its publisher is trusted. A quant
    /// can fail both — the level and the publisher are one decision.
    pub fn rejections(
        &self,
        policy: &QuantPolicy,
        trusted: &TrustedPublishers,
    ) -> Vec<QuantRejection> {
        let mut reasons = Vec::new();
        if self.level < policy.min_level {
            reasons.push(QuantRejection::BelowFloor {
                level: self.level,
                floor: policy.min_level,
            });
        }
        if !trusted.trusts(&self.publisher) {
            reasons.push(QuantRejection::UntrustedPublisher {
                publisher: self.publisher.clone(),
            });
        }
        reasons
    }

    /// A quant is admissible when it has no rejections.
    pub fn admissible(&self, policy: &QuantPolicy, trusted: &TrustedPublishers) -> bool {
        self.rejections(policy, trusted).is_empty()
    }
}

/// Why a quant is not admissible (invariant 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantRejection {
    /// The quant level is below the policy floor.
    BelowFloor { level: u32, floor: u32 },
    /// The quant's publisher is not one the fleet trusts.
    UntrustedPublisher { publisher: String },
}

impl QuantRejection {
    pub fn as_str(&self) -> String {
        match self {
            QuantRejection::BelowFloor { level, floor } => {
                format!("below the Q{floor} floor (Q{level})")
            }
            QuantRejection::UntrustedPublisher { publisher } => {
                format!("untrusted publisher `{publisher}`")
            }
        }
    }
}

/// A rejected quant in the [`AcquisitionDecision::NoTrustedQuant`] record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedQuant {
    pub name: String,
    pub reasons: Vec<QuantRejection>,
}

impl RejectedQuant {
    pub fn line(&self) -> String {
        let reasons = self
            .reasons
            .iter()
            .map(QuantRejection::as_str)
            .collect::<Vec<_>>()
            .join("; ");
        format!("{} ({reasons})", self.name)
    }
}

/// Gate 3 (placement): room for the chosen quant's weights plus the full KV cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The room the placement offers.
    pub available_mib: u64,
    /// The full KV cache the deployment requires.
    pub kv_mib: u64,
}

impl Placement {
    /// Room for a quant of `weights_mib` plus the full KV cache?
    pub fn fits(&self, weights_mib: u64) -> bool {
        self.kv_mib.saturating_add(weights_mib) <= self.available_mib
    }
}

/// Walk the four gates in [`GATE_ORDER`] and return the first that stops the
/// process (invariant 1). The expensive download gate is reached only when the
/// three cheap gates have all passed; `decide` never reaches it otherwise.
pub fn decide(
    support: &RuntimeSupport,
    quants: &[Quant],
    policy: &QuantPolicy,
    trusted: &TrustedPublishers,
    placement: &Placement,
) -> AcquisitionDecision {
    // Gate 1 (invariant 1 + 4): the runtime must load the architecture first,
    // and only a *validated* "no" is a finding.
    if !support.supported {
        return if support.source_valid {
            AcquisitionDecision::RuntimeUnsupported {
                reference: support.reference.clone(),
            }
        } else {
            AcquisitionDecision::SupportUnverified {
                source: support.source.clone(),
            }
        };
    }
    // Gate 2 (invariant 2): a policy-compliant quant from a trusted publisher.
    let rejected: Vec<RejectedQuant> = quants
        .iter()
        .map(|q| RejectedQuant {
            name: q.name.clone(),
            reasons: q.rejections(policy, trusted),
        })
        .collect();
    let Some(chosen) = quants
        .iter()
        .filter(|q| q.rejections(policy, trusted).is_empty())
        .min_by_key(|q| q.weights_mib)
    else {
        return AcquisitionDecision::NoTrustedQuant { rejected };
    };
    // Gate 3: placement room for the chosen quant's weights + full KV.
    let needed = placement.kv_mib.saturating_add(chosen.weights_mib);
    if !placement.fits(chosen.weights_mib) {
        return AcquisitionDecision::NoPlacementRoom {
            quant: chosen.name.clone(),
            needed_mib: needed,
            available_mib: placement.available_mib,
        };
    }
    // Gate 4: the download is authorised.
    AcquisitionDecision::Acquire {
        quant: chosen.name.clone(),
        weights_mib: chosen.weights_mib,
    }
}

/// The outcome of [`decide`]: which gate stopped the process, or the download
/// plan when all the cheap gates passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquisitionDecision {
    /// Gate 1: the runtime does not load this architecture yet, per a
    /// *validated* source. `reference` names the open upstream PR that would
    /// add support, when one was found. The download is never reached.
    RuntimeUnsupported { reference: Option<String> },
    /// Gate 1: the source reported no support but was not validated (invariant
    /// 4), so the negative is not a finding. Get a validated source; the
    /// download is not reached.
    SupportUnverified { source: String },
    /// Gate 2: no policy-compliant quant from a trusted publisher exists.
    /// `rejected` records every candidate and why it was refused.
    NoTrustedQuant { rejected: Vec<RejectedQuant> },
    /// Gate 3: placement has no room for the chosen quant's weights + full KV.
    NoPlacementRoom {
        quant: String,
        needed_mib: u64,
        available_mib: u64,
    },
    /// All three cheap gates passed; the download is authorised.
    Acquire { quant: String, weights_mib: u64 },
}

impl AcquisitionDecision {
    /// The gate that stopped the process, or [`AcquisitionGate::Download`] if all passed.
    pub fn gate(&self) -> AcquisitionGate {
        match self {
            AcquisitionDecision::RuntimeUnsupported { .. }
            | AcquisitionDecision::SupportUnverified { .. } => AcquisitionGate::RuntimeSupport,
            AcquisitionDecision::NoTrustedQuant { .. } => AcquisitionGate::QuantPolicy,
            AcquisitionDecision::NoPlacementRoom { .. } => AcquisitionGate::PlacementRoom,
            AcquisitionDecision::Acquire { .. } => AcquisitionGate::Download,
        }
    }

    /// Whether the download is authorised (only [`AcquisitionDecision::Acquire`]).
    pub fn acquires(&self) -> bool {
        matches!(self, AcquisitionDecision::Acquire { .. })
    }

    /// One line for the acquisition record and the monitor log.
    pub fn line(&self) -> String {
        match self {
            AcquisitionDecision::RuntimeUnsupported { reference } => match reference {
                Some(reference) => format!(
                    "acquire refused at gate 1 (runtime support): upstream has no support yet \
                     (`{reference}` is still open); the download is not reached"
                ),
                None => "acquire refused at gate 1 (runtime support): no upstream support found; \
                         the download is not reached"
                    .to_string(),
            },
            AcquisitionDecision::SupportUnverified { source } => format!(
                "gate 1 (runtime support) inconclusive: `{source}` has not been validated against a \
                 positive control; its negative is not a finding — check the upstream tracker"
            ),
            AcquisitionDecision::NoTrustedQuant { rejected } => {
                let parts = rejected
                    .iter()
                    .map(RejectedQuant::line)
                    .collect::<Vec<_>>()
                    .join("; ");
                format!(
                    "acquire refused at gate 2 (quant policy): no policy-compliant quant from a \
                     trusted publisher — {parts}"
                )
            }
            AcquisitionDecision::NoPlacementRoom {
                quant,
                needed_mib,
                available_mib,
            } => format!(
                "acquire refused at gate 3 (placement): `{quant}` needs {needed_mib} MiB for \
                 weights + full KV; only {available_mib} MiB is available"
            ),
            AcquisitionDecision::Acquire { quant, weights_mib } => format!(
                "acquire authorised: download `{quant}` ({weights_mib} MiB)"
            ),
        }
    }
}

/// The lifecycle of a model's weights on the fleet (invariant 3). The GLM scar
/// (#299) is [`DeploymentState::AcquiredUnservable`]: capital spent and
/// parked, which reads as ordinary backlog unless it has its own name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentState {
    /// The weights have not been acquired.
    NotAcquired,
    /// The download is in progress.
    Acquiring,
    /// Acquired and the runtime serves it.
    Servable,
    /// Acquired but the runtime cannot load it (e.g. pending upstream
    /// architecture support). A distinct state: capital spent and parked, not
    /// ordinary backlog.
    AcquiredUnservable,
}

impl DeploymentState {
    /// Whether the state holds spent capital that cannot serve — the state
    /// that "reads as ordinary backlog" when it is not named (invariant 3).
    pub fn parked(&self) -> bool {
        matches!(self, DeploymentState::AcquiredUnservable)
    }

    pub fn line(&self) -> String {
        match self {
            DeploymentState::NotAcquired => "not acquired".to_string(),
            DeploymentState::Acquiring => "acquiring".to_string(),
            DeploymentState::Servable => "servable".to_string(),
            DeploymentState::AcquiredUnservable => {
                "acquired but unservable — capital spent and parked pending runtime support"
                    .to_string()
            }
        }
    }
}

/// Whether a probe has been validated against its positive controls (invariant
/// 4). A probe that cannot produce a positive on a known-good input has a
/// broken method; its negatives are not findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeValidity {
    /// The probe produced the expected positive on every known-good input. Its
    /// negatives are findings.
    Validated,
    /// No positive control has been established: the probe has never been
    /// shown to produce a positive.
    Unvalidated,
    /// The probe returned negative on a known-good input; its method does not
    /// work here.
    Broken { failed_control: String },
}

impl ProbeValidity {
    /// Whether the probe is validated and its negatives are findings.
    pub fn is_valid(&self) -> bool {
        matches!(self, ProbeValidity::Validated)
    }
}

/// A probe of the world — e.g. "does the runtime support architecture X?"
/// (invariant 4). A *negative* result is a finding only after the probe has
/// been validated against a positive control; before that, a zero means "the
/// method does not work here", not "the input is negative".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub name: String,
    validity: ProbeValidity,
}

impl Probe {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            validity: ProbeValidity::Unvalidated,
        }
    }

    /// Record the probe's result on a known-good input (a positive control). A
    /// negative result breaks the probe (the first such input is recorded and
    /// cannot be re-attributed); a positive validates a probe not yet broken.
    pub fn record_control(&mut self, input: &str, returned_positive: bool) {
        if !returned_positive {
            if matches!(self.validity, ProbeValidity::Broken { .. }) {
                return;
            }
            self.validity = ProbeValidity::Broken {
                failed_control: input.to_string(),
            };
            return;
        }
        if matches!(self.validity, ProbeValidity::Unvalidated) {
            self.validity = ProbeValidity::Validated;
        }
    }

    pub fn validity(&self) -> &ProbeValidity {
        &self.validity
    }

    /// Interpret this probe's negative result (invariant 4).
    pub fn negative_verdict(&self) -> ProbeVerdict {
        match &self.validity {
            ProbeValidity::Validated => ProbeVerdict::Finding,
            ProbeValidity::Unvalidated => ProbeVerdict::Unvalidated,
            ProbeValidity::Broken { failed_control } => ProbeVerdict::Broken {
                failed_control: failed_control.clone(),
            },
        }
    }
}

/// The verdict on a *negative* result from a [`Probe`] (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// The probe is validated; the negative is a real finding.
    Finding,
    /// The probe has never produced a positive; the negative is not a finding.
    Unvalidated,
    /// The probe failed a known-good input; the negative is not a finding.
    Broken { failed_control: String },
}

impl ProbeVerdict {
    /// Whether the negative is a finding.
    pub fn is_finding(&self) -> bool {
        matches!(self, ProbeVerdict::Finding)
    }

    pub fn line(&self) -> String {
        match self {
            ProbeVerdict::Finding => "negative is a finding".to_string(),
            ProbeVerdict::Unvalidated => {
                "negative is not a finding: the probe has never produced a positive \
                 (zero means the method does not work here)"
                    .to_string()
            }
            ProbeVerdict::Broken { failed_control } => format!(
                "negative is not a finding: the probe failed the known-good input `{failed_control}`"
            ),
        }
    }
}
