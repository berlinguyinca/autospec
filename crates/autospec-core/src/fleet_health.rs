//! Per-repository fleet health: the outcome of the pipeline, not its
//! components (issue #4068).
//!
//! The incident: one repository in the fleet sat for three days with open
//! issues, staged specs, and zero agents running. The fleet status line for it
//! read `nothing to do`, because the thing that renders the status reports the
//! components — a scheduler entry, a worker, a queue file — and here all three
//! were present and none of them was working. The scheduler that would have
//! dispatched this repo lived on another machine, in a script with a name
//! matching the hostname pattern, on no crontab at all. A repo whose
//! dispatcher was never installed, and a repo with genuinely nothing to do,
//! render the same line, so the difference between the two is invisible to the
//! one person who could act on it.
//!
//! The invariants, in the order the incident produced them:
//!
//! * the health report is per repository and covers **every** repository the
//!   fleet serves, whether or not that repository has a scheduler entry on this
//!   machine (invariant 1); a repository the report never observed is its own
//!   state and never renders as idle;
//! * `stalled` — dispatchable work, no agents, and no dispatch for longer
//!   than the interval — is never rendered the same as `idle`, no work
//!   available (invariant 2); the stall interval is a parameter, not a
//!   constant baked into the renderer;
//! * a dispatcher intended to be driven by a human rather than a schedule is
//!   **declared** as one, with the declaration recorded somewhere, so "not on a
//!   schedule" is a decision on record rather than an omission
//!   indistinguishable from one (invariant 3); a declaration that names no
//!   record is not a declaration;
//! * the report states the outcome per repository — healthy, stalled, idle,
//!   awaiting an operator — and carries the observation (counts, last
//!   dispatch) beside the verdict, so the verdict can be argued with
//!   (invariant 4).
//!
//! Everything here is pure: no I/O, no clock. The caller supplies the served
//! repository list, the per-repository observations with `now` folded into
//! elapsed durations, and the stall interval.

use std::collections::BTreeSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::stored_output::format_age;

/// How long a repository may hold dispatchable work with no agents and no
/// dispatch before it is reported stalled.
///
/// Two hours is deliberately generous: it is several multiples of the interval
/// a healthy dispatcher runs on, so a repo is not called stalled while its
/// scheduler is merely between passes. Callers pass their own value — the
/// interval is configuration, and a renderer that hard-codes it cannot be
/// argued with when it is wrong.
pub const DEFAULT_STALL_AFTER: Duration = Duration::from_secs(2 * 60 * 60);

/// How a repository's dispatcher is meant to be driven.
///
/// `Undeclared` is a variant rather than a default because it is the state the
/// incident was found in: nobody had decided either way and written it down,
/// and the absence of a schedule was read as the absence of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchMode {
    /// A scheduler (cron, systemd timer, monitor loop) is expected to drive it.
    Scheduled,
    /// A human drives it. Legitimate — but it must be declared, and the
    /// declaration must name where it is recorded (see [`RepoState::declared_in`]).
    OperatorDriven,
    /// Nobody has said. Not a decision, and reported as its own gap.
    Undeclared,
}

impl DispatchMode {
    /// The token written in a fleet declaration file.
    pub fn as_str(self) -> &'static str {
        match self {
            DispatchMode::Scheduled => "scheduled",
            DispatchMode::OperatorDriven => "operator-driven",
            DispatchMode::Undeclared => "undeclared",
        }
    }

    /// Parse a declaration token. Anything unrecognised — including an empty
    /// field — is [`DispatchMode::Undeclared`], never a permissive default.
    pub fn parse(token: &str) -> Self {
        match token.trim().to_ascii_lowercase().as_str() {
            "scheduled" | "schedule" | "cron" | "timer" | "monitor" => DispatchMode::Scheduled,
            "operator-driven" | "operator" | "manual" | "human" => DispatchMode::OperatorDriven,
            _ => DispatchMode::Undeclared,
        }
    }
}

