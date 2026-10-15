//! Fleet dispatch status classification (#3918).
//!
//! A dispatch batch sends issues to a fleet of agent runners. Each runner
//! leaves one observable fact pair behind: a wall-clock duration and a
//! transcript. This module turns those facts into status records, and a
//! batch of records into a summary the operator can act on.
//!
//! The defects addressed:
//!
//! 1. **Auth, endpoint, and context failures were classified as `NO-OUTPUT`.**
//!    A runner that died on a bad credential in two seconds was recorded the
//!    same as an agent that finished and legitimately changed nothing.
//!    `INFRA-FAIL` is now a distinct terminal status for those launch-time
//!    infrastructure failures, and it does **not** consume the issue's
//!    attempt count: the run never attempted the spec, so charging it an
//!    attempt burns the budget on infrastructure, not on work.
//! 2. **The duration floor.** A run that ends below the duration floor
//!    cannot have executed the spec, whatever its transcript says. It is
//!    `LAUNCH-FAIL` regardless of transcript content, and it also does not
//!    consume an attempt.
//! 3. **Transcripts under the size threshold are quoted verbatim** in the
//!    status record. Longer transcripts are recorded by byte count only, so
//!    a status line stays printable.
//! 4. **`agent_rc=0` with zero output is a contradiction, not a quiet
//!    success** (issue #3749). A runner that exited cleanly but produced
//!    nothing is the shape of a silent failure — the agent believed it
//!    succeeded while writing nothing. Such a record carries the
//!    [`NO_OUTPUT_RC_ZERO`] failure signature, and N of them in one batch
//!    raise the same `REPEATED_IDENTICAL_FAILURE` fault as N identical
//!    infrastructure failures. A nonzero exit with zero output stays a plain
//!    `NO-OUTPUT`: the agent died, which is plausible and not a
//!    contradiction.
//! 5. **Exit code 0 is not proof the agent ran** (spec 2026-10-14 §7 item 6).
//!    `agent_rc=0` with no session record, or zero completed assistant turns,
//!    is `AGENT-NEVER-STARTED` ([`never_started`]): a start failure that
//!    consumes no attempt. Records without the evidence fields keep their
//!    old classification.
//!
//! The batch layer: `summarize_batch` counts the statuses and raises a
//! fleet-level fault (`REPEATED_IDENTICAL_FAILURE`) when the same failure
//! (status + signature) repeats within one dispatch batch — N runners dying
//! on the same credential is one fault to fix, not N attempts to retry.
//! `idle_subfleet_lines` reports sub-fleets with zero running agents while
//! open eligible work is still queued for them.
//!
//! The reconciler's memory lives here too (issue #4211): a stateless
//! reconciler submitted the same impossible worker 102 times in 7 days
//! because nothing remembered that the previous 101 attempts died the same
//! way. [`ReconcilerMemory`] tracks consecutive start failures per claim,
//! suspends a claim once they cross a threshold, and renders the
//! "never converged" state — `want=1 up=0 (102 consecutive start failures,
//! last: unknown model)` — distinct from a plain "converging" line. The
//! threshold is the per-claim *rate* threshold that must be paired with the
//! per-attempt cost threshold [`FleetDispatchPolicy::duration_floor_secs`].

mod never_started;

use std::collections::BTreeMap;

