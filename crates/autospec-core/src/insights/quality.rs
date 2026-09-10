//! Insights: quality finding ingestion (issue #3847).
//!
//! Spec: [`docs/specs/2026-09-08-continuous-improvement-engine.md`](../../../../docs/specs/2026-09-08-continuous-improvement-engine.md)
//! §19 (Code-Quality Feedback Loop), §32 (Quality Dashboard), §35 (Evidence
//! Preservation), §50 (Failure Handling).
//!
//! This module owns the "Structured Finding → Session Correlation" edge of
//! the §19 pipeline:
//!
//! ```text
//! Implementation → Quality Gate Failure → Structured Finding
//!   → Session Correlation → Recurring Pattern → Improvement Proposal
//! ```
//!
//! Each [`QualitySource`] adapter parses one raw quality-gate report (clippy
//! diagnostics, rustc type errors, complexity metrics, a duplication report,
//! reviewer comments) into typed [`QualityFinding`] records. [`record`]
//! upserts those records into the insights store, always linked to the
//! session that produced them:
//!
//! * reviewer output → `review_findings` (taxonomy in the `category` column)
//! * scanner output  → `quality_findings` (taxonomy in the `gate` column)
//!
//! The upsert key is (session, rule, path, line): re-ingesting the same
//! report updates the same row instead of duplicating it.
//!
//! Failure handling (spec §50, "corrupted session → quarantine session and
//! continue"): an unparsable report never aborts the run. [`ingest_report`]
//! writes exactly one quarantine row for it and returns [`Ok`] with a
//! partial outcome; the quarantine row is still linked to a session and a
//! rule, so no data lands unattributed.

use crate::error::AutospecError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{AnyPool, Executor, Row};

/// Rule id assigned to quarantine rows for unparsable reports.
pub const QUARANTINE_RULE_ID: &str = "quarantine";

/// §19 source taxonomy for a quality finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualitySourceKind {
    /// Lint scanner (e.g. cargo clippy) diagnostics.
    Lint,
    /// Compiler type checking.
    TypeCheck,
    /// Complexity scanner metrics.
    Complexity,
    /// Duplication detection.
    Duplication,
    /// Reviewer (human or agent) feedback.
    Reviewer,
}

impl QualitySourceKind {
    /// Machine name of the source.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lint => "lint",
            Self::TypeCheck => "type_check",
            Self::Complexity => "complexity",
            Self::Duplication => "duplication",
            Self::Reviewer => "reviewer",
        }
    }

    /// §19 taxonomy label stored in the row's taxonomy column.
    pub fn category(self) -> &'static str {
        match self {
            Self::Lint => "linter",
            Self::TypeCheck => "type_checking",
            Self::Complexity => "complexity_scanner",
            Self::Duplication => "duplication_detection",
            Self::Reviewer => "reviewer_feedback",
        }
    }

    /// Destination table: reviewer output → `review_findings`, scanner
    /// output → `quality_findings`.
    pub fn table(self) -> &'static str {
        match self {
            Self::Reviewer => "review_findings",
            _ => "quality_findings",
        }
    }

    /// Name of the taxonomy column in the destination table.
    pub fn taxonomy_column(self) -> &'static str {
        match self {
            Self::Reviewer => "category",
            _ => "gate",
        }
    }
}

/// A structured quality finding correlated to the session that produced it.
///
/// §35 evidence preservation: `session_id` and `rule_id` are never empty —
/// every row must trace back to a session and a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityFinding {
    /// Session that produced the finding (§19 "Session Correlation").
    pub session_id: String,
    /// Which kind of quality gate produced it.
    pub source: QualitySourceKind,
    /// Rule/identifier of the finding (e.g. `clippy::unwrap_used`, `E0308`,
    /// `MOCK_DB`).
    pub rule_id: String,
    /// Severity reported by the gate (e.g. `error`, `warning`); `None` when
    /// the gate reports none.
    pub severity: Option<String>,
    /// File path the finding points at, when the report carries one.
    pub path: Option<String>,
    /// 1-based line number, when the report carries one.
    pub line: Option<u32>,
    /// Human-readable finding message.
    pub message: String,
    /// §19 taxonomy label (adapters set this from
    /// [`QualitySourceKind::category`]).
    pub category: String,
}

/// A quality-gate report adapter (§19 "Sources").
///
/// Adapters parse one raw report into typed findings. A report that cannot
/// be parsed at the envelope/section level is an error — [`ingest_report`]
/// quarantines it instead of aborting the run (spec §50).
pub trait QualitySource {
    /// The kind of gate this adapter parses.
    fn kind(&self) -> QualitySourceKind;

