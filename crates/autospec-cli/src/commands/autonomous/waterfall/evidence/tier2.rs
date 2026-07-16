use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use autospec_core::autonomous::no_work::DryReason;
use autospec_core::autonomous::waterfall::{sha256_hex, FunnelCounts, TierReceipt, TierStatus};
use autospec_core::state::json::{JsonParser, JsonValue};

use super::super::WaterfallStoreError;
use super::{canonical, Tier2EvidenceArtifact, WaterfallEvidenceArtifact};

const DISABLED_PRODUCER: &str = "rust-tier2-disabled-policy-v1";
const DISABLED_POLICY: &str = "{\"schema\":1,\"kind\":\"tier2_policy\",\"mode\":\"disabled\",\"reason\":\"tier2_local_discovery_disabled_by_policy\",\"policy_source\":\"checked_in\"}\n";
const FUNNEL_KEYS: [&str; 5] = [
    "observed",
    "deduplicated",
    "verified",
    "roi_approved",
    "ranked",
];

pub(super) fn verify_tier2(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    let expected = artifacts_for(receipt)?;
    if receipt.evidence().len() != expected.len() {
        return invalid("Tier 2 receipt has missing, duplicate, or unexpected evidence references");
    }
    let mut predecessor = None;
    for (artifact, evidence) in expected.into_iter().zip(receipt.evidence()) {
        let reference = WaterfallEvidenceArtifact::Tier2(artifact).reference(pass_id)?;
        if evidence.reference != reference {
            return invalid("Tier 2 receipt evidence references are not exact and ordered");
        }
        let path = root.join(&reference);
        let contents = fs::read_to_string(&path).map_err(|error| {
            WaterfallStoreError::InvalidReceipt(if error.kind() == io::ErrorKind::NotFound {
                format!("missing sealed waterfall evidence {reference}")
            } else {
                format!("cannot read waterfall evidence {}: {error}", path.display())
            })
        })?;
        if sha256_hex(contents.as_bytes()) != evidence.digest {
            return invalid("Tier 2 evidence digest does not match its receipt");
        }
        validate_document(artifact, &contents, predecessor, receipt)?;
        predecessor = Some(evidence.digest.as_str());
    }
    if !matches_terminal_status(receipt) {
        return invalid("Tier 2 receipt status does not match its sealed funnel");
    }
    Ok(())
}

fn artifacts_for(receipt: &TierReceipt) -> Result<Vec<Tier2EvidenceArtifact>, WaterfallStoreError> {
    let complete = || {
        vec![
            Tier2EvidenceArtifact::Collector,
            Tier2EvidenceArtifact::Generated,
            Tier2EvidenceArtifact::Dedup,
            Tier2EvidenceArtifact::Verification,
            Tier2EvidenceArtifact::RoiRank,
        ]
    };
    match receipt.status() {
        TierStatus::NotRun { reason }
            if reason == autospec_core::autonomous::tier2::DISABLED_REASON =>
        {
            Ok(vec![Tier2EvidenceArtifact::Policy])
        }
        TierStatus::Exhausted {
            reason:
                DryReason::NoProposalsGenerated
                | DryReason::VerificationRejected
                | DryReason::RoiFiltered,
        } => Ok(complete()),
        TierStatus::Produced { .. } => Ok(complete()),
        TierStatus::Failed { reason } => {
            let (stage, _) = parse_failure(reason).ok_or_else(|| {
                WaterfallStoreError::InvalidReceipt(
                    "Tier 2 failed receipt has an invalid status reason".to_string(),
                )
            })?;
            let mut artifacts = match stage {
                "collector" => Vec::new(),
                "generator" => vec![Tier2EvidenceArtifact::Collector],
                "deduplicator" => vec![
                    Tier2EvidenceArtifact::Collector,
                    Tier2EvidenceArtifact::Generated,
                ],
                "verifier" => vec![
                    Tier2EvidenceArtifact::Collector,
                    Tier2EvidenceArtifact::Generated,
                    Tier2EvidenceArtifact::Dedup,
                ],
                "roi_rank" => vec![
                    Tier2EvidenceArtifact::Collector,
                    Tier2EvidenceArtifact::Generated,
                    Tier2EvidenceArtifact::Dedup,
                    Tier2EvidenceArtifact::Verification,
                ],
                _ => unreachable!("closed Tier 2 failure stage"),
            };
            artifacts.push(Tier2EvidenceArtifact::Failure);
            Ok(artifacts)
        }
        _ => invalid("Tier 2 receipt has an invalid terminal status"),
    }
}

