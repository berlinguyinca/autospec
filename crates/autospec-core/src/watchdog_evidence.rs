//! A watchdog that kills must first capture why (issue #3792).
//!
//! The incident: InferWeave #45 — a keystone unblocking 81 downstream issues
//! — ran 46 minutes and produced nothing:
//!
//! ```text
//! status=NO-OUTPUT  agent_rc=143  agent_secs=2761  changed_files=0
//! ```
//!
//! Its whole log was 2,903 bytes and ended with
//! `STALLED: no session or output activity for 45m after 46m; terminating
//! agent`. The watchdog detected the stall reliably and terminated cleanly,
//! and it recorded nothing about the cause. Reconstructing from outside
//! afterwards — the endpoint it was given (`qwen3.8-27b-22771168`) was
//! healthy and still running seven hours later; the neighbouring jobs on
//! that worker ended at 07:59 and began at 08:50, so it was not
//! oversubscribed during the 08:03–08:49 window; a sibling agent dispatched
//! in the same second to a different endpoint completed normally in 673 s —
//! still does not answer the question. Was the endpoint reachable from that
//! node? Did the agent ever issue a request? Did it get a response and fail
//! to write? Was `agent.out` ever created, or was the watchdog testing a
//! path that never existed? Every one of those was answerable at the instant
//! of termination and is unanswerable now.
//!
//! The evidence exists only while the process does; killing it destroys the
//! only opportunity to collect it. So termination is preceded by a capture
//! step whose output is written to durable storage **before** the signal is
//! sent, and the capture includes all of it, each piece cheap and available
//! then:
//!
//! 1. **Liveness of every external dependency the process was given** — a
//!    probe of the endpoint: one request, its status and latency. That
//!    distinguishes "the model never answered" from "the agent never
//!    asked" ([`Probe`]).
//! 2. **The state of the artifacts the watchdog was watching** — whether the
//!    watched path exists at all, and its size and mtime. A watchdog firing
//!    on a file that was never created is a different failure from one
//!    firing on a file that stopped growing ([`ArtifactState`]).
//! 3. **The tail of whatever the agent did write, wherever it wrote it**
//!    ([`OutputTail`]).
//! 4. **The process state at kill time** — running, sleeping, blocked on
//!    I/O — which separates a hung network read from a spinning loop
//!    ([`ProcessState`]).
//!
//! The status a terminated run records names the evidence file (AC3), and
//! `NO-OUTPUT` is never a terminal classification on its own: it is
//! accompanied by the captured cause or by an explicit statement that
//! capture failed (AC4).
//!
//! The module is pure: the caller probes the dependencies and inspects the
//! artifacts, reports what it observed, and persists the rendered record;
//! this code never spawns, never reads a clock.

use serde::{Deserialize, Serialize};

/// The default evidence filename suffix, joined onto the run id by
/// [`evidence_file_path`]: the run's evidence file is
/// `<dir>/run-<id>-stall-evidence.txt`.
pub const EVIDENCE_SUFFIX: &str = "stall-evidence.txt";

// --- The capture: what the caller must have observed -----------------------

/// The state of the watched artifact at kill time.
///
/// A watchdog firing on a file that was never created is a different
/// failure from one firing on a file that stopped growing; the record must
/// carry enough to tell them apart. `mtime` is the observation the caller
/// reported (e.g. the `find -newermt` timestamp), not a value read here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArtifactState {
    /// The watched path exists: its size and mtime, as observed.
    Present { size_bytes: u64, mtime: String },
    /// The watched path does not exist: the watchdog was testing a path
    /// that was never created.
    Missing,
}

impl ArtifactState {
    /// One line of the evidence record.
    pub fn line(&self, path: &str) -> String {
        match self {
            ArtifactState::Present { size_bytes, mtime } => {
                format!("{path}: present, {size_bytes} bytes, mtime {mtime}")
            }
            ArtifactState::Missing => format!("{path}: missing — never created"),
        }
    }
}

/// The last lines of whatever the agent did write, and where it wrote them.
///
/// The agent may write to a different path than the one the watchdog was
/// watching; the capture takes the tail from wherever output actually
/// landed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputTail {
    /// The path the tail was taken from.
    pub source: String,
    /// The last lines, in order. Empty is a legitimate capture: the agent
    /// wrote nothing anywhere.
    pub lines: Vec<String>,
}

impl OutputTail {
    /// The tail as one line of the evidence record: the lines joined by `; `,
    /// or an explicit empty marker.
    pub fn line(&self) -> String {
        if self.lines.is_empty() {
            format!("tail of {}: (empty)", self.source)
        } else {
            format!("tail of {}: {}", self.source, self.lines.join("; "))
        }
    }
}

/// The process state at kill time. Separates a hung network read
/// (sleeping or blocked on I/O) from a spinning loop (running).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Sleeping,
    BlockedIo,
    Zombie,
    Unknown,
}

