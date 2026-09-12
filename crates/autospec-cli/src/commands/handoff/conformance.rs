//! Consumer conformance for `autospec.implementation-handoff.v1` (issue #3441).
//!
//! The handoff producer (issue #3440) ships in this repository; the
//! deployment-owned serverless consumer does not. The spec
//! (`docs/specs/2026-08-31-automatic-spec-projects-design.md`,
//! "top-level implementation gateway") therefore keeps the feature Blocked
//! until the deployed consumer publishes a conformance receipt tied to its
//! deployed revision proving:
//!
//! 1. its sole implementation-dispatch function consumes
//!    `autospec.implementation-handoff.v1` and no direct implementation-agent
//!    branch remains reachable;
//! 2. status and cancellation are consumed from the protocol's typed run
//!    identity / cancellation token, not from terminal prose;
//! 3. the receipt's schema and signature verify, and its claimed
//!    consumer revision matches the deployed consumer.
//!
//! This module validates a recorded consumer trace against the receipt and
//! emits a typed receipt verdict. It is read-only: it never mutates a
//! deployment, never guesses an external repository, and never fabricates a
//! receipt — a signature that does not verify is rejected, not recomputed.
//!
//! Contract documentation: `docs/contracts/autospec-implementation-handoff-v1.md`.

use std::fs;
use std::path::Path;

use autospec_core::autonomous::waterfall::sha256_hex;
use serde_json::{json, Value};

use super::CommandFailure;
use super::HANDOFF_SCHEMA;

/// Receipt schema published by the deployment-owned consumer (schema version 1).
pub const RECEIPT_SCHEMA: &str = "autospec.implementation-handoff-receipt.v1";
/// Recorded consumer-trace schema consumed by the conformance suite.
pub const TRACE_SCHEMA: &str = "autospec.implementation-handoff-consumer-trace.v1";
/// Verdict schema emitted by `autospec handoff conformance`.
pub const VERDICT_SCHEMA: &str = "autospec.implementation-handoff-receipt-verdict.v1";

/// Trace event types.
pub const EVENT_HANDOFF_REQUEST: &str = "handoff_request";
pub const EVENT_DISPATCH: &str = "dispatch";
pub const EVENT_STATUS: &str = "status";
pub const EVENT_CANCELLATION: &str = "cancellation";
/// An explicit direct implementation-agent dispatch: always a conformance
/// failure when present in the trace.
pub const EVENT_DIRECT_DISPATCH: &str = "direct_dispatch";

/// The only typed status source: the protocol's machine-readable run identity.
pub const STATUS_SOURCE_TYPED: &str = "run-identity";
/// The only typed cancellation source: the protocol's cancellation token.
pub const CANCELLATION_SOURCE_TYPED: &str = "cancellation-token";

/// Typed conformance finding codes.
pub const CODE_RECEIPT_MALFORMED: &str = "RECEIPT_MALFORMED";
pub const CODE_RECEIPT_SCHEMA_MISMATCH: &str = "RECEIPT_SCHEMA_MISMATCH";
pub const CODE_HANDOFF_SCHEMA_MISMATCH: &str = "HANDOFF_SCHEMA_MISMATCH";
pub const CODE_RECEIPT_SIGNATURE_MISMATCH: &str = "RECEIPT_SIGNATURE_MISMATCH";
pub const CODE_TRACE_MALFORMED: &str = "TRACE_MALFORMED";
pub const CODE_TRACE_SCHEMA_MISMATCH: &str = "TRACE_SCHEMA_MISMATCH";
pub const CODE_REVISION_MISMATCH: &str = "REVISION_MISMATCH";
pub const CODE_PROTOCOL_NOT_CONSUMED: &str = "PROTOCOL_NOT_CONSUMED";
pub const CODE_DIRECT_DISPATCH_REACHABLE: &str = "DIRECT_DISPATCH_REACHABLE";
pub const CODE_STATUS_UNTYPED: &str = "STATUS_UNTYPED";
pub const CODE_CANCELLATION_UNTYPED: &str = "CANCELLATION_UNTYPED";
pub const CODE_RECEIPT_CHECK_MISMATCH: &str = "RECEIPT_CHECK_MISMATCH";

const PREREQUISITE_SATISFIED: &str = "satisfied";
const PREREQUISITE_BLOCKED: &str = "blocked";

