use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use autospec_core::autonomous::no_work::DryReason;
use autospec_core::autonomous::tier4::{DISABLED_REASON, TIER4_RANK_LIMIT};
use autospec_core::autonomous::waterfall::{sha256_hex, FunnelCounts, TierReceipt, TierStatus};
use autospec_core::state::json::{JsonParser, JsonValue};

use super::super::WaterfallStoreError;
use super::{
    canonical, tier4_consistency, tier4_shape, Tier4EvidenceArtifact, WaterfallEvidenceArtifact,
};

const DISABLED_PRODUCER: &str = "rust-tier4-disabled-policy-v1";
const PRODUCER: &str = "rust-tier4-external-discovery-receipts-v1";
const DISABLED_POLICY: &str = "{\"schema\":1,\"kind\":\"tier4_policy\",\"mode\":\"disabled\",\"reason\":\"tier4_external_discovery_disabled_by_checked_in_policy\",\"policy_source\":\"checked_in\"}\n";
const FUNNEL_KEYS: [&str; 5] = [
    "observed",
    "deduplicated",
    "verified",
    "roi_approved",
    "ranked",
];

pub(super) fn verify_tier4(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    let expected = artifacts_for(receipt)?;
    if receipt.evidence().len() != expected.len() {
        return invalid("Tier 4 receipt has missing, duplicate, or unexpected evidence references");
    }
    let mut predecessor = None;
    for (artifact, evidence) in expected.into_iter().zip(receipt.evidence()) {
        let reference = WaterfallEvidenceArtifact::Tier4(artifact).reference(pass_id)?;
        if evidence.reference != reference {
            return invalid("Tier 4 receipt evidence references are not exact and ordered");
        }
        let contents = fs::read_to_string(root.join(&reference)).map_err(|error| {
            WaterfallStoreError::InvalidReceipt(if error.kind() == io::ErrorKind::NotFound {
                format!("missing sealed waterfall evidence {reference}")
            } else {
                format!("cannot read waterfall evidence {reference}: {error}")
            })
        })?;
        if sha256_hex(contents.as_bytes()) != evidence.digest {
            return invalid("Tier 4 evidence digest does not match its receipt");
        }
        validate_document(artifact, &contents, predecessor, receipt)?;
        predecessor = Some(evidence.digest.as_str());
    }
    if !matches_terminal_status(receipt) {
        return invalid("Tier 4 receipt status does not match sealed funnel");
    }
    if matches!(
        receipt.status(),
        TierStatus::Exhausted { .. } | TierStatus::Produced { .. }
    ) {
        tier4_consistency::verify_completed_facts(root, receipt)?;
    }
    Ok(())
}

fn artifacts_for(receipt: &TierReceipt) -> Result<Vec<Tier4EvidenceArtifact>, WaterfallStoreError> {
    let complete = || {
        vec![
            Tier4EvidenceArtifact::SourcePolicy,
            Tier4EvidenceArtifact::Sources,
            Tier4EvidenceArtifact::Generated,
            Tier4EvidenceArtifact::Dedup,
            Tier4EvidenceArtifact::Verification,
            Tier4EvidenceArtifact::RoiRank,
        ]
    };
    match receipt.status() {
        TierStatus::NotRun { reason } if reason == DISABLED_REASON => {
            if receipt.producer_version() != DISABLED_PRODUCER {
                return invalid("Tier 4 disabled receipt has an invalid producer identity");
            }
            Ok(vec![Tier4EvidenceArtifact::Policy])
        }
        TierStatus::Exhausted {
            reason:
                DryReason::NoProposalsGenerated
                | DryReason::VerificationRejected
                | DryReason::RoiFiltered,
        }
        | TierStatus::Produced { .. } => {
            if receipt.producer_version() != PRODUCER {
                return invalid("Tier 4 receipt has an invalid producer identity");
            }
            Ok(complete())
        }
        TierStatus::Failed { reason } => {
            if receipt.producer_version() != PRODUCER {
                return invalid("Tier 4 failed receipt has an invalid producer identity");
            }
            let stage = parse_failure(reason)?.0;
            let mut artifacts = match stage {
                "source_policy" => Vec::new(),
                "sources" => vec![Tier4EvidenceArtifact::SourcePolicy],
                "generator" => vec![
                    Tier4EvidenceArtifact::SourcePolicy,
                    Tier4EvidenceArtifact::Sources,
                ],
                "deduplicator" => vec![
                    Tier4EvidenceArtifact::SourcePolicy,
                    Tier4EvidenceArtifact::Sources,
                    Tier4EvidenceArtifact::Generated,
                ],
                "verifier" | "roi_rank" => vec![
                    Tier4EvidenceArtifact::SourcePolicy,
                    Tier4EvidenceArtifact::Sources,
                    Tier4EvidenceArtifact::Generated,
                    Tier4EvidenceArtifact::Dedup,
                ],
                _ => unreachable!("closed Tier 4 stage"),
            };
            if stage == "roi_rank" {
                artifacts.push(Tier4EvidenceArtifact::Verification);
            }
            artifacts.push(Tier4EvidenceArtifact::Failure);
            Ok(artifacts)
        }
        _ => invalid("Tier 4 receipt has an invalid terminal status"),
    }
}