use serde::de::{Error as DeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use never_started::{never_started, AGENT_NEVER_STARTED_STATUS};

/// Default wall-clock floor: a run shorter than this never executed the spec.
pub const DEFAULT_DURATION_FLOOR_SECS: u64 = 30;
/// Default transcript quote threshold in bytes.
pub const DEFAULT_TRANSCRIPT_QUOTE_BYTES: usize = 4096;
/// Default count of identical failures in one batch that raises a fleet fault.
pub const DEFAULT_REPEAT_FAULT_THRESHOLD: usize = 3;

/// Status: the agent ran and produced output. Spec success is judged downstream.
pub const OK_STATUS: &str = "OK";
/// Status: the agent ran its full duration and produced no output.
pub const NO_OUTPUT_STATUS: &str = "NO-OUTPUT";
/// Status: auth, endpoint, or context failure. Terminal; does not consume an attempt.
pub const INFRA_FAIL_STATUS: &str = "INFRA-FAIL";
/// Status: the run ended below the duration floor. Does not consume an attempt.
pub const LAUNCH_FAIL_STATUS: &str = "LAUNCH-FAIL";

/// Fleet fault code: identical failures repeated within one dispatch batch.
pub const REPEATED_IDENTICAL_FAILURE: &str = "REPEATED_IDENTICAL_FAILURE";
/// Sub-fleet report code: no agents running while eligible work is open.
pub const SUBFLEET_IDLE: &str = "SUBFLEET-IDLE";
/// Failure signature: the agent exited 0 but produced no output — a
/// contradiction that must be reported, not read as success (issue #3749).
pub const NO_OUTPUT_RC_ZERO: &str = "NO-OUTPUT-RC-ZERO";

/// The column order of the `agent-status.tsv` record.
pub const TSV_HEADER: &str =
    "issue\tstatus\tduration_secs\ttranscript_bytes\tfailure_signature\tconsumes_attempt\tquoted_transcript";

/// Terminal status of one agent run, from the fleet's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RunStatus {
    Ok,
    NoOutput,
    InfraFail,
    LaunchFail,
    AgentNeverStarted,
}

impl RunStatus {
    /// The status string as recorded in `agent-status.tsv` (hyphenated).
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Ok => OK_STATUS,
            RunStatus::NoOutput => NO_OUTPUT_STATUS,
            RunStatus::InfraFail => INFRA_FAIL_STATUS,
            RunStatus::LaunchFail => LAUNCH_FAIL_STATUS,
            RunStatus::AgentNeverStarted => AGENT_NEVER_STARTED_STATUS,
        }
    }

    /// Parse a status string. Accepts the hyphenated canonical form and the
    /// underscored serde-legacy form so old status records keep reading.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            OK_STATUS => Some(RunStatus::Ok),
            NO_OUTPUT_STATUS | "NO_OUTPUT" => Some(RunStatus::NoOutput),
            INFRA_FAIL_STATUS | "INFRA_FAIL" => Some(RunStatus::InfraFail),
            LAUNCH_FAIL_STATUS | "LAUNCH_FAIL" => Some(RunStatus::LaunchFail),
            AGENT_NEVER_STARTED_STATUS | "AGENT_NEVER_STARTED" => {
                Some(RunStatus::AgentNeverStarted)
            }
            _ => None,
        }
    }
}

impl Serialize for RunStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RunStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RunStatusVisitor;
        impl<'de> Visitor<'de> for RunStatusVisitor {
            type Value = RunStatus;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a run status string (OK, NO-OUTPUT, INFRA-FAIL, LAUNCH-FAIL)")
            }
            fn visit_str<E: DeError>(self, value: &str) -> Result<RunStatus, E> {
                RunStatus::parse(value)
                    .ok_or_else(|| E::custom(format_args!("unknown run status {value:?}")))
            }
        }
        deserializer.deserialize_str(RunStatusVisitor)
    }
}

/// Which infrastructure layer failed. Drives the failure signature so that
/// identical failures group together in the batch summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InfraCategory {
    /// Authentication / credential failure (401/403, invalid key, expired token).
    Auth,
    /// Endpoint / network failure (refused, unresolvable, unreachable).
    Endpoint,
    /// Model context failure (prompt too long, context window exceeded).
    Context,
}

impl InfraCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            InfraCategory::Auth => "auth",
            InfraCategory::Endpoint => "endpoint",
            InfraCategory::Context => "context",
        }
    }
}

