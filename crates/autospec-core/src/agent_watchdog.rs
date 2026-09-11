//! Bounded agent calls and the watchdog that reaps the ones that escape
//! them (issue #4258).
//!
//! The incident: the agent runner bounded its model call with
//! `timeout 2700 pi --print ...` — 45 minutes. Bare `timeout` sends SIGTERM
//! and then waits indefinitely, and the model client does not die on
//! SIGTERM while blocked on a request to a worker. Measured on the live
//! fleet: **19 of 21 running agents were past the limit, one of them by
//! 6 hours 50 minutes**, each holding a GPU worker slot and an 8-hour
//! allocation while producing exactly 0 bytes. The held requests made the
//! fleet look saturated — a timeout failure in the agent layer that read as
//! a serving-layer outage.
//!
//! Every available signal said "fine": the Slurm job was `RUNNING`, the
//! wrapper's `.out` file existed, and the agent count was at its configured
//! maximum. The invariants this module enforces, in the order the incident
//! produced them:
//!
//! 1. **Every timeout must be escalated.** A timeout without a
//!    non-ignorable kill escalation is a comment, not a control.
//!    [`Escalation`] renders `timeout -k <grace> <limit> ...`, and
//!    [`inspect_timeout_command`] rejects a bare `timeout`.
//! 2. **A supervisor must verify the child actually died.** Sending a
//!    signal is not killing. [`verify_call`] fails a call that outlived
//!    `limit + grace` or that has not been observed to return — the 7-hour
//!    case becomes detectable in ~46 minutes.
//! 3. **Liveness is output, never process state.** "The job is RUNNING"
//!    was true for all 19. [`liveness`] decides from a positive artifact of
//!    progress; the process state is accepted and ignored by design.
//! 4. **The reaper's detector cannot be fooled by buffering.** A working
//!    process can also show an empty redirect target, so "the log is empty"
//!    is never a hang signal — and [`AgentObservation`] deliberately
//!    carries no output-size field. The detector is the supervisor's own
//!    bookkeeping: "still inside the call long after the limit should have
//!    fired" ([`WatchdogSweep`]).
//! 5. **Cap the blast radius of an automated killer.** The sweep kills at
//!    most [`DEFAULT_KILL_CAP`] (5) agents; if the detection is ever wrong,
//!    5 is recoverable and 22 is not. A summary line is rendered every
//!    sweep, even an idle one, so a dead watchdog is visible.
//!
//! The module is pure: the caller runs the bounded call and reports the
//! observed wrapper state; this code never spawns, never reads a clock.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default SIGKILL escalation grace: 60s after the SIGTERM.
pub const DEFAULT_KILL_GRACE: Duration = Duration::from_secs(60);
/// Default detection margin: an in-call agent is hung `limit + 30m` out,
/// giving the escalation (and a legitimately slow return) room to land
/// before the reaper acts.
pub const DEFAULT_HANG_MARGIN: Duration = Duration::from_secs(1800);
/// Default kills per sweep. 5 is recoverable; 22 is not (invariant 5).
pub const DEFAULT_KILL_CAP: usize = 5;

/// A bounding timeout with a non-ignorable escalation: SIGTERM at the
/// limit, SIGKILL `grace` later (invariant 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Escalation {
    pub limit: Duration,
    pub grace: Duration,
}

/// Why [`Escalation::new`] refused the pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationError {
    /// Zero grace is a bare `timeout`: SIGTERM only, and the child may
    /// ignore it. A timeout that can be ignored by the thing it is bounding
    /// is a comment, not a control.
    NoEscalation,
}

impl EscalationError {
    pub fn as_str(self) -> &'static str {
        match self {
            EscalationError::NoEscalation => "no-escalation",
        }
    }
}

impl Escalation {
    /// `limit` is the SIGTERM instant, `grace` the SIGKILL instant after it.
    /// Zero `grace` is rejected: that is the bare-`timeout` defect.
    pub fn new(limit: Duration, grace: Duration) -> Result<Escalation, EscalationError> {
        if grace.is_zero() {
            return Err(EscalationError::NoEscalation);
        }
        Ok(Escalation { limit, grace })
    }

    /// The limit plus the grace: the latest instant the child may still be
    /// alive and the supervisor may still be innocent.
    pub fn hard_deadline(&self) -> Duration {
        self.limit + self.grace
    }

    /// Render the bounding command: `timeout -k <grace> <limit> <args...>`.
    /// The caller already handles `rc=124 -> TIMEOUT`; this gives it a chance.
    pub fn render_command(&self, args: &str) -> String {
        format!(
            "timeout -k {} {} {}",
            secs(self.grace),
            secs(self.limit),
            args
        )
    }
}