impl ProcessState {
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessState::Running => "running",
            ProcessState::Sleeping => "sleeping",
            ProcessState::BlockedIo => "blocked on I/O",
            ProcessState::Zombie => "zombie",
            ProcessState::Unknown => "unknown",
        }
    }
}

/// One request to a dependency: its status and latency, or the reason it
/// did not respond.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ProbeOutcome {
    /// The dependency answered one request: the model did answer.
    Responded { status: u16, latency_ms: u64 },
    /// The dependency did not respond, with the observed reason: the model
    /// never answered.
    Unreachable { reason: String },
}

/// An external dependency the process was given (the model endpoint, a
/// registry, an upstream host) and the result of probing it at kill time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    /// The dependency's name, as the process was given it — e.g.
    /// `qwen3.8-27b-22771168`.
    pub dependency: String,
    pub outcome: ProbeOutcome,
}

impl Probe {
    /// Whether the probe found the dependency unreachable.
    pub fn is_unreachable(&self) -> bool {
        matches!(self.outcome, ProbeOutcome::Unreachable { .. })
    }

    /// One line of the evidence record.
    pub fn line(&self) -> String {
        match &self.outcome {
            ProbeOutcome::Responded { status, latency_ms } => format!(
                "probe {} → HTTP {status} in {latency_ms} ms",
                self.dependency
            ),
            ProbeOutcome::Unreachable { reason } => {
                format!("probe {} → unreachable ({reason})", self.dependency)
            }
        }
    }
}

/// The complete evidence a watchdog captures before it kills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The run being terminated.
    pub run_id: String,
    /// Liveness of every external dependency the run was given. A capture
    /// with no probes is a capture of nothing: it is rejected by
    /// [`capture`], not defaulted.
    pub probes: Vec<Probe>,
    /// The path the watchdog was watching.
    pub watched_path: String,
    /// The state of that path at kill time.
    pub watched: ArtifactState,
    /// The tail of whatever the run did write, wherever it wrote it.
    pub tail: OutputTail,
    /// The process state at kill time.
    pub state: ProcessState,
}

impl Evidence {
    /// The first unreachable dependency, in probe order: the cause a
    /// stalled run most often had, named by the dependency the process was
    /// given.
    pub fn first_unreachable(&self) -> Option<&Probe> {
        self.probes.iter().find(|probe| probe.is_unreachable())
    }

    /// The primary cause, as one line. The precedence is the order the
    /// questions were asked in the incident investigation: an unreachable
    /// dependency is a cause by itself; a watched path that was never
    /// created is a different failure from one that stopped growing;
    /// otherwise the artifact stalled with every dependency answering.
    pub fn cause_line(&self) -> String {
        if let Some(probe) = self.first_unreachable() {
            return format!(
                "unreachable dependency {}: {}",
                probe.dependency,
                match &probe.outcome {
                    ProbeOutcome::Unreachable { reason } => reason.as_str(),
                    _ => unreachable!("first_unreachable only yields Unreachable probes"),
                }
            );
        }
        match &self.watched {
            ArtifactState::Missing => format!(
                "watched path {} was never created — the watchdog was testing a path that never existed",
                self.watched_path
            ),
            ArtifactState::Present { .. } => format!(
                "watched path {} stopped growing with every probed dependency answering",
                self.watched_path
            ),
        }
    }

    /// The record as it goes to durable storage: the header, the watched
    /// artifact state, one line per dependency probe, the output tail, and
    /// the process state at kill time.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("run {}\n", self.run_id));
        out.push_str(&format!(
            "watched: {}\n",
            self.watched.line(&self.watched_path)
        ));
        out.push_str("probes:\n");
        for probe in &self.probes {
            out.push_str(&format!("  {}\n", probe.line()));
        }
        out.push_str(&format!("  {}\n", self.tail.line()));
        out.push_str(&format!("process state at kill: {}\n", self.state.as_str()));
        out
    }
}

/// What went wrong with the capture, when the capture went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureError {
    /// The run id is empty: the evidence file would be unaddressable.
    EmptyRunId,
    /// No dependency was probed: a capture that does not probe the
    /// dependencies the run was given is a capture of nothing.
    NoDependencyProbes,
    /// The watched path is not named: the capture cannot state whether the
    /// path was ever created.
    EmptyWatchedPath,
}

impl CaptureError {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureError::EmptyRunId => "run id is empty: the evidence file would be unaddressable",
            CaptureError::NoDependencyProbes => {
                "no dependency was probed: the capture must cover every external dependency the run was given"
            }
            CaptureError::EmptyWatchedPath => {
                "the watched path is not named: the capture cannot state whether it was ever created"
            }
        }
    }
}

/// The default evidence file for a run: `<dir>/run-<id>-stall-evidence.txt`.
pub fn evidence_file_path(dir: &str, run_id: &str) -> String {
    format!("{dir}/run-{run_id}-{EVIDENCE_SUFFIX}")
}