/// Auth failure signatures, in priority order (first match wins).
const AUTH_SIGNATURES: &[&str] = &[
    "401 unauthorized",
    "403 forbidden",
    "invalid credentials",
    "invalid credential",
    "invalid api key",
    "incorrect api key",
    "invalid_api_key",
    "api key rejected",
    "authentication failed",
    "auth failed",
    "token expired",
    "token invalid",
    "not authenticated",
    "unauthorized",
    "access denied",
];

/// Endpoint failure signatures, in priority order.
const ENDPOINT_SIGNATURES: &[&str] = &[
    "connection refused",
    "connection reset",
    "failed to connect",
    "could not resolve",
    "name or service not known",
    "no route to host",
    "network unreachable",
    "dns lookup failed",
    "endpoint not found",
    "endpoint unavailable",
    "econnrefused",
    "econnreset",
];

/// Context failure signatures, in priority order.
const CONTEXT_SIGNATURES: &[&str] = &[
    "context length exceeded",
    "context_length_exceeded",
    "context length",
    "context_length",
    "maximum context",
    "max context tokens",
    "context window",
    "prompt is too long",
    "too many tokens",
    "range of input length",
];

/// One observable run, as the fleet controller sees it before classification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Issue identifier (e.g. `51`).
    pub issue: String,
    /// Wall-clock duration of the run in seconds.
    pub duration_secs: u64,
    /// The run's full transcript (stderr + final message).
    pub transcript: String,
    /// The agent's exit code when the runner reports it. Used to flag
    /// `agent_rc=0` with zero output as the [`NO_OUTPUT_RC_ZERO`] contradiction.
    #[serde(default)]
    pub agent_rc: Option<i32>,
    /// Whether the runner left a session record behind. `None` for records
    /// that predate the field; see [`never_started`].
    #[serde(default)]
    pub session_recorded: Option<bool>,
    /// Completed assistant turns (`stopReason:"error"` does not count).
    /// `None` for records that predate the field.
    #[serde(default)]
    pub assistant_turns: Option<u32>,
}

/// Tunables for classification and batch summarization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetDispatchPolicy {
    /// A run shorter than this many seconds is `LAUNCH-FAIL`.
    pub duration_floor_secs: u64,
    /// Transcripts of at most this many bytes are quoted verbatim in the record.
    pub transcript_quote_bytes: usize,
    /// Identical failures at or above this count in one batch raise a fleet fault.
    pub repeat_fault_threshold: usize,
}

impl Default for FleetDispatchPolicy {
    fn default() -> Self {
        Self {
            duration_floor_secs: DEFAULT_DURATION_FLOOR_SECS,
            transcript_quote_bytes: DEFAULT_TRANSCRIPT_QUOTE_BYTES,
            repeat_fault_threshold: DEFAULT_REPEAT_FAULT_THRESHOLD,
        }
    }
}

impl FleetDispatchPolicy {
    /// All three knobs must be positive; a zero floor or quote budget is a
    /// misconfiguration, not a degenerate-but-valid policy.
    pub fn new(
        duration_floor_secs: u64,
        transcript_quote_bytes: usize,
        repeat_fault_threshold: usize,
    ) -> Option<Self> {
        if duration_floor_secs == 0 || transcript_quote_bytes == 0 || repeat_fault_threshold == 0 {
            return None;
        }
        Some(Self {
            duration_floor_secs,
            transcript_quote_bytes,
            repeat_fault_threshold,
        })
    }
}

/// A classified run, ready for `agent-status.tsv` and the batch summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStatusRecord {
    pub issue: String,
    pub status: RunStatus,
    pub duration_secs: u64,
    /// Transcript length in bytes, always recorded.
    pub transcript_bytes: usize,
    /// `INFRA-FAIL/<category>/<signature>` for infra failures,
    /// [`NO_OUTPUT_RC_ZERO`] for a zero-output run whose agent exited 0,
    /// else `None`.
    pub failure_signature: Option<String>,
    /// The transcript verbatim when it fits the quote threshold, else `None`.
    pub quoted_transcript: Option<String>,
    /// Whether this run consumed the issue's attempt budget. `INFRA-FAIL`
    /// and `LAUNCH-FAIL` never do: the spec was never attempted.
    pub consumes_attempt: bool,
}

