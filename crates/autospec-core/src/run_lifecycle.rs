//! A run is visible in every state it was in (issue #3606).
//!
//! Three agent runs produced eighteen bytes and no status file:
//!
//! ```text
//! autospec/out/issue-3539/agent.out   18 bytes   "Connection error."
//! autospec/out/issue-3574/agent.out   18 bytes   "Connection error."
//! autospec/out/issue-3590/agent.out   18 bytes   "Connection error."
//! ```
//!
//! Each directory contained `agent.out` and nothing else — no
//! `status.txt`, no `changes.patch`, no test log. The runner exited
//! before writing any of them. One of the runs went **3 h 03 m** before
//! dying, and the entire record of it was two words. Because
//! `status.txt` was never written, those three runs do not appear as
//! failures in any tally — they appear as *nothing*: a pipeline that
//! reports 12 VERIFIED out of 12 status files while three runs died
//! silently is not reporting a success rate. The failure and the absence
//! look identical.
//!
//! The same missing signal has a mirror image that cost more: `iw-30`
//! had been `RUNNING` for 2 h 21 m with `agent.out` at 0 bytes, and every
//! observable said dead — it was working perfectly, nine slots
//! generating across six workers. A supervisor with no way to tell the
//! two apart eventually guesses, and either guess is expensive: leave it
//! and a corpse holds a GPU slot, cancel it and hours of paid-for
//! generation are thrown away.
//!
//! And the run that produced nothing left no transcript: the session
//! lives in node-local scratch and is destroyed when the job ends, so a
//! 1 h 47 m GPU allocation left 411 bytes of key-value pairs and no way
//! to tell which of four different causes it was — budget burned in
//! `reasoning_content` (`finish=length`, empty content: 36 of 68 runs),
//! read without ever acting, no change needed, or the harness failing
//! underneath. Those four demand different responses, and two of them
//! are worth retrying and two are not.
//!
//! Six invariants, each a primitive here:
//!
//! 1. **The status record is written first, not last.** It is created at
//!    start with `status=RUNNING`, the endpoint and the model, and
//!    updated on every transition ([`StatusRecord::start`],
//!    [`StatusRecord::line`]): a killed run leaves a record of where it
//!    was, and "which worker did this run use" is answerable after the
//!    fact. Through the lifecycle API a record can only begin
//!    `RUNNING` and only settle through [`StatusRecord::finish`] — there
//!    is no path to a record that starts at its own verdict.
//! 2. **A run with no status record is FAILED in every tally.** The
//!    denominator is the runs the dispatcher started, never the status
//!    files that exist; missing records and records still reading
//!    `RUNNING` are failures ([`tally`], [`Tally::reconciles`]).
//! 3. **Liveness is written periodically while work is happening** — a
//!    heartbeat line with a timestamp and a work counter, flushed
//!    unbuffered ([`heartbeat_line`], [`liveness`]) — so a progressing
//!    run is distinguishable from a stopped one *without inspecting
//!    anything outside the run's own output*. A line whose counter did
//!    not advance proves the process is alive, not that work happened;
//!    silence past the window is a real signal, not an inference; and a
//!    running record with no line at all is a finding
//!    ([`unrecorded_liveness_finding`]) — the 2 h 21 m / 0-byte case.
//! 4. **An endpoint unreachable at start costs seconds, not a scheduled
//!    slot.** The dispatcher probes the endpoint before consuming a
//!    slot ([`audit_dispatch`]); after the fact, a connection loss with
//!    no liveness line inside the fail-fast window is
//!    [`LossClass::UnreachableAtStart`] — a dispatch defect — distinct
//!    from a preemption after progress ([`classify_loss`]).
//! 5. **A lost connection is retried against a different pool member,
//!    never the same one.** The retry re-reads the endpoint directory
//!    and picks a healthy peer ([`reselect`], deterministic, fail-closed
//!    when no peer remains); a retry to the endpoint the previous
//!    attempt just lost is a finding ([`same_endpoint_retries`]).
//! 6. **The transcript is evidence, and a failure must not destroy the
//!    evidence it names.** The session transcript is the one artefact
//!    that separates the four no-output causes, and it is copied into
//!    the shared output directory before the job exits whenever the run
//!    is about to be recorded no-output, stall-killed or non-zero
//!    ([`transcript_policy`], [`transcript_verdict`]); with the
//!    transcript gone the cause is [`NoOutputCause::Indeterminate`],
//!    never a guess, and the retry decision
//!    ([`NoOutputCause::retry_worthy`]) is only made on a separable
//!    cause. `finish_reason` and the final response's token counts are
//!    fields in the status file ([`FinishReason`], [`Terminal`]), not a
//!    comment in a shell script.
//!
//! Everything here is pure: the record holds in-memory state and the
//! caller performs I/O with the lines it renders. No clock, no
//! subprocesses — the caller supplies `now`.

use std::collections::BTreeMap;

use crate::execution::endpoint::StatusReason;
use crate::heartbeat::{format_utc, HEARTBEAT_PREFIX};
use crate::run_status::{canonical_status, Status};

/// The in-flight token a run writes at start and carries until it
/// settles. Deliberately outside the [`run_status`](crate::run_status)
/// vocabulary, which is the set of *terminal* statuses the runner
/// emits: `RUNNING` is what the record says before there is a verdict,
/// and a consumer's terminal match list must not be forced to cover it.
pub const RUNNING: &str = "RUNNING";

/// The exit code of a run the stall watchdog killed with SIGTERM.
pub const STALL_KILL_RC: i32 = 143;

/// The silence window after which "no line" is a real signal about a
/// running record: the heartbeat is flushed unbuffered while work
/// happens, so past this the absence of a line is evidence, not
/// inference.
pub const DEFAULT_STALE_AFTER_SECS: u64 = 600;

