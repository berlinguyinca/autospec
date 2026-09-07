use std::collections::BTreeMap;

use crate::state::json::{JsonParser, JsonValue};

use super::{
    AuditDimension, AuditFinding, FindingConfidence, FindingSeverity, FindingStatus, LedgerEntry,
    QualityLedger, QUALITY_LEDGER_SCHEMA,
};

pub(super) fn ledger_json(ledger: &QualityLedger) -> String {
    format!(
        "{{\"schema\":{},\"repo\":\"{}\",\"pass_id\":{},\"revision\":\"{}\",\"digest\":\"{}\",\
         \"reaudits_this_cycle\":{},\"remediation_rounds_this_cycle\":{},\"quality_work\":{},\
         \"feature_work\":{},\"entries\":[{}],\"latest_fingerprints\":[{}]}}",
        QUALITY_LEDGER_SCHEMA,
        escape_json(ledger.repo()),
        ledger.pass_id(),
        escape_json(ledger.revision()),
        ledger.digest(),
        ledger.reaudits_this_cycle(),
        ledger.remediation_rounds_this_cycle(),
        ledger.quality_work(),
        ledger.feature_work(),
        ledger
            .entries()
            .iter()
            .map(entry_json)
            .collect::<Vec<_>>()
            .join(","),
        ledger
            .latest_fingerprints()
            .iter()
            .map(|fingerprint| format!("\"{fingerprint}\""))
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn entry_json(entry: &LedgerEntry) -> String {
    let finding = &entry.finding;
    format!(
        "{{\"fingerprint\":\"{}\",\"dimension\":\"{}\",\"severity\":\"{}\",\"confidence\":\"{}\",\
         \"evidence\":\"{}\",\"affected_paths\":[{}],\"regression_test_required\":{},\
         \"credential_gated\":{},\"safe_to_autofix\":{},\"existing_issue\":{},\
         \"status\":\"{}\",\"first_seen_pass\":{},\"last_seen_pass\":{},\"issue\":{},\
         \"attempts\":{},\"last_reason\":{}}}",
        entry.fingerprint(),
        finding.dimension.as_str(),
        finding.severity.as_str(),
        finding.confidence.as_str(),
        escape_json(&finding.evidence),
        finding
            .affected_paths
            .iter()
            .map(|path| format!("\"{}\"", escape_json(path)))
            .collect::<Vec<_>>()
            .join(","),
        finding.regression_test_required,
        finding.credential_gated,
        finding.safe_to_autofix,
        existing_issue_json(finding.existing_issue),
        entry.status.as_str(),
        entry.first_seen_pass,
        entry.last_seen_pass,
        issue_json(entry.issue),
        entry.attempts,
        optional_string_json(entry.last_reason.as_deref()),
    )
}

fn existing_issue_json(issue: Option<u64>) -> String {
    issue.map_or_else(|| "null".to_string(), |issue| issue.to_string())
}

fn issue_json(issue: Option<u64>) -> String {
    existing_issue_json(issue)
}

fn optional_string_json(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".to_string(),
        |value| format!("\"{}\"", escape_json(value)),
    )
}

pub(super) fn parse_ledger(input: &str) -> Result<QualityLedger, String> {
    let mut object = JsonParser::new(input)
        .parse()?
        .into_object("quality ledger")?;
    require_only_keys(
        &object,
        &[
            "schema",
            "repo",
            "pass_id",
            "revision",
            "digest",
            "reaudits_this_cycle",
            "remediation_rounds_this_cycle",
            "quality_work",
            "feature_work",
            "entries",
            "latest_fingerprints",
        ],
        "quality ledger",
    )?;
    let schema = take_required(&mut object, "schema", "quality ledger")?
        .into_number("quality ledger.schema")?;
    if schema != QUALITY_LEDGER_SCHEMA {
        return Err(format!("unsupported quality ledger schema: {schema}"));
    }
    let repo =
        take_required(&mut object, "repo", "quality ledger")?.into_string("quality ledger.repo")?;
    let pass_id = take_required(&mut object, "pass_id", "quality ledger")?
        .into_number("quality ledger.pass_id")?;
    let revision = take_required(&mut object, "revision", "quality ledger")?
        .into_string("quality ledger.revision")?;
    let digest = take_required(&mut object, "digest", "quality ledger")?
        .into_string("quality ledger.digest")?;
    let reaudits = take_required(&mut object, "reaudits_this_cycle", "quality ledger")?
        .into_number("quality ledger.reaudits_this_cycle")?;
    let rounds = take_required(
        &mut object,
        "remediation_rounds_this_cycle",
        "quality ledger",
    )?
    .into_number("quality ledger.remediation_rounds_this_cycle")?;
    let quality_work = take_required(&mut object, "quality_work", "quality ledger")?
        .into_number("quality ledger.quality_work")?;
    let feature_work = take_required(&mut object, "feature_work", "quality ledger")?
        .into_number("quality ledger.feature_work")?;
    let entries = parse_entries(take_required(&mut object, "entries", "quality ledger")?)?;
    let latest = parse_fingerprints(take_required(
        &mut object,
        "latest_fingerprints",
        "quality ledger",
    )?)?;

    let mut ledger = QualityLedger::new(repo)?;
    ledger.pass_id = pass_id;
    ledger.revision = revision;
    ledger.reaudits_this_cycle = reaudits;
    ledger.remediation_rounds_this_cycle = rounds;
    ledger.quality_work = quality_work;
    ledger.feature_work = feature_work;
    ledger.entries = entries;
    ledger.latest_fingerprints = latest;
    ledger.validate_invariants()?;
    if ledger.digest() != digest {
        return Err("quality ledger digest does not match its recorded audit".to_string());
    }
    Ok(ledger)
}

fn parse_entries(value: JsonValue) -> Result<Vec<LedgerEntry>, String> {
    value
        .into_array("quality ledger.entries")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("quality ledger.entries[{index}]");
            let mut object = value.into_object(&context)?;
            require_only_keys(
                &object,
                &[
                    "fingerprint",
                    "dimension",
                    "severity",
                    "confidence",
                    "evidence",
                    "affected_paths",
                    "regression_test_required",
                    "credential_gated",
                    "safe_to_autofix",
                    "existing_issue",
                    "status",
                    "first_seen_pass",
                    "last_seen_pass",
                    "issue",
                    "attempts",
                    "last_reason",
                ],
                &context,
            )?;
            let fingerprint = take_required(&mut object, "fingerprint", &context)?
                .into_string(&format!("{context}.fingerprint"))?;
            let dimension = AuditDimension::parse(
                &take_required(&mut object, "dimension", &context)?
                    .into_string(&format!("{context}.dimension"))?,
            )?;
            let severity = FindingSeverity::parse(
                &take_required(&mut object, "severity", &context)?
                    .into_string(&format!("{context}.severity"))?,
            )?;
            let confidence = FindingConfidence::parse(
                &take_required(&mut object, "confidence", &context)?
                    .into_string(&format!("{context}.confidence"))?,
            )?;
            let evidence = take_required(&mut object, "evidence", &context)?
                .into_string(&format!("{context}.evidence"))?;
            let affected_paths = take_required(&mut object, "affected_paths", &context)?
                .into_array(&format!("{context}.affected_paths"))?
                .into_iter()
                .map(|value| value.into_string(&context))
                .collect::<Result<Vec<_>, _>>()?;
            let regression_test_required =
                take_required(&mut object, "regression_test_required", &context)?
                    .into_bool(&format!("{context}.regression_test_required"))?;
            let credential_gated = take_required(&mut object, "credential_gated", &context)?
                .into_bool(&format!("{context}.credential_gated"))?;
            let safe_to_autofix = take_required(&mut object, "safe_to_autofix", &context)?
                .into_bool(&format!("{context}.safe_to_autofix"))?;
            let existing_issue = optional_number(
                take_required(&mut object, "existing_issue", &context)?,
                &format!("{context}.existing_issue"),
            )?;
            let status = FindingStatus::parse(
                &take_required(&mut object, "status", &context)?
                    .into_string(&format!("{context}.status"))?,
            )?;
            let first_seen_pass = take_required(&mut object, "first_seen_pass", &context)?
                .into_number(&format!("{context}.first_seen_pass"))?;
            let last_seen_pass = take_required(&mut object, "last_seen_pass", &context)?
                .into_number(&format!("{context}.last_seen_pass"))?;
            let issue = optional_number(
                take_required(&mut object, "issue", &context)?,
                &format!("{context}.issue"),
            )?;
            let attempts = take_required(&mut object, "attempts", &context)?
                .into_number(&format!("{context}.attempts"))?;
            let last_reason = optional_string(
                take_required(&mut object, "last_reason", &context)?,
                &format!("{context}.last_reason"),
            )?;

            let finding = AuditFinding {
                dimension,
                severity,
                confidence,
                evidence,
                affected_paths,
                regression_test_required,
                credential_gated,
                safe_to_autofix,
                existing_issue,
            };
            if finding.fingerprint() != fingerprint {
                return Err(format!("{context}.fingerprint does not match the finding"));
            }
            Ok(LedgerEntry {
                finding,
                status,
                first_seen_pass,
                last_seen_pass,
                issue,
                attempts,
                last_reason,
            })
        })
        .collect()
}

fn parse_fingerprints(value: JsonValue) -> Result<Vec<String>, String> {
    value
        .into_array("quality ledger.latest_fingerprints")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("quality ledger.latest_fingerprints[{index}]");
            let fingerprint = value.into_string(&context)?;
            if fingerprint.len() != 64
                || !fingerprint.bytes().all(|byte| {
                    byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
                })
            {
                return Err(format!(
                    "{context} must be 64 lower-case hexadecimal characters"
                ));
            }
            Ok(fingerprint)
        })
        .collect()
}

fn optional_number(value: JsonValue, context: &str) -> Result<Option<u64>, String> {
    match value {
        JsonValue::Null => Ok(None),
        other => Ok(Some(other.into_number(context)?)),
    }
}

fn optional_string(value: JsonValue, context: &str) -> Result<Option<String>, String> {
    match value {
        JsonValue::Null => Ok(None),
        other => Ok(Some(other.into_string(context)?)),
    }
}

fn require_only_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        return Err(format!("unexpected {context} field: {key}"));
    }
    Ok(())
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {context} field: {key}"))
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character if character.is_control() => format!("\\u{:04x}", character as u32)
                .chars()
                .collect::<Vec<_>>(),
            character => vec![character],
        })
        .collect()
}