impl RunStatusRecord {
    /// One human-readable report line.
    pub fn line(&self) -> String {
        let mut line = format!(
            "{} issue={} duration={}s transcript={}b",
            self.status.as_str(),
            self.issue,
            self.duration_secs,
            self.transcript_bytes
        );
        if let Some(signature) = &self.failure_signature {
            line.push_str(&format!(" signature={signature}"));
        }
        if !self.consumes_attempt {
            line.push_str(" no-attempt");
        }
        if let Some(quoted) = &self.quoted_transcript {
            line.push_str(&format!(" transcript={:?}", escape_control(quoted)));
        }
        line
    }

    /// The start outcome this run represents, from the reconciler's point of
    /// view (issue #4211): a worker that never came up (`LAUNCH-FAIL`,
    /// `INFRA-FAIL`) is a start failure; one that ran (`OK`, `NO-OUTPUT`)
    /// started. A cancellation is not a run status and is fed to
    /// [`ReconcilerMemory`] directly as [`StartOutcome::Cancelled`].
    pub fn start_outcome(&self) -> StartOutcome {
        match self.status {
            RunStatus::LaunchFail | RunStatus::InfraFail | RunStatus::AgentNeverStarted => {
                StartOutcome::Failed {
                    reason: self
                        .failure_signature
                        .clone()
                        .unwrap_or_else(|| self.status.as_str().to_string()),
                }
            }
            RunStatus::Ok | RunStatus::NoOutput => StartOutcome::Started,
        }
    }

    /// One `agent-status.tsv` row. Control characters inside the quoted
    /// transcript are escaped so the file stays a single row per run.
    pub fn tsv_line(&self) -> String {
        let signature = self
            .failure_signature
            .clone()
            .unwrap_or_else(|| "-".to_string());
        let quoted = self
            .quoted_transcript
            .as_ref()
            .map(|quoted| escape_control(quoted))
            .unwrap_or_else(|| "-".to_string());
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.issue,
            self.status.as_str(),
            self.duration_secs,
            self.transcript_bytes,
            signature,
            self.consumes_attempt,
            quoted
        )
    }
}

/// Escape the control characters that would break a TSV row or a report line.
fn escape_control(text: &str) -> String {
    text.replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Match a transcript against the infrastructure failure signatures.
///
/// Categories are checked auth -> endpoint -> context; within a category the
/// first signature listed wins. The matched literal is returned so the
/// failure signature is stable and groupable.
pub fn infra_signature(transcript: &str) -> Option<(InfraCategory, &'static str)> {
    let lowered = transcript.to_lowercase();
    let probe = |category: InfraCategory,
                 table: &'static [&'static str]|
     -> Option<(InfraCategory, &'static str)> {
        table
            .iter()
            .copied()
            .find(|signature| lowered.contains(signature))
            .map(|signature| (category, signature))
    };
    probe(InfraCategory::Auth, AUTH_SIGNATURES)
        .or_else(|| probe(InfraCategory::Endpoint, ENDPOINT_SIGNATURES))
        .or_else(|| probe(InfraCategory::Context, CONTEXT_SIGNATURES))
}