/// The window within which a connection loss means the endpoint was
/// unreachable at start: the fail-fast probe costs seconds, so a loss
/// this early with no liveness line is the probe being skipped, not a
/// preemption.
pub const DEFAULT_FAIL_FAST_WINDOW_SECS: u64 = 30;

/// One liveness line the runner appends while work is happening,
/// flushed unbuffered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heartbeat {
    /// Epoch seconds the line was flushed.
    pub ts: u64,
    /// A work counter (tokens, turn number, tool calls). A line whose
    /// counter does not advance since the previous one proves the
    /// process is alive, not that work happened.
    pub counter: u64,
}

/// One liveness line, flushed unbuffered while work is happening:
/// `heartbeat: 2026-09-07T22:12:00Z counter=4521 attempt=2`.
pub fn heartbeat_line(ts: u64, counter: u64, attempt: u32) -> String {
    format!(
        "{HEARTBEAT_PREFIX} {} counter={counter} attempt={attempt}",
        format_utc(ts)
    )
}

/// The final response's termination token, recorded as a field in the
/// status file rather than a comment: `finish=length` with empty
/// content was 36 of 68 runs and lived only in a shell-script comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// The model stopped on its own.
    Stop,
    /// The model hit the token budget. With empty content this is the
    /// reasoning-burn signature: the whole budget spent in
    /// `reasoning_content`.
    Length,
    /// The model asked for a tool call.
    ToolCalls,
    /// A provider safety filter stopped the response.
    ContentFilter,
}

impl FinishReason {
    /// The wire token the endpoint reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ToolCalls => "tool_calls",
            Self::ContentFilter => "content_filter",
        }
    }

    /// Fail closed: a token the endpoint did not define is an error,
    /// never a guess.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "stop" => Ok(Self::Stop),
            "length" => Ok(Self::Length),
            "tool_calls" => Ok(Self::ToolCalls),
            "content_filter" => Ok(Self::ContentFilter),
            other => Err(format!("unknown finish_reason: {other}")),
        }
    }
}

/// The terminal state the runner records in the status file when the
/// run settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    /// The status token as recorded, verbatim (e.g. `NO-OUTPUT`).
    pub status: String,
    /// The agent process's exit code. [`STALL_KILL_RC`] is the stall
    /// watchdog's SIGTERM.
    pub agent_rc: i32,
    /// The final response's finish reason, when the endpoint reported
    /// one.
    pub finish_reason: Option<FinishReason>,
    /// The final response's input token count, when reported.
    pub input_tokens: Option<u64>,
    /// The final response's output token count, when reported.
    pub output_tokens: Option<u64>,
    /// Whether the session transcript was copied into the shared output
    /// directory before the job exited.
    pub transcript_copied: bool,
    /// Epoch seconds the record was settled.
    pub finished_at: u64,
}

impl Terminal {
    /// Construct a terminal state. `None` when the status token is
    /// empty: the record must say what happened.
    pub fn new(status: impl Into<String>, agent_rc: i32, finished_at: u64) -> Option<Self> {
        let status = status.into();
        if status.trim().is_empty() {
            return None;
        }
        Some(Self {
            status,
            agent_rc,
            finish_reason: None,
            input_tokens: None,
            output_tokens: None,
            transcript_copied: false,
            finished_at,
        })
    }

    /// Whether the exit code is the stall watchdog's SIGTERM kill.
    pub fn is_stall_killed(&self) -> bool {
        self.agent_rc == STALL_KILL_RC
    }

    /// Whether the status resolves to the vocabulary's no-output token,
    /// alias included.
    pub fn is_no_output(&self) -> bool {
        matches!(
            canonical_status(&self.status),
            Some(Status::NoOutput) | Some(Status::TimeoutNoOutput)
        )
    }
}

/// One endpoint assignment for a run, in order; the last is the
/// current one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The worker endpoint the attempt ran against.
    pub endpoint: String,
    /// Epoch seconds the attempt started.
    pub at: u64,
}

/// A transition that cannot apply to the record's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleError {
    /// The run is already settled: no transition applies.
    AlreadySettled,
    /// The transition names no endpoint: the record could not say where
    /// the run is.
    EmptyEndpoint,
}

/// One run's status record: the structured state behind the status
/// file.
///
/// The record is written first, not last: it exists at start, before
/// any work, and is updated on every transition, so a killed run leaves
/// a record of where it was. Through the lifecycle API a record can
/// only begin `RUNNING` ([`StatusRecord::start`]) and only settle
/// through [`StatusRecord::finish`] — there is no path to a record that
/// starts at its own verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRecord {
    /// The run's id, e.g. `issue-3590`.
    pub run_id: String,
    /// The model the run was dispatched for: "which model did this run
    /// use" is answerable after the fact.
    pub model: String,
    /// The endpoint assignments, in order; the last is the current
    /// attempt.
    pub attempts: Vec<Attempt>,
    /// Epoch seconds the run started (first attempt).
    pub started_at: u64,
    /// Liveness lines appended while work was happening.
    pub heartbeats: Vec<Heartbeat>,
    /// The terminal state, `None` while the run is in flight.
    pub terminal: Option<Terminal>,
}

impl StatusRecord {
    /// Open the record at start, before any work: `status=RUNNING` with
    /// the endpoint and the model. `None` on an empty run id, endpoint
    /// or model: a record that cannot say where the run is is not a
    /// record.
    pub fn start(
        run_id: impl Into<String>,
        endpoint: impl Into<String>,
        model: impl Into<String>,
        started_at: u64,
    ) -> Option<Self> {
        let run_id = run_id.into();
        let endpoint = endpoint.into();
        let model = model.into();
        if run_id.trim().is_empty() || endpoint.trim().is_empty() || model.trim().is_empty() {
            return None;
        }
        Some(Self {
            run_id,
            model,
            attempts: vec![Attempt {
                endpoint,
                at: started_at,
            }],
            started_at,
            heartbeats: Vec::new(),
            terminal: None,
        })
    }

