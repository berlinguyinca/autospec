//! A log's last line is only "now" if its mtime says so (issue #4246).
//!
//! The incident: sweeping cron logs for `error|fail`, the sweep returned
//! two lines — a bash syntax error in `regsweep.sh`, the `*/5` cron job
//! that registers worker endpoints with the gateway. The conclusion was
//! immediate and wrong: registrations lapse, workers lost, the reported
//! `503 no worker for model`. The file's mtime was **three days old**;
//! `bash -n` reported the script valid; the component's own log
//! (`regsweep.log`) had been written **two minutes earlier** and ended
//! `sweep done: pool=10 gateway=10`; the gateway reported 10 registered
//! workers. The script had been briefly broken during an edit; cron
//! logged it once; `cron-regsweep.log` was never written to again. Its
//! last line looked exactly like current state.
//!
//! The rule: **a log's last line is only "now" if its mtime says so.**
//! `tail` answers "what was written last", which is a different question
//! from "what is happening" — and the two diverge precisely when a
//! component has *stopped*, which is the case you are usually
//! investigating.
//!
//! The four rules this module makes checkable:
//!
//! 1. **Read mtime with every tail.** [`SweepHit`] cannot be constructed
//!    without the file's mtime — a match without its age is not
//!    evidence. [`freshness`] decides whether that mtime is inside the
//!    investigation window, and [`hit_line`] renders every match with
//!    the age alongside, plainly saying when a match comes from a file
//!    older than the window.
//! 2. **Prefer the live check to the log.** State beats history whenever
//!    state is queryable: [`outage_grounding`] refuses an outage claim
//!    grounded in a log when the live query exists, and decides the
//!    claim from the live query when it was taken.
//! 3. **Two logs for one component is a trap.** A supervisor's error
//!    log is silent on success, and therefore always shows the last
//!    failure — arbitrarily far in the past — as though it were the
//!    present. [`read_last_line`] decides whether a file's last line is
//!    evidence about now or about its mtime; [`authoritative_log`]
//!    picks, among a component's logs, the one that is the present —
//!    the fresh one, or none.
//! 4. **A stale error file should not survive its cause.** An
//!    error-only log needs per-line timestamps or rotation;
//!    [`error_log_standing`] names the failure mode it degrades to
//!    without them — [`ErrorLogStanding::Monument`], a permanent
//!    monument to one bad afternoon.
//!
//! Everything here is pure: no I/O, no clock. The caller supplies
//! `now`, the mtimes, and the matched lines; this module decides what
//! they mean.

use std::time::Duration;

use crate::stored_output::format_age;

// ── Rule 1: mtime is part of the evidence ─────────────────────────────────

/// What a log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogRole {
    /// The component's own log: records success and failure alike.
    Operational,
    /// A supervisor's error log: appended only on failure. Silent on
    /// success — its last entry is always the last failure, however far
    /// in the past (rule 3).
    ErrorOnly,
}

/// A log file as an error sweep sees it. `mtime_secs` is not optional:
/// a match without the file's last-write time is not evidence (rule 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    pub path: String,
    /// Last write, epoch seconds, as the sweep's host reported it.
    pub mtime_secs: u64,
    pub role: LogRole,
}

/// Whether a file's last write is inside the investigation window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Last written inside the window: the file's content is evidence
    /// about now.
    Fresh,
    /// Last written outside the window: the file's content is evidence
    /// about its mtime, and the silence since is the file's current
    /// statement.
    Stale,
}

/// Age in seconds. A clock that rewinds (mtime after `now`) is zero
/// age, never an underflow.
pub fn age_secs(now: u64, mtime: u64) -> u64 {
    now.saturating_sub(mtime)
}

/// Is a file last written at `mtime` still "now", at `now`, for a
/// window of `window_secs`?
///
/// Exactly at the window edge is not *older than* the window, so it is
/// still [`Freshness::Fresh`].
pub fn freshness(now: u64, mtime: u64, window_secs: u64) -> Freshness {
    if age_secs(now, mtime) > window_secs {
        Freshness::Stale
    } else {
        Freshness::Fresh
    }
}

/// One matched line from an error sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepHit {
    pub file: LogFile,
    /// The matched line, verbatim.
    pub line: String,
}

