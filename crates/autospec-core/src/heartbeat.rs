//! Heartbeat liveness for periodic autospec processes (issue #3995).
//!
//! A periodic process proves it ran by writing a heartbeat line to its own
//! log on every pass — the timestamp of the pass and its outcome, whether or
//! not the pass did any work. Liveness is judged from the newest heartbeat
//! (or from the scheduler entry that drives the process), never from the
//! mtime of a log file. The mtime is what a writer leaves behind, not what
//! the process asserts, and it lies in both directions at once:
//!
//! * A pass that ran and did nothing still refreshes the file it touches —
//!   and the log an operator is watching may not be the log the process
//!   writes at all (the cron wrapper's `cron-topup.log` catches the
//!   wrapper's empty stdout while the dispatcher writes `topup.log`), so a
//!   "stale" mtime can indict a healthy process.
//! * A repaired script keeps its old log — whose last line is the old syntax
//!   error — so recency alone cannot distinguish "fixed" from "broken".
//!
//! The rules this module encodes:
//!
//! * the heartbeat line is the process's own word; a pass that ran and did
//!   nothing must still beat, with an outcome that says so;
//! * the log path a check reads is verified against the path the process
//!   declares — a mismatch is a **broken check**, not a dead process, and no
//!   statement about the process's liveness may be drawn from it;
//! * a check may say "no heartbeat since T"; it may never say "dead".
//!
//! Everything here is pure and testable: no I/O, no clock, no subprocesses —
//! except [`write_heartbeat`], which appends exactly one line. The caller
//! supplies `now` and the lines it read.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Prefix marking a log line as a heartbeat.
pub const HEARTBEAT_PREFIX: &str = "heartbeat:";

/// How many intervals a process may go without a heartbeat before it is
/// reported stale. Silence within this window is explainable (the next pass
/// is scheduled); silence past it is reported as silence — never as death.
pub const DEFAULT_STALE_AFTER_INTERVALS: u64 = 2;