    /// The endpoint the current attempt runs against.
    pub fn endpoint(&self) -> &str {
        // start() requires exactly one attempt; redeploy() only appends.
        self.attempts
            .last()
            .expect("a record always has its first attempt")
            .endpoint
            .as_str()
    }

    /// 1-based number of the current attempt.
    pub fn attempt_number(&self) -> u32 {
        self.attempts.len() as u32
    }

    /// Whether the run is still in flight.
    pub fn is_running(&self) -> bool {
        self.terminal.is_none()
    }

    /// Record a re-dispatch against another pool member after a
    /// connection loss: the transition the record updates on. Refused
    /// once the run has settled, or when the endpoint is empty.
    pub fn redeploy(&mut self, endpoint: impl Into<String>, at: u64) -> Result<(), LifecycleError> {
        let endpoint = endpoint.into();
        if self.terminal.is_some() {
            return Err(LifecycleError::AlreadySettled);
        }
        if endpoint.trim().is_empty() {
            return Err(LifecycleError::EmptyEndpoint);
        }
        self.attempts.push(Attempt { endpoint, at });
        Ok(())
    }

    /// Append a liveness line while work is happening. A line whose
    /// counter does not advance is still recorded: it proves the
    /// process is alive, and refusing to write it would turn a slow
    /// phase into silence — the exact signal the kill decision must not
    /// run on.
    pub fn heartbeat(&mut self, ts: u64, counter: u64) -> Result<(), LifecycleError> {
        if self.terminal.is_some() {
            return Err(LifecycleError::AlreadySettled);
        }
        self.heartbeats.push(Heartbeat { ts, counter });
        Ok(())
    }

    /// Settle the record with the terminal state. Refused once the run
    /// has settled: the status file has one verdict.
    pub fn finish(&mut self, terminal: Terminal) -> Result<(), LifecycleError> {
        if self.terminal.is_some() {
            return Err(LifecycleError::AlreadySettled);
        }
        self.terminal = Some(terminal);
        Ok(())
    }

    /// The current-state line for the status file, overwritten on every
    /// transition: the in-flight record with the latest liveness, or
    /// the terminal record with the final fields.
    pub fn line(&self) -> String {
        let head = format!(
            "status={} endpoint={} model={} attempt={} started={}",
            match &self.terminal {
                None => RUNNING,
                Some(term) => term.status.as_str(),
            },
            self.endpoint(),
            self.model,
            self.attempt_number(),
            format_utc(self.started_at)
        );
        match &self.terminal {
            None => {
                let mut line = head;
                if let Some(last) = self.heartbeats.last() {
                    line.push_str(&format!(
                        " last_heartbeat={} counter={}",
                        format_utc(last.ts),
                        last.counter
                    ));
                }
                line
            }
            Some(term) => {
                let mut line = format!(
                    "{head} finished={} agent_rc={}",
                    format_utc(term.finished_at),
                    term.agent_rc
                );
                if let Some(reason) = term.finish_reason {
                    line.push_str(&format!(" finish_reason={}", reason.as_str()));
                }
                if let Some(tokens) = term.input_tokens {
                    line.push_str(&format!(" input_tokens={tokens}"));
                }
                if let Some(tokens) = term.output_tokens {
                    line.push_str(&format!(" output_tokens={tokens}"));
                }
                line.push_str(&format!(
                    " transcript={}",
                    if term.transcript_copied { "yes" } else { "no" }
                ));
                line
            }
        }
    }
}

/// What the run's own output says about its progress — without
/// inspecting anything outside the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// The run settled: liveness no longer applies; the terminal record
    /// explains it.
    Settled,
    /// The latest heartbeat is within the window and its counter
    /// advanced since the previous line: work is demonstrably
    /// happening.
    Progressing,
    /// The latest heartbeat is within the window but its counter did
    /// not advance since the previous line: the process is alive;
    /// progress is unproven. A kill decision needs more than this.
    Alive,
    /// At least one line was recorded and the latest is at least
    /// `stale_after_secs` old: no line for N minutes — a real signal,
    /// not an inference.
    Stalled {
        /// Seconds since the latest line.
        silent_secs: u64,
    },
    /// No line was recorded: a progressing run is indistinguishable
    /// from a stopped one from the run's own output. The state the
    /// supervisor must never be put in.
    Unrecorded,
}

/// Judge a record's liveness from its own lines at `now`.
///
/// A clock rewind reads as zero silence, never as stalled (the
/// saturating subtraction). A settled record is `Settled`: the
/// terminal record, not the heartbeat, is its evidence.
pub fn liveness(record: &StatusRecord, now: u64, stale_after_secs: u64) -> Liveness {
    if record.terminal.is_some() {
        return Liveness::Settled;
    }
    let Some(last) = record.heartbeats.last() else {
        return Liveness::Unrecorded;
    };
    let silent = now.saturating_sub(last.ts);
    if silent >= stale_after_secs {
        return Liveness::Stalled {
            silent_secs: silent,
        };
    }
    let prev = if record.heartbeats.len() >= 2 {
        record.heartbeats[record.heartbeats.len() - 2].counter
    } else {
        0
    };
    if last.counter > prev {
        Liveness::Progressing
    } else {
        Liveness::Alive
    }
}

/// The finding for a running record with no liveness line: the state a
/// supervisor cannot read, where a healthy run and a dead one look
/// identical from the run's own output (the 2 h 21 m / 0-byte case).
/// `None` when the record carries at least one line, or has settled.
pub fn unrecorded_liveness_finding(record: &StatusRecord) -> Option<String> {
    if record.is_running() && record.heartbeats.is_empty() {
        Some(format!(
            "LIVENESS_UNRECORDED: run {} is running with no liveness line: a progressing run is indistinguishable from a stopped one without inspecting outside the run's own output",
            record.run_id
        ))
    } else {
        None
    }
}