/// One typed conformance finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceFinding {
    pub code: &'static str,
    pub detail: String,
}

fn finding(code: &'static str, detail: impl Into<String>) -> ConformanceFinding {
    ConformanceFinding {
        code,
        detail: detail.into(),
    }
}

/// The portfolio blocked-prerequisite projection carried by every verdict
/// (spec: repository-local completion stays Blocked until the receipt
/// matches the deployed consumer revision).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerequisite {
    pub state: &'static str,
    pub detail: String,
}

/// The typed receipt verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptVerdict {
    pub admitted: bool,
    pub findings: Vec<ConformanceFinding>,
    pub prerequisite: Prerequisite,
}

impl ReceiptVerdict {
    pub fn to_json(&self) -> Value {
        json!({
            "schema": VERDICT_SCHEMA,
            "admitted": self.admitted,
            "findings": self
                .findings
                .iter()
                .map(|f| json!({ "code": f.code, "detail": f.detail }))
                .collect::<Vec<_>>(),
            "prerequisite": {
                "state": self.prerequisite.state,
                "detail": self.prerequisite.detail,
            },
        })
    }
}

/// The expected receipt signature: sha256 over the canonical compact JSON
/// form (keys sorted — serde_json's default map ordering) of the receipt
/// with the `signature` field removed. The consumer signs the same
/// canonicalization, so a receipt that does not reproduce its signature
/// was fabricated or tampered with.
pub fn expected_signature(receipt: &Value) -> String {
    let mut body = receipt.clone();
    if let Value::Object(fields) = &mut body {
        fields.remove("signature");
    }
    sha256_hex(serde_json::to_string(&body).unwrap_or_default().as_bytes())
}