/// Classify one run. Precedence: the duration floor first (a short run never
/// ran, whatever it printed), then infrastructure signatures, then
/// `AGENT-NEVER-STARTED` evidence, then the transcript's emptiness.
pub fn classify_run(run: &RunRecord, policy: &FleetDispatchPolicy) -> RunStatusRecord {
    let transcript_bytes = run.transcript.len();
    let quoted_transcript =
        (transcript_bytes <= policy.transcript_quote_bytes).then(|| run.transcript.clone());

    if run.duration_secs < policy.duration_floor_secs {
        return RunStatusRecord {
            issue: run.issue.clone(),
            status: RunStatus::LaunchFail,
            duration_secs: run.duration_secs,
            transcript_bytes,
            failure_signature: None,
            quoted_transcript,
            consumes_attempt: false,
        };
    }

    if let Some((category, signature)) = infra_signature(&run.transcript) {
        return RunStatusRecord {
            issue: run.issue.clone(),
            status: RunStatus::InfraFail,
            duration_secs: run.duration_secs,
            transcript_bytes,
            failure_signature: Some(format!(
                "{}/{}/{}",
                INFRA_FAIL_STATUS,
                category.as_str(),
                signature
            )),
            quoted_transcript,
            consumes_attempt: false,
        };
    }

    if never_started(run.agent_rc, run.session_recorded, run.assistant_turns) {
        return RunStatusRecord {
            issue: run.issue.clone(),
            status: RunStatus::AgentNeverStarted,
            duration_secs: run.duration_secs,
            transcript_bytes,
            failure_signature: Some(AGENT_NEVER_STARTED_STATUS.to_string()),
            quoted_transcript,
            consumes_attempt: false,
        };
    }

    let status = if run.transcript.trim().is_empty() {
        RunStatus::NoOutput
    } else {
        RunStatus::Ok
    };
    // A clean exit with zero output is a contradiction: the agent reported
    // success while writing nothing. Sign it so it is reported (and groups
    // into a fleet fault) instead of read as a quiet success. A nonzero exit
    // with no output is a plausible failure and stays unsigned.
    let failure_signature = if status == RunStatus::NoOutput && run.agent_rc == Some(0) {
        Some(NO_OUTPUT_RC_ZERO.to_string())
    } else {
        None
    };
    RunStatusRecord {
        issue: run.issue.clone(),
        status,
        duration_secs: run.duration_secs,
        transcript_bytes,
        failure_signature,
        quoted_transcript,
        consumes_attempt: true,
    }
}

/// A fleet-level fault raised by a dispatch batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetFault {
    /// Always [`REPEATED_IDENTICAL_FAILURE`] today; a code so new fault
    /// kinds can join without breaking the record shape.
    pub code: String,
    pub status: RunStatus,
    /// The shared signature of the repeated failures.
    pub signature: String,
    pub count: usize,
    /// The issues affected, in batch order.
    pub issues: Vec<String>,
}

impl FleetFault {
    /// One report line: this is one fault to fix, not N retries to absorb.
    pub fn line(&self) -> String {
        format!(
            "FLEET-FAULT code={} status={} signature={} count={} issues={} — repeated identical failure in one dispatch batch; treat as one fleet-level fault and stop re-dispatching until it is fixed",
            self.code,
            self.status.as_str(),
            self.signature,
            self.count,
            self.issues.join(",")
        )
    }
}

/// The summary of one dispatch batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchSummary {
    pub ok: usize,
    pub no_output: usize,
    pub infra_fail: usize,
    pub launch_fail: usize,
    /// Runs classified `AGENT-NEVER-STARTED`; absent in older summaries.
    #[serde(default)]
    pub never_started: usize,
    pub faults: Vec<FleetFault>,
}

impl BatchSummary {
    pub fn total(&self) -> usize {
        self.ok + self.no_output + self.infra_fail + self.launch_fail + self.never_started
    }

    /// Report lines: one summary line, then one line per fleet fault.
    pub fn lines(&self) -> Vec<String> {
        // `never-started` is printed only when nonzero so the established
        // line stays byte-identical for every batch without such runs.
        let never_started = match self.never_started {
            0 => String::new(),
            count => format!(" never-started={count}"),
        };
        let mut lines = vec![format!(
            "BATCH-SUMMARY ok={} no-output={} infra-fail={} launch-fail={}{} total={}",
            self.ok,
            self.no_output,
            self.infra_fail,
            self.launch_fail,
            never_started,
            self.total()
        )];
        lines.extend(self.faults.iter().map(FleetFault::line));
        lines
    }
}