    /// Parse one raw report. The report envelope must carry a non-empty
    /// `session_id`; individual malformed entries are skipped.
    fn parse(&self, raw: &str) -> Result<Vec<QualityFinding>, AutospecError>;
}

/// Lint scanner adapter (e.g. cargo clippy machine diagnostics).
///
/// Report shape:
///
/// ```json
/// {
///   "session_id": "sess-1",
///   "diagnostics": [
///     {
///       "code": {"code": "clippy::unwrap_used"},
///       "message": "called `unwrap()` on a `Result` value",
///       "level": "warning",
///       "spans": [{"file_name": "crates/a/src/b.rs", "line_start": 12}]
///     }
///   ]
/// }
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct LintSource;

impl QualitySource for LintSource {
    fn kind(&self) -> QualitySourceKind {
        QualitySourceKind::Lint
    }

    fn parse(&self, raw: &str) -> Result<Vec<QualityFinding>, AutospecError> {
        let context = "lint report";
        let (session_id, value) = parse_envelope(raw, context)?;
        let diagnostics = findings_array(&value, "diagnostics", context)?;
        let kind = self.kind();
        let mut findings = Vec::new();
        for diagnostic in diagnostics {
            let rule_id = diagnostic
                .get("code")
                .and_then(|code| code.get("code"))
                .and_then(Value::as_str);
            let message = diagnostic.get("message").and_then(Value::as_str);
            let (Some(rule_id), Some(message)) = (rule_id, message) else {
                continue;
            };
            let severity = diagnostic
                .get("level")
                .and_then(Value::as_str)
                .map(str::to_string);
            let (path, line) = diagnostic
                .get("spans")
                .and_then(Value::as_array)
                .and_then(|spans| spans.first())
                .map(|span| {
                    (
                        span.get("file_name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        span.get("line_start")
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok()),
                    )
                })
                .unwrap_or((None, None));
            findings.push(QualityFinding {
                session_id: session_id.clone(),
                source: kind,
                rule_id: rule_id.to_string(),
                severity,
                path,
                line,
                message: message.to_string(),
                category: kind.category().to_string(),
            });
        }
        Ok(findings)
    }
}

/// Compiler type-checking adapter (rustc JSON errors).
///
/// Report shape:
///
/// ```json
/// {
///   "session_id": "sess-2",
///   "errors": [
///     {"code": "E0308", "message": "mismatched types", "file": "crates/a/src/b.rs", "line": 7, "level": "error"}
///   ]
/// }
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct TypeCheckSource;

impl QualitySource for TypeCheckSource {
    fn kind(&self) -> QualitySourceKind {
        QualitySourceKind::TypeCheck
    }

    fn parse(&self, raw: &str) -> Result<Vec<QualityFinding>, AutospecError> {
        let context = "type check report";
        let (session_id, value) = parse_envelope(raw, context)?;
        let errors = findings_array(&value, "errors", context)?;
        let kind = self.kind();
        let mut findings = Vec::new();
        for error in errors {
            let rule_id = error.get("code").and_then(Value::as_str);
            let message = error.get("message").and_then(Value::as_str);
            let (Some(rule_id), Some(message)) = (rule_id, message) else {
                continue;
            };
            findings.push(QualityFinding {
                session_id: session_id.clone(),
                source: kind,
                rule_id: rule_id.to_string(),
                severity: error
                    .get("level")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                path: error
                    .get("file")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                line: error
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
                message: message.to_string(),
                category: kind.category().to_string(),
            });
        }
        Ok(findings)
    }
}

/// Complexity scanner adapter.
///
/// Report shape:
///
/// ```json
/// {
///   "session_id": "sess-3",
///   "functions": [
///     {"name": "process", "path": "crates/a/src/b.rs", "line": 10, "cyclomatic": 42}
///   ]
/// }
/// ```
///
/// Only functions whose cyclomatic complexity is strictly above the
/// threshold produce a finding (rule id `COMPLEXITY`).
#[derive(Debug, Clone)]
pub struct ComplexitySource {
    /// Cyclomatic complexity threshold; functions strictly above it are
    /// reported.
    pub threshold: u32,
}

impl ComplexitySource {
    /// Default cyclomatic complexity threshold.
    pub const DEFAULT_THRESHOLD: u32 = 15;

    /// Create a source with an explicit threshold.
    pub fn new(threshold: u32) -> Self {
        Self { threshold }
    }
}

impl Default for ComplexitySource {
    fn default() -> Self {
        Self {
            threshold: Self::DEFAULT_THRESHOLD,
        }
    }
}

