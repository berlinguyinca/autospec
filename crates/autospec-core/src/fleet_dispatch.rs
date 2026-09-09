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
//!
//! The batch layer: `summarize_batch` counts the statuses and raises a
//! fleet-level fault (`REPEATED_IDENTICAL_FAILURE`) when the same failure
//! (status + signature) repeats within one dispatch batch — N runners dying
//! on the same credential is one fault to fix, not N attempts to retry.
//! `idle_subfleet_lines` reports sub-fleets with zero running agents while
//! open eligible work is still queued for them.

use std::collections::BTreeMap;

use serde::de::{Error as DeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
}

impl RunStatus {
    /// The status string as recorded in `agent-status.tsv` (hyphenated).
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Ok => OK_STATUS,
            RunStatus::NoOutput => NO_OUTPUT_STATUS,
            RunStatus::InfraFail => INFRA_FAIL_STATUS,
            RunStatus::LaunchFail => LAUNCH_FAIL_STATUS,
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
    /// `INFRA-FAIL/<category>/<signature>` for infra failures, else `None`.
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
/// ran, whatever it printed), then infrastructure signatures, then the
/// transcript's emptiness.
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

    let status = if run.transcript.trim().is_empty() {
        RunStatus::NoOutput
    } else {
        RunStatus::Ok
    };
    RunStatusRecord {
        issue: run.issue.clone(),
        status,
        duration_secs: run.duration_secs,
        transcript_bytes,
        failure_signature: None,
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
    pub faults: Vec<FleetFault>,
}

impl BatchSummary {
    pub fn total(&self) -> usize {
        self.ok + self.no_output + self.infra_fail + self.launch_fail
    }

    /// Report lines: one summary line, then one line per fleet fault.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "BATCH-SUMMARY ok={} no-output={} infra-fail={} launch-fail={} total={}",
            self.ok,
            self.no_output,
            self.infra_fail,
            self.launch_fail,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> FleetDispatchPolicy {
        FleetDispatchPolicy::default()
    }

    fn run(issue: &str, duration_secs: u64, transcript: &str) -> RunRecord {
        RunRecord {
            issue: issue.to_string(),
            duration_secs,
            transcript: transcript.to_string(),
        }
    }

    /// The acceptance test from the issue: a bad-credential dispatch must be
    /// `INFRA-FAIL`, never `NO-OUTPUT`, and must not consume an attempt.
    #[test]
    fn bad_credential_is_infra_fail_not_no_output() {
        let record = classify_run(
            &run(
                "51",
                40,
                "POST /v1/chat/completions -> 401 Unauthorized: invalid api key",
            ),
            &policy(),
        );
        assert_eq!(record.status, RunStatus::InfraFail);
        assert_ne!(record.status, RunStatus::NoOutput);
        assert!(!record.consumes_attempt);
        assert_eq!(
            record.failure_signature.as_deref(),
            Some("INFRA-FAIL/auth/401 unauthorized")
        );

        let mut ledger = AttemptLedger::new();
        ledger.apply(std::slice::from_ref(&record));
        assert_eq!(
            ledger.attempts("51"),
            0,
            "INFRA-FAIL must not consume an attempt"
        );
    }

    #[test]
    fn short_run_below_floor_is_launch_fail_regardless_of_transcript() {
        // A 2-second run with an infra-looking transcript is still a launch
        // failure: it ended before it could have executed the spec.
        let record = classify_run(
            &run("52", 2, "connection refused while starting the session"),
            &policy(),
        );
        assert_eq!(record.status, RunStatus::LaunchFail);
        assert!(!record.consumes_attempt);
        assert_eq!(record.failure_signature, None);

        // A 2-second run that printed a healthy-looking transcript is the
        // same: below the floor, nothing ran.
        let healthy = classify_run(
            &run("53", 1, "loaded worktree, applied 3 edits, tests green"),
            &policy(),
        );
        assert_eq!(healthy.status, RunStatus::LaunchFail);
    }

    #[test]
    fn floor_is_strictly_less_than() {
        let at_floor = classify_run(&run("7", 30, "did the work"), &policy());
        assert_eq!(at_floor.status, RunStatus::Ok);
        let below_floor = classify_run(&run("7", 29, "did the work"), &policy());
        assert_eq!(below_floor.status, RunStatus::LaunchFail);
    }

    #[test]
    fn empty_transcript_at_duration_is_no_output() {
        let record = classify_run(&run("60", 120, "   \n  "), &policy());
        assert_eq!(record.status, RunStatus::NoOutput);
        assert!(
            record.consumes_attempt,
            "a run that ran but changed nothing does consume an attempt"
        );
        assert_eq!(record.failure_signature, None);
    }

