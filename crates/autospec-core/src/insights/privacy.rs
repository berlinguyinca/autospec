//! §39 privacy controls for the continuous improvement engine: secret
//! redaction, repository allowlists, and retention.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §39
//! (privacy and security), §35 (evidence preservation), §34 (storage,
//! issue #3827 tables `sessions` / `session_events`).
//!
//! Flow (outline step 7): **redact, check the allowlist, then hand off.**
//! [`Redactor::gate`] performs the three in that order and redacts the
//! payload *before* the allowlist verdict, so a payload for an unlisted
//! repository still never leaves this module unredacted. Every hit is
//! recorded in the [`RedactionReport`] with a stable placeholder and the
//! JSON field path it replaced, so redacted evidence stays traceable
//! (§35).
//!
//! All gates are deterministic (spec §4.1): no LLM, no network, no clock
//! reads — except [`enforce_retention`], whose clock is injectable through
//! [`enforce_retention_at`].

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use sqlx::AnyPool;

use crate::error::AutospecError;
use crate::insights::config::{InsightsConfig, RetentionConfig};

/// Prefix of every stable redaction placeholder (`[REDACTED:<kind>]`).
pub const REDACTED_PLACEHOLDER_PREFIX: &str = "[REDACTED:";

/// `sk-`-prefixed tokens (outline detector 2).
pub const KIND_SK_TOKEN: &str = "sk-token";
/// `scheme://userinfo@host` connection strings (outline detector 3).
pub const KIND_DSN_CREDENTIAL: &str = "dsn-credential";
/// `Authorization` headers and `authorization` object keys.
pub const KIND_AUTHORIZATION: &str = "authorization";
/// `Bearer <token>` occurrences inside strings.
pub const KIND_BEARER_TOKEN: &str = "bearer-token";
/// High-entropy strings (outline detector 1).
pub const KIND_HIGH_ENTROPY: &str = "high-entropy";

/// Minimum tail length after `sk-` for a token-shaped secret.
const SK_TOKEN_MIN_TAIL: usize = 16;
/// Minimum token length for the high-entropy detector.
const HIGH_ENTROPY_MIN_LEN: usize = 16;
/// Minimum Shannon entropy (bits/char) for the high-entropy detector.
const HIGH_ENTROPY_MIN_BITS: f64 = 3.8;

/// One redaction hit — the §35 traceability record for one replaced value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionEntry {
    /// JSON field path that was redacted, e.g. `$.payload.note`.
    pub path: String,
    /// Secret shape that matched (one of the `KIND_*` constants).
    pub kind: &'static str,
    /// Stable placeholder written in place of the secret.
    pub placeholder: String,
}

impl RedactionEntry {
    fn new(path: String, kind: &'static str) -> Self {
        Self {
            path,
            kind,
            placeholder: placeholder(kind),
        }
    }
}

/// Every redaction [`Redactor::redact`] applied to one value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RedactionReport {
    pub entries: Vec<RedactionEntry>,
}

impl RedactionReport {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn kinds(&self) -> impl Iterator<Item = &'static str> {
        self.entries.iter().map(|entry| entry.kind)
    }
}

/// A stable, deterministic placeholder: the same kind always yields the
/// same token, and the token carries no information from the original
/// value.
fn placeholder(kind: &'static str) -> String {
    format!("[REDACTED:{kind}]")
}

/// The §39 privacy gate (issue #3840). Built from the `InsightsConfig`
/// privacy and retention keys; the repository allowlist is supplied by the
/// operator and is **deny-by-default** — a repository the allowlist omits
/// is not allowed.
#[derive(Debug, Clone, PartialEq)]
pub struct Redactor {
    redact_secrets: bool,
    allowlist: Vec<String>,
    retention: RetentionConfig,
}