/// Assemble the capture for a stalled run and name the file its record
/// goes to.
///
/// This is the caller's pre-kill step: the caller probes the dependencies,
/// inspects the watched artifact and the output, calls this, and writes
/// the rendered record to the returned path **before** the signal is sent.
/// Refusals here are the same discipline as elsewhere in the repo: the
/// capture names what would tell it, rather than guessing.
pub fn capture(dir: &str, evidence: &Evidence) -> Result<(Evidence, String), CaptureError> {
    if evidence.run_id.is_empty() {
        return Err(CaptureError::EmptyRunId);
    }
    if evidence.probes.is_empty() {
        return Err(CaptureError::NoDependencyProbes);
    }
    if evidence.watched_path.is_empty() {
        return Err(CaptureError::EmptyWatchedPath);
    }
    Ok((evidence.clone(), evidence_file_path(dir, &evidence.run_id)))
}

// --- The order: capture first, signal second --------------------------------

/// Whether the capture and the signal happened in the order the invariant
/// requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationOrder {
    /// The evidence record was written to durable storage before the
    /// signal was sent: the invariant holds.
    CaptureFirst,
    /// The signal was sent before the evidence was durable: the incident.
    /// The kill may have been justified — the stall was real — and the
    /// evidence it destroyed is unrecoverable by definition.
    KilledBeforeCapture,
}

/// Invariant 1: classify the order of the two events.
///
/// `evidence_durable` is whether the rendered record was written to durable
/// storage; `before_signal` is whether that write completed before the
/// signal was sent.
pub fn termination_order(evidence_durable: bool, before_signal: bool) -> TerminationOrder {
    if evidence_durable && before_signal {
        TerminationOrder::CaptureFirst
    } else {
        TerminationOrder::KilledBeforeCapture
    }
}

/// Whether a termination order may be recorded as a compliant kill.
pub fn capture_gate(order: TerminationOrder) -> bool {
    order == TerminationOrder::CaptureFirst
}

// --- The record: the status names its evidence ------------------------------

/// What a terminated run records as the cause behind its status.
///
/// `NO-OUTPUT` is never a terminal classification on its own (AC4): the
/// record carries either the captured cause or an explicit statement that
/// capture failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum Cause {
    /// The capture was taken and persisted; `line` is its primary cause
    /// ([`Evidence::cause_line`]).
    Captured { evidence: Evidence, line: String },
    /// The capture was attempted and failed; the failure is stated, not
    /// swallowed.
    CaptureFailed { reason: String },
}

impl Cause {
    pub fn is_explicit(&self) -> bool {
        match self {
            Cause::Captured { .. } => true,
            Cause::CaptureFailed { reason } => !reason.trim().is_empty(),
        }
    }
}

/// The status a terminated run records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRecord {
    /// The run's status, e.g. `NO-OUTPUT`.
    pub status: String,
    /// The cause accompanying the status; `None` is a bare classification.
    pub cause: Option<Cause>,
    /// The evidence file the record names.
    pub evidence_file: Option<String>,
}

/// How a recorded status fares against AC4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusVerdict {
    /// The status is accompanied by a named cause: terminal.
    Accompanied,
    /// `NO-OUTPUT` on its own, with neither a captured cause nor an
    /// explicit capture failure: the terminal classification AC4 forbids.
    BareNoOutput,
}

/// AC4 gate: `NO-OUTPUT` is never a terminal classification on its own.
///
/// It is accompanied by the captured cause, or by an explicit statement
/// that capture failed. Every other status is out of scope here: this gate
/// is about the one classification the incident turned into a non-answer.
pub fn status_gate(status: &str, cause: Option<&Cause>) -> StatusVerdict {
    if status != "NO-OUTPUT" {
        return StatusVerdict::Accompanied;
    }
    match cause {
        Some(cause) if cause.is_explicit() => StatusVerdict::Accompanied,
        _ => StatusVerdict::BareNoOutput,
    }
}

impl StatusRecord {
    /// Whether this record may be recorded as the run's terminal status.
    pub fn gate(&self) -> StatusVerdict {
        status_gate(&self.status, self.cause.as_ref())
    }

    /// The recorded status line.
    ///
    /// The incident's line — `status=NO-OUTPUT agent_rc=143
    /// agent_secs=2761 changed_files=0` — is kept and extended: AC3
    /// requires the line to name the evidence file, and a `NO-OUTPUT`
    /// status renders its accompanying cause on the same line.
    pub fn render(&self) -> String {
        let mut line = format!("status={}", self.status);
        if let Some(file) = &self.evidence_file {
            line.push_str(&format!(" evidence={file}"));
        }
        if let Some(cause) = &self.cause {
            line.push_str(&format!(
                " cause={}",
                match cause {
                    Cause::Captured { line, .. } => line.clone(),
                    Cause::CaptureFailed { reason } => format!("capture failed: {reason}"),
                }
            ));
        }
        line
    }
}