/// The reachability probe the dispatcher runs against the granted
/// endpoint before it consumes a scheduled slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartProbe {
    /// The endpoint answered: dispatch may proceed.
    Reachable,
    /// The endpoint did not answer: refuse, and cost seconds.
    Unreachable,
}

/// The dispatch decision, audited against the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchAudit {
    /// No slot was consumed, or a passed probe preceded it.
    Sound,
    /// A slot was consumed without any probe: the fail-fast check was
    /// skipped.
    Unprobed,
    /// A slot was consumed after the probe reported the endpoint
    /// unreachable: the refusal was ignored.
    PastRefusal,
}

/// Audit a dispatch decision against its reachability probe: a slot is
/// not consumed without a passed probe (invariant 4).
pub fn audit_dispatch(probe: Option<StartProbe>, slot_consumed: bool) -> DispatchAudit {
    if !slot_consumed {
        return DispatchAudit::Sound;
    }
    match probe {
        Some(StartProbe::Reachable) => DispatchAudit::Sound,
        Some(StartProbe::Unreachable) => DispatchAudit::PastRefusal,
        None => DispatchAudit::Unprobed,
    }
}

impl DispatchAudit {
    /// The report line; `None` for [`DispatchAudit::Sound`] — a sound
    /// dispatch has nothing to report.
    pub fn warn_line(&self) -> Option<String> {
        match self {
            Self::Sound => None,
            Self::Unprobed => Some(
                "WARN: dispatch: a scheduled slot was consumed without a reachability probe; an endpoint unreachable at start should cost seconds, not a slot".to_string(),
            ),
            Self::PastRefusal => Some(
                "WARN: dispatch: a scheduled slot was consumed after the probe reported the endpoint unreachable".to_string(),
            ),
        }
    }
}

/// What a connection-loss record says about where the endpoint died.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossClass {
    /// The endpoint died after the run showed progress — or after the
    /// fail-fast window: an expected preemption. Re-resolve against a
    /// different pool member.
    PreemptedMidRun,
    /// The run died fast without a single liveness line: the endpoint
    /// was unreachable at start and the fail-fast probe was skipped or
    /// ignored. A dispatch defect, distinct from fleet preemption.
    UnreachableAtStart,
}

impl LossClass {
    /// The stable label for logs and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreemptedMidRun => "preempted-mid-run",
            Self::UnreachableAtStart => "unreachable-at-start",
        }
    }
}

/// Classify a connection-loss terminal.
///
/// `None` when the record did not end in the connection-error token
/// (or has not settled): the classification does not apply. The
/// discriminator is the pair (liveness lines, elapsed time): a loss
/// with no line inside the fail-fast window means the endpoint was
/// unreachable at start — the dispatch defect; a loss after any line,
/// or after the window, means the endpoint was reachable at start and
/// died under preemption — the normal case.
pub fn classify_loss(record: &StatusRecord, fail_fast_window_secs: u64) -> Option<LossClass> {
    let Some(term) = &record.terminal else {
        return None;
    };
    if term.status.trim() != StatusReason::ConnectionError.as_str() {
        return None;
    }
    let elapsed = term.finished_at.saturating_sub(record.started_at);
    if record.heartbeats.is_empty() && elapsed <= fail_fast_window_secs {
        Some(LossClass::UnreachableAtStart)
    } else {
        Some(LossClass::PreemptedMidRun)
    }
}

/// The refusal a re-resolution emits when no healthy peer remains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReResolutionRefusal {
    /// Every healthy peer the re-read reported is the failed worker
    /// itself (or the read reported none): there is no different pool
    /// member to retry against.
    NoHealthyPeer {
        /// The failed endpoint a retry must not return to.
        failed: String,
    },
}

impl ReResolutionRefusal {
    /// The refusal line: names the endpoint the retry was about to
    /// return to, so the operator reads the answer instead of guessing
    /// a default.
    pub fn line(&self) -> String {
        match self {
            Self::NoHealthyPeer { failed } => format!(
                "FATAL: no healthy pool member other than {failed} in the endpoint directory; refusing to re-grant the dead endpoint"
            ),
        }
    }
}

/// Pick the replacement endpoint after a connection loss: from the
/// fresh read of the endpoint directory, a healthy peer different from
/// the failed worker (invariant 5).
///
/// Deterministic: the lowest endpoint string. Fails closed when no peer
/// remains — a re-read that reports only the dead worker is a stale
/// directory, not a fleet with one member, and retrying the dead
/// endpoint is a spin, never the default.
pub fn reselect(failed: &str, healthy_peers: &[String]) -> Result<String, ReResolutionRefusal> {
    healthy_peers
        .iter()
        .filter(|peer| !peer.trim().is_empty() && peer.as_str() != failed)
        .min_by(|a, b| a.cmp(b))
        .cloned()
        .ok_or_else(|| ReResolutionRefusal::NoHealthyPeer {
            failed: failed.to_string(),
        })
}

/// The 1-based attempt indices where a run retried the endpoint its
/// previous attempt had just lost: a re-resolution that picked the same
/// pool member. A later attempt returning to an *earlier* endpoint is
/// not a finding: a preempted worker can be re-registered and
/// re-granted.
pub fn same_endpoint_retries(record: &StatusRecord) -> Vec<usize> {
    record
        .attempts
        .iter()
        .zip(record.attempts.iter().skip(1))
        .enumerate()
        .filter(|(_, (prev, cur))| prev.endpoint == cur.endpoint)
        .map(|(i, _)| i + 2)
        .collect()
}