/// Format UTC epoch seconds as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Second precision, UTC — the shape `date -u +%Y-%m-%dT%H:%M:%SZ` emits and
/// the shape shell heartbeat lines carry.
pub fn format_utc(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let rem = (epoch_secs % 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Parse `YYYY-MM-DDTHH:MM:SSZ` as UTC epoch seconds.
///
/// Accepts exactly second-precision UTC stamps; offsets, fractional seconds,
/// and non-leap February 29ths are rejected.
pub fn parse_utc(stamp: &str) -> Option<u64> {
    let b = stamp.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let year = dec4(b, 0)? as i64;
    let month = dec2(b, 5)?;
    let day = dec2(b, 8)?;
    let hour = dec2(b, 11)?;
    let minute = dec2(b, 14)?;
    let second = dec2(b, 17)?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_civil_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let secs = days_from_civil(year, month, day) * 86_400
        + hour as i64 * 3_600
        + minute as i64 * 60
        + second as i64;
    (secs >= 0).then_some(secs as u64)
}

fn dec2(b: &[u8], i: usize) -> Option<u32> {
    Some((b[i] as char).to_digit(10)? * 10 + (b[i + 1] as char).to_digit(10)?)
}

fn dec4(b: &[u8], i: usize) -> Option<u32> {
    Some(
        (b[i] as char).to_digit(10)? * 1000
            + (b[i + 1] as char).to_digit(10)? * 100
            + (b[i + 2] as char).to_digit(10)? * 10
            + (b[i + 3] as char).to_digit(10)?,
    )
}

fn days_in_civil_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days since 1970-01-01 -> (year, month, day) in the proleptic Gregorian
/// calendar.
fn civil_from_days(z0: i64) -> (i64, u32, u32) {
    let z = z0 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// (year, month, day) in the proleptic Gregorian calendar -> days since
/// 1970-01-01.
fn days_from_civil(year0: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year0 - 1 } else { year0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400; // [0, 399]
    let doy =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) as i64 + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// One heartbeat line, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    /// Step name as written in the line.
    pub step: String,
    /// When the pass ran, UTC epoch seconds.
    pub at: u64,
    /// Outcome of the pass (e.g. `"dispatched 3"`, `"nothing to do"`);
    /// empty when the writer recorded none.
    pub outcome: String,
}

impl Heartbeat {
    /// The line this heartbeat is recorded as. Matches the shell library's
    /// shape (`heartbeat: <step> <UTC timestamp>`), with the outcome appended
    /// when present.
    pub fn line(&self) -> String {
        if self.outcome.is_empty() {
            format!("{} {} {}", HEARTBEAT_PREFIX, self.step, format_utc(self.at))
        } else {
            format!(
                "{} {} {} {}",
                HEARTBEAT_PREFIX,
                self.step,
                format_utc(self.at),
                self.outcome
            )
        }
    }
}

/// Parse one line of a process log.
///
/// Returns `None` for any line that is not a well-formed heartbeat with a
/// well-formed UTC stamp. Accepts the shell shape
/// `heartbeat: <step> <stamp>` and the outcome shape
/// `heartbeat: <step> <stamp> <outcome...>`.
pub fn parse_heartbeat(line: &str) -> Option<Heartbeat> {
    let rest = line.trim_start().strip_prefix(HEARTBEAT_PREFIX)?;
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    let step = tokens[0];
    if step.is_empty() || step.starts_with(' ') {
        return None;
    }
    let at = parse_utc(tokens[1])?;
    let outcome = tokens[2..].join(" ");
    Some(Heartbeat {
        step: step.to_string(),
        at,
        outcome,
    })
}

/// The newest heartbeat for `step` among these lines, if any.
pub fn latest_heartbeat(log_lines: &[&str], step: &str) -> Option<Heartbeat> {
    log_lines
        .iter()
        .filter_map(|line| parse_heartbeat(line))
        .filter(|hb| hb.step == step)
        .max_by_key(|hb| hb.at)
}

/// A verdict about a process drawn from its heartbeats.
///
/// The wording is load-bearing: silence is reported as silence
/// ("no heartbeat since T"), never as death.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Liveness {
    /// The process beat within the silence window. The pass outcome is
    /// irrelevant: a pass that ran and did nothing is still liveness.
    Live { silent_secs: u64 },
    /// The last beat is outside the silence window. Report it as
    /// "no heartbeat since <at>" — the check observed silence, not death.
    Stale { silent_secs: u64, last: Heartbeat },
    /// The log holds no heartbeat at all. Report it as "no heartbeat on
    /// record" — the process may be running and simply not yet logged.
    NoRecord,
}

impl Liveness {
    /// The one sentence a report is allowed to say. This is the most a check
    /// may claim about the process; "dead" is not in the vocabulary.
    pub fn describe(&self) -> String {
        match self {
            Liveness::Live { silent_secs } => format!("live: heartbeat {silent_secs}s ago"),
            Liveness::Stale { silent_secs, last } => format!(
                "no heartbeat since {} ({silent_secs}s of silence)",
                format_utc(last.at)
            ),
            Liveness::NoRecord => "no heartbeat on record".to_string(),
        }
    }
}

/// Judge a process's liveness from the heartbeat lines of its own log.
///
/// `now_secs` is supplied by the caller — this module never reads the clock
/// and never the file mtime. A beat stamped in the future is a clock
/// anomaly, but it can never make a process look stale.
pub fn assess_liveness(
    step: &str,
    log_lines: &[&str],
    interval_secs: u64,
    now_secs: u64,
) -> Liveness {
    let last = match latest_heartbeat(log_lines, step) {
        Some(last) => last,
        None => return Liveness::NoRecord,
    };
    let silent = now_secs.saturating_sub(last.at);
    if silent <= interval_secs.saturating_mul(DEFAULT_STALE_AFTER_INTERVALS) {
        Liveness::Live {
            silent_secs: silent,
        }
    } else {
        Liveness::Stale {
            silent_secs: silent,
            last,
        }
    }
}

/// A health check over a periodic process's log.
#[derive(Debug, Clone)]
pub struct LogHealthCheck {
    /// Step name as it appears in the process's heartbeat lines.
    pub step: String,
    /// The log path the process declares as its own — from its topology
    /// declaration or its script, not from this check.
    pub process_log: PathBuf,
    /// The interval the process is scheduled at.
    pub interval_secs: u64,
}

/// What a health check concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthVerdict {
    /// The check read the process's own log and judged liveness from the
    /// heartbeats in it.
    Process(Liveness),
    /// The check read a path other than the one the process writes to.
    /// That is a broken check, not a dead process: no statement about the
    /// process's liveness may be drawn from this verdict.
    BrokenCheck {
        expected: PathBuf,
        observed: PathBuf,
    },
}

impl LogHealthCheck {
    /// Judge from the lines the caller already read from `observed_log`, at
    /// the caller-supplied `now_secs`.
    ///
    /// The path is verified before anything else, because a reading taken
    /// from the wrong file is not evidence about the process at all —
    /// neither its silence nor its activity.
    pub fn verdict(&self, observed_log: &Path, log_lines: &[&str], now_secs: u64) -> HealthVerdict {
        if observed_log != self.process_log.as_path() {
            return HealthVerdict::BrokenCheck {
                expected: self.process_log.clone(),
                observed: observed_log.to_path_buf(),
            };
        }
        HealthVerdict::Process(assess_liveness(
            &self.step,
            log_lines,
            self.interval_secs,
            now_secs,
        ))
    }
}

/// Append a heartbeat line to the process's own log — the pass's proof that
/// it ran.
///
/// The line records the step, the UTC timestamp, and the outcome of the
/// pass. A pass that did nothing must still call this, with an outcome that
/// says so (`"nothing to do"`): ran-and-did-nothing is still ran.
pub fn write_heartbeat(
    log_path: &Path,
    step: &str,
    at: SystemTime,
    outcome: &str,
) -> io::Result<()> {
    let at_secs = at
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "heartbeat timestamp is before the unix epoch",
            )
        })?
        .as_secs();
    if let Some(parent) = log_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let line = Heartbeat {
        step: step.to_string(),
        at: at_secs,
        outcome: outcome.to_string(),
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    file.write_all(format!("{}\n", line.line()).as_bytes())
}
