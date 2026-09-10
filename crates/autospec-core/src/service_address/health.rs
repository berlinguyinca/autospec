//! Health of a component that serves work *and* belongs to a pool.

use std::fmt;

use super::registration::RegistrationOutcome;

/// Health of a component that both serves work and belongs to a pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentHealth {
    /// Serving, and the pool knows about it.
    Healthy,
    /// Serving its own workload but **not in the pool it was created for**.
    /// This is a distinct unhealthy state: the GPUs are busy, agents dispatch
    /// directly, tests pass — and the gateway's inventory is wrong.
    DegradedNotInPool {
        /// Registration attempts made since the last success.
        attempts: u32,
        /// What the most recent attempt reported.
        last: RegistrationOutcome,
    },
    /// Not serving.
    Down,
}

impl ComponentHealth {
    /// True only for a component that serves *and* is in the pool.
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }
}

impl fmt::Display for ComponentHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healthy => f.write_str("healthy"),
            Self::DegradedNotInPool { attempts, last } => {
                write!(f, "degraded: not in pool after {attempts} attempts; {last}")
            }
            Self::Down => f.write_str("down"),
        }
    }
}

/// Combine "is it serving" with "did it join the pool".
///
/// A serving component whose latest registration failed is
/// [`ComponentHealth::DegradedNotInPool`], never `Healthy`: "serving anyway" is
/// exactly the fold that hid the drain for a day.
pub fn component_health(
    serving: bool,
    registration: RegistrationOutcome,
    attempts: u32,
) -> ComponentHealth {
    if !serving {
        return ComponentHealth::Down;
    }
    if registration.succeeded() {
        ComponentHealth::Healthy
    } else {
        ComponentHealth::DegradedNotInPool {
            attempts,
            last: registration,
        }
    }
}

/// What a health check actually observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthEvidence {
    /// The service answered the probe (any status: an answer proves it is up).
    Responded { status: u16 },
    /// The probe got no answer.
    NoResponse,
    /// The scheduler believes the job is running. Carries no information about
    /// the service behind it, and is the assertion the reconciler made.
    SchedulerJobState { state: String },
}

/// The verdict a health check may report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceHealth {
    /// The service responded.
    Up,
    /// The service did not respond.
    Down { reason: String },
    /// The check asserted a scheduler's belief rather than a response, so it
    /// has no evidence about the service either way. This is never `Up`, and
    /// never `Down` — the check, not the service, is what is broken.
    AssertedWrongThing { claim: String },
}

/// Judge a service from the evidence a check gathered.
///
/// Liveness of a container is not health of a service, and neither is
/// reachability of an address some third party recorded. Only a response from
/// the service counts as health.
pub fn service_health(evidence: &HealthEvidence) -> ServiceHealth {
    match evidence {
        HealthEvidence::Responded { .. } => ServiceHealth::Up,
        HealthEvidence::NoResponse => ServiceHealth::Down {
            reason: "service did not respond to the health probe".to_string(),
        },
        HealthEvidence::SchedulerJobState { state } => ServiceHealth::AssertedWrongThing {
            claim: format!("scheduler reports job state {state:?}; no service response observed"),
        },
    }
}
