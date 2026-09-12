//! Never report a cause you have not established, and log every destructive
//! action with its origin (issue #4423).
//!
//! Five workers were cancelled simultaneously and the gateway recorded all
//! five as **crashed** — `"worker crashed (no walltime deadline known)"` on
//! a job whose `sacct` row said `CANCELLED by 298907`. They were not
//! crashes: a cancellation is the fleet (or an operator) doing something
//! deliberate, a crash is a fault to investigate, and the two call for
//! opposite responses. The gateway asserted the stronger claim while
//! admitting it had no evidence for it.
//!
//! The contract this module makes checkable:
//!
//! 1. **A vanished worker is classified from scheduler terminal evidence,
//!    never from absence.** [`classify`] takes the `State` field of
//!    `sacct -j <id> --format=State,ExitCode` and returns a
//!    [`Disposition`]. `CANCELLED` is a deliberate cancellation
//!    ([`Disposition::Cancelled`], carrying the cancelling uid when the
//!    reason records one), `COMPLETED` is the walltime deadline reached
//!    normally, and a fault state (`FAILED`, `OUT_OF_MEMORY`, `NODE_FAIL`,
//!    `BOOT_FAIL`, `TIMEOUT`) is a real [`Disposition::Crashed`]. When
//!    `sacct` is unavailable or the state does not parse
//!    ([`Disposition::Unknown`]) the message says *"worker disappeared;
//!    cause unknown"* — it never names a cause it has not established. A
//!    cancelled worker and a crashed worker stay separate, so the log can
//!    support the opposite responses each demands.
//! 2. **Every cancellation logs who/what/why before the call.** A
//!    [`CancelNotice`] records the origin (the script issuing it), the job
//!    it cancels, and the reason, and [`CancelNotice::log_line`] renders a
//!    single line to the shared cancellation file — so a five-worker loss
//!    is attributable after the fact instead of surfacing only in an
//!    unrelated error scan. The destructive action is recorded before it
//!    is issued, never after.
//! 3. **The telemetry distinguishes cancelled from crashed.**
//!    [`DispositionCounters`] keeps `cancelled`, `crashed`, `completed`
//!    and `unknown` apart and renders them on one line, so the difference
//!    between a deliberate fleet action and a fault is visible without
//!    reading logs.

use std::fmt;

/// Terminal scheduler states that mean the job *faulted*: a crash to
/// investigate, the opposite of a deliberate cancellation.
const CRASH_STATES: &[&str] = &[
    "FAILED",
    "OUT_OF_MEMORY",
    "NODE_FAIL",
    "BOOT_FAIL",
    "TIMEOUT",
];

/// How a vanished worker's job ended, established from scheduler terminal
/// evidence rather than from the worker's absence alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// The job was cancelled deliberately (`sacct State=CANCELLED`): the
    /// fleet or an operator did something, not a fault to investigate.
    Cancelled {
        /// The cancelling uid when the state records one (e.g. the `298907`
        /// in `CANCELLED by 298907`), otherwise `None`.
        by_user: Option<String>,
    },
    /// The job reached its walltime deadline normally (`COMPLETED`).
    Completed,
    /// The job actually faulted (`FAILED`, `OUT_OF_MEMORY`, `NODE_FAIL`,
    /// `BOOT_FAIL`, `TIMEOUT`): a fault to investigate.
    Crashed,
    /// No terminal evidence available (`sacct` unavailable, no row, or a
    /// state that does not classify): the worker disappeared but the cause
    /// was not established. This is never reported as a crash.
    Unknown,
}

impl fmt::Display for Disposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled { by_user } => match by_user {
                Some(uid) => write!(f, "worker cancelled (by {uid})"),
                None => f.write_str("worker cancelled"),
            },
            Self::Completed => f.write_str("worker completed (walltime reached)"),
            Self::Crashed => f.write_str("worker crashed"),
            // The exact wording the issue demands when sacct cannot answer:
            // name no cause that has not been established.
            Self::Unknown => f.write_str("worker disappeared; cause unknown"),
        }
    }
}