fn validate_document(
    artifact: Tier4EvidenceArtifact,
    contents: &str,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if !canonical::matches_document(
        contents,
        tier4_shape::kind(artifact),
        tier4_shape::keys(artifact),
    ) {
        return invalid("Tier 4 evidence must be canonical one-line JSON");
    }
    let object = JsonParser::new(contents)
        .parse()
        .map_err(|error| {
            WaterfallStoreError::InvalidReceipt(format!("invalid Tier 4 JSON: {error}"))
        })?
        .into_object("Tier 4 evidence")
        .map_err(WaterfallStoreError::InvalidReceipt)?;
    if number(&object, "schema") != Some(1)
        || string(&object, "kind") != Some(tier4_shape::kind(artifact))
        || !exact_keys(&object, tier4_shape::keys(artifact))
        || !tier4_shape::has_canonical_nested_keys(artifact, contents, &object)
    {
        return invalid("Tier 4 evidence has an invalid schema or kind");
    }
    match artifact {
        Tier4EvidenceArtifact::Policy => validate_policy(&object, contents, predecessor, receipt),
        Tier4EvidenceArtifact::SourcePolicy if predecessor.is_some() => {
            invalid("Tier 4 source policy evidence has an invalid dependency link")
        }
        Tier4EvidenceArtifact::Sources
        | Tier4EvidenceArtifact::Generated
        | Tier4EvidenceArtifact::Dedup
        | Tier4EvidenceArtifact::Verification
        | Tier4EvidenceArtifact::RoiRank
            if string(&object, "predecessor_digest") != predecessor =>
        {
            invalid("Tier 4 evidence predecessor digest does not match")
        }
        Tier4EvidenceArtifact::Failure => validate_failure(&object, predecessor, receipt),
        _ => Ok(()),
    }?;
    if artifact == Tier4EvidenceArtifact::RoiRank {
        let Some(JsonValue::Object(funnel)) = object.get("funnel") else {
            return invalid("Tier 4 rank evidence is missing funnel counts");
        };
        if number(&object, "rank_limit") != Some(TIER4_RANK_LIMIT)
            || !matches_funnel(funnel, receipt.funnel())
        {
            return invalid("Tier 4 rank evidence does not match the receipt funnel");
        }
    }
    Ok(())
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
        return invalid("Tier 4 policy evidence is not the checked-in disabled policy");
    }
    Ok(())
}

fn validate_failure(
    object: &BTreeMap<String, JsonValue>,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    let TierStatus::Failed { reason } = receipt.status() else {
        return invalid("Tier 4 failure evidence is linked to a non-failed receipt");
    };
    let (stage, code) = parse_failure(reason)?;
    let Some(JsonValue::Object(funnel)) = object.get("funnel") else {
        return invalid("Tier 4 failure evidence is missing funnel counts");
    };
    if optional_string(object, "predecessor_digest") != Some(predecessor)
        || string(object, "stage") != Some(stage)
        || string(object, "code") != Some(code)
        || string(object, "status_reason") != Some(reason)
        || !string(object, "detail").is_some_and(bounded_detail)
        || !matches_funnel(funnel, receipt.funnel())
        || !matches_failure_prefix(stage, receipt.funnel())
    {
        return invalid("Tier 4 failure evidence does not match its receipt status");
    }
    Ok(())
}

fn matches_terminal_status(receipt: &TierReceipt) -> bool {
    let funnel = receipt.funnel();
    match receipt.status() {
        TierStatus::NotRun { reason } => reason == DISABLED_REASON && zero(funnel),
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        } => funnel.observed == 0,
        TierStatus::Exhausted {
            reason: DryReason::VerificationRejected,
        } => funnel.observed > 0 && funnel.verified == 0,
        TierStatus::Exhausted {
            reason: DryReason::RoiFiltered,
        } => funnel.verified > 0 && funnel.roi_approved == 0 && funnel.ranked == 0,
        TierStatus::Produced { count } => *count > 0 && *count == funnel.ranked,
        TierStatus::Failed { .. } => true,
        _ => false,
    }
}

fn matches_failure_prefix(stage: &str, funnel: &FunnelCounts) -> bool {
    match stage {
        "source_policy" | "sources" | "generator" => zero(funnel),
        "deduplicator" => {
            funnel.deduplicated == 0
                && funnel.verified == 0
                && funnel.roi_approved == 0
                && funnel.ranked == 0
        }
        "verifier" => funnel.verified == 0 && funnel.roi_approved == 0 && funnel.ranked == 0,
        "roi_rank" => funnel.roi_approved == 0 && funnel.ranked == 0,
        _ => false,
    }
}

fn parse_failure(reason: &str) -> Result<(&'static str, &str), WaterfallStoreError> {
    for stage in [
        "source_policy",
        "sources",
        "generator",
        "deduplicator",
        "verifier",
        "roi_rank",
    ] {
        let Some(code) = reason.strip_prefix(&format!("tier4_{stage}_")) else {
            continue;
        };
        if autospec_core::autonomous::tier4::Tier4FailureCode::parse(code).is_ok() {
            return Ok((stage, code));
        }
    }
    invalid("Tier 4 failed receipt has an invalid status reason")
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

fn bounded_detail(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 240
}

fn invalid<T>(message: &str) -> Result<T, WaterfallStoreError> {
    Err(WaterfallStoreError::InvalidReceipt(message.to_string()))
}