/// Summarize a dispatch batch: count the statuses and raise a fleet-level
/// fault for every failure identity (status + signature) that repeats at or
/// above the policy's threshold within this batch.
pub fn summarize_batch(records: &[RunStatusRecord], policy: &FleetDispatchPolicy) -> BatchSummary {
    let mut summary = BatchSummary {
        ok: 0,
        no_output: 0,
        infra_fail: 0,
        launch_fail: 0,
        never_started: 0,
        faults: Vec::new(),
    };
    // Failure identity: (status, signature). NO-OUTPUT and LAUNCH-FAIL have
    // no finer signature, so each status is one identity.
    let mut groups: BTreeMap<(RunStatus, String), Vec<String>> = BTreeMap::new();

    for record in records {
        match record.status {
            RunStatus::Ok => summary.ok += 1,
            RunStatus::NoOutput => summary.no_output += 1,
            RunStatus::InfraFail => summary.infra_fail += 1,
            RunStatus::LaunchFail => summary.launch_fail += 1,
            RunStatus::AgentNeverStarted => summary.never_started += 1,
        }
        if !matches!(record.status, RunStatus::Ok) {
            let identity = record
                .failure_signature
                .clone()
                .unwrap_or_else(|| record.status.as_str().to_string());
            groups
                .entry((record.status, identity))
                .or_default()
                .push(record.issue.clone());
        }
    }

    summary.faults = groups
        .into_iter()
        .filter(|(_, issues)| issues.len() >= policy.repeat_fault_threshold)
        .map(|((status, signature), issues)| FleetFault {
            code: REPEATED_IDENTICAL_FAILURE.to_string(),
            status,
            signature,
            count: issues.len(),
            issues,
        })
        .collect();

    summary
}

/// The dispatch-side state of one sub-fleet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubfleetState {
    pub name: String,
    /// Agents currently running in this sub-fleet.
    pub running_agents: u32,
    /// Open issues eligible to run here.
    pub open_eligible: u32,
}

/// Report lines for sub-fleets with zero running agents while open eligible
/// work is still queued for them. A sub-fleet in this state is not receiving
/// dispatch; the work is open and eligible and nothing is running it.
pub fn idle_subfleet_lines(states: &[SubfleetState]) -> Vec<String> {
    states
        .iter()
        .filter(|state| state.running_agents == 0 && state.open_eligible > 0)
        .map(|state| {
            format!(
                "{SUBFLEET_IDLE} subfleet={} running-agents=0 open-eligible={} — eligible work is open but no agent is running in this sub-fleet; check the dispatch path to it",
                state.name, state.open_eligible
            )
        })
        .collect()
}

/// Per-issue attempt accounting across classified runs.
///
/// Only records whose `consumes_attempt` is true advance the counter, so a
/// batch of `INFRA-FAIL`/`LAUNCH-FAIL` runs leaves every counter untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptLedger {
    attempts: BTreeMap<String, u32>,
}

impl AttemptLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold a batch of classified runs into the ledger.
    pub fn apply(&mut self, records: &[RunStatusRecord]) {
        for record in records {
            if record.consumes_attempt {
                *self.attempts.entry(record.issue.clone()).or_insert(0) += 1;
            }
        }
    }

    /// Attempts consumed so far for one issue.
    pub fn attempts(&self, issue: &str) -> u32 {
        self.attempts.get(issue).copied().unwrap_or(0)
    }

    /// Issues with at least one consumed attempt.
    pub fn issues(&self) -> &BTreeMap<String, u32> {
        &self.attempts
    }
}

/// Default consecutive start-failure threshold: after this many in a row the
/// reconciler suspends the claim (issue #4211). Three is generous — a worker
/// that cannot start fails identically every time, so three is more than
/// enough to tell "still starting" apart from "will never start".
pub const DEFAULT_CONSECUTIVE_FAILURE_THRESHOLD: u32 = 3;

