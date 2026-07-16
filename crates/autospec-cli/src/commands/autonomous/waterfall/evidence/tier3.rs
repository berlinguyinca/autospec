use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use autospec_core::autonomous::no_work::DryReason;
use autospec_core::autonomous::tier3::{Tier3Failure, DISABLED_REASON, TIER3_RANK_LIMIT};
use autospec_core::autonomous::waterfall::{sha256_hex, FunnelCounts, TierReceipt, TierStatus};
use autospec_core::state::json::{JsonParser, JsonValue};

use super::super::WaterfallStoreError;
use super::{
    canonical,
    tier3_shape::{has_canonical_nested_keys, keys, kind, FUNNEL_KEYS},
    Tier3EvidenceArtifact, WaterfallEvidenceArtifact,
};

const DISABLED_PRODUCER: &str = "rust-tier3-disabled-policy-v1";
const DISABLED_POLICY: &str = "{\"schema\":1,\"kind\":\"tier3_policy\",\"mode\":\"disabled\",\"reason\":\"tier3_metadata_disabled_by_checked_in_policy\",\"policy_source\":\"checked_in\"}\n";

pub(super) fn verify_tier3(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    let expected = artifacts_for(receipt)?;
    if receipt.evidence().len() != expected.len() {
        return invalid("Tier 3 receipt has missing, duplicate, or unexpected evidence references");
    }
    let mut predecessor = None;
    for (artifact, evidence) in expected.into_iter().zip(receipt.evidence()) {
        let reference = WaterfallEvidenceArtifact::Tier3(artifact).reference(pass_id)?;
        if evidence.reference != reference {
            return invalid("Tier 3 receipt evidence references are not exact and ordered");
        }
        let contents = fs::read_to_string(root.join(&reference)).map_err(|error| {
            WaterfallStoreError::InvalidReceipt(if error.kind() == io::ErrorKind::NotFound {
                format!("missing sealed waterfall evidence {reference}")
            } else {
                format!("cannot read waterfall evidence {reference}: {error}")
            })
        })?;
        if sha256_hex(contents.as_bytes()) != evidence.digest {
            return invalid("Tier 3 evidence digest does not match its receipt");
        }
        validate_document(artifact, &contents, predecessor, receipt)?;
        predecessor = Some(evidence.digest.as_str());
    }
    matches_terminal_status(receipt)
        .then_some(())
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt(
                "Tier 3 receipt status does not match sealed funnel".to_string(),
            )
        })?;
    if matches!(
        receipt.status(),
        TierStatus::Exhausted {
            reason: DryReason::NoMetadataFindings
        } | TierStatus::Produced { .. }
    ) {
        super::tier3_consistency::verify_completed_facts(root, receipt)?;
    }
    Ok(())
}

fn artifacts_for(receipt: &TierReceipt) -> Result<Vec<Tier3EvidenceArtifact>, WaterfallStoreError> {
    if !matches!(receipt.status(), TierStatus::NotRun { .. })
        && receipt.producer_version() != "rust-tier3-metadata-receipts-v1"
    {
        return invalid("Tier 3 receipt has an invalid producer identity");
    }
    let complete = || {
        vec![
            Tier3EvidenceArtifact::Architecture,
            Tier3EvidenceArtifact::Coverage,
            Tier3EvidenceArtifact::Debt,
            Tier3EvidenceArtifact::Findings,
        ]
    };
    match receipt.status() {
        TierStatus::NotRun { reason } if reason == DISABLED_REASON => {
            Ok(vec![Tier3EvidenceArtifact::Policy])
        }
        TierStatus::Exhausted {
            reason: DryReason::NoMetadataFindings,
        }
        | TierStatus::Produced { .. } => Ok(complete()),
        TierStatus::Failed { reason } => {
            let (stage, _) = Tier3Failure::parse_status_reason(reason)
                .map_err(WaterfallStoreError::InvalidReceipt)?;
            let mut artifacts = match stage.as_str() {
                "architecture" => Vec::new(),
                "coverage" => vec![Tier3EvidenceArtifact::Architecture],
                "debt" => vec![
                    Tier3EvidenceArtifact::Architecture,
                    Tier3EvidenceArtifact::Coverage,
                ],
                "ranking" => vec![
                    Tier3EvidenceArtifact::Architecture,
                    Tier3EvidenceArtifact::Coverage,
                    Tier3EvidenceArtifact::Debt,
                ],
                _ => unreachable!("closed Tier 3 stage"),
            };
            artifacts.push(Tier3EvidenceArtifact::Failure);
            Ok(artifacts)
        }
        _ => invalid("Tier 3 receipt has an invalid terminal status"),
    }
}