impl Redactor {
    /// Build the gate from the `insights:` configuration privacy keys
    /// (outline step 1). Until an allowlist is set, every repository is
    /// denied.
    pub fn from_config(config: &InsightsConfig) -> Self {
        Self {
            redact_secrets: config.privacy.redact_secrets,
            allowlist: Vec::new(),
            retention: config.retention.clone(),
        }
    }

    /// Set the repository allowlist (outline step 5). An entry matches the
    /// repository exactly or as an organization prefix: `acme` allows
    /// `acme/any-repo` but not `acme-evil/any-repo`.
    pub fn with_allowlist(mut self, repos: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.allowlist = repos.into_iter().map(Into::into).collect();
        self
    }

    /// Whether secret redaction is enabled by the privacy configuration.
    pub fn redaction_enabled(&self) -> bool {
        self.redact_secrets
    }

    /// The retention windows this gate was built with.
    pub fn retention(&self) -> &RetentionConfig {
        &self.retention
    }

    /// Redact every recognized secret in `value` (outline step 2). Returns
    /// the redacted value and the report of every hit; the input is never
    /// mutated.
    pub fn redact(&self, value: &Value) -> (Value, RedactionReport) {
        if !self.redact_secrets {
            return (value.clone(), RedactionReport::default());
        }
        let mut report = RedactionReport::default();
        let redacted = redact_value(value, "$", &mut report);
        (redacted, report)
    }

    /// Repository allowlist (outline step 5): `false` when the allowlist is
    /// empty or omits `repo` — deny by default.
    pub fn repo_allowed(&self, repo: &str) -> bool {
        let repo = repo.trim();
        if repo.is_empty() {
            return false;
        }
        self.allowlist.iter().any(|entry| {
            let entry = entry.trim();
            !entry.is_empty() && (entry == repo || repo.starts_with(&format!("{entry}/")))
        })
    }

    /// The hand-off flow (outline step 7): redact first, check the
    /// allowlist second, then hand off. Redaction happens *before* the
    /// verdict, so a payload for a denied repository is redacted too.
    pub fn gate(&self, repo: &str, payload: &Value) -> Handoff {
        let (payload, report) = self.redact(payload);
        Handoff {
            repo: repo.to_string(),
            allowed: self.repo_allowed(repo),
            payload,
            report,
        }
    }
}

/// The outcome of the redact → allowlist → hand-off flow.
#[derive(Debug, Clone, PartialEq)]
pub struct Handoff {
    pub repo: String,
    /// Whether `repo` is on the allowlist.
    pub allowed: bool,
    /// The payload after redaction (redacted even when `allowed` is false).
    pub payload: Value,
    pub report: RedactionReport,
}

struct Span {
    start: usize,
    end: usize,
    kind: &'static str,
    /// Lower wins on overlap; high-entropy is the weakest claim.
    priority: u8,
}

/// Every recognized secret shape in `s` (outline step 3): `sk-` tokens,
/// DSN credentials, `Authorization` headers, bearer tokens, and
/// high-entropy strings.
fn secret_spans(s: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    spans.extend(sk_token_spans(s));
    spans.extend(dsn_credential_spans(s));
    spans.extend(authorization_spans(s));
    spans.extend(bearer_spans(s));
    spans.extend(high_entropy_spans(s));
    spans
}

/// `sk-` followed by at least [`SK_TOKEN_MIN_TAIL`] token characters.
/// The `sk-` must start a word, so `risk-…` is not a hit.
fn sk_token_spans(s: &str) -> Vec<Span> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    for (i, _) in s.match_indices("sk-") {
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let tail = s[i + 3..]
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
            .count();
        if tail >= SK_TOKEN_MIN_TAIL {
            out.push(Span {
                start: i,
                end: i + 3 + tail,
                kind: KIND_SK_TOKEN,
                priority: 0,
            });
        }
    }
    out
}