fn is_nonempty_str(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

fn events_of(trace: &Value) -> Vec<Value> {
    trace
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn event_types(trace: &Value, ty: &str) -> Vec<Value> {
    events_of(trace)
        .into_iter()
        .filter(|event| {
            event.get("type").and_then(Value::as_str) == Some(ty)
        })
        .collect()
}

fn has_direct_dispatch(trace: &Value) -> bool {
    !event_types(trace, EVENT_DIRECT_DISPATCH).is_empty()
        || event_types(trace, EVENT_DISPATCH)
            .iter()
            .any(|event| {
                event
                    .get("via")
                    .and_then(Value::as_str)
                    .is_some_and(|via| via != HANDOFF_SCHEMA)
            })
}

fn has_untyped_status(trace: &Value) -> bool {
    let status = event_types(trace, EVENT_STATUS);
    status.is_empty()
        || status
            .iter()
            .any(|event| {
                event
                    .get("source")
                    .and_then(Value::as_str)
                    != Some(STATUS_SOURCE_TYPED)
            })
}

fn has_untyped_cancellation(trace: &Value) -> bool {
    let cancellation = event_types(trace, EVENT_CANCELLATION);
    cancellation.is_empty()
        || cancellation
            .iter()
            .any(|event| {
                event
                    .get("source")
                    .and_then(Value::as_str)
                    != Some(CANCELLATION_SOURCE_TYPED)
            })
}

/// Does the trace prove the consumer consumed the handoff protocol through
/// its dispatch function? Requires at least one `handoff_request` naming the
/// v1 handoff schema and at least one `dispatch` routed via it.
fn protocol_consumed(trace: &Value) -> bool {
    event_types(trace, EVENT_HANDOFF_REQUEST)
        .iter()
        .any(|event| {
            event
                .get("schema")
                .and_then(Value::as_str)
                == Some(HANDOFF_SCHEMA)
        })
        && event_types(trace, EVENT_DISPATCH)
            .iter()
            .any(|event| {
                event
                    .get("via")
                    .and_then(Value::as_str)
                    == Some(HANDOFF_SCHEMA)
            })
}

/// Validate the recorded consumer trace against the consumer's published
/// receipt. Pure and total: malformed input is a typed finding, never a
/// panic. A receipt is admitted only when every check passes.
pub fn verify(trace: &Value, receipt: &Value) -> ReceiptVerdict {
    let mut findings: Vec<ConformanceFinding> = Vec::new();

    // ── receipt schema / signature ─────────────────────────────────────
    if !receipt.is_object() {
        findings.push(finding(
            CODE_RECEIPT_MALFORMED,
            "receipt is not a JSON object",
        ));
    } else {
        let schema = receipt.get("schema").and_then(Value::as_str);
        if schema != Some(RECEIPT_SCHEMA) {
            findings.push(finding(
                CODE_RECEIPT_SCHEMA_MISMATCH,
                format!(
                    "receipt schema must be {RECEIPT_SCHEMA}, got {schema:?}"
                ),
            ));
        }
        let handoff_schema = receipt.get("handoff_schema").and_then(Value::as_str);
        if handoff_schema != Some(HANDOFF_SCHEMA) {
            findings.push(finding(
                CODE_HANDOFF_SCHEMA_MISMATCH,
                format!(
                    "receipt must bind {HANDOFF_SCHEMA}, got {handoff_schema:?}"
                ),
            ));
        }
        if !is_nonempty_str(receipt.get("consumer_revision")) {
            findings.push(finding(
                CODE_RECEIPT_MALFORMED,
                "receipt consumer_revision must be a non-empty string",
            ));
        }
        let signature = receipt.get("signature").and_then(Value::as_str);
        if !is_nonempty_str(receipt.get("signature")) {
            findings.push(finding(
                CODE_RECEIPT_SIGNATURE_MISMATCH,
                "receipt signature is missing",
            ));
        } else if signature != Some(expected_signature(receipt).as_str()) {
            findings.push(finding(
                CODE_RECEIPT_SIGNATURE_MISMATCH,
                "receipt signature does not verify; the receipt was \
                 fabricated or tampered with",
            ));
        }
        // Self-attested checks must not under-claim against a conforming
        // trace; a false check means the consumer does not claim
        // conformance.
        for check in [
            "direct_dispatch_unreachable",
            "status_typed",
            "cancellation_typed",
        ] {
            match receipt
                .get("checks")
                .and_then(|checks| checks.get(check))
                .and_then(Value::as_bool)
            {
                Some(true) => {}
                Some(false) => findings.push(finding(
                    CODE_RECEIPT_CHECK_MISMATCH,
                    format!("receipt check {check} is false"),
                )),
                None => findings.push(finding(
                    CODE_RECEIPT_CHECK_MISMATCH,
                    format!("receipt check {check} must be a boolean"),
                )),
            }
        }
    }

    // ── consumer trace ─────────────────────────────────────────────────
    if !trace.is_object() {
        findings.push(finding(
            CODE_TRACE_MALFORMED,
            "consumer trace is not a JSON object",
        ));
    } else {
        let schema = trace.get("schema").and_then(Value::as_str);
        if schema != Some(TRACE_SCHEMA) {
            findings.push(finding(
                CODE_TRACE_SCHEMA_MISMATCH,
                format!("trace schema must be {TRACE_SCHEMA}, got {schema:?}"),
            ));
        }
        if !is_nonempty_str(trace.get("consumer_revision")) {
            findings.push(finding(
                CODE_TRACE_MALFORMED,
                "trace consumer_revision must be a non-empty string",
            ));
        }
        if !events_of(trace).iter().all(Value::is_object) {
            findings.push(finding(
                CODE_TRACE_MALFORMED,
                "trace events must be JSON objects",
            ));
        }

        // Deployed revision identity: the receipt is stale unless its
        // claimed revision equals the deployed consumer revision the trace
        // was recorded from.
        let receipt_revision = receipt
            .get("consumer_revision")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let trace_revision = trace
            .get("consumer_revision")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if is_nonempty_str(trace.get("consumer_revision"))
            && is_nonempty_str(receipt.get("consumer_revision"))
            && receipt_revision != trace_revision
        {
            findings.push(finding(
                CODE_REVISION_MISMATCH,
                format!(
                    "receipt claims consumer revision {receipt_revision:?} but \
                     the deployed consumer runs {trace_revision:?}"
                ),
            ));
        }

        if !protocol_consumed(trace) {
            findings.push(finding(
                CODE_PROTOCOL_NOT_CONSUMED,
                "trace does not prove the sole implementation-dispatch \
                 function consumes the handoff protocol",
            ));
        }
        if has_direct_dispatch(trace) {
            findings.push(finding(
                CODE_DIRECT_DISPATCH_REACHABLE,
                "trace shows an implementation dispatch that bypasses the \
                 handoff protocol",
            ));
        }
        if has_untyped_status(trace) {
            findings.push(finding(
                CODE_STATUS_UNTYPED,
                format!(
                    "status must be consumed from the typed run identity \
                     ({STATUS_SOURCE_TYPED})"
                ),
            ));
        }
        if has_untyped_cancellation(trace) {
            findings.push(finding(
                CODE_CANCELLATION_UNTYPED,
                format!(
                    "cancellation must use the typed cancellation token \
                     ({CANCELLATION_SOURCE_TYPED})"
                ),
            ));
        }
    }

    let admitted = findings.is_empty();
    let prerequisite = if admitted {
        Prerequisite {
            state: PREREQUISITE_SATISFIED,
            detail: format!(
                "portfolio prerequisite satisfied: the receipt matches the \
                 deployed consumer revision and the consumer exclusively uses {HANDOFF_SCHEMA}"
            ),
        }
    } else {
        Prerequisite {
            state: PREREQUISITE_BLOCKED,
            detail: format!(
                "portfolio prerequisite blocked: {}",
                findings
                    .iter()
                    .map(|f| f.code)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    };
    ReceiptVerdict {
        admitted,
        findings,
        prerequisite,
    }
}

const USAGE: &str = "\
USAGE:
    autospec handoff conformance --receipt PATH --trace PATH

Reads the consumer's published receipt and the recorded consumer trace
(both JSON, schemas in docs/contracts/autospec-implementation-handoff-v1.md)
and prints the typed receipt verdict. Exits 0 when the receipt is admitted,
1 when it is rejected. Read-only: no deployment mutation, no receipt
fabrication.";

fn run_conformance(args: &[String]) -> Result<(), CommandFailure> {
    let mut receipt_path: Option<&str> = None;
    let mut trace_path: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let mut value = || -> Result<&str, CommandFailure> {
            i += 1;
            args.get(i)
                .map(String::as_str)
                .ok_or_else(|| CommandFailure::diagnostic(format!("missing value for {arg}")))
        };
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--receipt" => receipt_path = Some(value()?),
            "--trace" => trace_path = Some(value()?),
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec handoff conformance option: {other}\n{USAGE}"
                )))
            }
        }
        i += 1;
    }
    let (receipt_path, trace_path) = match (receipt_path, trace_path) {
        (Some(receipt), Some(trace)) => (receipt, trace),
        _ => {
            return Err(CommandFailure::diagnostic(format!(
                "missing required --receipt PATH and --trace PATH\n{USAGE}"
            )))
        }
    };

    let verdict = conformance_verdict(Path::new(receipt_path), Path::new(trace_path))?;
    println!("{}", verdict.to_json());
    if verdict.admitted {
        Ok(())
    } else {
        Err(CommandFailure::status(
            format!(
                "conformance rejected: {}",
                verdict
                    .findings
                    .iter()
                    .map(|f| f.code)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            1,
        ))
    }
}