/// A population tally: the success rate a pipeline reports about its
/// runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    /// Every run the dispatcher knows it started: the denominator.
    pub dispatched: usize,
    /// Dispatched runs with no status record at all: the silent
    /// deaths, counted as failures, never dropped.
    pub missing: usize,
    /// Records still reading `RUNNING` after the run ended: killed
    /// runs, counted as failures.
    pub stale_running: usize,
    /// Settled records whose status resolves to `VERIFIED`.
    pub verified: usize,
    /// Settled records whose status does not: failures, each with a
    /// record saying what happened.
    pub failed: usize,
}

/// Tally a population of runs.
///
/// The denominator is the runs the dispatcher started, never the status
/// files that exist: a pipeline that reports 12 VERIFIED out of 12
/// status files while three runs died silently is not reporting a
/// success rate (issue #3606). A dispatched run with no record and a
/// record stuck at `RUNNING` are failures — the failure and the
/// absence must not look identical.
pub fn tally(dispatched: &[String], records: &BTreeMap<String, StatusRecord>) -> Tally {
    let mut out = Tally {
        dispatched: dispatched.len(),
        ..Tally::default()
    };
    for run_id in dispatched {
        match records.get(run_id) {
            None => out.missing += 1,
            Some(record) => match &record.terminal {
                None => out.stale_running += 1,
                Some(term) => {
                    if canonical_status(&term.status) == Some(Status::Verified) {
                        out.verified += 1;
                    } else {
                        out.failed += 1;
                    }
                }
            },
        }
    }
    out
}

impl Tally {
    /// Whether the buckets add up to the denominator: a tally that does
    /// not reconcile is reporting a state that cannot exist.
    pub fn reconciles(&self) -> bool {
        self.missing + self.stale_running + self.verified + self.failed == self.dispatched
    }

    /// The line every counter prints. The denominator is the dispatched
    /// runs; the failure breakdown names the silent deaths
    /// separately, so `no-record` is never smoothed into an ordinary
    /// failure.
    pub fn line(&self) -> String {
        if self.dispatched == 0 {
            return "tally: no runs dispatched".to_string();
        }
        let failed = self.missing + self.stale_running + self.failed;
        let mut line = format!("tally: {}/{} verified", self.verified, self.dispatched);
        if failed > 0 {
            let mut parts: Vec<String> = Vec::new();
            if self.missing > 0 {
                parts.push(format!("no-record={}", self.missing));
            }
            if self.stale_running > 0 {
                parts.push(format!("stale-running={}", self.stale_running));
            }
            if self.failed > 0 {
                parts.push(format!("terminal={}", self.failed));
            }
            line.push_str(&format!("; {failed} failed ({})", parts.join(", ")));
        }
        if failed > 0 {
            format!("WARN: {line}")
        } else {
            line
        }
    }
}

/// Whether the session transcript must be copied into the shared
/// output directory before the job exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptPolicy {
    /// The run settled with output and a clean exit: the transcript is
    /// not load-bearing; copying it is the runner's call.
    Optional,
    /// The run is about to be recorded in a state where the transcript
    /// is the only artefact that explains why: copying it is mandatory
    /// before the job exits.
    Required {
        /// The recorded state that makes the transcript load-bearing.
        reason: String,
    },
}

/// The copy policy for the session transcript: the floor the issue
/// names — no-output, stall-killed or non-zero. A runner that copies
/// unconditionally is stricter and compliant; one that copies only on
/// green is the incident. When both the status and the exit code make
/// the transcript load-bearing, the reason carries both.
pub fn transcript_policy(term: &Terminal) -> TranscriptPolicy {
    let mut reasons: Vec<String> = Vec::new();
    if term.is_no_output() {
        reasons.push(format!(
            "status={} is recorded with no artifact; the transcript is the only record of why",
            term.status
        ));
    }
    if term.agent_rc != 0 {
        let mut rc_reason = format!("agent_rc={}", term.agent_rc);
        if term.is_stall_killed() {
            rc_reason.push_str(" (stall-killed)");
        }
        reasons.push(rc_reason);
    }
    match reasons.len() {
        0 => TranscriptPolicy::Optional,
        1 => TranscriptPolicy::Required {
            reason: reasons.into_iter().next().expect("len 1"),
        },
        _ => TranscriptPolicy::Required {
            reason: reasons.join("; "),
        },
    }
}

/// Whether the run's output directory kept the evidence its status
/// record names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptVerdict {
    /// The transcript was copied, or the policy did not require it.
    Kept,
    /// The run ended in a state where the transcript is the only
    /// artefact that separates the causes, and it was destroyed with
    /// the node-local scratch: a failure that names the evidence it
    /// destroys.
    DestroyedEvidence {
        /// The recorded state that made the transcript load-bearing.
        reason: String,
    },
}

/// Verdict on the transcript's fate given the terminal state
/// (invariant 6).
pub fn transcript_verdict(term: &Terminal) -> TranscriptVerdict {
    match transcript_policy(term) {
        TranscriptPolicy::Required { reason } if !term.transcript_copied => {
            TranscriptVerdict::DestroyedEvidence { reason }
        }
        _ => TranscriptVerdict::Kept,
    }
}

/// The evidence the session transcript carries for a no-output run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TranscriptEvidence {
    /// Tool calls the agent made during the session.
    pub tool_calls: u32,
    /// Whether the transcript records a harness or endpoint failure
    /// underneath the agent (connection error, provider error).
    pub harness_error: bool,
}