fn validate_document(
    artifact: Tier3EvidenceArtifact,
    contents: &str,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if !canonical::matches_document(contents, kind(artifact), keys(artifact)) {
        return invalid("Tier 3 evidence must be canonical one-line JSON");
    }
    let object = JsonParser::new(contents)
        .parse()
        .map_err(|error| {
            WaterfallStoreError::InvalidReceipt(format!("invalid Tier 3 JSON: {error}"))
        })?
        .into_object("Tier 3 evidence")
        .map_err(WaterfallStoreError::InvalidReceipt)?;
    if number(&object, "schema") != Some(1)
        || string(&object, "kind") != Some(kind(artifact))
        || !exact_keys(&object, keys(artifact))
        || !has_canonical_nested_keys(artifact, contents, &object)
    {
        return invalid("Tier 3 evidence has an invalid schema or kind");
    }
    match artifact {
        Tier3EvidenceArtifact::Policy => validate_policy(&object, contents, predecessor, receipt),
        Tier3EvidenceArtifact::Architecture => {
            if predecessor.is_some() {
                return invalid("Tier 3 architecture evidence has an invalid dependency link");
            }
            validate_adapter(&object, "architecture")
        }
        Tier3EvidenceArtifact::Coverage | Tier3EvidenceArtifact::Debt => {
            if string(&object, "predecessor_digest") != predecessor {
                return invalid("Tier 3 evidence predecessor digest does not match");
            }
            validate_adapter(
                &object,
                if artifact == Tier3EvidenceArtifact::Coverage {
                    "coverage"
                } else {
                    "debt"
                },
            )
        }
        Tier3EvidenceArtifact::Findings => validate_findings(&object, predecessor, receipt),
        Tier3EvidenceArtifact::Failure => validate_failure(&object, predecessor, receipt),
    }
}

fn validate_policy(
    object: &BTreeMap<String, JsonValue>,
    contents: &str,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if contents != DISABLED_POLICY
        || predecessor.is_some()
        || receipt.producer_version() != DISABLED_PRODUCER
        || string(object, "mode") != Some("disabled")
        || string(object, "reason") != Some(DISABLED_REASON)
        || string(object, "policy_source") != Some("checked_in")
        || !zero(receipt.funnel())
    {
        return invalid("Tier 3 policy evidence is not the checked-in disabled policy");
    }
    Ok(())
}

fn validate_adapter(
    object: &BTreeMap<String, JsonValue>,
    kind: &str,
) -> Result<(), WaterfallStoreError> {
    if !bounded(string(object, "adapter_version")) || !bounded(string(object, "rule_version")) {
        return invalid("Tier 3 adapter identity is invalid");
    }
    let Some(JsonValue::Array(findings)) = object.get("findings") else {
        return invalid("Tier 3 adapter findings are invalid");
    };
    let mut prior = None;
    for finding in findings {
        let value = finding_object(finding, kind)?;
        if prior
            .as_ref()
            .is_some_and(|previous: &(u64, String, String, u64, String, u64)| value <= *previous)
        {
            return invalid("Tier 3 adapter findings are not canonical");
        }
        prior = Some(value);
    }
    Ok(())
}

