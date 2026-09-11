//! Restart safety and dependent safety are different properties, and only one
//! of them is visible from inside the service (issue #4241).
//!
//! The incident: a gateway was restarted to deploy a fix. Every local check
//! passed — the build works from current `main`, the preflight refuses to
//! bind unless auth covers the money routes, the reconciler starts a
//! replacement, and the published address file updates on restart. The
//! replacement was healthy in ~90 s and the new address was published
//! correctly.
//!
//! What the checks did not cover: who depends on it. A second machine
//! running the public endpoint kept an inbound SSH tunnel to the gateway and
//! re-registered the gateway's models on a 60 s timer. The gateway moved
//! hosts; the tunnel did not follow. SSH keeps a local forward listening even
//! after the remote side dies: the process never exits, so
//! `Restart=always` never fired, and the unit sat **2 days 23 hours**
//! forwarding to an address that no longer existed while reporting
//! `active (running)`. The public endpoint lost all of the gateway's
//! capacity and answered `/healthz` 200 the whole time. Both sides reported
//! healthy, and neither health check was asking the question that had
//! actually failed.
//!
//! The invariants this module makes checkable:
//!
//! 1. **Restart safety and dependent safety are different properties, and
//!    only one of them is visible from inside the service.** "Can this come
//!    back?" is locally answerable; "what breaks while it is down, and what
//!    fails to notice it came back somewhere else?" is not. Both must be
//!    answered before a deliberate restart, and the gate holds on the missing
//!    half. ([`restart_gate`], [`RestartVerdict`])
//! 2. **Enumerate dependents before a disruptive action, and record the
//!    enumeration.** For a service that publishes an address the search is
//!    mechanical: grep every repo and every host for the published address
//!    file, the service's hostname pattern, and hardcoded host:port pairs. A
//!    sweep that searched nothing is not an enumeration, and an enumeration
//!    that was not written down dies with whoever ran it. ([`DependentSweep`],
//!    [`Locator`])
//! 3. **A dependent on another machine is the one you forget.** Everything on
//!    the cluster reads the record and recovers by itself; the only casualty
//!    is the one across the boundary a local grep does not cross. Cross-host
//!    dependents must be written down somewhere an operator of *either* side
//!    will see. ([`DependentSweep::unrecorded_cross_host`])
//! 4. **"The service is up" is not "the service is serving."** A health
//!    endpoint that cannot go red when the system has no capacity is a
//!    liveness probe wearing a readiness probe's name, and a forwarding unit
//!    whose process never exits is invisible to restart supervision.
//!    ([`health_is_truthful`], [`forwarder_assessment`])
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The caller
//! runs the searches, probes the endpoints, reads the unit states, and
//! reports what it observed; this module decides what the observations mean
//! for a deliberate restart.

use serde::{Deserialize, Serialize};

// --- Invariant 1: the gate answers both questions -------------------------

/// The verdict of a deliberate restart, after both safety questions have been
/// asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartVerdict {
    /// Both questions are answered: the service can come back, the dependent
    /// sweep searched, the enumeration is written down, and every cross-host
    /// dependent is recorded somewhere an operator of either side will see.
    Proceed,
    /// The service itself cannot come back — the half that is visible from
    /// inside the service fails. Nothing else matters.
    NotRestartSafe,
    /// The service can come back, but dependents were never enumerated: no
    /// sweep exists, or it searched nothing. This is the incident's state —
    /// every local check passed.
    DependentsNotEnumerated,
    /// The sweep found cross-host dependents that are not written down
    /// anywhere an operator of either side will see. The gate names them.
    UnrecordedCrossHostDependents {
        /// `host: what` for each unrecorded cross-host dependent.
        unrecorded: Vec<String>,
    },
    /// The sweep searched and found dependents, but the enumeration itself
    /// was not recorded: it exists only in the head of whoever ran it.
    EnumerationNotRecorded,
}

impl RestartVerdict {
    /// True only for [`RestartVerdict::Proceed`].
    pub fn proceeds(&self) -> bool {
        matches!(self, Self::Proceed)
    }

    /// The one-line gate report. Every hold names the missing half, so the
    /// two blocks — "cannot come back" and "dependents unanswered" — stay
    /// distinguishable in a log line.
    pub fn line(&self) -> String {
        match self {
            Self::Proceed => "restart safe and dependents enumerated — proceed".to_string(),
            Self::NotRestartSafe => {
                "the service itself cannot come back — do not restart".to_string()
            }
            Self::DependentsNotEnumerated => {
                "dependents not enumerated: no searches recorded — grep the \
                 address file, the hostname pattern, and the host:port pairs \
                 before restarting"
                    .to_string()
            }
            Self::UnrecordedCrossHostDependents { unrecorded } => format!(
                "cross-host dependents not written down anywhere an operator \
                 of either side will see: {} — record them before restarting",
                unrecorded.join("; ")
            ),
            Self::EnumerationNotRecorded => {
                "the dependent sweep was not written down: an enumeration \
                 that dies with its author is not a record — record it before \
                 restarting"
                    .to_string()
            }
        }
    }
}