impl QualitySource for ComplexitySource {
    fn kind(&self) -> QualitySourceKind {
        QualitySourceKind::Complexity
    }

    fn parse(&self, raw: &str) -> Result<Vec<QualityFinding>, AutospecError> {
        let context = "complexity report";
        let (session_id, value) = parse_envelope(raw, context)?;
        let functions = findings_array(&value, "functions", context)?;
        let kind = self.kind();
        let mut findings = Vec::new();
        for function in functions {
            let cyclomatic = match function
                .get("cyclomatic")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
            {
                Some(v) if v > self.threshold => v,
                _ => continue,
            };
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed>");
            findings.push(QualityFinding {
                session_id: session_id.clone(),
                source: kind,
                rule_id: "COMPLEXITY".to_string(),
                severity: Some("warning".to_string()),
                path: function
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                line: function
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
                message: format!(
                    "function `{name}` cyclomatic complexity {cyclomatic} exceeds threshold {}",
                    self.threshold
                ),
                category: kind.category().to_string(),
            });
        }
        Ok(findings)
    }
}

/// Duplication detection adapter (jscpd-style line-oriented report).
///
/// Report shape:
///
/// ```text
/// # session: sess-4
/// dup crates/a/src/b.rs:10 <-> crates/c/src/d.rs:40
/// ```
///
/// The first line must be the `# session:` header; each `dup` line names two
/// `path:line` locations joined by `<->`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DuplicationSource;

impl QualitySource for DuplicationSource {
    fn kind(&self) -> QualitySourceKind {
        QualitySourceKind::Duplication
    }

    fn parse(&self, raw: &str) -> Result<Vec<QualityFinding>, AutospecError> {
        let context = "duplication report";
        let mut session_id = None;
        let mut findings = Vec::new();
        for (index, raw_line) in raw.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(header) = line.strip_prefix("# session:") {
                let session = header.trim();
                if session.is_empty() || session_id.is_some() {
                    return Err(AutospecError::parse(
                        context,
                        format!("line {}: malformed `# session:` header", index + 1),
                    ));
                }
                session_id = Some(session.to_string());
                continue;
            }
            if let Some(pair) = line.strip_prefix("dup ") {
                if let Some((first, second)) = parse_dup_pair(pair) {
                    let kind = self.kind();
                    findings.push(QualityFinding {
                        session_id: session_id.clone().unwrap_or_default(),
                        source: kind,
                        rule_id: "DUPLICATE_CODE".to_string(),
                        severity: Some("info".to_string()),
                        path: Some(first.0),
                        line: Some(first.1),
                        message: format!("duplicated with {}:{}", second.0, second.1),
                        category: kind.category().to_string(),
                    });
                }
            }
        }
        match session_id {
            Some(session) if !session.is_empty() => Ok(findings),
            _ => Err(AutospecError::parse(context, "missing `# session:` header")),
        }
    }
}

/// Reviewer feedback adapter (agent LGTM / reviewer comments).
///
/// Report shape:
///
/// ```json
/// {
///   "session_id": "sess-5",
///   "review": {
///     "verdict": "changes_requested",
///     "comments": [
///       {"rule_id": "MOCK_DB", "severity": "error", "path": "tests/unit/x.bats", "line": 5, "message": "DB mock in test"}
///     ]
///   }
/// }
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct ReviewerSource;

impl QualitySource for ReviewerSource {
    fn kind(&self) -> QualitySourceKind {
        QualitySourceKind::Reviewer
    }

    fn parse(&self, raw: &str) -> Result<Vec<QualityFinding>, AutospecError> {
        let context = "reviewer report";
        let (session_id, value) = parse_envelope(raw, context)?;
        let review = value
            .get("review")
            .and_then(Value::as_object)
            .ok_or_else(|| AutospecError::parse(context, "missing `review` object"))?;
        let comments = review
            .get("comments")
            .and_then(Value::as_array)
            .ok_or_else(|| AutospecError::parse(context, "missing `review.comments` array"))?;
        let kind = self.kind();
        let mut findings = Vec::new();
        for comment in comments {
            let rule_id = comment.get("rule_id").and_then(Value::as_str);
            let message = comment.get("message").and_then(Value::as_str);
            let (Some(rule_id), Some(message)) = (rule_id, message) else {
                continue;
            };
            findings.push(QualityFinding {
                session_id: session_id.clone(),
                source: kind,
                rule_id: rule_id.to_string(),
                severity: comment
                    .get("severity")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                path: comment
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                line: comment
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
                message: message.to_string(),
                category: kind.category().to_string(),
            });
        }
        Ok(findings)
    }
}

