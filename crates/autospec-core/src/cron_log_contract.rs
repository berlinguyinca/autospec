//! One log per scheduled job, named by the schedule (issue #4293).
//!
//! The incident: fleet scripts were scheduled like
//!
//! ```text
//! */10 * * * * /bin/bash $L/topup.sh >> $L/logs/cron-topup.log 2>&1
//! ```
//!
//! while each script set its own destination internally
//! (`LOG="$L/logs/topup.log"`). The `cron-*.log` files therefore held
//! exactly what a healthy job never writes through that path — stray
//! wrapper output: 0 lines, mtime days old. The empty logs at the paths an
//! operator checks read as "systemic failure", and the natural next action
//! (restart the fleet, re-add the cron entries) would have been destructive
//! on a healthy one. What exonerated the fleet was incidental: the
//! lockfile mtimes. (Second time this shape has nearly caused a false
//! outage: #4246 was a real log whose writer stopped; here the writer is
//! fine and the log is a decoy.)
//!
//! The fix is in the supervisor, not in the bash layout. Four invariants,
//! one primitive each:
//!
//! 1. **A scheduled job writes to exactly one log, and the schedule names
//!    it.** Either the script owns the path and the cron line has no file
//!    redirect, or the cron line owns the redirect and the script sets no
//!    `LOG`. Two *different* candidate destinations is the incident: the
//!    observer picks the one the schedule names, which is the one that
//!    reads stale while the job is healthy. [`audit_destination`]
//!    classifies the pair and names both paths.
//! 2. **A decoy log is worse than no log.** An empty file at the path the
//!    operator will check asserts "ran, said nothing". A redirect that is
//!    not the job's own log must therefore be named `<job>.stderr`, so its
//!    emptiness reads as good news: [`stderr_name`],
//!    [`stray_redirect_name`].
//! 3. **Every periodic job emits one heartbeat line per run — including
//!    no-op runs.** The no-op run skips work-conditional code and runs the
//!    main flow, so the guarantee is a heartbeat call *on the main flow*,
//!    not one in each branch: [`scan_heartbeat_coverage`]. (The line
//!    format and the liveness judgment from it are [`crate::heartbeat`];
//!    this is the script-side half: is there a beat the no-op run cannot
//!    skip?)
//! 4. **Liveness is checkable without reading any log at all** — not log
//!    mtimes, not lockfile mtimes (the lockfile worked by accident, not
//!    design). The job records each run in durable state and the status
//!    command renders that: [`JobRunState`], [`JobStatus`]. The wording
//!    keeps [`crate::heartbeat`]'s discipline: silence is reported as
//!    silence, never as death.
//!
//! Everything here is pure and deterministic — no clock, no file reads —
//! except [`JobStatus::to_json`] / [`JobStatus::from_json`], which are the
//! persistence boundary the caller invokes. The shell scanners are
//! token-level by design (the same family as the other autospec ratchets):
//! they do not interpret the script, they count what is there.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::heartbeat::{format_utc, DEFAULT_STALE_AFTER_INTERVALS};

// ── Invariant 1: one log per job, named by the schedule ──────────────────

/// Which process stream a cron redirect writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stream {
    /// Stdout only (`>`, `>>`).
    Stdout,
    /// Stderr only (`2>`, `2>>`).
    Stderr,
    /// Both streams (`&>`, `&>>`, or stdout plus a `2>&1` dup onto it).
    Both,
}

/// One file destination a cron command line names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronRedirect {
    /// The redirect target as written (unexpanded — `$L/logs/cron-topup.log`).
    pub target: String,
    /// Which stream(s) land there.
    pub stream: Stream,
}

/// A cron command with its file redirects separated from the command itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CronCommand {
    /// The command with every redirect token removed, single-spaced.
    pub command: String,
    /// The file destinations, stdout first, in order.
    pub redirects: Vec<CronRedirect>,
}

impl CronCommand {
    /// The distinct file destinations this line names, in order.
    pub fn destinations(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in &self.redirects {
            if !out.iter().any(|t| t == &r.target) {
                out.push(r.target.clone());
            }
        }
        out
    }

    /// The log destinations this line names: the targets of stdout or
    /// both-stream redirects, in order.
    ///
    /// A stderr-only redirect is not a log destination — its file is
    /// diagnostic, empty when healthy, and named for that
    /// ([`stray_redirect_name`]). The incident's `>> cron-topup.log 2>&1`
    /// is a log destination; a corrected `2>> topup.stderr` is not.
    pub fn log_destinations(&self) -> Vec<String> {
        self.redirects
            .iter()
            .filter(|r| r.stream != Stream::Stderr)
            .map(|r| r.target.clone())
            .collect()
    }