    #[test]
    fn long_run_with_output_is_ok() {
        let record = classify_run(
            &run("61", 300, "patch generated: 2 files changed"),
            &policy(),
        );
        assert_eq!(record.status, RunStatus::Ok);
        assert!(record.consumes_attempt);
    }

    #[test]
    fn endpoint_failure_is_infra_fail() {
        let record = classify_run(
            &run(
                "54",
                90,
                "error: failed to connect to api.internal: connection refused",
            ),
            &policy(),
        );
        assert_eq!(record.status, RunStatus::InfraFail);
        assert_eq!(
            record.failure_signature.as_deref(),
            Some("INFRA-FAIL/endpoint/connection refused")
        );
    }

    #[test]
    fn context_failure_is_infra_fail() {
        let record = classify_run(
            &run(
                "55",
                200,
                "API Error: prompt is too long: 210000 tokens > 200000 maximum",
            ),
            &policy(),
        );
        assert_eq!(record.status, RunStatus::InfraFail);
        assert_eq!(
            record.failure_signature.as_deref(),
            Some("INFRA-FAIL/context/prompt is too long")
        );
    }

    #[test]
    fn auth_category_wins_over_later_categories() {
        let record = classify_run(
            &run(
                "56",
                100,
                "401 Unauthorized; also connection refused on retry",
            ),
            &policy(),
        );
        assert_eq!(record.status, RunStatus::InfraFail);
        assert!(record
            .failure_signature
            .as_deref()
            .unwrap()
            .starts_with("INFRA-FAIL/auth/"));
    }

    #[test]
    fn short_transcript_is_quoted_verbatim() {
        let transcript = "401 Unauthorized: bad token\nat fetch (line 12)\ttabbed";
        let record = classify_run(&run("57", 45, transcript), &policy());
        assert_eq!(record.quoted_transcript.as_deref(), Some(transcript));
        assert_eq!(record.transcript_bytes, transcript.len());
    }

    #[test]
    fn long_transcript_is_not_quoted_but_counted() {
        let transcript = "x".repeat(5000);
        let record = classify_run(&run("58", 60, &transcript), &policy());
        assert_eq!(record.quoted_transcript, None);
        assert_eq!(record.transcript_bytes, 5000);
    }

    #[test]
    fn quote_boundary_is_inclusive() {
        let transcript = "y".repeat(DEFAULT_TRANSCRIPT_QUOTE_BYTES);
        let at_threshold = classify_run(&run("59", 60, &transcript), &policy());
        assert_eq!(
            at_threshold.quoted_transcript.as_deref(),
            Some(transcript.as_str())
        );

        let over = classify_run(
            &run("59", 60, &"y".repeat(DEFAULT_TRANSCRIPT_QUOTE_BYTES + 1)),
            &policy(),
        );
        assert_eq!(over.quoted_transcript, None);
    }

    #[test]
    fn tsv_line_escapes_control_characters() {
        let record = classify_run(&run("62", 45, "line one\nline two\ttabbed"), &policy());
        let line = record.tsv_line();
        let columns = line.split('\t').count();
        assert_eq!(
            columns, 7,
            "a quoted transcript must not add TSV columns: {line:?}"
        );
        assert!(line.contains("line one\\nline two\\ttabbed"));
    }

    #[test]
    fn batch_summary_counts_and_raises_fault_on_repetition() {
        let records = vec![
            classify_run(&run("51", 40, "401 Unauthorized"), &policy()),
            classify_run(&run("52", 38, "401 Unauthorized"), &policy()),
            classify_run(
                &run("53", 41, "401 Unauthorized: invalid api key"),
                &policy(),
            ),
            classify_run(&run("60", 120, ""), &policy()),
            classify_run(&run("61", 300, "did the work"), &policy()),
        ];
        let summary = summarize_batch(&records, &policy());
        assert_eq!(
            (
                summary.ok,
                summary.no_output,
                summary.infra_fail,
                summary.launch_fail
            ),
            (1, 1, 3, 0)
        );
        assert_eq!(summary.total(), 5);
        assert_eq!(summary.faults.len(), 1);
        let fault = &summary.faults[0];
        assert_eq!(fault.code, REPEATED_IDENTICAL_FAILURE);
        assert_eq!(fault.status, RunStatus::InfraFail);
        assert_eq!(fault.count, 3);
        assert_eq!(
            fault.issues,
            vec!["51".to_string(), "52".to_string(), "53".to_string()]
        );

        let lines = summary.lines();
        assert_eq!(
            lines[0],
            "BATCH-SUMMARY ok=1 no-output=1 infra-fail=3 launch-fail=0 total=5"
        );
        assert!(lines[1].starts_with("FLEET-FAULT code=REPEATED_IDENTICAL_FAILURE"));
    }