/// What was observed at one repository, at one instant.
///
/// Every field is a count or an elapsed duration the caller measured; this
/// module never measures anything, so the verdict stays arguable against the
/// numbers it was drawn from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoObservation {
    /// Issues open on the tracker.
    pub open_issues: usize,
    /// Specs staged and ready to be dispatched.
    pub staged_specs: usize,
    /// Agents currently running against the repository.
    pub agents_in_flight: usize,
    /// How long ago the last dispatch attempt was made, or `None` when no
    /// dispatch has ever been recorded.
    pub last_dispatch: Option<Duration>,
}

impl RepoObservation {
    /// Work this repository could hand to an agent right now. Open issues are
    /// not dispatchable — they have no spec — so only staged specs count.
    pub fn has_dispatchable_work(&self) -> bool {
        self.staged_specs > 0
    }

    /// The observation's own line: the numbers, without any verdict on them.
    pub fn line(&self) -> String {
        let last = match self.last_dispatch {
            Some(age) => format!("last dispatch {} ago", format_age(age)),
            None => "no dispatch ever recorded".to_string(),
        };
        format!(
            "open={} staged={} agents={} {}",
            self.open_issues, self.staged_specs, self.agents_in_flight, last
        )
    }
}

/// One repository as the fleet sees it: the served-list entry, its declared
/// dispatch mode, and whatever was observed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoState {
    /// Repository name, matching the served-list entry exactly.
    pub repo: String,
    /// The declared dispatch mode.
    pub dispatch: DispatchMode,
    /// Where the declaration is recorded — a runbook path, a cron owner, an
    /// issue number. Required for [`DispatchMode::OperatorDriven`].
    pub declared_in: String,
    /// The observation, or `None` when the report never reached this repo.
    pub observation: Option<RepoObservation>,
}

impl RepoState {
    /// A repository on the served list that was never observed.
    pub fn unobserved(repo: &str) -> Self {
        RepoState {
            repo: repo.to_string(),
            dispatch: DispatchMode::Undeclared,
            declared_in: String::new(),
            observation: None,
        }
    }

    /// The mode the report acts on. An operator-driven declaration that names
    /// no record is an assertion, not a decision on file, and degrades to
    /// [`DispatchMode::Undeclared`] — the same rule as a waiver marker without
    /// a reason.
    pub fn effective_mode(&self) -> DispatchMode {
        match self.dispatch {
            DispatchMode::OperatorDriven if self.declared_in.trim().is_empty() => {
                DispatchMode::Undeclared
            }
            mode => mode,
        }
    }
}

/// The stall interval and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetHealthPolicy {
    /// Silence longer than this, with dispatchable work and no agents, is a
    /// stall. Zero means "any gap at all stalls", which is a legitimate but
    /// noisy choice; the caller owns it.
    pub stall_after: Duration,
}

impl Default for FleetHealthPolicy {
    fn default() -> Self {
        FleetHealthPolicy {
            stall_after: DEFAULT_STALL_AFTER,
        }
    }
}

/// The verdict for one repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoHealth {
    /// Agents in flight — work is moving.
    Healthy,
    /// Dispatchable work, no agents, and a dispatch inside the stall window:
    /// the dispatcher is alive and its workers are simply full.
    DispatchedRecently,
    /// Dispatchable work, no agents, no dispatch within the stall window (or
    /// none ever). The pipeline is broken somewhere and nothing says so.
    Stalled,
    /// Dispatchable work, no agents, and the dispatcher is **declared**
    /// operator-driven: the work waits for a human by decision, not by
    /// accident. Never reported as stalled.
    AwaitingOperator,
    /// Nothing dispatchable — no staged specs. This is the only state that
    /// means "nothing to do".
    Idle,
    /// The report has no observation for this repository. Never collapsed into
    /// [`RepoHealth::Idle`]: not looking is not the same as nothing to do.
    Unobserved,
}

