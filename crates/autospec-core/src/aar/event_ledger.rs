//! The routing-ledger half of a Pi event: the row contract, its wire
//! round-trip, and the audit that reads a ledger back.
//!
//! [`PiEventRecord`] is built by [`crate::aar::pi_events::normalize_event`];
//! this module owns what happens to it on the way in and out of the ledger.
//! Three rules make the file safe to append to from several processes at once:
//! a row is appendable only when it validates, one line is exactly one record,
//! and no operation here rewrites a line that is already on disk.
//!
//! As elsewhere in AAR this is pure: text in, records and text out. The caller
//! holds the file handle.

use serde_json::Value;

use crate::aar::pi_events::{first_text, PiEventRecord, EVENT_RECORD_TYPE};

/// The `record_type` value used when auditing a legacy dispatch row.
pub const DISPATCH_RECORD_TYPE: &str = "dispatch";

impl PiEventRecord {
    /// The four identity fields every row must carry, naming the first blank.
    pub fn missing_identity_field(&self) -> Option<&'static str> {
        for (name, value) in [
            ("timestamp", self.timestamp.as_str()),
            ("session_id", self.session_id.as_str()),
            ("work_item_id", self.work_item_id.as_str()),
            ("agent_role", self.agent_role.as_str()),
        ] {
            if value.trim().is_empty() {
                return Some(name);
            }
        }
        None
    }

    /// Reject a row that cannot be correlated or aggregated.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(field) = self.missing_identity_field() {
            return Err(format!(
                "{} event requires a non-empty {}",
                self.event.as_str(),
                field
            ));
        }
        if self.schema_version == 0 {
            return Err("event row requires a non-zero schema_version".to_string());
        }
        if self.record_type != EVENT_RECORD_TYPE {
            return Err(format!(
                "event row record_type must be {}",
                EVENT_RECORD_TYPE
            ));
        }
        if let Some(finding) = self.incoherent_metrics() {
            return Err(finding);
        }
        Ok(())
    }

    /// The first metric pair that contradicts the other, if any. A pair is only
    /// checked when both sides were measured: two unknowns are a gap, not a
    /// conflict.
    fn incoherent_metrics(&self) -> Option<String> {
        let cache_tokens = self
            .cache_hit_tokens
            .0
            .zip(self.cache_miss_tokens.0)
            .map(|(hit, miss)| hit.saturating_add(miss));
        [
            not_exceeding(
                cache_tokens,
                self.input_tokens.0,
                "cache tokens exceed the input tokens",
            ),
            not_exceeding(
                self.context_used_tokens.0,
                self.context_window_tokens.0,
                "context usage exceeds the context window",
            ),
            not_exceeding(
                self.tests_failed.0,
                self.tests_total.0,
                "failed tests exceed the test total",
            ),
        ]
        .into_iter()
        .flatten()
        .next()
    }

    /// Render the row as one JSON line for the ledger.
    pub fn to_json_line(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("cannot serialize event row: {}", e))
    }

    /// Parse one ledger line back into a row.
    pub fn from_json_line(line: &str) -> Result<Self, String> {
        serde_json::from_str(line).map_err(|e| format!("malformed event row: {}", e))
    }
}

/// `message` when a measured part exceeds its measured whole.
fn not_exceeding(part: Option<u64>, whole: Option<u64>, message: &str) -> Option<String> {
    match (part, whole) {
        (Some(part), Some(whole)) if part > whole => {
            Some(format!("{}: {} > {}", message, part, whole))
        }
        _ => None,
    }
}

/// Render records as ledger lines, ready for an append.
pub fn to_ledger_lines(records: &[PiEventRecord]) -> Result<String, String> {
    let mut out = String::new();
    for record in records {
        out.push_str(&record.to_json_line()?);
        out.push('\n');
    }
    Ok(out)
}

/// What an audit of ledger text found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerAudit {
    /// Event rows satisfying the identity and metric contract.
    pub events: usize,
    /// Legacy dispatch rows, left alone by this contract.
    pub dispatches: usize,
    /// `<line>: <finding>` for every row that is neither.
    pub findings: Vec<String>,
}

impl LedgerAudit {
    pub fn ok(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Validate the rows of a routing ledger: event rows against this contract,
/// dispatch rows by presence of a dispatch id. A row that is neither is a
/// finding -- the ledger holds records, not prose.
pub fn audit_ledger(text: &str) -> LedgerAudit {
    let mut audit = LedgerAudit::default();
    for (index, line) in text.lines().enumerate() {
        if !line.trim().is_empty() {
            audit_line(line, index + 1, &mut audit);
        }
    }
    audit
}

/// Classify one ledger line, recording the row or its finding.
fn audit_line(line: &str, line_number: usize, audit: &mut LedgerAudit) {
    let raw: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => return audit.found(line_number, format!("malformed json: {}", e)),
    };
    let record_type = first_text(&raw, &["record_type"]).unwrap_or_default();
    if record_type == EVENT_RECORD_TYPE {
        match PiEventRecord::from_json_line(line).and_then(|row| row.validate().map(|_| row)) {
            Ok(_) => audit.events += 1,
            Err(finding) => audit.found(line_number, finding),
        }
    } else if record_type.is_empty() && first_text(&raw, &["dispatch_id"]).is_some() {
        audit.dispatches += 1;
    } else {
        audit.found(
            line_number,
            "neither an event nor a dispatch record".to_string(),
        );
    }
}

impl LedgerAudit {
    /// Record a finding against a 1-based ledger line.
    fn found(&mut self, line_number: usize, finding: String) {
        self.findings.push(format!("{}: {}", line_number, finding));
    }
}
