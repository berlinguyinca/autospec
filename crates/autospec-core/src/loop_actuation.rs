//! Readiness loops and their actuators (issue #4268).
//!
//! The InferWeave frontier loop recomputed which issues had every
//! dependency closed and reported `8 ready, 0 dispatched, 8 blocked:
//! no staged spec`, then ran again 30 minutes later with the same
//! numbers. `iw-dispatch.sh` — the actuator the loop was feeding —
//! requires a staged spec at `iw/issues/<n>.md`, and no component in
//! the loop produced that file, so the loop could never dispatch. The
//! readiness computation was decoration: it reported decisions it
//! could not execute, and the report showed the ready and blocked
//! counts without ever saying it could not run its own decision, so
//! the numbers read as steady progress over a loop that was dead.
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **A loop that reports decisions must report whether it could
//!    execute them.** [`LoopReport`] carries the actuator status next
//!    to the ready/dispatched/blocked counts, and [`actuation_findings`]
//!    is the lint: a report with decisions but no stated actuator
//!    status is a finding, and a report whose counts do not reconcile
//!    is one too.
//! 2. **Every precondition the actuator enforces must have an owner
//!    that satisfies it.** [`Precondition`] names the precondition and
//!    its owner; [`precondition_findings`] flags the unsatisfied
//!    precondition with no owner — the block that can never clear. A
//!    hold with a named owner is temporary and renders with the owner,
//!    not silently.
//! 3. **A readiness loop is not working until it has been observed to
//!    dispatch.** [`EndToEndEvidence::loop_working`] — passes with
//!    zero dispatches are compute, not act, and the audit says so.

use std::collections::BTreeMap;

/// Invariant 1: whether the loop's actuator could execute a dispatch
/// on a pass.
///
/// A report of ready/dispatched/blocked counts without this is the
/// incident: numbers that read as steady progress over a loop that
/// could never dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActuatorStatus {
    /// The actuator could execute. If `dispatched < ready` on the
    /// report, the gap is a dispatch defect (capacity, ordering) —
    /// distinct from a precondition gap — and is flagged as one.
    CouldExecute,
    /// The actuator could not dispatch. `reason` is the unmet
    /// precondition as the actuator names it (e.g. `no staged spec`);
    /// `owner` is the component responsible for satisfying it — `None`
    /// is the incident, where nobody satisfied the precondition and
    /// the block was permanent.
    CannotExecute {
        reason: String,
        owner: Option<String>,
    },
}

impl ActuatorStatus {
    /// The fragment the report line contributes for this status:
    /// `could execute`, or `cannot execute: <reason>` with the owner
    /// named — `(owner: <owner>)`, or `(no owner)` when the block has
    /// no release path.
    pub fn fragment(&self) -> String {
        match self {
            ActuatorStatus::CouldExecute => "could execute".to_string(),
            ActuatorStatus::CannotExecute { reason, owner } => match owner {
                Some(owner) => format!("cannot execute: {reason} (owner: {owner})"),
                None => format!("cannot execute: {reason} (no owner)"),
            },
        }
    }
}

/// Invariant 1: the loop's report for one pass.
///
/// The actuator status is a fact about the report, not an assumption
/// the reader makes about the loop: the incident's report had no such
/// field at all, and the "8 ready, 0 dispatched" line was therefore
/// unfalsifiable — it could not be told from "dispatches are slow".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopReport {
    /// Issues the loop judged ready (every dependency closed).
    pub ready: usize,
    /// Issues the loop dispatched on this pass.
    pub dispatched: usize,
    /// Issues held, by the block reason as the loop saw it.
    pub blocked: BTreeMap<String, usize>,
    /// Whether the actuator could execute a dispatch on this pass.
    /// `None` is the incident configuration: the loop reported
    /// decisions without reporting whether it could execute them.
    pub actuator: Option<ActuatorStatus>,
}

impl LoopReport {
    /// The rendered report line. The actuator status is always named
    /// when the loop reported a decision (`ready > 0` or any blocked):
    /// a `None` status renders as `actuator: not reported` rather than
    /// being dropped, and an idle loop that made no decision renders
    /// the counts only.
    pub fn line(&self) -> String {
        let mut parts = vec![format!(
            "{} ready, {} dispatched",
            self.ready, self.dispatched
        )];
        if !self.blocked.is_empty() {
            let blocked: Vec<String> = self
                .blocked
                .iter()
                .map(|(reason, n)| format!("{n} blocked: {reason}"))
                .collect();
            parts.push(blocked.join("; "));
        }
        if self.ready > 0 || !self.blocked.is_empty() {
            let actuator = match &self.actuator {
                Some(status) => status.fragment(),
                None => "actuator: not reported".to_string(),
            };
            parts.push(actuator);
        }
        parts.join(", ")
    }

    /// Invariant 1 (reconciliation): dispatched and blocked issues are
    /// both ready issues, so `dispatched + blocked <= ready`. A report
    /// that violates this is reporting a state that cannot exist — the
    /// same rule as a frontier whose numbers do not reconcile
    /// (`FrontierCounts::line`).
    pub fn reconciles(&self) -> bool {
        let blocked: usize = self.blocked.values().sum();
        self.dispatched + blocked <= self.ready
    }
}

