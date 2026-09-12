//! Conformance command for the deployment-owned handoff consumer (issue #3441).
//!
//! Recorded consumer fixtures, no external deployment mutation: the suite
//! proves the conformance command admits a valid receipt and rejects the
//! invalid receipt classes — stale deployed revision, reachable direct
//! dispatch, untyped status/cancellation, and a fabricated signature.

#[path = "../src/commands/mod.rs"]
mod commands;

use commands::handoff::conformance::{
    expected_signature, verify, CANCELLATION_SOURCE_TYPED, CODE_CANCELLATION_UNTYPED,
    CODE_DIRECT_DISPATCH_REACHABLE, CODE_RECEIPT_SIGNATURE_MISMATCH, CODE_REVISION_MISMATCH,
    CODE_STATUS_UNTYPED, EVENT_CANCELLATION, EVENT_DIRECT_DISPATCH, EVENT_DISPATCH,
    EVENT_HANDOFF_REQUEST, EVENT_STATUS, RECEIPT_SCHEMA, STATUS_SOURCE_TYPED, TRACE_SCHEMA,
};
use commands::handoff::HANDOFF_SCHEMA;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let serial = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-handoff-conformance-{name}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, file: &str, value: &Value) -> PathBuf {
        let path = self.0.join(file);
        fs::write(&path, value.to_string()).unwrap();
        path
    }
}

/// Recorded consumer fixture: the deployed consumer's handling of one
/// implementation request, consumed through the handoff protocol.
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

/// A correctly signed receipt for the deployed revision.
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

fn codes(verdict: &commands::handoff::conformance::ReceiptVerdict) -> Vec<&'static str> {
    verdict.findings.iter().map(|f| f.code).collect()
}

#[test]
fn producer_and_receipt_bind_the_same_schema_version() {
    // The receipt binds the producer's v1 handoff schema verbatim.
    assert_eq!(HANDOFF_SCHEMA, "autospec.implementation-handoff.v1");
}

#[test]
fn valid_receipt_is_admitted() {
    let verdict = verify(&trace("rev-abc"), &receipt("rev-abc"));
    assert!(verdict.admitted, "{verdict:?}");
    assert!(verdict.findings.is_empty());
    assert_eq!(verdict.prerequisite.state, "satisfied");
    let rendered = verdict.to_json();
    assert_eq!(
        rendered["admitted"],
        Value::Bool(true),
        "verdict must render its typed admission"
    );
}

#[test]
fn stale_deployed_revision_receipt_is_rejected() {
    // The deployed consumer moved on to rev-2; the receipt still claims rev-1.
    let verdict = verify(&trace("rev-2"), &receipt("rev-1"));
    assert!(!verdict.admitted, "stale revision must not be admitted");
    assert_eq!(codes(&verdict), vec![CODE_REVISION_MISMATCH]);
    assert_eq!(verdict.prerequisite.state, "blocked");
}

#[test]
fn direct_dispatch_receipt_is_rejected() {
    let mut t = trace("rev-1");
    let events = t.get_mut("events").and_then(Value::as_array_mut).unwrap();
    events.push(json!({ "type": EVENT_DIRECT_DISPATCH, "agent": "impl-agent" }));
    let verdict = verify(&t, &receipt("rev-1"));
    assert!(!verdict.admitted, "direct dispatch must not be admitted");
    assert!(codes(&verdict).contains(&CODE_DIRECT_DISPATCH_REACHABLE));
}

#[test]
fn untyped_status_receipt_is_rejected() {
    let mut t = trace("rev-1");
    for event in t["events"].as_array_mut().unwrap() {
        if event["type"] == EVENT_STATUS {
            event["source"] = Value::String("terminal-prose".to_string());
        }
    }
    let verdict = verify(&t, &receipt("rev-1"));
    assert!(
        !verdict.admitted,
        "prose-scraped status must not be admitted"
    );
    assert!(codes(&verdict).contains(&CODE_STATUS_UNTYPED));
}

#[test]
fn untyped_cancellation_receipt_is_rejected() {
    let mut t = trace("rev-1");
    for event in t["events"].as_array_mut().unwrap() {
        if event["type"] == EVENT_CANCELLATION {
            event["source"] = Value::String("process-kill".to_string());
        }
    }
    let verdict = verify(&t, &receipt("rev-1"));
    assert!(
        !verdict.admitted,
        "kill-based cancellation must not be admitted"
    );
    assert!(codes(&verdict).contains(&CODE_CANCELLATION_UNTYPED));
}

#[test]
fn fabricated_signature_receipt_is_rejected() {
    // A receipt whose claimed revision was edited after signing: the
    // signature cannot verify and the revision is stale at the same time.
    let mut r = receipt("rev-1");
    r["consumer_revision"] = Value::String("rev-2".to_string());
    let verdict = verify(&trace("rev-2"), &r);
    assert!(!verdict.admitted, "an unsigned edit must not be admitted");
    assert!(codes(&verdict).contains(&CODE_RECEIPT_SIGNATURE_MISMATCH));
}

#[test]
fn conformance_command_admits_valid_receipt() {
    let fixture = Fixture::new("valid");
    let receipt_path = fixture.write("receipt.json", &receipt("rev-1"));
    let trace_path = fixture.write("trace.json", &trace("rev-1"));
    let result = commands::handoff::run(&[
        "conformance".to_string(),
        "--receipt".to_string(),
        receipt_path.to_string_lossy().into_owned(),
        "--trace".to_string(),
        trace_path.to_string_lossy().into_owned(),
    ]);
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn conformance_command_rejects_invalid_receipt() {
    let fixture = Fixture::new("stale");
    let receipt_path = fixture.write("receipt.json", &receipt("rev-1"));
    let trace_path = fixture.write("trace.json", &trace("rev-2"));
    let result = commands::handoff::run(&[
        "conformance".to_string(),
        "--receipt".to_string(),
        receipt_path.to_string_lossy().into_owned(),
        "--trace".to_string(),
        trace_path.to_string_lossy().into_owned(),
    ]);
    let failure = result.expect_err("a stale receipt must reject with non-zero status");
    assert_eq!(failure.exit_code, 1, "rejection must exit 1");
}