impl RepoHealth {
    /// The stable machine-readable state name.
    pub fn label(self) -> &'static str {
        match self {
            RepoHealth::Healthy => "HEALTHY",
            RepoHealth::DispatchedRecently => "DISPATCHING",
            RepoHealth::Stalled => "STALLED",
            RepoHealth::AwaitingOperator => "OPERATOR",
            RepoHealth::Idle => "IDLE",
            RepoHealth::Unobserved => "UNOBSERVED",
        }
    }

    /// The verdict sentence, with no numbers in it — the numbers are rendered
    /// beside it by [`RepoStatus::line`].
    pub fn describe(self) -> &'static str {
        match self {
            RepoHealth::Healthy => "agents running",
            RepoHealth::DispatchedRecently => "dispatched inside the stall window",
            RepoHealth::Stalled => "staged work, no agents, no dispatch: stalled",
            RepoHealth::AwaitingOperator => "staged work awaiting an operator (declared)",
            RepoHealth::Idle => "no staged specs: nothing to do",
            RepoHealth::Unobserved => "never observed: no verdict possible",
        }
    }

    /// Whether this state is the one the incident was: work present, nobody
    /// working, nobody scheduled to.
    pub fn is_stalled(self) -> bool {
        self == RepoHealth::Stalled
    }

    /// Whether a human should look at this repository. Idle is not actionable;
    /// neither is a healthy repo. Unobserved is, because the blind spot is the
    /// finding.
    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            RepoHealth::Stalled | RepoHealth::Unobserved | RepoHealth::AwaitingOperator
        )
    }
}

/// One repository's entry in the report: the verdict plus everything it was
/// derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoStatus {
    /// Repository name.
    pub repo: String,
    /// Mode the verdict was drawn under — the *effective* mode, so a blank
    /// declaration already reads as undeclared here.
    pub mode: DispatchMode,
    /// The verdict.
    pub health: RepoHealth,
    /// The observation the verdict came from, kept beside it (invariant 4).
    pub observation: Option<RepoObservation>,
}

impl RepoStatus {
    /// The report line: state, verdict, numbers, mode. A stalled repo also
    /// names whether its dispatch mode was ever declared, because the fix for
    /// "stalled and undeclared" may be "nobody installed the scheduler" or
    /// "nobody wrote down that a human does this by hand", and the operator
    /// needs to know which question to answer.
    pub fn line(&self) -> String {
        let mut line = format!(
            "{} [{}]: {}",
            self.repo,
            self.health.label(),
            self.health.describe()
        );
        if let Some(obs) = &self.observation {
            line.push_str(&format!(" ({})", obs.line()));
        }
        if self.mode == DispatchMode::Undeclared && self.health != RepoHealth::Unobserved {
            line.push_str(" (dispatch mode undeclared: scheduled or operator-driven?)");
        }
        line
    }
}

/// Assess one repository against the policy.
///
/// The order of the checks is the order of confidence: agents running is an
/// observation of work happening and outranks everything else; a declared
/// operator-driven dispatcher is a recorded decision and outranks the stall
/// test; no dispatchable work is the only genuine "nothing to do"; the stall
/// test runs last, on repos that have work, no agents, and no decision on file
/// excusing the gap.
pub fn assess(state: &RepoState, policy: &FleetHealthPolicy) -> RepoStatus {
    let mode = state.effective_mode();
    let Some(obs) = &state.observation else {
        return RepoStatus {
            repo: state.repo.clone(),
            mode,
            health: RepoHealth::Unobserved,
            observation: None,
        };
    };

    let health = if obs.agents_in_flight > 0 {
        RepoHealth::Healthy
    } else if mode == DispatchMode::OperatorDriven {
        // Declared: the absence of a schedule is the decision, so the gap
        // between dispatches is not evidence of anything. Still reported, so
        // the work waiting for a human stays visible.
        RepoHealth::AwaitingOperator
    } else if !obs.has_dispatchable_work() {
        RepoHealth::Idle
    } else {
        let silent_too_long = match obs.last_dispatch {
            // A repo with staged work that has never dispatched anything is
            // stalled at once: there is no schedule to wait for.
            None => true,
            Some(age) => age >= policy.stall_after,
        };
        if silent_too_long {
            RepoHealth::Stalled
        } else {
            RepoHealth::DispatchedRecently
        }
    };

    RepoStatus {
        repo: state.repo.clone(),
        mode,
        health,
        observation: Some(obs.clone()),
    }
}

