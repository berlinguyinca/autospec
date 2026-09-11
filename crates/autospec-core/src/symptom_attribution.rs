//! Symptom attribution: which component emitted the symptom, decided
//! before any metric is read (issue #4247).
//!
//! The incident: users reported `503 no worker for model`. The hive
//! gateway — the one on the GPU cluster, reached over the SSH tunnel —
//! reported **10 workers registered, all four models warm,
//! `served_503=0`**, every endpoint answering, and its registration
//! sweep healthy every 5 minutes. Every check on that component was
//! fine. The users were still getting 503s, and the diagnosis spent an
//! hour re-checking the fleet, the tunnel, the sweep and the worker
//! endpoints.
//!
//! There are two gateways:
//!
//! ```text
//! llm.metabolomics.us -> nginx -> inferweave-gateway ON THE EDGE HOST
//!                               -> SSH tunnel (127.0.0.1:20580)
//!                                  -> the hive gateway (Slurm job) -> workers
//! ```
//!
//! The edge gateway keeps its own worker pool and emits its own 503s.
//! The hive gateway was **off the user's request path from the first
//! command** — nothing the operator did would have reached it through
//! the public URL. The edge gateway's log gave the answer in seconds:
//! 374 failed registrations in three hours, all for one model.
//!
//! The invariants this module makes checkable, in the order the
//! incident produced them:
//!
//! 1. **Trace the request path before reading any metric.**
//!    [`RequestPath`] enumerates every hop from the user's arrival
//!    point to the hardware, by name, and which hops can emit the
//!    symptom under diagnosis. A diagnosis with no path is a guess:
//!    the 503 could have come from any of four components, and the
//!    one re-checked was not among the ones on the path.
//! 2. **A component reporting zero errors is evidence only that *it*
//!    is healthy.** [`assess`] grades a [`HealthCheck`] against the
//!    path: a clean check is [`Evidence::Decisive`] only when the
//!    checked component is the sole possible emitter on the user's
//!    path; otherwise it is [`Evidence::ComponentScoped`], and a check
//!    on a component not on the path at all is [`Evidence::OffPath`] —
//!    no evidence, and worse than none, because a clean result on the
//!    wrong component builds false confidence.
//! 3. **A stack with two instances of the same service is a standing
//!    trap.** [`duplicate_services`] finds them; a diagnosis about a
//!    trap service must name the specific instance the symptom
//!    belongs to, and [`judge`] renders a service-only claim
//!    (`"the gateway is healthy"`) as [`TrapVerdict::Unattributed`].
//! 4. **Prefer the symptom-side probe.** One authenticated request to
//!    the public URL identifies the failing hop — the first hop that
//!    cannot answer. [`select_probe`] orders it ahead of every
//!    component-side check, and [`judge_probe`] names the incident's
//!    error when the reachable, credentialed component was probed
//!    instead.
//!
//! Everything here is pure: no I/O, no clock, no subprocesses. The
//! caller observes the components and reports them; this code never
//! opens a connection.

use std::fmt;

/// Why a [`RequestPath`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// A path with no hops. The user's request arrives somewhere, so a
    /// diagnosis with no path has nothing to attribute to.
    Empty,
    /// The same hop appears twice. A hop is one named position in the
    /// path, and a repeated name would hide two different positions
    /// behind one.
    DuplicateHop { name: String },
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => write!(f, "request path has no hops"),
            PathError::DuplicateHop { name } => {
                write!(f, "hop `{name}` appears more than once in the request path")
            }
        }
    }
}

impl std::error::Error for PathError {}

/// One named hop on the user's request path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub name: String,
    /// Whether this hop can emit the symptom under diagnosis (e.g.
    /// synthesize a `503 no worker for model` of its own).
    pub can_emit: bool,
}

impl Hop {
    pub fn new(name: impl Into<String>, can_emit: bool) -> Self {
        Hop {
            name: name.into(),
            can_emit,
        }
    }
}