fn conformance_verdict(receipt_path: &Path, trace_path: &Path) -> Result<ReceiptVerdict, CommandFailure> {
    let load = |path: &Path, what: &str| -> Result<Value, CommandFailure> {
        let bytes = fs::read(path).map_err(|error| {
            CommandFailure::diagnostic(format!("cannot read {what} file {}: {error}", path.display()))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            CommandFailure::diagnostic(format!("{what} file {path:?} is not valid JSON: {error}"))
        })
    };
    let receipt = load(receipt_path, "receipt")?;
    let trace = load(trace_path, "consumer trace")?;
    Ok(verify(&trace, &receipt))
}

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    run_conformance(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(revision: &str) -> Value {
        json!({
            "schema": TRACE_SCHEMA,
            "consumer_revision": revision,
            "events": [
                {
                    "type": EVENT_HANDOFF_REQUEST,
                    "schema": HANDOFF_SCHEMA,
                    "route": "run"
                },
                {
                    "type": EVENT_DISPATCH,
                    "target": "autospec-run",
                    "via": HANDOFF_SCHEMA
                },
                { "type": EVENT_STATUS, "source": STATUS_SOURCE_TYPED },
                { "type": EVENT_CANCELLATION, "source": CANCELLATION_SOURCE_TYPED }
            ]
        })
    }

    fn receipt(revision: &str) -> Value {
        let mut receipt = json!({
            "schema": RECEIPT_SCHEMA,
            "handoff_schema": HANDOFF_SCHEMA,
            "consumer_revision": revision,
            "checks": {
                "direct_dispatch_unreachable": true,
                "status_typed": true,
                "cancellation_typed": true
            }
        });
        let signature = expected_signature(&receipt);
        if let Value::Object(fields) = &mut receipt {
            fields.insert("signature".to_string(), Value::String(signature));
        }
        receipt
    }

    fn codes(verdict: &ReceiptVerdict) -> Vec<&'static str> {
        verdict
            .findings
            .iter()
            .map(|f| f.code)
            .collect()
    }

    #[test]
    fn valid_receipt_is_admitted_with_satisfied_prerequisite() {
        let verdict = verify(&trace("rev-1"), &receipt("rev-1"));
        assert!(verdict.admitted, "{verdict:?}");
        assert!(verdict.findings.is_empty());
        assert_eq!(verdict.prerequisite.state, PREREQUISITE_SATISFIED);
    }

    #[test]
    fn stale_receipt_revision_is_rejected() {
        let verdict = verify(&trace("rev-2"), &receipt("rev-1"));
        assert!(!verdict.admitted);
        assert_eq!(codes(&verdict), vec![CODE_REVISION_MISMATCH]);
        assert_eq!(verdict.prerequisite.state, PREREQUISITE_BLOCKED);
    }

    #[test]
    fn direct_dispatch_is_rejected() {
        let mut t = trace("rev-1");
        if let Value::Object(fields) = &mut t {
            if let Some(events) = fields.get_mut("events").and_then(Value::as_array_mut) {
                events.push(json!({ "type": EVENT_DIRECT_DISPATCH, "agent": "impl-agent" }));
            }
        }
        let verdict = verify(&t, &receipt("rev-1"));
        assert!(!verdict.admitted);
        assert!(codes(&verdict).contains(&CODE_DIRECT_DISPATCH_REACHABLE));
    }

    #[test]
    fn untyped_status_and_cancellation_are_rejected() {
        let mut t = trace("rev-1");
        let events = t
            .get_mut("events")
            .and_then(Value::as_array_mut)
            .unwrap();
        for event in events.iter_mut() {
            if event.get("type").and_then(Value::as_str) == Some(EVENT_STATUS) {
                *event = json!({ "type": EVENT_STATUS, "source": "terminal-prose" });
            }
            if event.get("type").and_then(Value::as_str) == Some(EVENT_CANCELLATION) {
                *event = json!({ "type": EVENT_CANCELLATION, "source": "process-kill" });
            }
        }
        let verdict = verify(&t, &receipt("rev-1"));
        assert!(!verdict.admitted);
        let found = codes(&verdict);
        assert!(found.contains(&CODE_STATUS_UNTYPED));
        assert!(found.contains(&CODE_CANCELLATION_UNTYPED));
    }

    #[test]
    fn tampered_receipt_signature_is_rejected() {
        let mut r = receipt("rev-1");
        if let Value::Object(fields) = &mut r {
            fields.insert("consumer_revision".to_string(), Value::String("rev-2".to_string()));
        }
        // The signature was computed for rev-1: the edit is not signed.
        let verdict = verify(&trace("rev-1"), &r);
        assert!(!verdict.admitted);
        assert!(codes(&verdict).contains(&CODE_RECEIPT_SIGNATURE_MISMATCH));
        assert!(codes(&verdict).contains(&CODE_REVISION_MISMATCH));
    }

    #[test]
    fn wrong_handoff_schema_is_rejected() {
        let mut r = receipt("rev-1");
        if let Value::Object(fields) = &mut r {
            fields.insert(
                "handoff_schema".to_string(),
                Value::String("autospec.implementation-handoff.v2".to_string()),
            );
        }
        let signature = expected_signature(&r);
        if let Value::Object(fields) = &mut r {
            fields.insert("signature".to_string(), Value::String(signature));
        }
        let verdict = verify(&trace("rev-1"), &r);
        assert!(!verdict.admitted);
        assert!(codes(&verdict).contains(&CODE_HANDOFF_SCHEMA_MISMATCH));
    }

    #[test]
    fn malformed_inputs_are_findings_not_panics() {
        let verdict = verify(&Value::Null, &Value::Null);
        assert!(!verdict.admitted);
        assert!(codes(&verdict).contains(&CODE_TRACE_MALFORMED));
        assert!(codes(&verdict).contains(&CODE_RECEIPT_MALFORMED));
    }
}