/// Invariant 1, as a lint over a report.
///
/// - `ACTUATION_NOT_REPORTED`: the loop reported a decision (`ready > 0`
///   or any blocked) without stating whether the actuator could
///   execute — the incident line, with no way to tell the loop could
///   never dispatch.
/// - `REPORT_DOES_NOT_RECONCILE`: `dispatched + blocked > ready` — the
///   counts describe a state that cannot exist.
/// - `ACTUATION_GAP`: the actuator reported `CouldExecute` yet
///   `dispatched < ready` — the actuator could have dispatched and did
///   not; that is a dispatch defect, distinct from a precondition gap.
pub fn actuation_findings(report: &LoopReport) -> Vec<String> {
    let mut findings = Vec::new();
    let blocked_total: usize = report.blocked.values().sum();
    if (report.ready > 0 || !report.blocked.is_empty()) && report.actuator.is_none() {
        findings.push(format!(
            "ACTUATION_NOT_REPORTED: reported {} ready and {} blocked without stating whether the actuator could execute",
            report.ready, blocked_total
        ));
    }
    if !report.reconciles() {
        findings.push(format!(
            "REPORT_DOES_NOT_RECONCILE: dispatched ({}) + blocked ({}) > ready ({})",
            report.dispatched, blocked_total, report.ready
        ));
    }
    if matches!(report.actuator, Some(ActuatorStatus::CouldExecute))
        && report.dispatched < report.ready
    {
        findings.push(format!(
            "ACTUATION_GAP: actuator could execute but dispatched {} of {} ready",
            report.dispatched, report.ready
        ));
    }
    findings
}

/// Invariant 2: a precondition the actuator enforces before it will
/// dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Precondition {
    /// The precondition as the actuator names it (e.g. `staged spec at
    /// iw/issues/<n>.md`).
    pub name: String,
    /// Whether it currently holds.
    pub satisfied: bool,
    /// The component that satisfies it (e.g. `iw-stage.sh, called from
    /// the frontier loop`). `None` is the defect: a precondition nobody
    /// satisfies is a permanent block, not a hold.
    pub owner: Option<String>,
}

impl Precondition {
    /// The hold line for this precondition, naming the owner. A
    /// precondition with no owner renders `(no owner)` — the line that
    /// says the block can never clear, not that it will.
    pub fn hold_line(&self) -> String {
        match &self.owner {
            Some(owner) => format!("blocked: {} (owner: {owner})", self.name),
            None => format!("blocked: {} (no owner)", self.name),
        }
    }
}

/// Invariant 2, as a lint over the actuator's precondition set.
///
/// `OWNERLESS_PRECONDITION`: an unsatisfied precondition with no owner
/// — the incident. The loop reports the block on every pass and
/// nothing in the report says the block can never clear. Satisfied
/// preconditions and holds with a named owner are not findings: the
/// latter are temporary and visible via [`Precondition::hold_line`].
pub fn precondition_findings(preconditions: &[Precondition]) -> Vec<String> {
    preconditions
        .iter()
        .filter(|p| !p.satisfied && p.owner.is_none())
        .map(|p| {
            format!(
                "OWNERLESS_PRECONDITION: '{}' is not satisfied and no component is responsible for satisfying it",
                p.name
            )
        })
        .collect()
}

/// Invariant 3: whether the loop has been observed to act
/// end-to-end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndToEndEvidence {
    /// The loop has completed `passes` passes and dispatched in at
    /// least one of them.
    Dispatched { passes: usize },
    /// The loop has completed `passes` passes and never dispatched —
    /// it computes, it does not act.
    NeverDispatched { passes: usize },
}

impl EndToEndEvidence {
    /// A readiness loop is working only when it has been observed to
    /// dispatch. `NeverDispatched` at any pass count — including the
    /// large ones — is the incident: N passes at 30 minutes each with
    /// the same numbers is not operation, it is a permanent block
    /// reporting itself.
    pub fn loop_working(&self) -> bool {
        matches!(self, EndToEndEvidence::Dispatched { .. })
    }

    /// The rendered evidence line.
    pub fn line(&self) -> String {
        match self {
            EndToEndEvidence::Dispatched { passes } => {
                format!("end-to-end: dispatched in {passes} pass(es) — working")
            }
            EndToEndEvidence::NeverDispatched { passes } => {
                format!("end-to-end: 0 dispatched in {passes} pass(es) — compute, not act")
            }
        }
    }
}

/// The combined audit for a readiness loop: the report's actuation
/// findings, the precondition findings, and the end-to-end evidence.
///
/// `NOT_OBSERVED_TO_ACT`: the loop has never been observed to
/// dispatch — a readiness loop is not proven working until one full
/// cycle has dispatched, and the audit says so rather than letting the
/// loop age into "steady operation". An empty result with
/// [`EndToEndEvidence::loop_working`] true is the loop that is not
/// decoration.
pub fn audit(
    report: &LoopReport,
    preconditions: &[Precondition],
    evidence: EndToEndEvidence,
) -> Vec<String> {
    let mut findings = actuation_findings(report);
    findings.extend(precondition_findings(preconditions));
    if !evidence.loop_working() {
        findings.push(format!("NOT_OBSERVED_TO_ACT: {}", evidence.line()));
    }
    findings
}