/// The whole fleet: every repository the served list names, plus any
/// repository that was observed without being on that list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetReport {
    /// The interval the verdicts were drawn under, echoed so a reader can
    /// tell a stall from a merely quiet repo.
    pub stall_after: Duration,
    /// One entry per served repository, in the served list's order.
    pub repos: Vec<RepoStatus>,
    /// Repositories that were observed but appear on no served list — the
    /// blind spot that made the incident invisible, since a repo missing from
    /// the list is a repo the report would never have asked about.
    pub unserved: Vec<String>,
}

impl FleetReport {
    /// Build the report over every repository the fleet serves.
    ///
    /// `served` is authoritative for coverage: an entry with no matching
    /// [`RepoState`] becomes [`RepoHealth::Unobserved`] rather than being
    /// dropped. A [`RepoState`] naming a repository absent from `served` is
    /// recorded in [`FleetReport::unserved`] — reported, not silently
    /// evaluated into a list nobody agreed to cover.
    pub fn build(served: &[String], states: &[RepoState], policy: &FleetHealthPolicy) -> Self {
        let mut repos = Vec::with_capacity(served.len());
        for name in served {
            let state = states
                .iter()
                .find(|s| &s.repo == name)
                .cloned()
                .unwrap_or_else(|| RepoState::unobserved(name));
            repos.push(assess(&state, policy));
        }

        let served_set: BTreeSet<&str> = served.iter().map(String::as_str).collect();
        let unserved = states
            .iter()
            .filter(|s| !served_set.contains(s.repo.as_str()))
            .map(|s| s.repo.clone())
            .collect();

        FleetReport {
            stall_after: policy.stall_after,
            repos,
            unserved,
        }
    }

    /// Repositories in the stalled state.
    pub fn stalled(&self) -> Vec<&RepoStatus> {
        self.repos
            .iter()
            .filter(|r| r.health == RepoHealth::Stalled)
            .collect()
    }

    /// Repositories the report could not observe.
    pub fn unobserved(&self) -> Vec<&RepoStatus> {
        self.repos
            .iter()
            .filter(|r| r.health == RepoHealth::Unobserved)
            .collect()
    }

    /// Repositories whose dispatch mode nobody has declared. These may all be
    /// healthy right now; the gap is that nobody wrote down what drives them,
    /// so their silence means nothing either way.
    pub fn undeclared_dispatchers(&self) -> Vec<&RepoStatus> {
        self.repos
            .iter()
            .filter(|r| r.mode == DispatchMode::Undeclared)
            .collect()
    }

    /// Count of entries in one state, used by the summary.
    fn count(&self, health: RepoHealth) -> usize {
        self.repos.iter().filter(|r| r.health == health).count()
    }

    /// The one-line summary. The stall count leads: this line is read to
    /// decide whether anything needs attention, and `idle` is the answer that
    /// must never be the one a stalled fleet gets.
    pub fn summary_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for health in [
            RepoHealth::Stalled,
            RepoHealth::Unobserved,
            RepoHealth::AwaitingOperator,
            RepoHealth::DispatchedRecently,
            RepoHealth::Healthy,
            RepoHealth::Idle,
        ] {
            let n = self.count(health);
            if n > 0 {
                parts.push(format!("{n} {}", health.label().to_ascii_lowercase()));
            }
        }
        let mut line = format!(
            "{} repos in {} stall window: {}",
            self.repos.len(),
            format_age(self.stall_after),
            parts.join(", ")
        );
        if !self.unserved.is_empty() {
            line.push_str(&format!(
                "; {} observed but on no served list: {}",
                self.unserved.len(),
                self.unserved.join(",")
            ));
        }
        let undeclared = self.undeclared_dispatchers().len();
        if undeclared > 0 {
            line.push_str(&format!("; {undeclared} dispatch mode undeclared"));
        }
        line
    }

    /// The full report: every repository's line, then the summary.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for status in &self.repos {
            out.push_str(&status.line());
            out.push('\n');
        }
        out.push_str(&self.summary_line());
        out.push('\n');
        out
    }
}