/// The reconciler's observation of one worker submission for a claim
/// (issue #4211): did the worker come up, did it die before it could, or was
/// the submission cancelled?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    /// The worker came up. Resets the claim's consecutive-failure counter.
    Started,
    /// The worker never came up. `reason` is the observable failure (e.g.
    /// `unknown model`); it advances the claim's consecutive-failure counter.
    Failed { reason: String },
    /// The submission was cancelled before it could start or fail. It is
    /// neither a start nor a failure and leaves the counter untouched.
    Cancelled,
}

/// A claim the reconciler has suspended because it cannot converge
/// (issue #4211).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuspendedClaim {
    pub claim: String,
    pub consecutive_failures: u32,
    pub last_failure: String,
}

impl SuspendedClaim {
    /// One report line, no embedded newline: the claim cannot converge and
    /// must not keep being submitted into the same wall.
    pub fn line(&self) -> String {
        format!(
            "CLAIM-SUSPENDED claim={} consecutive-start-failures={} last={} — the claim cannot converge; do not keep submitting into the same wall until the last failure is fixed",
            self.claim, self.consecutive_failures, self.last_failure
        )
    }
}

/// What a [`ReconcilerMemory::record`] call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordEffect {
    /// The submission was recorded; the claim is not suspended.
    Submitted,
    /// This submission crossed the consecutive-failure threshold, so the
    /// claim is now suspended. Report it — do not keep submitting.
    Suspended { suspended: SuspendedClaim },
}

/// Per-claim reconciler memory: consecutive start failures and suspension
/// (issue #4211).
///
/// The statelessness that hid the incident: a reconciler that reads the
/// declared intent, observes `up=0`, and submits, with no memory that the
/// previous 101 attempts died the same way. A claim that can never be
/// satisfied is indistinguishable, to that loop, from one whose worker has
/// simply not started yet. This type is that memory.
///
/// It is pure in-memory state — no I/O, no clock — so the fleet reconcile
/// loop can adopt it as its source of truth without changing what it
/// observes.
///
/// The consecutive-failure threshold is the per-claim *rate* threshold that
/// invariant 4 of #4211 demands be paired with the per-attempt cost
/// threshold ([`FleetDispatchPolicy::duration_floor_secs`]): a worker that
/// dies cheaply in 0.4s a hundred times is more dangerous than one that
/// burns an hour before dying, precisely because nothing runs long enough
/// to be noticed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcilerMemory {
    /// Consecutive start failures after which a claim is suspended.
    threshold: u32,
    /// Consecutive start failures per claim, reset on any `Started`.
    consecutive: BTreeMap<String, u32>,
    /// The most recent start-failure reason per claim.
    last_failure: BTreeMap<String, String>,
    /// Claims currently suspended.
    suspended: BTreeMap<String, SuspendedClaim>,
}

impl ReconcilerMemory {
    /// Build the memory with an explicit consecutive-failure threshold. A
    /// zero threshold (a degenerate "suspend on the first failure") falls
    /// back to [`DEFAULT_CONSECUTIVE_FAILURE_THRESHOLD`].
    pub fn new(consecutive_failure_threshold: u32) -> Self {
        let threshold = if consecutive_failure_threshold == 0 {
            DEFAULT_CONSECUTIVE_FAILURE_THRESHOLD
        } else {
            consecutive_failure_threshold
        };
        Self {
            threshold,
            consecutive: BTreeMap::new(),
            last_failure: BTreeMap::new(),
            suspended: BTreeMap::new(),
        }
    }