fn validate_findings(
    object: &BTreeMap<String, JsonValue>,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if string(object, "predecessor_digest") != predecessor
        || number(object, "rank_limit") != Some(TIER3_RANK_LIMIT)
    {
        return invalid("Tier 3 findings evidence has an invalid dependency link or rank limit");
    }
    let Some(JsonValue::Object(funnel)) = object.get("funnel") else {
        return invalid("Tier 3 findings evidence is missing funnel counts");
    };
    if !matches_funnel(funnel, receipt.funnel()) {
        return invalid("Tier 3 findings funnel does not match receipt");
    }
    let Some(JsonValue::Array(deduplicated)) = object.get("deduplicated") else {
        return invalid("Tier 3 findings evidence is missing deduplicated rows");
    };
    let Some(JsonValue::Array(ranked)) = object.get("ranked") else {
        return invalid("Tier 3 findings evidence is missing ranked rows");
    };
    if deduplicated.len() as u64 != receipt.funnel().deduplicated
        || ranked.len() as u64 != receipt.funnel().ranked
        || receipt.funnel().verified != receipt.funnel().deduplicated
        || receipt.funnel().roi_approved != receipt.funnel().deduplicated
    {
        return invalid("Tier 3 findings counts do not match the receipt");
    }
    let mut prior = None;
    for finding in deduplicated {
        let value = deduplicated_order(finding)?;
        if prior
            .as_ref()
            .is_some_and(|previous: &(u64, String, String, u64, String)| value <= *previous)
        {
            return invalid("Tier 3 deduplicated findings are not canonical");
        }
        prior = Some(value);
    }
    let mut prior = None;
    for finding in ranked {
        let value = finding_object(finding, "any")?;
        if prior
            .as_ref()
            .is_some_and(|previous: &(u64, String, String, u64, String, u64)| value <= *previous)
        {
            return invalid("Tier 3 ranked findings are not canonical");
        }
        prior = Some(value);
    }
    Ok(())
}

fn validate_failure(
    object: &BTreeMap<String, JsonValue>,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    let TierStatus::Failed { reason } = receipt.status() else {
        return invalid("Tier 3 failure evidence is linked to a non-failed receipt");
    };
    let (stage, code) =
        Tier3Failure::parse_status_reason(reason).map_err(WaterfallStoreError::InvalidReceipt)?;
    let Some(JsonValue::Object(funnel)) = object.get("funnel") else {
        return invalid("Tier 3 failure evidence is missing funnel counts");
    };
    if optional_string(object, "predecessor_digest") != Some(predecessor)
        || string(object, "stage") != Some(stage.as_str())
        || string(object, "code") != Some(code.as_str())
        || string(object, "status_reason") != Some(reason)
        || !bounded_detail(string(object, "detail"))
        || !matches_funnel(funnel, receipt.funnel())
        || !zero(receipt.funnel())
    {
        return invalid("Tier 3 failure evidence does not match the receipt status");
    }
    Ok(())
}

fn matches_terminal_status(receipt: &TierReceipt) -> bool {
    match receipt.status() {
        TierStatus::NotRun { reason } => reason == DISABLED_REASON && zero(receipt.funnel()),
        TierStatus::Exhausted {
            reason: DryReason::NoMetadataFindings,
        } => zero(receipt.funnel()),
        TierStatus::Produced { count } => *count == receipt.funnel().ranked && *count > 0,
        TierStatus::Failed { .. } => zero(receipt.funnel()),
        _ => false,
    }
}