/// The detection rule, rendered: the file's mtime alongside every
/// match, and a plain `STALE` flag when the match comes from a file
/// older than the investigation window.
pub fn hit_line(hit: &SweepHit, now: u64, window_secs: u64) -> String {
    let age = age_secs(now, hit.file.mtime_secs);
    match freshness(now, hit.file.mtime_secs, window_secs) {
        Freshness::Fresh => format!(
            "[fresh, last written {} ago] {}: {}",
            format_age(Duration::from_secs(age)),
            hit.file.path,
            hit.line
        ),
        Freshness::Stale => format!(
            "[STALE, last written {} ago — outside the {} investigation window] {}: {}",
            format_age(Duration::from_secs(age)),
            format_age(Duration::from_secs(window_secs)),
            hit.file.path,
            hit.line
        ),
    }
}

/// What a whole sweep's matches say about the present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepVerdict {
    /// No matches. Nothing to declare — and for a component that may
    /// have stopped, zero error matches is not evidence of health
    /// either.
    Empty,
    /// Every matched file was written inside the window.
    AllFresh,
    /// Some matches are from inside the window, some from outside.
    /// The lines must say which is which ([`hit_line`]).
    Mixed,
    /// Every matched file is outside the window: the sweep is evidence
    /// about the past, not the present.
    StaleOnly,
}

pub fn sweep_verdict(hits: &[SweepHit], now: u64, window_secs: u64) -> SweepVerdict {
    if hits.is_empty() {
        return SweepVerdict::Empty;
    }
    let stale = hits
        .iter()
        .filter(|h| freshness(now, h.file.mtime_secs, window_secs) == Freshness::Stale)
        .count();
    if stale == 0 {
        SweepVerdict::AllFresh
    } else if stale == hits.len() {
        SweepVerdict::StaleOnly
    } else {
        SweepVerdict::Mixed
    }
}

/// The sweep's report: one [`hit_line`] per match, then the verdict —
/// saying plainly when every match comes from a file older than the
/// investigation window.
pub fn sweep_report(hits: &[SweepHit], now: u64, window_secs: u64) -> String {
    let mut out = hits
        .iter()
        .map(|h| hit_line(h, now, window_secs))
        .collect::<Vec<_>>()
        .join("\n");
    match sweep_verdict(hits, now, window_secs) {
        SweepVerdict::Empty => out.push_str("no matches"),
        SweepVerdict::AllFresh => {
            out.push_str("\nall matches from files written inside the window")
        }
        SweepVerdict::Mixed => {
            out.push_str("\nmixed: STALE-flagged matches are about the past, not the present")
        }
        SweepVerdict::StaleOnly => out.push_str(
            "\nSTALE ONLY: every match is from a file older than the \
investigation window — this is evidence about the past; query the live \
state before declaring anything",
        ),
    }
    out
}

// ── Rule 2: state beats history ──────────────────────────────────────────

/// A direct query of a component's current state — the gateway's
/// `/v1/workers`, `squeue`, a health endpoint. The 15-second answer the
/// incident had available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveState {
    /// What was queried, e.g. `"gateway /v1/workers"`.
    pub source: String,
    /// What it observed, e.g. `"10 registered, all four models present"`.
    pub detail: String,
    /// Whether the observed state is healthy.
    pub healthy: bool,
}

/// What may ground a claim that "the component is broken now".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutageGrounding {
    /// A live query answered; it decides the claim, and the log is
    /// history. `supports` says whether what it observed supports the
    /// claim.
    Live { detail: String, supports: bool },
    /// No live query, but state is queryable: the log-grounded claim is
    /// refused — state beats history whenever state is queryable.
    RefusedStateQueryable,
    /// No live query, state not queryable, and the log is inside the
    /// window: the best available evidence.
    FreshLog,
    /// No live query, state not queryable, and the log is outside the
    /// window: the claim is evidence about the log's mtime.
    RefusedStale { age_secs: u64 },
}

/// Invariant 2: the grounding a "the component is broken now" claim may
/// rest on, given the age of the log the claim was read from and
/// whether a live state query exists or was taken.
pub fn outage_grounding(
    log_age: u64,
    window_secs: u64,
    state_queryable: bool,
    live: Option<&LiveState>,
) -> OutageGrounding {
    if let Some(l) = live {
        return OutageGrounding::Live {
            detail: format!("{}: {}", l.source, l.detail),
            supports: !l.healthy,
        };
    }
    if state_queryable {
        return OutageGrounding::RefusedStateQueryable;
    }
    if log_age <= window_secs {
        OutageGrounding::FreshLog
    } else {
        OutageGrounding::RefusedStale { age_secs: log_age }
    }
}