/// `scheme://userinfo@host` — the userinfo (`user:pass`) is the credential
/// part and is redacted; a URL without `@` has no credential and is
/// untouched.
fn dsn_credential_spans(s: &str) -> Vec<Span> {
    let mut out = Vec::new();
    for (i, _) in s.match_indices("://") {
        let scheme_start = s[..i]
            .bytes()
            .rposition(|b| !b.is_ascii_alphanumeric())
            .map(|p| p + 1)
            .unwrap_or(0);
        if scheme_start >= i {
            continue; // no scheme before ://
        }
        let after = &s[i + 3..];
        let at = match after.find('@') {
            Some(at) => at,
            None => continue,
        };
        if after[..at].is_empty() || after[..at].contains('/') {
            continue;
        }
        out.push(Span {
            start: i + 3,
            end: i + 3 + at,
            kind: KIND_DSN_CREDENTIAL,
            priority: 1,
        });
    }
    out
}

/// An `Authorization: <value>` header line — the value runs to the end of
/// the line.
fn authorization_spans(s: &str) -> Vec<Span> {
    let lower = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let needle = "authorization";
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find(needle) {
        let i = search_from + rel;
        let before_ok =
            i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        let after_word = i + needle.len();
        let after_ok =
            after_word >= bytes.len() || !(bytes[after_word].is_ascii_alphanumeric() || bytes[after_word] == b'_');
        if before_ok && after_ok {
            let mut j = after_word;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'=') {
                j += 1;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                let mut end = j;
                while end < bytes.len() && bytes[end] != b'\n' {
                    end += 1;
                }
                if end > j {
                    out.push(Span {
                        start: i,
                        end,
                        kind: KIND_AUTHORIZATION,
                        priority: 2,
                    });
                }
            }
        }
        search_from = i + 1;
    }
    out
}

/// `Bearer <token>` — the token runs to the next whitespace.
fn bearer_spans(s: &str) -> Vec<Span> {
    let lower = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let needle = "bearer";
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find(needle) {
        let i = search_from + rel;
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_word = i + needle.len();
        let after_ok = after_word >= bytes.len() || !bytes[after_word].is_ascii_alphanumeric();
        if before_ok && after_ok {
            let mut j = after_word;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b':') {
                j += 1;
            }
            let mut end = j;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            if end > j {
                out.push(Span {
                    start: i,
                    end,
                    kind: KIND_BEARER_TOKEN,
                    priority: 3,
                });
            }
        }
        search_from = i + 1;
    }
    out
}

/// Whitespace-delimited tokens of at least [`HIGH_ENTROPY_MIN_LEN`]
/// characters that mix digits and letters and carry at least
/// [`HIGH_ENTROPY_MIN_BITS`] bits/char of Shannon entropy — the shape of
/// random tokens, not of English prose.
fn high_entropy_spans(s: &str) -> Vec<Span> {
    let mut out = Vec::new();
    for (start, token) in s.match_indices(|c: char| !c.is_whitespace()) {
        if token.len() >= HIGH_ENTROPY_MIN_LEN
            && token.bytes().any(|b| b.is_ascii_digit())
            && token.bytes().any(|b| b.is_ascii_alphabetic())
            && shannon_entropy(token) >= HIGH_ENTROPY_MIN_BITS
        {
            out.push(Span {
                start,
                end: start + token.len(),
                kind: KIND_HIGH_ENTROPY,
                priority: 4,
            });
        }
    }
    out
}

/// Shannon entropy in bits/char over the bytes of `s`.
fn shannon_entropy(s: &str) -> f64 {
    let total = s.len() as f64;
    let mut freq = [0u32; 256];
    for &b in s.as_bytes() {
        freq[b as usize] += 1;
    }
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / total;
            -p * p.log2()
        })
        .sum()
}

/// Keep spans in priority order; a span that overlaps an already-kept span
/// loses, so one hit yields exactly one placeholder.
fn resolve_spans(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by_key(|s| (s.priority, s.start));
    let mut kept: Vec<Span> = Vec::new();
    for span in spans {
        let overlaps = kept
            .iter()
            .any(|k| span.start < k.end && k.start < span.end);
        if !overlaps {
            kept.push(span);
        }
    }
    kept
}