/// Upserts `findings` into the insights store, routing reviewer findings to
/// `review_findings` and scanner findings to `quality_findings`.
///
/// Upsert key: (session, rule, path, line) — re-recording the same finding
/// updates the existing row. Returns the number of rows upserted.
pub async fn record(pool: &AnyPool, findings: &[QualityFinding]) -> Result<usize, AutospecError> {
    let mut upserted = 0;
    for finding in findings {
        upsert_finding(pool, finding, "active").await?;
        upserted += 1;
    }
    Ok(upserted)
}

/// Outcome of ingesting one raw report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestOutcome {
    /// Findings parsed and upserted.
    pub recorded: usize,
    /// Quarantine rows written (0 or 1 per report).
    pub quarantined: usize,
}

impl IngestOutcome {
    /// Spec §50: a quarantined report does not abort the run, but the run
    /// must report as partial.
    pub fn is_partial(&self) -> bool {
        self.quarantined > 0
    }
}

/// Parses one raw report with `source` and records the resulting findings.
///
/// An unparsable report writes exactly one quarantine row (rule id
/// [`QUARANTINE_RULE_ID`], status `quarantined`) linked to `session_id` and
/// the call still returns [`Ok`] — the run continues, flagged partial (spec
/// §50).
pub async fn ingest_report(
    pool: &AnyPool,
    source: &dyn QualitySource,
    session_id: &str,
    raw: &str,
) -> Result<IngestOutcome, AutospecError> {
    if session_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "ingest_report: session_id must be non-empty",
        ));
    }
    match source.parse(raw) {
        Ok(findings) => {
            let recorded = record(pool, &findings).await?;
            Ok(IngestOutcome {
                recorded,
                quarantined: 0,
            })
        }
        Err(error) => {
            let kind = source.kind();
            let quarantine = QualityFinding {
                session_id: session_id.to_string(),
                source: kind,
                rule_id: QUARANTINE_RULE_ID.to_string(),
                severity: None,
                path: None,
                line: None,
                message: error.to_string(),
                category: kind.category().to_string(),
            };
            upsert_finding(pool, &quarantine, "quarantined").await?;
            Ok(IngestOutcome {
                recorded: 0,
                quarantined: 1,
            })
        }
    }
}

/// Deterministic row id for (session, rule, path, line) — the upsert key.
fn stable_finding_id(finding: &QualityFinding) -> String {
    let prefix = if finding.source == QualitySourceKind::Reviewer {
        "review-finding"
    } else {
        "quality-finding"
    };
    format!(
        "{prefix}|{}|{}|{}|{}",
        finding.session_id,
        finding.rule_id,
        finding.path.as_deref().unwrap_or("-"),
        finding
            .line
            .map(|l| l.to_string())
            .unwrap_or_else(|| "-".to_string()),
    )
}

async fn upsert_finding(
    pool: &AnyPool,
    finding: &QualityFinding,
    status: &str,
) -> Result<(), AutospecError> {
    if finding.session_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "quality finding is missing a session_id",
        ));
    }
    if finding.rule_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "quality finding is missing a rule_id",
        ));
    }
    let id = stable_finding_id(finding);
    let table = finding.source.table();
    let taxonomy = finding.source.taxonomy_column();
    let sql = format!(
        "INSERT INTO {table} (id, session_id, work_item_id, repo, {taxonomy}, severity, title, description, status) \
         VALUES (?, ?, NULL, NULL, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET \
             {taxonomy} = excluded.{taxonomy}, \
             severity = excluded.severity, \
             title = excluded.title, \
             description = excluded.description, \
             status = excluded.status"
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(&finding.session_id)
        .bind(&finding.category)
        .bind(&finding.severity)
        .bind(&finding.rule_id)
        .bind(&finding.message)
        .bind(status)
        .execute(pool)
        .await
        .map_err(|e| AutospecError::io("upsert quality finding", table, e))?;
    Ok(())
}

/// Parses the shared JSON envelope: a top-level object carrying a non-empty
/// `session_id`.
fn parse_envelope(raw: &str, context: &str) -> Result<(String, Value), AutospecError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| AutospecError::parse(context, format!("not valid JSON: {e}")))?;
    if !value.is_object() {
        return Err(AutospecError::parse(context, "report is not a JSON object"));
    }
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AutospecError::parse(context, "missing or empty session_id"))?
        .to_string();
    Ok((session_id, value))
}