/// What a no-output run was, from its own evidence. The four separable
/// causes demand different responses, and with the transcript
/// destroyed they are one status string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoOutputCause {
    /// The model spent the whole budget in reasoning and emitted
    /// nothing: `finish=length` with empty content. Retry with a larger
    /// budget.
    ReasoningBurn,
    /// The agent read for the whole run and never decided to act: no
    /// tool calls in the transcript. A blind re-dispatch repeats it;
    /// escalate.
    ReadWithoutActing,
    /// The agent worked correctly and concluded no change was needed:
    /// tool calls happened and the run settled cleanly. Do not retry.
    NoChangeNeeded,
    /// The harness or endpoint failed underneath the agent. Retry
    /// against a different pool member.
    HarnessFailure,
    /// The transcript is missing: the causes are not separable from
    /// what the run left behind.
    Indeterminate,
}

impl NoOutputCause {
    /// Whether a re-dispatch can do better than the run that just spent
    /// its GPU hours. `Indeterminate` is not retry-worthy: the retry
    /// decision is only made on a separable cause, never on a guess.
    pub fn retry_worthy(self) -> bool {
        matches!(self, Self::ReasoningBurn | Self::HarnessFailure)
    }
}

/// Diagnose a no-output run from the terminal state and, when it
/// survived, the transcript.
///
/// With the transcript gone the answer is [`NoOutputCause::Indeterminate`]
/// — the four causes collapse to one status string, which is exactly
/// the incident. Precedence: a harness failure underneath the agent
/// explains the no-output regardless of the finish token;
/// `finish=length` is the budget-burn signature; tool calls mean the
/// agent acted and concluded, their absence that it never did.
pub fn classify_no_output(
    term: &Terminal,
    transcript: Option<&TranscriptEvidence>,
) -> NoOutputCause {
    let Some(evidence) = transcript else {
        return NoOutputCause::Indeterminate;
    };
    if evidence.harness_error {
        return NoOutputCause::HarnessFailure;
    }
    if matches!(term.finish_reason, Some(FinishReason::Length)) {
        return NoOutputCause::ReasoningBurn;
    }
    if evidence.tool_calls > 0 {
        return NoOutputCause::NoChangeNeeded;
    }
    NoOutputCause::ReadWithoutActing
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_record(run_id: &str) -> StatusRecord {
        StatusRecord::start(run_id, "http://w1:8080", "m", 1_000).unwrap()
    }

    fn settled(record: StatusRecord, status: &str, rc: i32) -> StatusRecord {
        let mut record = record;
        record
            .finish(Terminal::new(status, rc, record.started_at + 100).unwrap())
            .unwrap();
        record
    }

    #[test]
    fn start_requires_run_id_endpoint_and_model() {
        assert!(StatusRecord::start("", "http://w1", "m", 0).is_none());
        assert!(StatusRecord::start("issue-1", "  ", "m", 0).is_none());
        assert!(StatusRecord::start("issue-1", "http://w1", "", 0).is_none());
        let record = StatusRecord::start("issue-1", "http://w1:8080", "m", 42).unwrap();
        assert_eq!(record.endpoint(), "http://w1:8080");
        assert_eq!(record.attempt_number(), 1);
        assert!(record.is_running());
    }

    #[test]
    fn running_line_carries_status_endpoint_model_and_liveness() {
        let mut record = running_record("issue-3590");
        let line = record.line();
        assert!(line.starts_with("status=RUNNING endpoint=http://w1:8080 model=m attempt=1 "));
        assert!(!line.contains("last_heartbeat="));
        record.heartbeat(1_100, 4521).unwrap();
        let line = record.line();
        assert!(line.contains("last_heartbeat="));
        assert!(line.contains("counter=4521"));
    }

    #[test]
    fn terminal_line_carries_finish_and_token_fields() {
        let mut record = running_record("issue-51");
        let mut term = Terminal::new("NO-OUTPUT", 0, 7_416).unwrap();
        term.finish_reason = Some(FinishReason::Length);
        term.input_tokens = Some(131_072);
        term.output_tokens = Some(8_192);
        record.finish(term).unwrap();
        let line = record.line();
        assert!(line.starts_with("status=NO-OUTPUT endpoint=http://w1:8080 model=m attempt=1 "));
        assert!(line.contains("agent_rc=0"));
        assert!(line.contains("finish_reason=length"));
        assert!(line.contains("input_tokens=131072"));
        assert!(line.contains("output_tokens=8192"));
        assert!(line.ends_with("transcript=no"));
    }

    #[test]
    fn terminal_with_empty_status_is_refused() {
        assert!(Terminal::new("  ", 0, 0).is_none());
        assert!(Terminal::new("NO-OUTPUT", 0, 0).is_some());
    }

    #[test]
    fn finish_reason_parse_fails_closed() {
        assert_eq!(FinishReason::parse("length").unwrap(), FinishReason::Length);
        assert_eq!(FinishReason::Length.as_str(), "length");
        assert!(FinishReason::parse("budget-exhausted").is_err());
    }

    #[test]
    fn transitions_after_settle_are_refused() {
        let mut record = settled(running_record("issue-1"), "VERIFIED", 0);
        assert!(!record.is_running());
        assert_eq!(
            record.redeploy("http://w2:8080", 2_000),
            Err(LifecycleError::AlreadySettled)
        );
        assert_eq!(
            record.heartbeat(2_000, 1),
            Err(LifecycleError::AlreadySettled)
        );
        assert_eq!(
            record.finish(Terminal::new("BUILD-FAIL", 101, 2_000).unwrap()),
            Err(LifecycleError::AlreadySettled)
        );
        let mut fresh = running_record("issue-2");
        assert_eq!(
            fresh.redeploy("  ", 2_000),
            Err(LifecycleError::EmptyEndpoint)
        );
    }

    #[test]
    fn liveness_distinguishes_the_four_states() {
        // No line at all: the state a supervisor cannot read.
        assert_eq!(
            liveness(&running_record("issue-1"), 5_000, DEFAULT_STALE_AFTER_SECS),
            Liveness::Unrecorded
        );
        // A line whose counter advances: work is demonstrably happening.
        let mut record = running_record("issue-2");
        record.heartbeat(4_900, 100).unwrap();
        assert_eq!(liveness(&record, 5_000, 600), Liveness::Progressing);
        // A line whose counter does not advance: alive, progress unproven.
        let mut record = running_record("issue-3");
        record.heartbeat(4_000, 100).unwrap();
        record.heartbeat(4_900, 100).unwrap();
        assert_eq!(liveness(&record, 5_000, 600), Liveness::Alive);
        // Silence past the window: a real signal, with the count.
        let mut record = running_record("issue-4");
        record.heartbeat(1_000, 100).unwrap();
        assert_eq!(
            liveness(&record, 5_000, 600),
            Liveness::Stalled { silent_secs: 4_000 }
        );
        // A clock rewind is zero silence, never stalled.
        assert_eq!(liveness(&record, 999, 600), Liveness::Progressing);
        // Settled: the terminal record is the evidence.
        assert_eq!(
            liveness(
                &settled(running_record("issue-5"), "VERIFIED", 0),
                5_000,
                600
            ),
            Liveness::Settled
        );
    }

    #[test]
    fn unrecorded_finding_fires_only_on_running_records_without_lines() {
        let record = running_record("issue-30");
        let finding = unrecorded_liveness_finding(&record).unwrap();
        assert!(finding.starts_with("LIVENESS_UNRECORDED:"));
        assert!(finding.contains("issue-30"));
        let mut record = running_record("issue-31");
        record.heartbeat(1_001, 5).unwrap();
        assert!(unrecorded_liveness_finding(&record).is_none());
        assert!(
            unrecorded_liveness_finding(&settled(running_record("issue-32"), "VERIFIED", 0))
                .is_none()
        );
    }

    #[test]
    fn audit_dispatch_flags_unprobed_and_past_refusal() {
        assert_eq!(audit_dispatch(None, false), DispatchAudit::Sound);
        assert_eq!(
            audit_dispatch(Some(StartProbe::Reachable), true),
            DispatchAudit::Sound
        );
        assert_eq!(audit_dispatch(None, true), DispatchAudit::Unprobed);
        assert_eq!(
            audit_dispatch(Some(StartProbe::Unreachable), true),
            DispatchAudit::PastRefusal
        );
        assert!(audit_dispatch(None, true)
            .warn_line()
            .unwrap()
            .contains("without a reachability probe"));
        assert!(audit_dispatch(Some(StartProbe::Unreachable), true)
            .warn_line()
            .unwrap()
            .contains("unreachable"));
        assert!(DispatchAudit::Sound.warn_line().is_none());
    }

    #[test]
    fn classify_loss_splits_fast_loss_from_preemption() {
        // No lines, dead inside the window: unreachable at start.
        let mut record = running_record("issue-a");
        record
            .finish(Terminal::new("connection-error", 1, 1_025).unwrap())
            .unwrap();
        assert_eq!(
            classify_loss(&record, DEFAULT_FAIL_FAST_WINDOW_SECS),
            Some(LossClass::UnreachableAtStart)
        );
        // No lines, but 3 h 03 m elapsed: the endpoint was reachable at
        // start; this is preemption, and the missing lines are a
        // separate finding.
        let mut record = running_record("issue-b");
        record
            .finish(Terminal::new("connection-error", 1, 1_000 + 10_983).unwrap())
            .unwrap();
        assert_eq!(
            classify_loss(&record, DEFAULT_FAIL_FAST_WINDOW_SECS),
            Some(LossClass::PreemptedMidRun)
        );
        // Any liveness line means the endpoint answered: preemption.
        let mut record = running_record("issue-c");
        record.heartbeat(1_010, 7).unwrap();
        record
            .finish(Terminal::new("connection-error", 1, 1_020).unwrap())
            .unwrap();
        assert_eq!(
            classify_loss(&record, DEFAULT_FAIL_FAST_WINDOW_SECS),
            Some(LossClass::PreemptedMidRun)
        );
        // Not a connection loss: the classification does not apply.
        let record = settled(running_record("issue-d"), "NO-OUTPUT", 0);
        assert_eq!(classify_loss(&record, DEFAULT_FAIL_FAST_WINDOW_SECS), None);
        // Not settled yet: nothing to classify.
        assert_eq!(
            classify_loss(&running_record("issue-e"), DEFAULT_FAIL_FAST_WINDOW_SECS),
            None
        );
    }

    #[test]
    fn reselect_picks_a_different_peer_and_fails_closed() {
        let peers = vec![
            "http://w3:8080".to_string(),
            "http://w1:8080".to_string(),
            "http://w2:8080".to_string(),
        ];
        // The failed worker is excluded; the choice is deterministic.
        assert_eq!(
            reselect("http://w1:8080", &peers).unwrap(),
            "http://w2:8080"
        );
        // Only the dead worker remains: a stale directory, not a fleet.
        assert_eq!(
            reselect("http://w1:8080", &["http://w1:8080".to_string()]),
            Err(ReResolutionRefusal::NoHealthyPeer {
                failed: "http://w1:8080".to_string()
            })
        );
        assert_eq!(
            reselect("http://w1:8080", &[]),
            Err(ReResolutionRefusal::NoHealthyPeer {
                failed: "http://w1:8080".to_string()
            })
        );
        // The refusal names the endpoint it refuses to re-grant.
        let refusal = ReResolutionRefusal::NoHealthyPeer {
            failed: "http://w1:8080".to_string(),
        };
        assert!(refusal.line().contains("http://w1:8080"));
        assert!(refusal.line().starts_with("FATAL:"));
    }

    #[test]
    fn same_endpoint_retries_flags_only_consecutive_retries() {
        let mut record = running_record("issue-1");
        assert!(same_endpoint_retries(&record).is_empty());
        record.redeploy("http://w2:8080", 2_000).unwrap();
        assert!(same_endpoint_retries(&record).is_empty());
        // A retry to the endpoint the previous attempt just lost.
        record.redeploy("http://w2:8080", 3_000).unwrap();
        assert_eq!(same_endpoint_retries(&record), vec![3]);
        // Returning to an earlier endpoint later is legitimate (the
        // worker was re-registered).
        record.redeploy("http://w1:8080", 4_000).unwrap();
        assert_eq!(same_endpoint_retries(&record), vec![3]);
    }

    #[test]
    fn tally_denominator_is_dispatched_never_records() {
        let dispatched = vec!["r1".to_string(), "r2".to_string(), "r3".to_string()];
        let mut records = BTreeMap::new();
        records.insert(
            "r1".to_string(),
            settled(running_record("r1"), "VERIFIED", 0),
        );
        records.insert(
            "r2".to_string(),
            settled(running_record("r2"), "NO-OUTPUT", 0),
        );
        records.insert("r3".to_string(), running_record("r3"));
        let out = tally(&dispatched, &records);
        assert_eq!(
            out,
            Tally {
                dispatched: 3,
                missing: 0,
                stale_running: 1,
                verified: 1,
                failed: 1
            }
        );
        assert!(out.reconciles());
        assert_eq!(
            out.line(),
            "WARN: tally: 1/3 verified; 2 failed (stale-running=1, terminal=1)"
        );
        // All green stays bare; the alias PASS resolves to VERIFIED.
        let mut records = BTreeMap::new();
        records.insert("r1".to_string(), settled(running_record("r1"), "PASS", 0));
        let out = tally(&dispatched, &records);
        assert_eq!(out.verified, 1);
        assert_eq!(out.missing + out.stale_running, 2);
        assert_eq!(
            out.line(),
            "WARN: tally: 1/3 verified; 2 failed (no-record=2)"
        );
        // An empty population is not 0/0-verified, it is no runs.
        let out = tally(&[], &records);
        assert_eq!(out.line(), "tally: no runs dispatched");
    }

    #[test]
    fn transcript_policy_is_the_floor_the_issue_names() {
        // No-output, rc 0: required.
        let term = Terminal::new("NO-OUTPUT", 0, 100).unwrap();
        match transcript_policy(&term) {
            TranscriptPolicy::Required { reason } => assert!(reason.contains("NO-OUTPUT")),
            other => panic!("no-output must require the transcript, got {other:?}"),
        }
        // Stall-killed: required, and the reason names it.
        let term = Terminal::new("NO-OUTPUT", STALL_KILL_RC, 100).unwrap();
        assert!(term.is_stall_killed());
        match transcript_policy(&term) {
            TranscriptPolicy::Required { reason } => assert!(reason.contains("stall-killed")),
            other => panic!("stall-killed must require the transcript, got {other:?}"),
        }
        // Non-zero rc under any status: required.
        let term = Terminal::new("BUILD-FAIL", 101, 100).unwrap();
        assert!(matches!(
            transcript_policy(&term),
            TranscriptPolicy::Required { .. }
        ));
        // Green with a clean exit: optional.
        let term = Terminal::new("VERIFIED", 0, 100).unwrap();
        assert_eq!(transcript_policy(&term), TranscriptPolicy::Optional);
    }

    #[test]
    fn transcript_verdict_flags_destroyed_evidence() {
        let term = Terminal::new("NO-OUTPUT", 0, 100).unwrap();
        match transcript_verdict(&term) {
            TranscriptVerdict::DestroyedEvidence { reason } => {
                assert!(reason.contains("NO-OUTPUT"));
            }
            other => panic!("no-output without transcript must destroy evidence, got {other:?}"),
        }
        let mut term = Terminal::new("NO-OUTPUT", 0, 100).unwrap();
        term.transcript_copied = true;
        assert_eq!(transcript_verdict(&term), TranscriptVerdict::Kept);
        let term = Terminal::new("VERIFIED", 0, 100).unwrap();
        assert_eq!(transcript_verdict(&term), TranscriptVerdict::Kept);
    }

    #[test]
    fn no_output_causes_separate_when_the_transcript_survived() {
        let burn = Terminal {
            finish_reason: Some(FinishReason::Length),
            ..Terminal::new("NO-OUTPUT", 0, 100).unwrap()
        };
        assert_eq!(
            classify_no_output(&burn, Some(&TranscriptEvidence::default())),
            NoOutputCause::ReasoningBurn
        );
        let clean = Terminal::new("NO-OUTPUT", 0, 100).unwrap();
        assert_eq!(
            classify_no_output(
                &clean,
                Some(&TranscriptEvidence {
                    tool_calls: 12,
                    harness_error: false
                })
            ),
            NoOutputCause::NoChangeNeeded
        );
        assert_eq!(
            classify_no_output(&clean, Some(&TranscriptEvidence::default())),
            NoOutputCause::ReadWithoutActing
        );
        // A harness failure underneath explains the no-output first.
        assert_eq!(
            classify_no_output(
                &clean,
                Some(&TranscriptEvidence {
                    tool_calls: 12,
                    harness_error: true
                })
            ),
            NoOutputCause::HarnessFailure
        );
        // Without the transcript the four collapse to one: not a guess.
        assert_eq!(
            classify_no_output(&burn, None),
            NoOutputCause::Indeterminate
        );
        assert!(!NoOutputCause::Indeterminate.retry_worthy());
        assert!(NoOutputCause::ReasoningBurn.retry_worthy());
        assert!(NoOutputCause::HarnessFailure.retry_worthy());
        assert!(!NoOutputCause::ReadWithoutActing.retry_worthy());
        assert!(!NoOutputCause::NoChangeNeeded.retry_worthy());
    }
}