    #[test]
    fn below_threshold_repetition_is_not_a_fault() {
        let records = vec![
            classify_run(&run("51", 40, "401 Unauthorized"), &policy()),
            classify_run(&run("52", 38, "401 Unauthorized"), &policy()),
        ];
        let summary = summarize_batch(&records, &policy());
        assert!(summary.faults.is_empty());
    }

    #[test]
    fn distinct_signatures_do_not_group() {
        let records = vec![
            classify_run(&run("51", 40, "401 Unauthorized"), &policy()),
            classify_run(&run("52", 38, "connection refused"), &policy()),
            classify_run(&run("53", 41, "prompt is too long"), &policy()),
        ];
        let summary = summarize_batch(&records, &policy());
        assert_eq!(summary.infra_fail, 3);
        assert!(
            summary.faults.is_empty(),
            "three different infrastructure failures are not one repeated failure"
        );
    }

    #[test]
    fn idle_subfleets_are_reported() {
        let states = vec![
            SubfleetState {
                name: "gw-issue-51-53".to_string(),
                running_agents: 0,
                open_eligible: 7,
            },
            SubfleetState {
                name: "busy".to_string(),
                running_agents: 2,
                open_eligible: 9,
            },
            SubfleetState {
                name: "empty".to_string(),
                running_agents: 0,
                open_eligible: 0,
            },
        ];
        let lines = idle_subfleet_lines(&states);
        assert_eq!(lines.len(), 1);
        assert!(lines[0]
            .starts_with("SUBFLEET-IDLE subfleet=gw-issue-51-53 running-agents=0 open-eligible=7"));
    }

    #[test]
    fn attempt_ledger_counts_only_consumed_runs() {
        let records = vec![
            classify_run(&run("51", 2, "401 Unauthorized"), &policy()), // LAUNCH-FAIL: no attempt
            classify_run(&run("51", 40, "401 Unauthorized"), &policy()), // INFRA-FAIL: no attempt
            classify_run(&run("51", 120, ""), &policy()),               // NO-OUTPUT: attempt
            classify_run(&run("52", 300, "did it"), &policy()),         // OK: attempt
        ];
        let mut ledger = AttemptLedger::new();
        ledger.apply(&records);
        assert_eq!(ledger.attempts("51"), 1);
        assert_eq!(ledger.attempts("52"), 1);
        assert_eq!(ledger.attempts("99"), 0);
    }

    #[test]
    fn status_serialization_uses_hyphenated_strings() {
        let record = classify_run(&run("51", 40, "401 Unauthorized"), &policy());
        let json = serde_json::to_string(&record).expect("record serializes");
        assert!(json.contains("\"INFRA-FAIL\""), "got {json}");
        assert!(json.contains("\"INFRA-FAIL/auth/401 unauthorized\""));

        let roundtrip: RunStatusRecord = serde_json::from_str(&json).expect("record deserializes");
        assert_eq!(roundtrip, record);

        // The underscored legacy form still parses.
        assert_eq!(RunStatus::parse("NO_OUTPUT"), Some(RunStatus::NoOutput));
        assert_eq!(RunStatus::parse("bogus"), None);
    }

    #[test]
    fn policy_requires_positive_knobs() {
        assert!(FleetDispatchPolicy::new(30, 4096, 3).is_some());
        assert!(FleetDispatchPolicy::new(0, 4096, 3).is_none());
        assert!(FleetDispatchPolicy::new(30, 0, 3).is_none());
        assert!(FleetDispatchPolicy::new(30, 4096, 0).is_none());
    }

    #[test]
    fn signature_priority_is_first_listed_match() {
        // "context length exceeded" contains "context length"; the more
        // specific signature must win.
        let found = infra_signature("Error: context length exceeded for model").unwrap();
        assert_eq!(found, (InfraCategory::Context, "context length exceeded"));
    }

    #[test]
    fn infra_match_is_case_insensitive() {
        assert_eq!(
            infra_signature("HTTP 401 UNAUTHORIZED from the gateway").map(|(c, _)| c),
            Some(InfraCategory::Auth)
        );
    }
}