/// The request path from the user's arrival point to the hardware, in
/// arrival order (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPath {
    hops: Vec<Hop>,
}

fn duplicate_hop_name(hops: &[Hop]) -> Option<String> {
    let mut seen: Vec<&str> = Vec::new();
    for hop in hops {
        if seen.contains(&hop.name.as_str()) {
            return Some(hop.name.clone());
        }
        seen.push(&hop.name);
    }
    None
}

impl RequestPath {
    /// Build a path from its hops, in arrival order. Rejects an empty
    /// path and a repeated hop name.
    pub fn new(hops: Vec<Hop>) -> Result<Self, PathError> {
        if hops.is_empty() {
            return Err(PathError::Empty);
        }
        if let Some(name) = duplicate_hop_name(&hops) {
            return Err(PathError::DuplicateHop { name });
        }
        Ok(Self { hops })
    }

    /// The hops, in arrival order.
    pub fn hops(&self) -> &[Hop] {
        &self.hops
    }

    /// Whether a hop with this name is on the path.
    pub fn contains(&self, name: &str) -> bool {
        self.hops.iter().any(|h| h.name == name)
    }

    /// The hops that can emit the symptom, in arrival order.
    pub fn emitters(&self) -> Vec<String> {
        self.hops
            .iter()
            .filter(|h| h.can_emit)
            .map(|h| h.name.clone())
            .collect()
    }
}

/// A health check observed on one component: the component's name and
/// how many of the symptom it reported (e.g. `served_503=0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthCheck {
    pub component: String,
    pub errors_observed: u64,
}

/// What a health check is evidence of, against the user's request path
/// (invariant 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// The checked component is on the user's path and reported the
    /// symptom: the emitter is located.
    Attributed { component: String },
    /// The checked component is on the path, reports the symptom zero
    /// times, and is the only hop that can emit it: the symptom is
    /// ruled out on the user's path.
    Decisive,
    /// The checked component is on the path and reports the symptom
    /// zero times, but other hops can emit it too: evidence that *it*
    /// is healthy, not that the user's symptom is absent. The
    /// incident's hive gateway: `served_503=0`, true and useless.
    ComponentScoped,
    /// The checked component is not on the user's request path at all:
    /// no evidence about the user's symptom, in either direction — and
    /// a clean result on it builds false confidence.
    OffPath,
}

/// Grade a health check against the user's request path (invariant 2).
///
/// The check answers "is *this component* healthy?", never "is the
/// user's symptom absent?" — unless the component is the only one on
/// the path that could produce the symptom.
pub fn assess(check: &HealthCheck, path: &RequestPath) -> Evidence {
    if !path.contains(&check.component) {
        return Evidence::OffPath;
    }
    if check.errors_observed > 0 {
        return Evidence::Attributed {
            component: check.component.clone(),
        };
    }
    let emitters = path.emitters();
    if emitters == vec![check.component.clone()] {
        Evidence::Decisive
    } else {
        Evidence::ComponentScoped
    }
}

/// One service instance in a stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub name: String,
    pub service: String,
    /// Whether user traffic reaches this instance.
    pub on_user_path: bool,
}

/// The names of services with more than one instance, sorted: the
/// standing traps (invariant 3). A service with one instance names its
/// component; a service with two does not.
pub fn duplicate_services(instances: &[Instance]) -> Vec<String> {
    let mut services: Vec<&str> = instances.iter().map(|i| i.service.as_str()).collect();
    services.sort();
    services.dedup();
    services
        .into_iter()
        .filter(|s| instances.iter().filter(|i| i.service == *s).count() > 1)
        .map(String::from)
        .collect()
}

/// A diagnosis claim about a service: the service it is about, and the
/// specific instance it names, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnosis {
    pub service: String,
    pub instance: Option<String>,
}