/// Extracts the report's findings array under `key`.
fn findings_array<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a [Value], AutospecError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|v| v.as_slice())
        .ok_or_else(|| AutospecError::parse(context, format!("missing `{key}` array")))
}

/// Parses one `path:line <-> path:line` pair; `None` for any malformed
/// shape (individual malformed entries are skipped, like the JSON adapters
/// skip malformed entries).
fn parse_dup_pair(pair: &str) -> Option<((String, u32), (String, u32))> {
    let (left, right) = pair.split_once(" <-> ")?;
    Some((parse_location(left)?, parse_location(right)?))
}

/// Parses one `path:line` location token.
fn parse_location(token: &str) -> Option<(String, u32)> {
    let (path, line) = token.rsplit_once(':')?;
    if path.is_empty() {
        return None;
    }
    Some((path.to_string(), line.parse::<u32>().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    /// Serializes the integration tests: `AUTOSPEC_TEST_DB_URL` names a
    /// shared disposable database whose per-test counts the assertions
    /// below depend on.
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static DIR_COUNTER: Mutex<u32> = Mutex::new(0);

    fn fresh_tmp_dir() -> PathBuf {
        let mut counter = DIR_COUNTER.lock().unwrap();
        *counter += 1;
        let dir = std::env::temp_dir().join(format!(
            "autospec-quality-test-{}-{}",
            std::process::id(),
            *counter
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Mirror of `migrations/insights/3000001_init.sql` (review_findings +
    /// quality_findings); the FK to sessions is omitted, like the
    /// `correlate::tests` mirror does.
    const TEST_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS review_findings (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            work_item_id TEXT,
            repo TEXT,
            category TEXT NOT NULL,
            severity TEXT,
            title TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS quality_findings (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            work_item_id TEXT,
            repo TEXT,
            gate TEXT NOT NULL,
            severity TEXT,
            title TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
    "#;

    /// Holds the test lock for the lifetime of the returned guard.
    fn lock_tests() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `AUTOSPEC_TEST_DB_URL` (a shared disposable database) when set; a
    /// fresh per-test SQLite file otherwise. Both are real databases; the
    /// tables are truncated after schema creation so counts start clean.
    async fn test_pool() -> (AnyPool, PathBuf) {
        let (url, dir) = match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(value) if !value.trim().is_empty() => (value, PathBuf::new()),
            _ => {
                let dir = fresh_tmp_dir();
                (
                    format!("sqlite://{}/insights-quality.db", dir.display()),
                    dir,
                )
            }
        };
        let pool = crate::resources::db::open_shared_db(&url).await.unwrap();
        pool.execute(TEST_SCHEMA).await.unwrap();
        pool.execute("DELETE FROM review_findings").await.unwrap();
        pool.execute("DELETE FROM quality_findings").await.unwrap();
        (pool, dir)
    }

    async fn count(pool: &AnyPool, table: &str, column: &str, value: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) AS n FROM {table} WHERE {column} = ?");
        let row = sqlx::query(&sql).bind(value).fetch_one(pool).await.unwrap();
        row.get::<i64, _>("n")
    }

    fn lint_report() -> String {
        json!({
            "session_id": "sess-1",
            "diagnostics": [
                {
                    "code": {"code": "clippy::unwrap_used"},
                    "message": "called `unwrap()` on a `Result` value",
                    "level": "warning",
                    "spans": [{"file_name": "crates/a/src/b.rs", "line_start": 12}]
                },
                {
                    "code": {"code": "clippy::todo"},
                    "message": "todo!",
                    "level": "warning",
                    "spans": [{"file_name": "crates/a/src/c.rs", "line_start": 3}]
                }
            ]
        })
        .to_string()
    }

    fn lint_finding(rule: &str, line: Option<u32>) -> QualityFinding {
        QualityFinding {
            session_id: "sess-1".to_string(),
            source: QualitySourceKind::Lint,
            rule_id: rule.to_string(),
            severity: Some("warning".to_string()),
            path: Some("crates/a/src/b.rs".to_string()),
            line,
            message: "finding message".to_string(),
            category: QualitySourceKind::Lint.category().to_string(),
        }
    }

    fn reviewer_finding() -> QualityFinding {
        QualityFinding {
            session_id: "sess-5".to_string(),
            source: QualitySourceKind::Reviewer,
            rule_id: "MOCK_DB".to_string(),
            severity: Some("error".to_string()),
            path: Some("tests/unit/x.bats".to_string()),
            line: Some(5),
            message: "DB mock in test".to_string(),
            category: QualitySourceKind::Reviewer.category().to_string(),
        }
    }

    #[test]
    fn lint_source_parses_diagnostics() {
        let findings = LintSource.parse(&lint_report()).unwrap();
        assert_eq!(findings.len(), 2);
        let finding = &findings[0];
        assert_eq!(finding.session_id, "sess-1");
        assert_eq!(finding.source, QualitySourceKind::Lint);
        assert_eq!(finding.rule_id, "clippy::unwrap_used");
        assert_eq!(finding.severity.as_deref(), Some("warning"));
        assert_eq!(finding.path.as_deref(), Some("crates/a/src/b.rs"));
        assert_eq!(finding.line, Some(12));
        assert_eq!(finding.message, "called `unwrap()` on a `Result` value");
        assert_eq!(finding.category, "linter");
    }

    #[test]
    fn lint_source_rejects_report_without_session() {
        let raw = json!({"diagnostics": []}).to_string();
        assert!(matches!(
            LintSource.parse(&raw).unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    #[test]
    fn lint_source_rejects_non_json() {
        assert!(matches!(
            LintSource.parse("### not a report ###").unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    #[test]
    fn lint_source_rejects_missing_diagnostics() {
        let raw = json!({"session_id": "s", "errors": []}).to_string();
        assert!(matches!(
            LintSource.parse(&raw).unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    #[test]
    fn lint_source_skips_malformed_entries() {
        let raw = json!({
            "session_id": "s",
            "diagnostics": [
                {"message": "no code"},
                {"code": {"code": "clippy::dbg_macro"}, "message": "dbg"}
            ]
        })
        .to_string();
        let findings = LintSource.parse(&raw).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, "clippy::dbg_macro");
        assert_eq!(findings[0].severity, None);
        assert_eq!(findings[0].path, None);
        assert_eq!(findings[0].line, None);
    }

    #[test]
    fn type_check_source_parses_errors() {
        let raw = json!({
            "session_id": "sess-2",
            "errors": [
                {"code": "E0308", "message": "mismatched types", "file": "crates/a/src/b.rs", "line": 7, "level": "error"}
            ]
        })
        .to_string();
        let findings = TypeCheckSource.parse(&raw).unwrap();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.session_id, "sess-2");
        assert_eq!(finding.source, QualitySourceKind::TypeCheck);
        assert_eq!(finding.rule_id, "E0308");
        assert_eq!(finding.severity.as_deref(), Some("error"));
        assert_eq!(finding.path.as_deref(), Some("crates/a/src/b.rs"));
        assert_eq!(finding.line, Some(7));
        assert_eq!(finding.category, "type_checking");
    }

    #[test]
    fn type_check_source_rejects_missing_errors() {
        let raw = json!({"session_id": "s"}).to_string();
        assert!(matches!(
            TypeCheckSource.parse(&raw).unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    #[test]
    fn complexity_source_flags_over_threshold() {
        let raw = json!({
            "session_id": "sess-3",
            "functions": [
                {"name": "process", "path": "crates/a/src/b.rs", "line": 10, "cyclomatic": 42},
                {"name": "small", "path": "crates/a/src/b.rs", "line": 50, "cyclomatic": 5}
            ]
        })
        .to_string();
        let findings = ComplexitySource::default().parse(&raw).unwrap();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule_id, "COMPLEXITY");
        assert_eq!(finding.line, Some(10));
        assert_eq!(finding.source, QualitySourceKind::Complexity);
        assert!(finding.message.contains("42"));
        assert_eq!(finding.category, "complexity_scanner");
    }

    #[test]
    fn complexity_source_threshold_is_configurable() {
        let raw = json!({
            "session_id": "sess-3",
            "functions": [
                {"name": "process", "path": "crates/a/src/b.rs", "line": 10, "cyclomatic": 42}
            ]
        })
        .to_string();
        assert!(ComplexitySource::new(100).parse(&raw).unwrap().is_empty());
    }

    #[test]
    fn complexity_source_rejects_missing_functions() {
        let raw = json!({"session_id": "s"}).to_string();
        assert!(matches!(
            ComplexitySource::default().parse(&raw).unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    #[test]
    fn duplication_source_parses_text_report() {
        let raw = "# session: sess-4\ndup crates/a.rs:10 <-> crates/b.rs:40\ndup crates/c.rs:1 <-> crates/d.rs:2\n";
        let findings = DuplicationSource.parse(raw).unwrap();
        assert_eq!(findings.len(), 2);
        let finding = &findings[0];
        assert_eq!(finding.session_id, "sess-4");
        assert_eq!(finding.rule_id, "DUPLICATE_CODE");
        assert_eq!(finding.path.as_deref(), Some("crates/a.rs"));
        assert_eq!(finding.line, Some(10));
        assert!(finding.message.contains("crates/b.rs:40"));
        assert_eq!(finding.category, "duplication_detection");
    }

    #[test]
    fn duplication_source_rejects_missing_header() {
        assert!(matches!(
            DuplicationSource
                .parse("dup a.rs:1 <-> b.rs:2")
                .unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    #[test]
    fn duplication_source_skips_malformed_lines() {
        let raw = "# session: s\ndup no separator here\n";
        assert!(DuplicationSource.parse(raw).unwrap().is_empty());
    }

    #[test]
    fn duplication_source_skips_malformed_location() {
        let raw = "# session: s\ndup a.rs:notanumber <-> b.rs:2\n";
        assert!(DuplicationSource.parse(raw).unwrap().is_empty());
    }

    #[test]
    fn duplication_source_reports_both_valid_and_malformed_lines() {
        let raw = "# session: s\ndup broken line\ndup a.rs:1 <-> b.rs:2\n";
        let findings = DuplicationSource.parse(raw).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, Some(1));
    }

    fn reviewer_report() -> String {
        json!({
            "session_id": "sess-5",
            "review": {
                "verdict": "changes_requested",
                "comments": [
                    {"rule_id": "MOCK_DB", "severity": "error", "path": "tests/unit/x.bats", "line": 5, "message": "DB mock in test"}
                ]
            }
        })
        .to_string()
    }

    #[test]
    fn reviewer_source_parses_comments() {
        let findings = ReviewerSource.parse(&reviewer_report()).unwrap();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.session_id, "sess-5");
        assert_eq!(finding.source, QualitySourceKind::Reviewer);
        assert_eq!(finding.rule_id, "MOCK_DB");
        assert_eq!(finding.severity.as_deref(), Some("error"));
        assert_eq!(finding.path.as_deref(), Some("tests/unit/x.bats"));
        assert_eq!(finding.line, Some(5));
        assert_eq!(finding.category, "reviewer_feedback");
    }

    #[test]
    fn reviewer_source_rejects_missing_review() {
        let raw = json!({"session_id": "s"}).to_string();
        assert!(matches!(
            ReviewerSource.parse(&raw).unwrap_err(),
            AutospecError::Parse { .. }
        ));
    }

    #[test]
    fn reviewer_source_skips_malformed_comments() {
        let raw = json!({
            "session_id": "s",
            "review": {"comments": [{"message": "no rule"}, {"rule_id": "R1", "message": "m"}]}
        })
        .to_string();
        let findings = ReviewerSource.parse(&raw).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, "R1");
    }

    #[test]
    fn source_kind_maps_to_tables() {
        assert_eq!(QualitySourceKind::Reviewer.table(), "review_findings");
        assert_eq!(QualitySourceKind::Reviewer.taxonomy_column(), "category");
        for kind in [
            QualitySourceKind::Lint,
            QualitySourceKind::TypeCheck,
            QualitySourceKind::Complexity,
            QualitySourceKind::Duplication,
        ] {
            assert_eq!(kind.table(), "quality_findings");
            assert_eq!(kind.taxonomy_column(), "gate");
        }
    }

    #[tokio::test]
    async fn record_upserts_by_rule_and_location() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        let finding = lint_finding("clippy::unwrap_used", Some(12));
        assert_eq!(record(&pool, &[finding.clone()]).await.unwrap(), 1);
        // Re-recording the same finding updates the same row.
        assert_eq!(record(&pool, &[finding]).await.unwrap(), 1);
        assert_eq!(
            count(&pool, "quality_findings", "session_id", "sess-1").await,
            1
        );
        // A different line is a different row.
        let other = lint_finding("clippy::unwrap_used", Some(13));
        record(&pool, &[other]).await.unwrap();
        assert_eq!(
            count(&pool, "quality_findings", "session_id", "sess-1").await,
            2
        );
    }

    #[tokio::test]
    async fn record_updates_existing_row_in_place() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        let mut finding = lint_finding("clippy::unwrap_used", Some(12));
        record(&pool, &[finding.clone()]).await.unwrap();
        finding.severity = Some("error".to_string());
        record(&pool, &[finding]).await.unwrap();
        assert_eq!(
            count(&pool, "quality_findings", "title", "clippy::unwrap_used").await,
            1
        );
        let row = pool
            .fetch_one("SELECT severity FROM quality_findings")
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("severity"), "error");
    }

    #[tokio::test]
    async fn record_routes_reviewer_to_review_findings() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        record(&pool, &[reviewer_finding()]).await.unwrap();
        assert_eq!(
            count(&pool, "review_findings", "session_id", "sess-5").await,
            1
        );
        assert_eq!(
            count(&pool, "quality_findings", "session_id", "sess-5").await,
            0
        );
        let row = pool
            .fetch_one("SELECT category FROM review_findings")
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("category"), "reviewer_feedback");
    }

    #[tokio::test]
    async fn record_stores_session_and_rule_on_every_row() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        record(&pool, &[lint_finding("clippy::todo", None)])
            .await
            .unwrap();
        let row = pool
            .fetch_one("SELECT session_id, title, status FROM quality_findings")
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("session_id"), "sess-1");
        assert_eq!(row.get::<String, _>("title"), "clippy::todo");
        assert_eq!(row.get::<String, _>("status"), "active");
    }

    #[tokio::test]
    async fn record_rejects_missing_session_or_rule() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        let mut finding = lint_finding("r", None);
        finding.session_id.clear();
        assert!(matches!(
            record(&pool, &[finding]).await.unwrap_err(),
            AutospecError::Validation { .. }
        ));
        let blank_rule = lint_finding("   ", None);
        assert!(matches!(
            record(&pool, &[blank_rule]).await.unwrap_err(),
            AutospecError::Validation { .. }
        ));
    }

    #[tokio::test]
    async fn ingest_unparsable_report_writes_one_quarantine_row_and_stays_ok() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        let outcome = ingest_report(&pool, &LintSource, "sess-1", "### not a report ###")
            .await
            .unwrap();
        assert_eq!(outcome.recorded, 0);
        assert_eq!(outcome.quarantined, 1);
        assert!(outcome.is_partial());
        assert_eq!(
            count(&pool, "quality_findings", "status", "quarantined").await,
            1
        );
        let row = pool
            .fetch_one(
                "SELECT session_id, title FROM quality_findings WHERE status = 'quarantined'",
            )
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("session_id"), "sess-1");
        assert_eq!(row.get::<String, _>("title"), QUARANTINE_RULE_ID);
        // Re-ingesting the same unparseable report does not add a row.
        ingest_report(&pool, &LintSource, "sess-1", "### not a report ###")
            .await
            .unwrap();
        assert_eq!(
            count(&pool, "quality_findings", "status", "quarantined").await,
            1
        );
    }

    #[tokio::test]
    async fn ingest_parsable_report_records_all_findings() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        let outcome = ingest_report(&pool, &LintSource, "sess-1", &lint_report())
            .await
            .unwrap();
        assert_eq!(outcome.recorded, 2);
        assert_eq!(outcome.quarantined, 0);
        assert!(!outcome.is_partial());
        assert_eq!(
            count(&pool, "quality_findings", "status", "active").await,
            2
        );
    }

    #[tokio::test]
    async fn ingest_quarantines_reviewer_reports_in_review_findings() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        let outcome = ingest_report(&pool, &ReviewerSource, "sess-5", "{broken")
            .await
            .unwrap();
        assert!(outcome.is_partial());
        assert_eq!(
            count(&pool, "review_findings", "status", "quarantined").await,
            1
        );
        assert_eq!(
            count(&pool, "quality_findings", "status", "quarantined").await,
            0
        );
    }

    #[tokio::test]
    async fn ingest_requires_a_session() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        assert!(matches!(
            ingest_report(&pool, &LintSource, "  ", "x")
                .await
                .unwrap_err(),
            AutospecError::Validation { .. }
        ));
    }

    #[tokio::test]
    async fn end_to_end_scan_and_review_into_one_store() {
        let _lock = lock_tests();
        let (pool, _keep) = test_pool().await;
        // Same shape as `lint_report()`, but bound to this test's session
        // (the finding's session comes from the report envelope).
        let lint = serde_json::json!({
            "session_id": "sess-9",
            "diagnostics": [
                {"code": {"code": "CLIPPY0001"}, "message": "if let is redundant",
                 "spans": [{"file_name": "src/main.rs", "line_start": 3}]}
            ]
        });
        ingest_report(
            &pool,
            &LintSource,
            "sess-9",
            &serde_json::to_string(&lint).unwrap(),
        )
        .await
        .unwrap();
        ingest_report(&pool, &ReviewerSource, "sess-9", &reviewer_report())
            .await
            .unwrap();
        assert_eq!(
            count(&pool, "quality_findings", "session_id", "sess-9").await,
            1
        );
        // The reviewer report's envelope session wins for its rows.
        assert_eq!(
            count(&pool, "review_findings", "session_id", "sess-5").await,
            1
        );
    }
}