/// Classify a vanished worker from its scheduler terminal state.
///
/// `state` is the `State` field of `sacct -j <id> --format=State,ExitCode`
/// (for example `CANCELLED`, `CANCELLED by 298907`, `COMPLETED`, `FAILED`).
/// `None` — `sacct` unavailable, no row, or the caller chose not to query —
/// is fail-closed [`Disposition::Unknown`]: the cause is not established,
/// so no cause is reported. This is the invariant the gateway in issue
/// #4423 violated when it logged `"worker crashed (no walltime deadline
/// known)"` for a job `sacct` said was `CANCELLED`.
pub fn classify(state: Option<&str>) -> Disposition {
    let Some(state) = state else {
        return Disposition::Unknown;
    };
    let state = state.trim();
    if state.is_empty() {
        return Disposition::Unknown;
    }

    // CANCELLED, optionally with a reason suffix: `CANCELLED by 298907`.
    if let Some(rest) = state.strip_prefix("CANCELLED") {
        let rest = rest.trim();
        let by_user = rest
            .strip_prefix("by")
            .map(|uid| uid.trim().to_owned())
            .filter(|uid| !uid.is_empty());
        return Disposition::Cancelled { by_user };
    }

    if state.eq_ignore_ascii_case("COMPLETED") {
        return Disposition::Completed;
    }

    if CRASH_STATES
        .iter()
        .any(|crash| state.eq_ignore_ascii_case(crash))
    {
        return Disposition::Crashed;
    }

    // A state we do not recognise is not evidence of a crash; report the
    // cause as unestablished rather than guessing.
    Disposition::Unknown
}

/// A worker cancellation to be issued, recorded with its origin before the
/// call.
///
/// Every cancel site logs who/what/why to the shared cancellation file
/// before issuing `scancel` — the ask that makes a five-worker loss
/// attributable after the fact instead of a mystery resolved by an
/// unrelated error scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelNotice {
    /// The scheduler job id being cancelled (`what`).
    pub job_id: u64,
    /// The script (or component) issuing the cancellation (`who`).
    pub origin: String,
    /// Why the worker is being cancelled (`why`).
    pub reason: String,
}

impl CancelNotice {
    /// The single log line written to the shared cancellation file before
    /// `scancel` is issued. It names the origin, the job and the reason, so
    /// a later reader can attribute the action without guessing.
    pub fn log_line(&self) -> String {
        format!(
            "scancel: {} cancels job {} — {}",
            self.origin, self.job_id, self.reason
        )
    }
}

/// Per-worker disposition counters, keeping `cancelled` apart from
/// `crashed` so the difference is visible without reading logs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispositionCounters {
    /// Deliberately cancelled workers (`CANCELLED`): fleet/operator action.
    pub cancelled: u64,
    /// Workers that actually faulted (`FAILED`/`OUT_OF_MEMORY`/...): faults
    /// to investigate.
    pub crashed: u64,
    /// Workers that reached their walltime normally (`COMPLETED`).
    pub completed: u64,
    /// Workers whose cause was not established (`sacct` unavailable).
    pub unknown: u64,
}

impl DispositionCounters {
    /// Record one worker's disposition into the matching counter.
    pub fn record(&mut self, disposition: &Disposition) {
        match disposition {
            Disposition::Cancelled { .. } => self.cancelled += 1,
            Disposition::Crashed => self.crashed += 1,
            Disposition::Completed => self.completed += 1,
            Disposition::Unknown => self.unknown += 1,
        }
    }

    /// One telemetry line distinguishing the counts.
    pub fn line(&self) -> String {
        format!(
            "workers: cancelled={} crashed={} completed={} unknown={}",
            self.cancelled, self.crashed, self.completed, self.unknown
        )
    }
}