    /// The consecutive-failure threshold in force.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    /// Record one worker submission for a claim.
    ///
    /// A [`StartOutcome::Failed`] advances the consecutive-failure counter
    /// and records the reason; crossing the threshold suspends the claim and
    /// returns [`RecordEffect::Suspended`]. A [`StartOutcome::Started`]
    /// resets the counter and releases a suspended claim — it converged after
    /// all. A [`StartOutcome::Cancelled`] leaves the counter untouched: it is
    /// neither a start nor a failure.
    pub fn record(&mut self, claim: &str, outcome: &StartOutcome) -> RecordEffect {
        match outcome {
            StartOutcome::Started => {
                self.consecutive.remove(claim);
                self.last_failure.remove(claim);
                self.suspended.remove(claim);
                RecordEffect::Submitted
            }
            StartOutcome::Cancelled => RecordEffect::Submitted,
            StartOutcome::Failed { reason } => {
                let counter = self.consecutive.entry(claim.to_string()).or_insert(0);
                *counter = counter.saturating_add(1);
                let failures = *counter;
                self.last_failure.insert(claim.to_string(), reason.clone());
                if failures >= self.threshold {
                    // Report only the transition into suspension. A claim
                    // already suspended keeps counting (the incident's 102)
                    // but is not re-reported on every further submission.
                    if self.suspended.contains_key(claim) {
                        RecordEffect::Submitted
                    } else {
                        let suspended = SuspendedClaim {
                            claim: claim.to_string(),
                            consecutive_failures: failures,
                            last_failure: reason.clone(),
                        };
                        self.suspended.insert(claim.to_string(), suspended.clone());
                        RecordEffect::Suspended { suspended }
                    }
                } else {
                    RecordEffect::Submitted
                }
            }
        }
    }

    /// Fold a batch of classified runs into the memory. Returns the claims
    /// newly suspended by this batch so the caller can report them.
    pub fn apply_runs(&mut self, records: &[RunStatusRecord]) -> Vec<SuspendedClaim> {
        let mut newly_suspended = Vec::new();
        for record in records {
            if let RecordEffect::Suspended { suspended } =
                self.record(&record.issue, &record.start_outcome())
            {
                newly_suspended.push(suspended);
            }
        }
        newly_suspended
    }

    /// Whether a claim is currently suspended.
    pub fn is_suspended(&self, claim: &str) -> bool {
        self.suspended.contains_key(claim)
    }

    /// Consecutive start failures recorded for a claim (0 if none).
    pub fn consecutive_failures(&self, claim: &str) -> u32 {
        self.consecutive.get(claim).copied().unwrap_or(0)
    }

    /// The most recent start-failure reason for a claim, if any.
    pub fn last_failure(&self, claim: &str) -> Option<&str> {
        self.last_failure.get(claim).map(String::as_str)
    }

    /// The claims currently suspended.
    pub fn suspended(&self) -> impl Iterator<Item = &SuspendedClaim> {
        self.suspended.values()
    }

    /// Release a suspended claim — the operator diagnosed and cleared it.
    /// Resets the counter and the recorded reason. Returns whether a
    /// suspension was released.
    pub fn clear(&mut self, claim: &str) -> bool {
        let released = self.suspended.remove(claim).is_some();
        self.consecutive.remove(claim);
        self.last_failure.remove(claim);
        released
    }

    /// The reconciler's per-claim status line (invariant 2 of #4211):
    /// "never converged" is a reportable state distinct from "converging".
    ///
    /// A claim with no recorded start failures renders as a plain
    /// `want=N up=N pending=N` — still converging, nothing known wrong. A
    /// claim with start failures carries the count and the last reason into
    /// the same line, so `want=1 up=0 pending=0` is never printed identically
    /// on the first attempt and the hundredth.
    pub fn status_line(&self, claim: &str, want: u32, up: u32, pending: u32) -> String {
        let base = format!("want={want} up={up} pending={pending}");
        let failures = self.consecutive_failures(claim);
        if failures == 0 {
            base
        } else {
            let last = self.last_failure(claim).unwrap_or("unknown");
            let suspended = if self.is_suspended(claim) {
                " (suspended)"
            } else {
                ""
            };
            format!("{base} ({failures} consecutive start failures, last: {last}{suspended})")
        }
    }

    /// One report line per suspended claim.
    pub fn suspended_lines(&self) -> Vec<String> {
        self.suspended().map(SuspendedClaim::line).collect()
    }
}

#[cfg(test)]
mod tests;