/// The deliberate-restart gate. Answers both safety questions, in the order
/// the incident produced them:
///
/// 1. Can the service come back? ([`RestartVerdict::NotRestartSafe`])
/// 2. Were dependents enumerated — searched for, and recorded?
///    ([`RestartVerdict::DependentsNotEnumerated`],
///    [`RestartVerdict::EnumerationNotRecorded`])
/// 3. Is every cross-host dependent written down somewhere an operator of
///    either side will see?
///    ([`RestartVerdict::UnrecordedCrossHostDependents`])
///
/// A gate that has only answered "can this come back?" must not proceed:
/// `sweep` is `None` in exactly that situation, and the verdict says so
/// instead of pretending the question was asked.
pub fn restart_gate(
    restart_safe: bool,
    sweep: Option<&DependentSweep>,
    service_host: &str,
) -> RestartVerdict {
    if !restart_safe {
        return RestartVerdict::NotRestartSafe;
    }
    let Some(sweep) = sweep else {
        return RestartVerdict::DependentsNotEnumerated;
    };
    if !sweep.searched_anything() {
        return RestartVerdict::DependentsNotEnumerated;
    }
    let unrecorded = sweep.unrecorded_cross_host(service_host);
    if !unrecorded.is_empty() {
        return RestartVerdict::UnrecordedCrossHostDependents { unrecorded };
    }
    if !sweep.is_recorded() {
        return RestartVerdict::EnumerationNotRecorded;
    }
    RestartVerdict::Proceed
}

// --- Invariant 2: the enumeration, recorded --------------------------------

/// One mechanical locator of a service's dependents: what a sweep greps for,
/// and the term it greps with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locator {
    /// The path of the address file the service publishes (e.g.
    /// `state/gateway-url`). Everything that reads this record recovers by
    /// itself; everything that does not read it is what the sweep exists to
    /// find.
    AddressFile { path: String },
    /// The service's hostname pattern (e.g. `hive-as-*`). A relocated
    /// service keeps its name, so the pattern — not the old hostname — is
    /// what a dependent may have captured.
    HostnamePattern { pattern: String },
    /// A hardcoded `host:port` pair captured from an address the service
    /// once published.
    HostPort { host: String, port: u16 },
}

impl Locator {
    /// The grep term the caller runs for this locator. Recording the
    /// enumeration therefore records *what was searched*, which makes the
    /// sweep reproducible by its reader.
    pub fn grep_term(&self) -> String {
        match self {
            Self::AddressFile { path } => path.clone(),
            Self::HostnamePattern { pattern } => pattern.clone(),
            Self::HostPort { host, port } => format!("{host}:{port}"),
        }
    }
}

/// One dependent the sweep found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependent {
    /// The host the dependent runs on.
    pub host: String,
    /// What it is and how it reaches the service (e.g. "inbound SSH tunnel
    /// re-registering the service's models on a 60 s timer").
    pub what: String,
    /// Where the dependency is written down — a place an operator of
    /// *either* side will see (the service-side runbook, the dependent-side
    /// config). `None` means not written down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_in: Option<String>,
}

/// The recorded enumeration of a service's dependents, from a sweep run
/// before a disruptive action.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependentSweep {
    /// Where the enumeration is written down (a file path, a runbook
    /// section, an issue). Empty means it lives only in the head of whoever
    /// ran the sweep — which is the same as not being recorded.
    pub recorded_in: String,
    /// The locators the sweep searched for. Empty means "found nothing
    /// because searched for nothing".
    pub locators: Vec<Locator>,
    /// The hosts the search covered. A local grep does not cross a host
    /// boundary; a search that covered no host covered nothing.
    pub hosts_searched: Vec<String>,
    /// The dependents the sweep found.
    pub dependents: Vec<Dependent>,
}

impl DependentSweep {
    /// True when the sweep actually searched: for at least one locator, on
    /// at least one host. A search needs both a *what* and a *where*; a
    /// sweep missing either half is not an enumeration.
    pub fn searched_anything(&self) -> bool {
        !self.locators.is_empty() && !self.hosts_searched.is_empty()
    }

    /// True when the enumeration is written down somewhere.
    pub fn is_recorded(&self) -> bool {
        !self.recorded_in.trim().is_empty()
    }