/// How a wrapper's `timeout` usage fares against invariant 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimeoutCommand {
    /// `timeout -k <grace> <limit> ...` — escalates to a non-ignorable
    /// kill. A control.
    Escalating { limit: Duration, grace: Duration },
    /// `timeout <limit> ...` — SIGTERM only. A comment, not a control.
    Bare { limit: Duration },
    /// No `timeout` wrapper at all: the call is unbounded.
    Unbounded,
}

impl TimeoutCommand {
    pub fn is_control(&self) -> bool {
        matches!(self, TimeoutCommand::Escalating { .. })
    }
}

/// Inspect a wrapper command line and classify its `timeout` usage.
///
/// Token-level, like the repo's other ratchets: it understands `timeout`,
/// the `-k`/`--kill-after` option (space- or `=`-separated), other options
/// it does not interpret, and GNU duration suffixes (`s`, `m`, `h`, `d`).
pub fn inspect_timeout_command(command: &str) -> TimeoutCommand {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let Some(start) = tokens.iter().position(|t| *t == "timeout") else {
        return TimeoutCommand::Unbounded;
    };
    let mut limit: Option<Duration> = None;
    let mut grace: Option<Duration> = None;
    let mut grace_value_next = false;
    for tok in &tokens[start + 1..] {
        if limit.is_some() {
            break; // the limit precedes the command; nothing after matters
        }
        if grace_value_next {
            grace = parse_secs(tok);
            grace_value_next = false;
            continue;
        }
        if let Some(v) = tok.strip_prefix("--kill-after=") {
            grace = parse_secs(v);
            continue;
        }
        if *tok == "-k" {
            grace_value_next = true;
            continue;
        }
        if let Some(v) = tok.strip_prefix("-k") {
            if !v.is_empty() {
                grace = parse_secs(v);
            }
            continue;
        }
        if tok.starts_with('-') {
            // an option this inspector does not interpret; its value, if
            // any, follows and is not the limit either
            continue;
        }
        if let Some(d) = parse_secs(tok) {
            limit = Some(d);
        }
    }
    match (limit, grace) {
        (Some(limit), Some(grace)) => TimeoutCommand::Escalating { limit, grace },
        (Some(limit), None) => TimeoutCommand::Bare { limit },
        (None, _) => TimeoutCommand::Unbounded,
    }
}

/// A GNU-style duration: a non-negative integer plus an optional suffix.
fn parse_secs(token: &str) -> Option<Duration> {
    let (num, mult): (&str, u64) = if let Some(v) = token.strip_suffix('s') {
        (v, 1)
    } else if let Some(v) = token.strip_suffix('m') {
        (v, 60)
    } else if let Some(v) = token.strip_suffix('h') {
        (v, 3600)
    } else if let Some(v) = token.strip_suffix('d') {
        (v, 86400)
    } else {
        (token, 1)
    };
    let secs: u64 = num.parse().ok()?;
    Some(Duration::from_secs(secs.checked_mul(mult)?))
}

fn secs(d: Duration) -> u64 {
    d.as_secs()
}

/// What the supervisor observed when (or while) the bounded call ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorVerdict {
    /// The call returned within the limit: the work (or the model) finished
    /// in time.
    InTime,
    /// The escalation did its job: the child died inside the grace window
    /// after the SIGTERM.
    Escalated,
    /// The child outlived `limit + grace`: the signal never landed and the
    /// supervisor never noticed. This is the 6h50m case — detectable at
    /// ~46 minutes, observed at 7 hours (invariant 2).
    ChildOutlivedBound,
    /// The wrapper has not regained control at the time of the check: the
    /// supervisor has not verified the child died. Sending the signal is
    /// not killing (invariant 2).
    NotVerified,
}

/// Invariant 2: assert the child actually died, not merely that a signal
/// was sent. `elapsed` is wall time from spawn to the wrapper regaining
/// control; `child_gone` is whether the wrapper has regained control at all.
pub fn verify_call(
    escalation: &Escalation,
    elapsed: Duration,
    child_gone: bool,
) -> SupervisorVerdict {
    if !child_gone {
        return SupervisorVerdict::NotVerified;
    }
    if elapsed > escalation.hard_deadline() {
        return SupervisorVerdict::ChildOutlivedBound;
    }
    if elapsed > escalation.limit {
        return SupervisorVerdict::Escalated;
    }
    SupervisorVerdict::InTime
}

/// The Slurm job state, as the scheduler reports it. Accepted by
/// [`liveness`] and ignored by it (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Exited,
}

/// A positive artifact of progress: a file the agent writes as it works.
/// The fleet's `build.log` is written immediately after the model call
/// returns, so its presence says the call completed and its absence, past
/// the bound, says it did not (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressArtifact {
    pub present: bool,
}

/// Whether the agent is making progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Liveness {
    Alive,
    Stalled,
}