// ── Rule 3: two logs for one component is a trap ─────────────────────────

/// Whether a file's last line is evidence about now or about its mtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastLineReading {
    /// Last written inside the window: the last line is the present.
    Now,
    /// Last written outside the window: the last line is the past. For
    /// an [`LogRole::ErrorOnly`] file the silence since is "no new
    /// failures" — not "the last failure is current".
    History { age_secs: u64 },
}

pub fn read_last_line(file: &LogFile, now: u64, window_secs: u64) -> LastLineReading {
    match freshness(now, file.mtime_secs, window_secs) {
        Freshness::Fresh => LastLineReading::Now,
        Freshness::Stale => LastLineReading::History {
            age_secs: age_secs(now, file.mtime_secs),
        },
    }
}

/// Which of a component's logs is the present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoritativeLog {
    /// A log written inside the window exists; it is the present.
    Fresh(String),
    /// Every log for the component is outside the window (or there is
    /// none): the component is not writing, and no log of its says what
    /// is happening. Query the live state.
    NoneFresh { oldest_age_secs: u64 },
}

/// Invariant 3: among a component's logs, the one that answers "what is
/// happening" is the freshest one — or none, in which case the answer
/// is not in any of them.
pub fn authoritative_log(logs: &[LogFile], now: u64, window_secs: u64) -> AuthoritativeLog {
    let mut oldest = 0;
    let mut seen = false;
    for file in logs {
        let age = age_secs(now, file.mtime_secs);
        if freshness(now, file.mtime_secs, window_secs) == Freshness::Fresh {
            return AuthoritativeLog::Fresh(file.path.clone());
        }
        if !seen || age > oldest {
            oldest = age;
            seen = true;
        }
    }
    AuthoritativeLog::NoneFresh {
        oldest_age_secs: oldest,
    }
}

// ── Rule 4: a stale error file should not survive its cause ──────────────

/// Safeguards an error-only log can carry so a stale entry cannot read
/// as the present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorLogHygiene {
    /// Every line carries its own timestamp, so a stale entry can be
    /// dated.
    pub per_line_timestamps: bool,
    /// The file is rotated (or archived), so it cannot outlive its
    /// cause.
    pub rotates: bool,
}

/// What a supervisor error log is, right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorLogStanding {
    /// The last failure is inside the window: the log is the present.
    RecentFailure,
    /// The last failure is outside the window, but a safeguard holds:
    /// the entry is dated or bounded.
    Dated,
    /// The last failure is outside the window with neither per-line
    /// timestamps nor rotation: a permanent monument to one bad
    /// afternoon — it presents an arbitrarily old failure as the present
    /// forever.
    Monument,
}

pub fn error_log_standing(
    file: &LogFile,
    hygiene: &ErrorLogHygiene,
    now: u64,
    window_secs: u64,
) -> ErrorLogStanding {
    match freshness(now, file.mtime_secs, window_secs) {
        Freshness::Fresh => ErrorLogStanding::RecentFailure,
        Freshness::Stale => {
            if hygiene.per_line_timestamps || hygiene.rotates {
                ErrorLogStanding::Dated
            } else {
                ErrorLogStanding::Monument
            }
        }
    }
}

/// The finding for an [`ErrorLogStanding::Monument`]: what the file is
/// doing and what would stop it. `None` for every other standing.
pub fn monument_finding(
    file: &LogFile,
    hygiene: &ErrorLogHygiene,
    now: u64,
    window_secs: u64,
) -> Option<String> {
    if error_log_standing(file, hygiene, now, window_secs) != ErrorLogStanding::Monument {
        return None;
    }
    let age = age_secs(now, file.mtime_secs);
    Some(format!(
        "{}: error-only log last written {} ago with no per-line timestamps \
and no rotation — it presents that failure as the present; timestamp every \
line, rotate the file, or archive it when its cause clears",
        file.path,
        format_age(Duration::from_secs(age))
    ))
}
