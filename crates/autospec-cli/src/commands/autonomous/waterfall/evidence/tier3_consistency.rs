use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use autospec_core::autonomous::waterfall::{TierReceipt, TierStatus};
use autospec_core::state::json::{JsonParser, JsonValue};

use super::super::WaterfallStoreError;

const RANK_LIMIT: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Finding {
    kind: u64,
    severity: u64,
    rule: String,
    path: String,
    line: u64,
    message: String,
}

pub(super) fn verify_completed_facts(
    root: &Path,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if !matches!(
        receipt.status(),
        TierStatus::Exhausted { .. } | TierStatus::Produced { .. }
    ) || receipt.evidence().len() != 4
    {
        return invalid("Tier 3 completed receipt has an invalid evidence shape");
    }
    let mut expected = Vec::new();
    for index in 0..3 {
        expected.extend(read_findings(
            root,
            &receipt.evidence()[index].reference,
            "findings",
        )?);
    }
    let actual = read_findings(root, &receipt.evidence()[3].reference, "deduplicated")?;
    let ranked = read_findings(root, &receipt.evidence()[3].reference, "ranked")?;
    if has_duplicate_identity(&expected)
        || receipt.funnel().observed != expected.len() as u64
        || receipt.funnel().deduplicated != expected.len() as u64
        || receipt.funnel().verified != expected.len() as u64
        || receipt.funnel().roi_approved != expected.len() as u64
    {
        return invalid("Tier 3 receipt funnel does not match its sealed adapter facts");
    }
    sort_deduplicated(&mut expected);
    if actual != expected {
        return invalid("Tier 3 findings do not match sealed adapter facts");
    }
    let mut expected_ranked = expected;
    sort_ranked(&mut expected_ranked);
    expected_ranked.truncate(RANK_LIMIT);
    if receipt.funnel().ranked != expected_ranked.len() as u64 || ranked != expected_ranked {
        return invalid("Tier 3 ranked findings do not match sealed adapter facts");
    }
    Ok(())
}

fn read_findings(
    root: &Path,
    reference: &str,
    field: &str,
) -> Result<Vec<Finding>, WaterfallStoreError> {
    let contents = fs::read_to_string(root.join(reference)).map_err(|error| {
        WaterfallStoreError::InvalidReceipt(if error.kind() == io::ErrorKind::NotFound {
            format!("missing sealed waterfall evidence {reference}")
        } else {
            format!("cannot read waterfall evidence {reference}: {error}")
        })
    })?;
    let mut object = JsonParser::new(&contents)
        .parse()
        .map_err(|error| {
            WaterfallStoreError::InvalidReceipt(format!("invalid Tier 3 JSON: {error}"))
        })?
        .into_object("Tier 3 evidence")
        .map_err(WaterfallStoreError::InvalidReceipt)?;
    let Some(JsonValue::Array(rows)) = object.remove(field) else {
        return invalid("Tier 3 evidence has invalid finding rows");
    };
    rows.iter().map(parse_finding).collect()
}

fn parse_finding(value: &JsonValue) -> Result<Finding, WaterfallStoreError> {
    let JsonValue::Object(object) = value else {
        return invalid("Tier 3 finding must be an object");
    };
    let kind = match string(object, "kind") {
        Some("architecture") => 0,
        Some("coverage") => 1,
        Some("debt") => 2,
        _ => return invalid("Tier 3 finding kind is invalid"),
    };
    let severity = match string(object, "severity") {
        Some("critical") => 0,
        Some("high") => 1,
        Some("medium") => 2,
        Some("low") => 3,
        _ => return invalid("Tier 3 finding severity is invalid"),
    };
    Ok(Finding {
        kind,
        severity,
        rule: string(object, "rule_id")
            .ok_or_else(|| invalid_error("Tier 3 finding rule is invalid"))?
            .to_string(),
        path: string(object, "path")
            .ok_or_else(|| invalid_error("Tier 3 finding path is invalid"))?
            .to_string(),
        line: number(object, "line")
            .ok_or_else(|| invalid_error("Tier 3 finding line is invalid"))?,
        message: string(object, "message")
            .ok_or_else(|| invalid_error("Tier 3 finding message is invalid"))?
            .to_string(),
    })
}

fn has_duplicate_identity(rows: &[Finding]) -> bool {
    let mut identities = BTreeSet::new();
    rows.iter()
        .any(|row| !identities.insert((row.kind, &row.rule, &row.path, row.line)))
}

fn sort_deduplicated(rows: &mut [Finding]) {
    rows.sort_by(|left, right| {
        (left.kind, &left.rule, &left.path, left.line, &left.message).cmp(&(
            right.kind,
            &right.rule,
            &right.path,
            right.line,
            &right.message,
        ))
    });
}

fn sort_ranked(rows: &mut [Finding]) {
    rows.sort_by(|left, right| {
        (
            left.severity,
            &left.rule,
            &left.path,
            left.line,
            &left.message,
        )
            .cmp(&(
                right.severity,
                &right.rule,
                &right.path,
                right.line,
                &right.message,
            ))
    });
}

fn string<'a>(
    object: &'a std::collections::BTreeMap<String, JsonValue>,
    key: &str,
) -> Option<&'a str> {
    match object.get(key) {
        Some(JsonValue::String(value)) => Some(value),
        _ => None,
    }
}

fn number(object: &std::collections::BTreeMap<String, JsonValue>, key: &str) -> Option<u64> {
    match object.get(key) {
        Some(JsonValue::Number(value)) => value.parse().ok(),
        _ => None,
    }
}

fn invalid_error(message: &str) -> WaterfallStoreError {
    WaterfallStoreError::InvalidReceipt(message.to_string())
}
fn invalid<T>(message: &str) -> Result<T, WaterfallStoreError> {
    Err(invalid_error(message))
}