/// Invariant 3: liveness is output, never process state. `state` is
/// accepted for the caller's evidence record and deliberately ignored —
/// "the job is RUNNING" was true for all 19 hung agents. Only a positive
/// artifact of progress makes an agent alive.
pub fn liveness(state: ProcessState, artifact: &ProgressArtifact) -> Liveness {
    let _ = state; // process state is not liveness evidence
    if artifact.present {
        Liveness::Alive
    } else {
        Liveness::Stalled
    }
}

/// One agent as the supervisor sees it.
///
/// Deliberately carries **no output-size field** (invariant 4): a working
/// process can show an empty redirect target while output is buffered, so
/// file size cannot enter the hang decision; the detector is the
/// supervisor's own call bookkeeping, which buffering cannot confound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentObservation {
    pub id: String,
    /// The wrapper has not yet regained control of the bounded call.
    pub in_call: bool,
    /// Wall time inside the call since it started (final value once the
    /// wrapper has regained control).
    pub call_elapsed: Duration,
    pub artifact: ProgressArtifact,
}

/// The reaper's policy (invariant 5): the escalation it enforces, the
/// detection margin past the limit, and the per-sweep kill cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchdogPolicy {
    pub escalation: Escalation,
    pub detection_margin: Duration,
    pub kill_cap: usize,
}

impl WatchdogPolicy {
    /// The fleet's shape: `limit` with the 60s SIGKILL grace, a 30-minute
    /// detection margin, and the 5-per-sweep cap.
    pub fn default_for_limit(limit: Duration) -> WatchdogPolicy {
        WatchdogPolicy {
            escalation: Escalation {
                limit,
                grace: DEFAULT_KILL_GRACE,
            },
            detection_margin: DEFAULT_HANG_MARGIN,
            kill_cap: DEFAULT_KILL_CAP,
        }
    }
}

/// One sweep's result. `checked` is the whole population observed; the
/// rest partition it: `healthy + killed + deferred + supervisor_violations
/// == checked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchdogSweep {
    pub checked: usize,
    pub healthy: usize,
    /// Hung agents killed this sweep, at most [`WatchdogPolicy::kill_cap`].
    pub killed: Vec<String>,
    /// Hung agents past the cap: left for the next sweep (invariant 5).
    pub deferred: Vec<String>,
    /// Calls that returned but outlived `limit + grace`: the supervisor
    /// failed to verify the child died (invariant 2). Not killed — they are
    /// gone; they are reported.
    pub supervisor_violations: Vec<String>,
}

impl WatchdogSweep {
    /// One summary line, rendered on every sweep — even an idle one — so a
    /// dead watchdog is visible (invariant 5). Shape: the fleet's first
    /// real sweep, `checked=21 healthy=13 hung_killed=5 hung_deferred=3`.
    pub fn summary_line(&self) -> String {
        let mut line = format!(
            "checked={} healthy={} hung_killed={}",
            self.checked,
            self.healthy,
            self.killed.len()
        );
        if !self.deferred.is_empty() {
            line.push_str(&format!(" hung_deferred={}", self.deferred.len()));
        }
        line
    }
}

/// Whether one observation is a hang (invariants 3 and 4): still inside
/// the call past `limit + margin` with no positive artifact of progress.
fn is_hung(obs: &AgentObservation, policy: &WatchdogPolicy) -> bool {
    obs.in_call
        && obs.call_elapsed > policy.escalation.limit + policy.detection_margin
        && !obs.artifact.present
}

/// A call that returned but outlived `limit + grace`: the supervisor failed
/// to verify the child died (invariant 2).
fn is_supervisor_violation(obs: &AgentObservation, policy: &WatchdogPolicy) -> bool {
    !obs.in_call
        && verify_call(&policy.escalation, obs.call_elapsed, true)
            == SupervisorVerdict::ChildOutlivedBound
}

/// Run one watchdog sweep over the observed agents: hung agents are killed
/// in sorted-id order up to the cap; a call that returned late is never
/// killed — it is gone — it is reported as a supervisor violation.
pub fn watchdog_sweep(agents: &[AgentObservation], policy: &WatchdogPolicy) -> WatchdogSweep {
    let mut hung: Vec<String> = agents
        .iter()
        .filter(|a| is_hung(a, policy))
        .map(|a| a.id.clone())
        .collect();
    hung.sort();

    let violations: Vec<String> = agents
        .iter()
        .filter(|a| is_supervisor_violation(a, policy))
        .map(|a| a.id.clone())
        .collect();

    let hung_total = hung.len();
    let cut = policy.kill_cap.min(hung_total);
    let killed = hung.split_at(cut).0.to_vec();
    let deferred = hung.split_at(cut).1.to_vec();
    let healthy = agents
        .len()
        .saturating_sub(hung_total)
        .saturating_sub(violations.len());

    WatchdogSweep {
        checked: agents.len(),
        healthy,
        killed,
        deferred,
        supervisor_violations: violations,
    }
}