    /// The command with no file redirect — the fix for a decoy where the
    /// script owns its log: the schedule should not name one.
    pub fn without_redirects(&self) -> String {
        self.command.clone()
    }
}

/// A full cron line: the 5-field schedule plus the command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronLine {
    /// The schedule as written (e.g. `*/10 * * * *`).
    pub schedule: String,
    /// The command and its redirects.
    pub command: CronCommand,
}

impl CronLine {
    /// Split a cron line into its 5 schedule fields and its command.
    ///
    /// Returns `None` for a line with fewer than 6 fields — a schedule with
    /// no command has no destination to audit.
    pub fn parse(line: &str) -> Option<CronLine> {
        let mut fields: Vec<(usize, usize)> = Vec::new();
        let mut start: Option<usize> = None;
        for (i, ch) in line.char_indices() {
            if ch.is_whitespace() {
                if let Some(s) = start {
                    fields.push((s, i));
                    start = None;
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        if let Some(s) = start {
            fields.push((s, line.len()));
        }
        if fields.len() < 6 {
            return None;
        }
        let schedule = line[fields[0].0..fields[4].1].to_string();
        let command = parse_command(&line[fields[5].0..]);
        Some(CronLine { schedule, command })
    }

    /// The line with every file redirect dropped — the corrected schedule
    /// for a job whose script owns its log (invariant 1, fix (a)).
    pub fn render_without_redirects(&self) -> String {
        format!("{} {}", self.schedule, self.command.command)
    }
}

/// Parse a cron command, separating file redirects from the command.
///
/// Recognized redirect tokens: `>`, `>>` (stdout); `2>`, `2>>` (stderr);
/// `&>`, `&>>` (both); `2>&1` (stderr dup onto the current stdout target);
/// `2>&-` (stderr to nowhere). Token-level: `>>file` without a space is not
/// recognized, and quoted arguments containing spaces are split.
pub fn parse_command(command: &str) -> CronCommand {
    let mut cmd_tokens: Vec<&str> = Vec::new();
    let mut stdout_target: Option<String> = None;
    let mut stderr_target: Option<String> = None;

    let toks = command.split_whitespace().collect::<Vec<_>>();
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            ">" | ">>" => {
                if i + 1 < toks.len() {
                    stdout_target = Some(toks[i + 1].to_string());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "2>" | "2>>" => {
                if i + 1 < toks.len() {
                    stderr_target = Some(toks[i + 1].to_string());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "&>" | "&>>" => {
                if i + 1 < toks.len() {
                    let t = toks[i + 1].to_string();
                    stdout_target = Some(t.clone());
                    stderr_target = Some(t);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "2>&1" => {
                if let Some(t) = &stdout_target {
                    stderr_target = Some(t.clone());
                }
                i += 1;
            }
            "2>&-" => i += 1,
            other => {
                cmd_tokens.push(other);
                i += 1;
            }
        }
    }

    let mut redirects = Vec::new();
    match (stdout_target, stderr_target) {
        (Some(s), Some(e)) if s == e => redirects.push(CronRedirect {
            target: s,
            stream: Stream::Both,
        }),
        (Some(s), Some(e)) => {
            redirects.push(CronRedirect {
                target: s,
                stream: Stream::Stdout,
            });
            redirects.push(CronRedirect {
                target: e,
                stream: Stream::Stderr,
            });
        }
        (Some(s), None) => redirects.push(CronRedirect {
            target: s,
            stream: Stream::Stdout,
        }),
        (None, Some(e)) => redirects.push(CronRedirect {
            target: e,
            stream: Stream::Stderr,
        }),
        (None, None) => {}
    }

    CronCommand {
        command: cmd_tokens.join(" "),
        redirects,
    }
}

/// The path a script declares as its own log: the first `LOG=<path>`
/// assignment.
///
/// Recognized forms: `LOG=<v>`, `LOG="<v>"`, `LOG='<v>'`, `export LOG=<v>`,
/// and the default form `LOG=${LOG:-<v>}` (the fallback is the declared
/// path). The value is returned as written — unexpanded — so it can be
/// compared textually with a cron redirect target from the same
/// deployment.
pub fn script_log(script: &str) -> Option<String> {
    for line in script.lines() {
        let t = line.trim();
        let Some(rest) = t
            .strip_prefix("export ")
            .and_then(|s| s.strip_prefix("LOG="))
            .or_else(|| t.strip_prefix("LOG="))
        else {
            continue;
        };
        let v = rest.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(v);
        if let Some(inner) = v.strip_prefix("${LOG:-").and_then(|s| s.strip_suffix('}')) {
            return Some(inner.to_string());
        }
        if v.is_empty() {
            return None;
        }
        return Some(v.to_string());
    }
    None
}

/// Who names the job's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DestinationOwner {
    /// The script declares `LOG=` and the schedule has no file redirect.
    Script,
    /// The cron line has a file redirect and the script sets no `LOG`.
    Schedule,
}

/// How the cron line and the script divide the job's log destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DestinationOwnership {
    /// Exactly one destination, declared by one side. Invariant 1 holds.
    Single { by: DestinationOwner, path: String },
    /// Both sides declare the same path: a redundant double-claim, but
    /// still one log. Invariant 1 holds.
    Shared { path: String },
    /// Both sides declare *different* paths: the incident. The schedule
    /// names `schedule`; the job writes `script`. An operator who checks
    /// the schedule's path reads a log that stays empty while the job is
    /// healthy.
    Decoy { schedule: String, script: String },
    /// Neither side declares a destination. There is no log to audit; the
    /// heartbeat invariant is the only liveness signal.
    None,
}

impl DestinationOwnership {
    /// The one-line finding, or `None` when invariant 1 holds.
    pub fn finding(&self, job: &str) -> Option<String> {
        match self {
            DestinationOwnership::Decoy { schedule, script } => Some(format!(
                "{job}: two log destinations — the schedule names {schedule}, the job writes {script}; \
                 the schedule's log reads empty while the job is healthy. Drop the cron redirect or \
                 rename it to {}",
                stderr_name(job)
            )),
            _ => None,
        }
    }
}

/// Classify where the job's log goes, given the schedule's command and the
/// script text (invariant 1).
///
/// Only log destinations are compared: a stderr-only redirect is not a log
/// ([`CronCommand::log_destinations`]) and a stray stderr name is
/// invariant 2's territory ([`stray_redirect_name`]).
///
/// The comparison is textual on unexpanded paths: both sides of one
/// deployment use the same variables, so `$L/logs/cron-topup.log` and
/// `$L/logs/topup.log` differ exactly when the files do.
pub fn audit_destination(cron: &CronCommand, script: &str) -> DestinationOwnership {
    let cron_targets = cron.log_destinations();
    let script_target = script_log(script);
    match (cron_targets.first(), script_target) {
        (None, None) => DestinationOwnership::None,
        (Some(s), None) => DestinationOwnership::Single {
            by: DestinationOwner::Schedule,
            path: s.clone(),
        },
        (None, Some(p)) => DestinationOwnership::Single {
            by: DestinationOwner::Script,
            path: p,
        },
        (Some(s), Some(p)) if s == &p => DestinationOwnership::Shared { path: p },
        (Some(s), Some(p)) => DestinationOwnership::Decoy {
            schedule: s.clone(),
            script: p,
        },
    }
}

// ── Invariant 2: a decoy log is worse than no log ─────────────────────────

/// The name a redirect should have when it is not the job's own log.
pub fn stderr_name(job: &str) -> String {
    format!("{job}.stderr")
}

/// The basename of a (possibly variable-bearing) path.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Check the name of a cron redirect whose target is *not* the job's own
/// log (invariant 2).
///
/// Such a file can be empty on every healthy run — it catches stray output
/// at best — and an empty file named `*.log` asserts "ran, said nothing" at
/// the path an operator will check. The name must read as good news when
/// empty: `<job>.stderr`.
///
/// Returns `Some(suggested)` with the `<job>.stderr` name when the current
/// name reads as the job's log (basename ends in `.log`), `None` when the
/// name is already fine.
pub fn stray_redirect_name(job: &str, target: &str) -> Option<String> {
    if basename(target).ends_with(".log") {
        Some(stderr_name(job))
    } else {
        None
    }
}

// ── Invariant 3: one heartbeat line per run, including no-ops ────────────

/// One heartbeat call site found in a script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatSite {
    /// 1-based line number.
    pub line: usize,
    /// On the main flow — not inside `if`/`case`/`while`/`until`/`for` —
    /// i.e. reachable on a no-op run.
    pub unconditional: bool,
}

/// Heartbeat call sites in a script, classified by flow position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HeartbeatCoverage {
    sites: Vec<HeartbeatSite>,
}

impl HeartbeatCoverage {
    /// All sites, in line order.
    pub fn sites(&self) -> &[HeartbeatSite] {
        &self.sites
    }

    /// How many sites exist.
    pub fn total(&self) -> usize {
        self.sites.len()
    }

    /// How many sites are on the main flow.
    pub fn unconditional(&self) -> usize {
        self.sites.iter().filter(|s| s.unconditional).count()
    }

    /// Invariant 3: the job beats on every run, no-op included, iff a
    /// heartbeat call is on the main flow. A beat in each branch of an
    /// `if` is not what this invariant guarantees — the no-op run takes
    /// exactly one branch, and "one heartbeat line per run" wants the beat
    /// once, at the point where the outcome (work done or "nothing to do")
    /// is known.
    pub fn beats_on_every_run(&self) -> bool {
        self.unconditional() > 0
    }

    /// The one-line finding for a job that cannot guarantee a no-op beat,
    /// or `None` when invariant 3 holds.
    pub fn finding(&self, job: &str) -> Option<String> {
        if self.beats_on_every_run() {
            return None;
        }
        Some(format!(
            "{job}: no unconditional heartbeat call site — a no-op run can write nothing; \
             beat once on the main flow, with an outcome that says so"
        ))
    }
}

/// True when any maximal `[A-Za-z0-9_]+` run in `line` ends in
/// `heartbeat`: `heartbeat`, `log_heartbeat`, `emit_heartbeat`, and the
/// inline log form `heartbeat: <step> ...` all match; `myheartbeats` does
/// not.
fn line_has_heartbeat(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphanumeric() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if line[start..i].ends_with("heartbeat") {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// +1 for a keyword that opens a conditional/loop scope, -1 for one that
/// closes it. Token-exact: `elif` is not `if`; `done` closes
/// `for`/`while`/`until`; `fi` closes `if`; `esac` closes `case`.
fn flow_delta(tok: &str) -> i8 {
    match tok {
        "if" | "while" | "until" | "for" | "case" => 1,
        "fi" | "done" | "esac" => -1,
        _ => 0,
    }
}

/// Scan a script for heartbeat call sites and their flow position
/// (invariant 3, script-side half).
///
/// Token-level static scan: a site is a non-comment line containing a word
/// that ends in `heartbeat`; a site is *unconditional* when it sits outside
/// every `if`/`case`/`while`/`until`/`for` scope at line granularity. A
/// loop body is conditional for this purpose: the body may run zero times,
/// which is the no-op run.
pub fn scan_heartbeat_coverage(script: &str) -> HeartbeatCoverage {
    let mut sites = Vec::new();
    let mut depth: i64 = 0;
    for (n, line) in script.lines().enumerate() {
        let t = line.trim();
        if !t.is_empty() && !t.starts_with('#') {
            if line_has_heartbeat(t) {
                sites.push(HeartbeatSite {
                    line: n + 1,
                    unconditional: depth <= 0,
                });
            }
        }
        for tok in
            t.split(|c: char| c.is_whitespace() || c == ';' || c == '|' || c == '(' || c == ')')
        {
            depth += i64::from(flow_delta(tok));
            if depth < 0 {
                depth = 0;
            }
        }
    }
    HeartbeatCoverage { sites }
}

// ── Invariant 4: liveness from durable state, not log mtimes ─────────────

/// Liveness of a job from its durable run state alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunLiveness {
    /// A success was recorded within the stale window.
    Live { silent_secs: u64 },
    /// The last success is outside the stale window — reported as silence,
    /// never as death.
    Stale { silent_secs: u64, last: u64 },
    /// No successful run on record yet.
    NoRecord,
}

impl RunLiveness {
    /// The wording, keeping [`crate::heartbeat`]'s discipline: silence is
    /// reported as silence.
    pub fn describe(&self) -> String {
        match self {
            RunLiveness::Live { silent_secs } => format!("last success {silent_secs}s ago"),
            RunLiveness::Stale { silent_secs, last } => format!(
                "no successful run since {} ({silent_secs}s of silence)",
                format_utc(*last)
            ),
            RunLiveness::NoRecord => "no successful run on record".to_string(),
        }
    }
}

/// Durable run state for one scheduled job. The supervisor updates this
/// after every run it observes; the status command renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRunState {
    /// Job name as scheduled (e.g. `topup`).
    pub job: String,
    /// Unix seconds of the most recent successful run, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_secs: Option<u64>,
    /// Unix seconds of the most recent attempt (success or failure), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_secs: Option<u64>,
    /// Failures since the last success (0 after a success).
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Total observed runs.
    #[serde(default)]
    pub run_count: u64,
}

impl JobRunState {
    /// Liveness from durable state alone — no log read, no mtime. The
    /// window is `interval * DEFAULT_STALE_AFTER_INTERVALS`, the same
    /// multiple the heartbeat module uses, so the two liveness signals
    /// agree on "how late is too late".
    pub fn liveness(&self, now_secs: u64, interval_secs: u64) -> RunLiveness {
        let Some(last) = self.last_success_secs else {
            return RunLiveness::NoRecord;
        };
        let silent = now_secs.saturating_sub(last);
        let window = interval_secs.saturating_mul(DEFAULT_STALE_AFTER_INTERVALS);
        if silent <= window {
            RunLiveness::Live {
                silent_secs: silent,
            }
        } else {
            RunLiveness::Stale {
                silent_secs: silent,
                last,
            }
        }
    }
}

/// Durable run state for a fleet of scheduled jobs, keyed by job name.
///
/// This is the state the status command reads. It is the replacement for
/// "look at the log mtimes" and the upgrade of "look at the lockfile
/// mtimes": it is written by the supervisor as part of observing the run,
/// not by the job incidentally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobStatus {
    states: BTreeMap<String, JobRunState>,
}

impl JobStatus {
    /// Empty status (no jobs observed yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful run of `job` at `at_secs` (durable boundary —
    /// the caller persists the result).
    pub fn record_success(&mut self, job: &str, at_secs: u64) {
        let st = self
            .states
            .entry(job.to_string())
            .or_insert_with(|| JobRunState {
                job: job.to_string(),
                last_success_secs: None,
                last_attempt_secs: None,
                consecutive_failures: 0,
                run_count: 0,
            });
        st.last_success_secs = Some(at_secs);
        st.last_attempt_secs = Some(at_secs);
        st.consecutive_failures = 0;
        st.run_count += 1;
    }

    /// Record a failed run of `job` at `at_secs` (durable boundary — the
    /// caller persists the result).
    pub fn record_failure(&mut self, job: &str, at_secs: u64) {
        let st = self
            .states
            .entry(job.to_string())
            .or_insert_with(|| JobRunState {
                job: job.to_string(),
                last_success_secs: None,
                last_attempt_secs: None,
                consecutive_failures: 0,
                run_count: 0,
            });
        st.last_attempt_secs = Some(at_secs);
        st.consecutive_failures = st.consecutive_failures.saturating_add(1);
        st.run_count += 1;
    }

    /// The run state for one job, if observed.
    pub fn get(&self, job: &str) -> Option<&JobRunState> {
        self.states.get(job)
    }

    /// All jobs, in name order (deterministic).
    pub fn jobs(&self) -> impl Iterator<Item = &JobRunState> {
        self.states.values()
    }

    /// One status line for one job, rendered from durable state alone
    /// (invariant 4). A job that was never observed gets a line that says
    /// exactly that — silence is reported as silence.
    pub fn status_line(&self, job: &str, now_secs: u64, interval_secs: u64) -> String {
        let Some(st) = self.states.get(job) else {
            return format!("{job}: no successful run on record");
        };
        let mut line = match st.liveness(now_secs, interval_secs) {
            RunLiveness::Live { silent_secs } => {
                format!("{job}: last success {silent_secs}s ago")
            }
            RunLiveness::Stale { silent_secs, last } => format!(
                "{job}: no successful run since {} ({silent_secs}s of silence)",
                format_utc(last)
            ),
            RunLiveness::NoRecord => match st.last_attempt_secs {
                Some(at) => format!(
                    "{job}: no successful run on record (last attempt {}s ago, {} consecutive failure{})",
                    now_secs.saturating_sub(at),
                    st.consecutive_failures,
                    if st.consecutive_failures == 1 { "" } else { "s" }
                ),
                None => format!("{job}: no successful run on record"),
            },
        };
        if st.run_count > 0 {
            line.push_str(&format!(" (run #{})", st.run_count));
        }
        line
    }

    /// One status line per observed job, in name order — the body of the
    /// status command (invariant 4).
    pub fn status_lines(&self, now_secs: u64, interval_secs: u64) -> Vec<String> {
        let names: Vec<String> = self.states.keys().cloned().collect();
        names
            .iter()
            .map(|j| self.status_line(j, now_secs, interval_secs))
            .collect()
    }

    /// Persist the whole fleet state (durable boundary).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("JobStatus serializes")
    }

    /// Restore fleet state from a previous persistence (durable boundary).
    pub fn from_json(v: &serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(v.clone())
    }
}