/// Whether a diagnosis is properly attributed for the stack it
/// describes (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrapVerdict {
    /// The service has exactly one instance: naming the service names
    /// the component.
    NoTrap,
    /// The service has more than one instance and the claim names one
    /// of them: properly attributed.
    Attributed { instance: String },
    /// The service has more than one instance and the claim names only
    /// the service: the trap is sprung — it does not say which
    /// instance the symptom belongs to. `instances` lists the ones it
    /// should have chosen from.
    Unattributed { instances: Vec<String> },
    /// The claim names an instance that does not belong to the
    /// service.
    NotAnInstance { instance: String },
    /// The service has no instance in the stack: the claim is not
    /// about this stack at all.
    UnknownService { service: String },
}

/// Judge a diagnosis claim against the stack's instances (invariant 3).
pub fn judge(diagnosis: &Diagnosis, instances: &[Instance]) -> TrapVerdict {
    let mut mine: Vec<String> = instances
        .iter()
        .filter(|i| i.service == diagnosis.service)
        .map(|i| i.name.clone())
        .collect();
    if mine.is_empty() {
        return TrapVerdict::UnknownService {
            service: diagnosis.service.clone(),
        };
    }
    if mine.len() == 1 {
        return TrapVerdict::NoTrap;
    }
    match &diagnosis.instance {
        Some(instance) if mine.iter().any(|n| n == instance) => TrapVerdict::Attributed {
            instance: instance.clone(),
        },
        Some(instance) => TrapVerdict::NotAnInstance {
            instance: instance.clone(),
        },
        None => {
            mine.sort();
            TrapVerdict::Unattributed { instances: mine }
        }
    }
}

/// Which side of a symptom a probe lives on (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSide {
    /// An authenticated request to the public entry the user is
    /// actually talking to. It localizes the failing hop: the first
    /// hop that cannot answer is the one to look at.
    SymptomSide,
    /// A check of an internal component the investigator has
    /// credentials and habit for. Reachable — and that is the trap —
    /// but not necessarily on the user's path.
    ComponentSide,
}

/// One probe that could be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub side: ProbeSide,
    pub target: String,
}

impl Probe {
    pub fn new(side: ProbeSide, target: impl Into<String>) -> Self {
        Probe {
            side,
            target: target.into(),
        }
    }
}

/// What [`select_probe`] chose to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeSelection {
    /// A symptom-side probe exists: it runs first, whatever order the
    /// rest were listed in.
    SymptomSide { target: String },
    /// No symptom-side probe is available: fall back to the first
    /// component-side probe, knowing it may be off the user's path.
    ComponentSideOnly { target: String },
    /// Nothing to probe.
    None,
}

/// Choose the first probe to run from the available ones (invariant 4).
/// The symptom side always wins over the component side.
pub fn select_probe(available: &[Probe]) -> ProbeSelection {
    if let Some(probe) = available.iter().find(|p| p.side == ProbeSide::SymptomSide) {
        return ProbeSelection::SymptomSide {
            target: probe.target.clone(),
        };
    }
    match available.first() {
        Some(probe) => ProbeSelection::ComponentSideOnly {
            target: probe.target.clone(),
        },
        None => ProbeSelection::None,
    }
}

/// Whether the probe the investigator actually ran was the right kind,
/// given what was available (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// The probe that ran was on the symptom's side — or it was the
    /// only kind available.
    CorrectSide,
    /// A symptom-side probe was available, but a component-side probe
    /// ran instead: the reachable component is not the user's
    /// component. The incident's hour.
    ComponentSideWhenSymptomSideAvailable { chosen: String, missed: String },
}

/// Judge the probe that ran against the probes that were available.
/// `chosen` must be one of `available`.
pub fn judge_probe(chosen: &Probe, available: &[Probe]) -> ProbeVerdict {
    if chosen.side == ProbeSide::SymptomSide {
        return ProbeVerdict::CorrectSide;
    }
    match available.iter().find(|p| p.side == ProbeSide::SymptomSide) {
        Some(missed) => ProbeVerdict::ComponentSideWhenSymptomSideAvailable {
            chosen: chosen.target.clone(),
            missed: missed.target.clone(),
        },
        None => ProbeVerdict::CorrectSide,
    }
}