/// Redact the recognized secret shapes in one string, recording one report
/// entry per kept hit. Returns `None` when nothing matched.
fn redact_string(s: &str, path: &str, report: &mut RedactionReport) -> Option<String> {
    let mut kept = resolve_spans(secret_spans(s));
    if kept.is_empty() {
        return None;
    }
    kept.sort_by_key(|s| s.start);
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;
    for span in kept {
        out.push_str(&s[cursor..span.start]);
        out.push_str(&placeholder(span.kind));
        report.entries.push(RedactionEntry::new(path.to_string(), span.kind));
        cursor = span.end;
    }
    out.push_str(&s[cursor..]);
    Some(out)
}

/// Redact every string leaf of a JSON value (payload evidence is stored as
/// JSON). Object keys named `authorization` (case-insensitive) are treated
/// as headers: their whole value is the secret. Non-string values pass
/// through untouched.
fn redact_value(value: &Value, path: &str, report: &mut RedactionReport) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                let child_path = child_path(path, key);
                let redacted = if key.eq_ignore_ascii_case("authorization") {
                    report.entries.push(RedactionEntry::new(child_path, KIND_AUTHORIZATION));
                    Value::String(placeholder(KIND_AUTHORIZATION))
                } else {
                    redact_value(child, &child_path, report)
                };
                out.insert(key.clone(), redacted);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, child)| redact_value(child, &format!("{path}[{i}]"), report))
                .collect(),
        ),
        Value::String(s) => redact_string(s, path, report).map_or_else(|| value.clone(), Value::String),
        other => other.clone(),
    }
}

/// `$.a.b` for identifier-like keys, `$["a b"]` otherwise.
fn child_path(parent: &str, key: &str) -> String {
    let simple = !key.is_empty()
        && key
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-');
    if simple {
        format!("{parent}.{key}")
    } else {
        format!("{parent}[{key:?}]")
    }
}

/// One bounded retention sweep (outline step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetentionSweep {
    /// `sessions` rows older than `raw_sessions_days` that were removed.
    pub raw_sessions_deleted: u64,
    /// `session_events` rows removed (expired by age, or attached to a raw
    /// session that left its own window).
    pub normalized_events_deleted: u64,
}

impl RetentionSweep {
    /// One-line verdict: the sweep names the row counts it touched, so a
    /// zero sweep is distinguishable from a broken one.
    pub fn line(&self) -> String {
        format!(
            "retention sweep: raw_sessions={} normalized_events={}",
            self.raw_sessions_deleted, self.normalized_events_deleted
        )
    }
}

/// Retention sweep (outline step 6): remove `sessions` rows older than
/// `raw_sessions_days` and `session_events` rows older than
/// `normalized_events_days`.
///
/// Normalized rows attached to a raw session that left its own window are
/// removed with that session (`session_events.session_id` references
/// `sessions.id`), so the sweep cannot orphan events.
pub async fn enforce_retention(
    pool: &AnyPool,
    config: &InsightsConfig,
) -> Result<RetentionSweep, AutospecError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AutospecError::state("insights retention", error.to_string()))?
        .as_secs() as i64;
    enforce_retention_at(pool, config, now).await
}

/// Clock-injected core of [`enforce_retention`]; `now` is unix seconds.
pub(crate) async fn enforce_retention_at(
    pool: &AnyPool,
    config: &InsightsConfig,
    now: i64,
) -> Result<RetentionSweep, AutospecError> {
    let raw_cutoff = cutoff_rfc3339(now, config.retention.raw_sessions_days);
    let events_cutoff = cutoff_rfc3339(now, config.retention.normalized_events_days);
    // The cutoffs come from this function's own clock (digits, `-`, `:`,
    // `T`, `Z`) — never user input.
    let events = sqlx::query(&format!(
        "DELETE FROM session_events \
         WHERE occurred_at < '{events_cutoff}' \
            OR session_id IN (SELECT id FROM sessions WHERE created_at < '{raw_cutoff}')"
    ))
    .execute(pool)
    .await
    .map_err(|error| AutospecError::state("insights retention", error.to_string()))?;
    let sessions = sqlx::query(&format!(
        "DELETE FROM sessions WHERE created_at < '{raw_cutoff}'"
    ))
    .execute(pool)
    .await
    .map_err(|error| AutospecError::state("insights retention", error.to_string()))?;
    Ok(RetentionSweep {
        raw_sessions_deleted: sessions.rows_affected(),
        normalized_events_deleted: events.rows_affected(),
    })
}