fn validate_document(
    artifact: Tier2EvidenceArtifact,
    contents: &str,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if !canonical::matches_document(contents, kind(artifact), keys(artifact)) {
        return invalid("Tier 2 evidence must be canonical one-line JSON");
    }
    let object = JsonParser::new(contents)
        .parse()
        .map_err(|error| {
            WaterfallStoreError::InvalidReceipt(format!("invalid Tier 2 JSON: {error}"))
        })?
        .into_object("Tier 2 evidence")
        .map_err(WaterfallStoreError::InvalidReceipt)?;
    if number(&object, "schema") != Some(1)
        || string(&object, "kind") != Some(kind(artifact))
        || !exact_keys(&object, keys(artifact))
    {
        return invalid("Tier 2 evidence has an invalid schema or kind");
    }
    match artifact {
        Tier2EvidenceArtifact::Policy => validate_policy(&object, contents, predecessor, receipt)?,
        Tier2EvidenceArtifact::Collector if predecessor.is_some() => {
            return invalid("Tier 2 collector evidence has an invalid dependency link");
        }
        Tier2EvidenceArtifact::Generated
        | Tier2EvidenceArtifact::Dedup
        | Tier2EvidenceArtifact::Verification
        | Tier2EvidenceArtifact::RoiRank
            if string(&object, "predecessor_digest") != predecessor =>
        {
            return invalid("Tier 2 evidence predecessor digest does not match");
        }
        Tier2EvidenceArtifact::Failure => validate_failure(&object, predecessor, receipt)?,
        _ => {}
    }
    if artifact == Tier2EvidenceArtifact::RoiRank {
        let Some(JsonValue::Object(funnel)) = object.get("funnel") else {
            return invalid("Tier 2 rank evidence is missing funnel counts");
        };
        if !matches_funnel(funnel, receipt.funnel()) {
            return invalid("Tier 2 rank evidence funnel does not match the receipt");
        }
    }
    if let TierStatus::Produced { count } = receipt.status() {
        if receipt.funnel().ranked != *count {
            return invalid("Tier 2 produced count does not match ranked funnel count");
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
    let funnel = receipt.funnel();
    if contents != DISABLED_POLICY
        || predecessor.is_some()
        || receipt.producer_version() != DISABLED_PRODUCER
        || string(object, "mode") != Some("disabled")
        || string(object, "reason") != Some(autospec_core::autonomous::tier2::DISABLED_REASON)
        || string(object, "policy_source") != Some("checked_in")
        || [
            funnel.observed,
            funnel.deduplicated,
            funnel.verified,
            funnel.roi_approved,
            funnel.ranked,
        ]
        .iter()
        .any(|count| *count != 0)
    {
        return invalid("Tier 2 policy evidence is not the checked-in disabled policy");
    }
    Ok(())
}

fn validate_failure(
    object: &BTreeMap<String, JsonValue>,
    predecessor: Option<&str>,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if optional_string(object, "predecessor_digest") != Some(predecessor) {
        return invalid("Tier 2 failure evidence predecessor digest does not match");
    }
    let TierStatus::Failed { reason } = receipt.status() else {
        return invalid("Tier 2 failure evidence is linked to a non-failed receipt");
    };
    let Some((stage, code)) = parse_failure(reason) else {
        return invalid("Tier 2 failure evidence has an invalid status reason");
    };
    let Some(JsonValue::Object(funnel)) = object.get("funnel") else {
        return invalid("Tier 2 failure evidence is missing funnel counts");
    };
    if string(object, "stage") != Some(stage)
        || string(object, "code") != Some(code)
        || string(object, "status_reason") != Some(reason)
        || !string(object, "detail")
            .is_some_and(|detail| !detail.trim().is_empty() && detail.chars().count() <= 240)
        || !matches_funnel(funnel, receipt.funnel())
        || !matches_failure_prefix(stage, receipt.funnel())
    {
        return invalid("Tier 2 failure evidence does not match the receipt status");
    }
    Ok(())
}

fn matches_failure_prefix(stage: &str, funnel: &FunnelCounts) -> bool {
    match stage {
        "collector" | "generator" => {
            funnel.observed == 0
                && funnel.deduplicated == 0
                && funnel.verified == 0
                && funnel.roi_approved == 0
                && funnel.ranked == 0
        }
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

fn matches_terminal_status(receipt: &TierReceipt) -> bool {
    let funnel = receipt.funnel();
    match receipt.status() {
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        } => funnel.observed == 0,
        TierStatus::Exhausted {
            reason: DryReason::VerificationRejected,
        } => funnel.observed > 0 && funnel.verified == 0,
        TierStatus::Exhausted {
            reason: DryReason::RoiFiltered,
        } => funnel.verified > 0 && funnel.roi_approved == 0 && funnel.ranked == 0,
        TierStatus::Produced { count } => *count > 0 && funnel.ranked == *count,
        TierStatus::Exhausted { .. } => false,
        TierStatus::Failed { .. } | TierStatus::NotRun { .. } => true,
        TierStatus::Blocked { .. } => false,
    }
}

fn kind(artifact: Tier2EvidenceArtifact) -> &'static str {
    match artifact {
        Tier2EvidenceArtifact::Policy => "tier2_policy",
        Tier2EvidenceArtifact::Collector => "tier2_collector",
        Tier2EvidenceArtifact::Generated => "tier2_generated",
        Tier2EvidenceArtifact::Dedup => "tier2_dedup",
        Tier2EvidenceArtifact::Verification => "tier2_verification",
        Tier2EvidenceArtifact::RoiRank => "tier2_roi_rank",
        Tier2EvidenceArtifact::Failure => "tier2_failure",
    }
}

fn keys(artifact: Tier2EvidenceArtifact) -> &'static [&'static str] {
    match artifact {
        Tier2EvidenceArtifact::Policy => &["schema", "kind", "mode", "reason", "policy_source"],
        Tier2EvidenceArtifact::Collector => &[
            "schema",
            "kind",
            "collector_version",
            "canonical_repo_scope",
            "domains",
        ],
        Tier2EvidenceArtifact::Generated => &[
            "schema",
            "kind",
            "predecessor_digest",
            "generator_identity",
            "generator_protocol_version",
            "proposals",
        ],
        Tier2EvidenceArtifact::Dedup => &[
            "schema",
            "kind",
            "predecessor_digest",
            "normalization_version",
            "groups",
        ],
        Tier2EvidenceArtifact::Verification => &[
            "schema",
            "kind",
            "predecessor_digest",
            "verifier_identity",
            "verifier_protocol_version",
            "verdicts",
        ],
        Tier2EvidenceArtifact::RoiRank => &[
            "schema",
            "kind",
            "predecessor_digest",
            "rank_limit",
            "funnel",
            "candidates",
            "ranked",
        ],
        Tier2EvidenceArtifact::Failure => &[
            "schema",
            "kind",
            "predecessor_digest",
            "stage",
            "code",
            "status_reason",
            "detail",
            "funnel",
        ],
    }
}

fn parse_failure(reason: &str) -> Option<(&'static str, &str)> {
    for stage in [
        "collector",
        "generator",
        "deduplicator",
        "verifier",
        "roi_rank",
    ] {
        let Some(code) = reason.strip_prefix(&format!("tier2_{stage}_")) else {
            continue;
        };
        if [
            "invalid_root",
            "path_escapes_root",
            "read_directory",
            "read_file",
            "invalid_utf8",
            "invalid_collector_schema",
            "missing_stage_result",
            "invalid_proposal",
            "duplicate_conflict",
            "invalid_verdict_coverage",
            "invalid_ranking",
            "count_overflow",
        ]
        .contains(&code)
        {
            return Some((stage, code));
        }
    }
    None
}

fn exact_keys(object: &BTreeMap<String, JsonValue>, required: &[&str]) -> bool {
    object.len() == required.len() && required.iter().all(|key| object.contains_key(*key))
}

fn matches_funnel(funnel: &BTreeMap<String, JsonValue>, expected: &FunnelCounts) -> bool {
    exact_keys(funnel, &FUNNEL_KEYS)
        && number(funnel, "observed") == Some(expected.observed)
        && number(funnel, "deduplicated") == Some(expected.deduplicated)
        && number(funnel, "verified") == Some(expected.verified)
        && number(funnel, "roi_approved") == Some(expected.roi_approved)
        && number(funnel, "ranked") == Some(expected.ranked)
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

fn invalid<T>(message: &str) -> Result<T, WaterfallStoreError> {
    Err(WaterfallStoreError::InvalidReceipt(message.to_string()))
}