fn finding_object(
    finding: &JsonValue,
    expected_kind: &str,
) -> Result<(u64, String, String, u64, String, u64), WaterfallStoreError> {
    let JsonValue::Object(object) = finding else {
        return invalid("Tier 3 finding must be an object");
    };
    if !exact_keys(
        object,
        &["kind", "severity", "rule_id", "path", "line", "message"],
    ) {
        return invalid("Tier 3 finding has unexpected keys");
    }
    let kind = string(object, "kind").and_then(kind_rank);
    if kind.is_none() || (expected_kind != "any" && string(object, "kind") != Some(expected_kind)) {
        return invalid("Tier 3 finding kind is invalid");
    }
    let severity = match string(object, "severity") {
        Some("critical") => 0,
        Some("high") => 1,
        Some("medium") => 2,
        Some("low") => 3,
        _ => return invalid("Tier 3 finding severity is invalid"),
    };
    let rule = string(object, "rule_id")
        .filter(|value| bounded(Some(value)))
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt("Tier 3 finding rule is invalid".to_string())
        })?;
    let path = string(object, "path").filter(valid_path).ok_or_else(|| {
        WaterfallStoreError::InvalidReceipt("Tier 3 finding path is invalid".to_string())
    })?;
    let line = number(object, "line")
        .filter(|line| *line > 0)
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt("Tier 3 finding line is invalid".to_string())
        })?;
    let message = string(object, "message")
        .filter(|value| bounded(Some(value)))
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt("Tier 3 finding message is invalid".to_string())
        })?;
    Ok((
        severity,
        rule.to_string(),
        path.to_string(),
        line,
        message.to_string(),
        kind.expect("validated Tier 3 finding kind"),
    ))
}

fn deduplicated_order(
    finding: &JsonValue,
) -> Result<(u64, String, String, u64, String), WaterfallStoreError> {
    let (_, rule, path, line, message, kind) = finding_object(finding, "any")?;
    Ok((kind, rule, path, line, message))
}

fn kind_rank(kind: &str) -> Option<u64> {
    match kind {
        "architecture" => Some(0),
        "coverage" => Some(1),
        "debt" => Some(2),
        _ => None,
    }
}

fn exact_keys(object: &BTreeMap<String, JsonValue>, required: &[&str]) -> bool {
    object.len() == required.len() && required.iter().all(|key| object.contains_key(*key))
}
fn string<'a>(object: &'a BTreeMap<String, JsonValue>, key: &str) -> Option<&'a str> {
    match object.get(key) {
        Some(JsonValue::String(value)) => Some(value),
        _ => None,
    }
}
fn optional_string<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    key: &str,
) -> Option<Option<&'a str>> {
    match object.get(key) {
        Some(JsonValue::Null) => Some(None),
        Some(JsonValue::String(value)) => Some(Some(value)),
        _ => None,
    }
}
fn number(object: &BTreeMap<String, JsonValue>, key: &str) -> Option<u64> {
    match object.get(key) {
        Some(JsonValue::Number(value)) => value.parse().ok(),
        _ => None,
    }
}
fn matches_funnel(funnel: &BTreeMap<String, JsonValue>, expected: &FunnelCounts) -> bool {
    exact_keys(funnel, &FUNNEL_KEYS)
        && number(funnel, "observed") == Some(expected.observed)
        && number(funnel, "deduplicated") == Some(expected.deduplicated)
        && number(funnel, "verified") == Some(expected.verified)
        && number(funnel, "roi_approved") == Some(expected.roi_approved)
        && number(funnel, "ranked") == Some(expected.ranked)
}
fn zero(funnel: &FunnelCounts) -> bool {
    [
        funnel.observed,
        funnel.deduplicated,
        funnel.verified,
        funnel.roi_approved,
        funnel.ranked,
    ]
    .iter()
    .all(|count| *count == 0)
}
fn bounded(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty() && text.chars().count() <= 200)
}
fn bounded_detail(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty() && text.chars().count() <= 240)
}
fn valid_path(value: &&str) -> bool {
    bounded(Some(value))
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.contains('\\')
                && !part.contains(':')
        })
}
fn invalid<T>(message: &str) -> Result<T, WaterfallStoreError> {
    Err(WaterfallStoreError::InvalidReceipt(message.to_string()))
}