/// `now - days` rendered as `YYYY-MM-DDTHH:MM:SSZ` (UTC). No date library
/// in the dependency set, so the calendar math is inline and unit-tested
/// (inverse of `days_from_civil`, same as `insights::events` / `tools`).
fn cutoff_rfc3339(now: i64, days: u64) -> String {
    let secs = now - (days as i64) * 86_400;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    )
}

/// Inverse of `days_from_civil` (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::Row;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static FILE_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Local fixture with 4 planted secrets — one per detector family
    /// (outline test).
    fn planted() -> Value {
        json!({
            "payload": {
                "openai": "sk-Ab3Cd4Ef5Gh6Ij7Kl8Mn9Op0Qr1St2", // linter:allow-SECURITY fabricated sk- shape, not a real secret
                "database": "postgres://insights:SuperSecret123@db.internal:5432/telemetry", // linter:allow-SECURITY fixture DSN, not a live credential
                "headers": { "authorization": "Bearer abc.def.ghi" }, // linter:allow-SECURITY fixture bearer value
                "note": "session ended with token aB3xQ9mK2vN7pL5wR8tY1z logged" // linter:allow-SECURITY fabricated high-entropy token
            }
        })
    }

    fn redactor() -> Redactor {
        Redactor::from_config(&InsightsConfig::default())
    }

    #[test]
    fn four_planted_secrets_yield_four_placeholders_and_four_report_entries() {
        let (redacted, report) = redactor().redact(&planted());

        assert_eq!(report.len(), 4);
        let serialized = redacted.to_string();
        assert_eq!(serialized.matches(REDACTED_PLACEHOLDER_PREFIX).count(), 4);
        // No planted secret survives.
        for secret in [
            "Ab3Cd4Ef5Gh6Ij7Kl8Mn9Op0Qr1St2",
            "insights:SuperSecret123",
            "abc.def.ghi",
            "aB3xQ9mK2vN7pL5wR8tY1z",
        ] {
            assert!(!serialized.contains(secret), "secret survived: {secret}");
        }
        // One entry per detector family.
        let kinds: Vec<&'static str> = report.kinds().collect();
        for kind in [
            KIND_SK_TOKEN,
            KIND_DSN_CREDENTIAL,
            KIND_AUTHORIZATION,
            KIND_HIGH_ENTROPY,
        ] {
            assert!(kinds.contains(&kind), "missing kind {kind}: {kinds:?}");
        }
    }

    #[test]
    fn each_redaction_records_its_field_path_for_evidence_traceability() {
        let (_, report) = redactor().redact(&planted());

        // serde_json::Map iterates in key order (BTreeMap without the
        // preserve_order feature).
        let paths: Vec<&str> = report.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "$.payload.database",
                "$.payload.headers.authorization",
                "$.payload.note",
                "$.payload.openai",
            ]
        );
        // The placeholder is stable per kind.
        assert_eq!(
            report.entries[0].placeholder,
            format!("[REDACTED:{}]", KIND_DSN_CREDENTIAL)
        );
    }

    #[test]
    fn placeholders_are_stable_across_runs() {
        let redactor = redactor();
        let (first, report_first) = redactor.redact(&planted());
        let (second, report_second) = redactor.redact(&planted());
        assert_eq!(first, second);
        assert_eq!(report_first, report_second);
    }

    #[test]
    fn redaction_is_disabled_by_the_privacy_config() {
        let mut config = InsightsConfig::default();
        config.privacy.redact_secrets = false;
        let redactor = Redactor::from_config(&config);
        assert!(!redactor.redaction_enabled());
        assert_eq!(redactor.retention().raw_sessions_days, 90);

        let (redacted, report) = redactor.redact(&planted());
        assert_eq!(redacted, planted());
        assert!(report.is_empty());
    }

    #[test]
    fn a_clean_payload_is_untouched_and_unreported() {
        let value = json!({
            "payload": {
                "note": "session summary: 12 tool calls in repo inferweave/autospec",
                "count": 7,
                "items": ["plain-text", null, true, 3.5]
            }
        });
        let (redacted, report) = redactor().redact(&value);
        assert_eq!(redacted, value);
        assert!(report.is_empty());
    }

    #[test]
    fn a_short_sk_token_and_a_risk_word_are_not_secrets() {
        let value = json!({"a": "sk-abc123", "b": "risk-AbCdEfGhIjKlMnOpQrSt"});
        let (redacted, report) = redactor().redact(&value);
        assert_eq!(redacted, value);
        assert!(report.is_empty());
    }

    #[test]
    fn a_dsn_without_credentials_is_untouched() {
        let value = json!({
            "url": "https://api.example.com/v1/telemetry",
            "note": "see https://docs.example.com/page for details"
        });
        let (redacted, report) = redactor().redact(&value);
        assert_eq!(redacted, value);
        assert!(report.is_empty());
    }

    #[test]
    fn a_bearer_token_in_a_string_is_redacted() {
        let value = json!({"log": "request failed: Bearer abc123xyz789def456ghi"});
        let (redacted, report) = redactor().redact(&value);
        assert_eq!(redacted["log"], "request failed: [REDACTED:bearer-token]");
        assert_eq!(report.len(), 1);
        assert_eq!(report.entries[0].kind, KIND_BEARER_TOKEN);
        assert_eq!(report.entries[0].path, "$.log");
    }

    #[test]
    fn an_authorization_header_line_is_redacted_as_one_span() {
        let value = json!({"log": "curl -H \"Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig\""});
        let (redacted, report) = redactor().redact(&value);
        assert_eq!(report.len(), 1);
        assert_eq!(report.entries[0].kind, KIND_AUTHORIZATION);
        assert_eq!(
            redacted["log"],
            "curl -H \"[REDACTED:authorization]\""
        );
    }

    #[test]
    fn array_items_are_redacted_with_indexed_paths() {
        // linter:allow-SECURITY fabricated sk- shape, not a real secret
        let value = json!({"events": ["sk-Ab3Cd4Ef5Gh6Ij7Kl8Mn9Op0Qr1St2", "clean"]});
        let (redacted, report) = redactor().redact(&value);
        assert_eq!(report.len(), 1);
        assert_eq!(report.entries[0].path, "$.events[0]");
        assert_eq!(redacted["events"][1], "clean");
    }

    #[test]
    fn repo_allowed_is_deny_by_default() {
        let redactor = redactor();
        assert!(!redactor.repo_allowed("acme/web"));
        assert!(!redactor.repo_allowed(""));
        assert!(!redactor.repo_allowed("   "));
    }

    #[test]
    fn repo_allows_listed_repos_and_their_orgs_only() {
        let redactor = redactor().with_allowlist(["acme", "globex/billing"]);
        assert!(redactor.repo_allowed("acme/web"));
        assert!(redactor.repo_allowed("globex/billing"));
        assert!(!redactor.repo_allowed("acme-evil/web"));
        assert!(!redactor.repo_allowed("globex/other"));
        assert!(!redactor.repo_allowed("nope/repo"));
    }

    #[test]
    fn gate_redacts_before_checking_the_allowlist() {
        let redactor = redactor().with_allowlist(["acme/web"]);

        let allowed = redactor.gate("acme/web", &planted());
        assert!(allowed.allowed);
        assert_eq!(allowed.repo, "acme/web");
        assert_eq!(allowed.report.len(), 4);

        let denied = redactor.gate("other/repo", &planted());
        assert!(!denied.allowed);
        // The denied payload is still redacted — redaction precedes the verdict.
        assert_eq!(denied.report.len(), 4);
    }

    #[test]
    fn the_cutoff_is_the_configured_days_before_now() {
        let now = 1_767_225_600; // 2026-01-01T00:00:00Z
        assert_eq!(cutoff_rfc3339(now, 0), "2026-01-01T00:00:00Z");
        assert_eq!(cutoff_rfc3339(now, 90), "2025-10-03T00:00:00Z");
        assert_eq!(cutoff_rfc3339(now, 365), "2025-01-01T00:00:00Z");
        assert_eq!(cutoff_rfc3339(0, 0), "1970-01-01T00:00:00Z");
        // Negative-epoch path (div_euclid, not truncation).
        assert_eq!(cutoff_rfc3339(0, 1), "1969-12-31T00:00:00Z");
        assert_eq!(cutoff_rfc3339(now + 43_200, 1), "2025-12-31T12:00:00Z");
    }

    // ── retention over a real database (no mocks) ──────────────────────────
    // Disposable PostgreSQL 16 under Apptainer via AUTOSPEC_TEST_DB_URL in
    // the operator full run; a disposable per-test SQLite file otherwise.

    fn test_db_url() -> String {
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => {
                let counter = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
                format!(
                    "sqlite://{}/autospec-insights-privacy-{}-{counter}.db",
                    std::env::temp_dir().display(),
                    std::process::id()
                )
            }
        }
    }

    /// The issue #3827 retention target tables (canonical migration shape,
    /// TEXT timestamps so the DDL is portable to the SQLite fallback).
    async fn open_test_pool() -> AnyPool {
        let pool = crate::resources::db::open_shared_db(&test_db_url())
            .await
            .expect("test database must open");
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                repo TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
                ended_at TEXT,
                created_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z'
            )",
        )
        .execute(&pool)
        .await
        .expect("sessions ddl");
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS session_events (
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                occurred_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
                payload TEXT,
                PRIMARY KEY (session_id, seq)
            )",
        )
        .execute(&pool)
        .await
        .expect("session_events ddl");
        pool
    }

    async fn insert_session(pool: &AnyPool, id: &str, created_at: &str) {
        sqlx::query(
            "INSERT INTO sessions (id, repo, status, created_at) VALUES ($1, 'acme/test', 'completed', $2)",
        )
        .bind(id)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert session");
    }

    async fn insert_event(pool: &AnyPool, session_id: &str, seq: i64, occurred_at: &str) {
        sqlx::query(
            "INSERT INTO session_events (session_id, seq, event_type, occurred_at) \
             VALUES ($1, $2, 'tool_call', $3)",
        )
        .bind(session_id)
        .bind(seq)
        .bind(occurred_at)
        .execute(pool)
        .await
        .expect("insert event");
    }

    async fn ids(pool: &AnyPool, table: &str, column: &str, prefix: &str) -> Vec<String> {
        let sql = format!("SELECT {column} FROM {table} WHERE {column} LIKE '{prefix}%' ORDER BY {column}");
        sqlx::raw_sql(&sql)
            .fetch_all(pool)
            .await
            .expect("select ids")
            .iter()
            .map(|row| row.try_get::<String, _>(0).expect("id column"))
            .collect()
    }

    async fn cleanup(pool: &AnyPool, prefix: &str) {
        sqlx::query(&format!("DELETE FROM session_events WHERE session_id LIKE '{prefix}%'"))
            .execute(pool)
            .await
            .expect("cleanup events");
        sqlx::query(&format!("DELETE FROM sessions WHERE id LIKE '{prefix}%'"))
            .execute(pool)
            .await
            .expect("cleanup sessions");
    }

    #[tokio::test]
    async fn enforce_retention_keeps_in_window_rows_and_removes_older_rows() {
        let _env = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let prefix = format!("priv-{}-", std::process::id());
        let old = format!("{prefix}old");
        let mid = format!("{prefix}mid");
        let fresh = format!("{prefix}fresh");

        insert_session(&pool, &old, "2020-01-01T00:00:00Z").await;
        insert_event(&pool, &old, 1, "2020-01-01T00:00:00Z").await;
        // mid: the session left the 90-day raw window but its event is
        // still inside the 365-day normalized window.
        insert_session(&pool, &mid, "2025-09-01T00:00:00Z").await;
        insert_event(&pool, &mid, 1, "2025-09-01T00:00:00Z").await;
        insert_session(&pool, &fresh, "2025-12-01T00:00:00Z").await;
        insert_event(&pool, &fresh, 1, "2025-12-01T00:00:00Z").await;

        // now = 2026-01-01T00:00:00Z; defaults: raw 90d, normalized 365d.
        let sweep = enforce_retention_at(&pool, &InsightsConfig::default(), 1_767_225_600)
            .await
            .expect("retention sweep");

        assert_eq!(sweep.raw_sessions_deleted, 2, "{}", sweep.line());
        // Two events: `old` expired by age, `mid` removed with its session
        // (session_events references sessions.id — no orphans).
        assert_eq!(
            sweep.normalized_events_deleted, 2,
            "{}",
            sweep.line()
        );
        assert_eq!(ids(&pool, "sessions", "id", &prefix).await, vec![fresh.clone()]);
        assert_eq!(
            ids(&pool, "session_events", "session_id", &prefix).await,
            vec![fresh]
        );
        cleanup(&pool, &prefix).await;
    }

    #[tokio::test]
    async fn a_sweep_with_no_expired_rows_reports_zero_deletions() {
        let _env = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let prefix = format!("priv-fresh-{}-", std::process::id());
        let fresh = format!("{prefix}fresh");
        // now = 2026-01-01T00:00:00Z; this row is 31 days old — inside
        // both windows. Other tests' rows are 2026-09-08 (default), also
        // inside both windows.
        insert_session(&pool, &fresh, "2025-12-01T00:00:00Z").await;
        insert_event(&pool, &fresh, 1, "2025-12-01T00:00:00Z").await;

        let sweep = enforce_retention_at(&pool, &InsightsConfig::default(), 1_767_225_600)
            .await
            .expect("retention sweep");

        assert_eq!(sweep, RetentionSweep::default(), "{}", sweep.line());
        assert_eq!(ids(&pool, "sessions", "id", &prefix).await, vec![fresh.clone()]);
        cleanup(&pool, &prefix).await;
    }

    #[tokio::test]
    async fn enforce_retention_removes_expired_rows_using_the_current_clock() {
        let _env = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let prefix = format!("priv-clock-{}-", std::process::id());
        let old = format!("{prefix}old");
        let fresh = format!("{prefix}fresh");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        insert_session(&pool, &old, "2020-01-01T00:00:00Z").await;
        insert_session(&pool, &fresh, &cutoff_rfc3339(now, 1)).await;
        insert_event(&pool, &old, 1, "2020-01-01T00:00:00Z").await;
        insert_event(&pool, &fresh, 1, &cutoff_rfc3339(now, 1)).await;

        let sweep = enforce_retention(&pool, &InsightsConfig::default())
            .await
            .expect("retention sweep");

        assert!(sweep.raw_sessions_deleted >= 1, "{}", sweep.line());
        assert!(sweep.normalized_events_deleted >= 1, "{}", sweep.line());
        assert_eq!(ids(&pool, "sessions", "id", &prefix).await, vec![fresh.clone()]);
        cleanup(&pool, &prefix).await;
    }
}