    /// The cross-host dependents that are not written down anywhere — the
    /// ones an operator of either side will not see (invariant 3). A local
    /// dependent with no record is not reported here: local dependents read
    /// the record and recover by themselves, and the boundary a local grep
    /// does not cross is exactly what this list is for.
    pub fn unrecorded_cross_host(&self, service_host: &str) -> Vec<String> {
        self.dependents
            .iter()
            .filter(|d| d.host != service_host && d.recorded_in.is_none())
            .map(|d| format!("{}: {}", d.host, d.what))
            .collect()
    }
}

// --- Invariant 4: "up" is not "serving" ------------------------------------

/// What a health endpoint claims about the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthClaim {
    /// "This process is alive." Liveness only; says nothing about capacity.
    Liveness,
    /// "This system can serve: it has capacity to answer." Readiness.
    Readiness,
}

/// The verdict on a health report, against what the system actually had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthVerdict {
    /// The report and the system agree — or the report is red, which is
    /// never a lie — or the endpoint only claims liveness, which the system
    /// genuinely did satisfy.
    Honest,
    /// The endpoint claims readiness and reports healthy while the system
    /// has no capacity: it cannot go red when it matters. A liveness probe
    /// wearing a readiness probe's name. This is the state that let
    /// `/healthz` answer 200 for 2 days 23 hours after the tunnel lost all
    /// capacity.
    LivenessWearingReadiness,
}

impl HealthVerdict {
    /// True when the report is a lie, not merely an understatement.
    pub fn is_dishonest(&self) -> bool {
        matches!(self, Self::LivenessWearingReadiness)
    }
}

/// Whether a health report is honest for the claim it makes.
///
/// A readiness claim (the endpoint is presented as "the system can serve")
/// that is green while the system has no capacity is
/// [`HealthVerdict::LivenessWearingReadiness`]. A liveness claim is never
/// a lie — it only asserted the process was alive — but it is not evidence
/// the system can serve, and treating it as such is the incident: the
/// endpoint that answered 200 while the tunnel forwarded nowhere was
/// truthful about liveness and useless about capacity.
pub fn health_is_truthful(
    claim: HealthClaim,
    reports_healthy: bool,
    has_capacity: bool,
) -> HealthVerdict {
    if claim == HealthClaim::Readiness && reports_healthy && !has_capacity {
        HealthVerdict::LivenessWearingReadiness
    } else {
        HealthVerdict::Honest
    }
}

/// What the caller observed about a forwarding unit — an SSH tunnel, a
/// sidecar proxy, a registration bridge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwarderEvidence {
    /// The unit manager reports `active (running)`.
    pub unit_active: bool,
    /// The forwarder's process has exited. Restart supervision
    /// (`Restart=always` and friends) fires on exit only.
    pub process_exited: bool,
    /// The address being forwarded to still accepts connections.
    pub remote_reachable: bool,
}

/// What a forwarding unit's state means for its supervision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwarderAssessment {
    /// Active, and forwarding to an address that is still there.
    Serving,
    /// The unit is `active (running)` but the remote address no longer
    /// exists. The process never exits, so restart supervision never fires;
    /// the unit reports healthy while serving nothing. Supervision cannot
    /// repair this — only an operator, or a unit that tracks the address,
    /// can. This is the incident's state, 2 days 23 hours in.
    StaleForward,
    /// The process exited, or the unit is down: restart supervision has
    /// something to fire on.
    Restartable,
}

impl ForwarderAssessment {
    /// True when restart supervision alone can repair this state. False for
    /// [`ForwarderAssessment::StaleForward`]: the process never exits, so
    /// `Restart=always` never fires.
    pub fn supervision_can_repair(&self) -> bool {
        !matches!(self, Self::StaleForward)
    }

    /// The one-line unit report.
    pub fn line(&self) -> String {
        match self {
            Self::Serving => "active (running), remote reachable — serving".to_string(),
            Self::StaleForward => "active (running) but the remote address no longer exists: \
                 the process never exits, so restart supervision never fires \
                 — active is not serving"
                .to_string(),
            Self::Restartable => {
                "process exited or unit down: restart supervision may fire".to_string()
            }
        }
    }
}

/// What a forwarding unit's observed state means for its supervision.
///
/// The fold to avoid: reading `active (running)` as "the forward works".
/// The unit manager reports the *process*; it does not know whether the
/// address being forwarded to still exists, and it has no reason to act
/// while the process keeps running.
pub fn forwarder_assessment(evidence: &ForwarderEvidence) -> ForwarderAssessment {
    if evidence.unit_active && !evidence.process_exited {
        if evidence.remote_reachable {
            ForwarderAssessment::Serving
        } else {
            ForwarderAssessment::StaleForward
        }
    } else {
        ForwarderAssessment::Restartable
    }
}
