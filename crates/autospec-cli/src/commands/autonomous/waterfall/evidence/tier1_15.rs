use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;

use autospec_core::autonomous::no_work::DryReason;
use autospec_core::autonomous::waterfall::{sha256_hex, FunnelCounts, TierReceipt, TierStatus};
use autospec_core::state::json::{JsonParser, JsonValue};

use super::super::WaterfallStoreError;
use super::{Tier15EvidenceArtifact, Tier1EvidenceArtifact, WaterfallEvidenceArtifact};

const TIER1_PRODUCER: &str = "rust-foreground-tier1-v1";
const TIER15_PRODUCER: &str = "rust-tier1_5-read-only-v1";

pub(super) fn verify_tier1(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if receipt.producer_version() != TIER1_PRODUCER || receipt.evidence().len() != 1 {
        return invalid("Tier 1 receipt producer or evidence set is not exact");
    }
    let artifact = match receipt.status() {
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        } => Tier1EvidenceArtifact::ReadyPage,
        TierStatus::Failed { .. } => Tier1EvidenceArtifact::ReadFailure,
        _ => return invalid("Tier 1 receipt has an invalid terminal status"),
    };
    let contents = sealed_contents(
        root,
        pass_id,
        WaterfallEvidenceArtifact::Tier1(artifact),
        receipt,
    )?;
    match artifact {
        Tier1EvidenceArtifact::ReadyPage => validate_ready_page(&contents, receipt),
        Tier1EvidenceArtifact::ReadFailure => validate_failure(&contents, receipt, "Tier 1"),
    }
}

pub(super) fn verify_tier15(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    if receipt.producer_version() != TIER15_PRODUCER || receipt.evidence().len() != 1 {
        return invalid("Tier 1.5 receipt producer or evidence set is not exact");
    }
    let artifact = match receipt.status() {
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        }
        | TierStatus::Produced { .. } => Tier15EvidenceArtifact::Observation,
        TierStatus::Failed { .. } => Tier15EvidenceArtifact::ReadFailure,
        _ => return invalid("Tier 1.5 receipt has an invalid terminal status"),
    };
    let contents = sealed_contents(
        root,
        pass_id,
        WaterfallEvidenceArtifact::Tier15(artifact),
        receipt,
    )?;
    match artifact {
        Tier15EvidenceArtifact::Observation => validate_observation(&contents, receipt),
        Tier15EvidenceArtifact::ReadFailure => validate_failure(&contents, receipt, "Tier 1.5"),
    }
}

fn sealed_contents(
    root: &Path,
    pass_id: u64,
    artifact: WaterfallEvidenceArtifact,
    receipt: &TierReceipt,
) -> Result<String, WaterfallStoreError> {
    let reference = artifact.reference(pass_id)?;
    let evidence = &receipt.evidence()[0];
    if evidence.reference != reference {
        return invalid("early-tier receipt evidence reference is not exact and ordered");
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
        return invalid("early-tier evidence digest does not match its receipt");
    }
    Ok(contents)
}

fn validate_ready_page(contents: &str, receipt: &TierReceipt) -> Result<(), WaterfallStoreError> {
    let mut object = parse_object(contents, "Tier 1 ready page")?;
    let schema = take_number(&mut object, "schema")?;
    let kind = take_string(&mut object, "kind")?;
    let gate = take_object(&mut object, "gate_counts")?;
    let worker = take_object(&mut object, "worker_cap")?;
    if !object.is_empty() || schema != 1 || kind != "ready_page" {
        return invalid("Tier 1 ready page has an invalid schema or keys");
    }
    let open = number(&gate, "open")?;
    let candidate = number(&gate, "candidate")?;
    let reviewed = number(&gate, "reviewed")?;
    let blocked = number(&gate, "blocked")?;
    let ready = number(&gate, "ready")?;
    let claimed = number(&gate, "claimed")?;
    let selected = number(&gate, "selected")?;
    let active = number(&worker, "active_count")?;
    let remaining = number(&worker, "remaining")?;
    let reached = boolean(&worker, "reached")?;
    if gate.len() != 7
        || worker.len() != 3
        || candidate != 0
        || claimed != 0
        || selected != 0
        || reached
        || receipt.funnel()
            != &FunnelCounts::new(open, candidate, reviewed, ready, selected)
                .map_err(WaterfallStoreError::InvalidReceipt)?
    {
        return invalid("Tier 1 ready page does not reconstruct its exhausted receipt");
    }
    let expected = format!(
        "{{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{{\"open\":{open},\"candidate\":{candidate},\"reviewed\":{reviewed},\"blocked\":{blocked},\"ready\":{ready},\"claimed\":{claimed},\"selected\":{selected}}},\"worker_cap\":{{\"active_count\":{active},\"remaining\":{remaining},\"reached\":{reached}}}}}\n"
    );
    if contents != expected {
        return invalid("Tier 1 ready page is not canonical one-line JSON");
    }
    Ok(())
}

fn validate_observation(contents: &str, receipt: &TierReceipt) -> Result<(), WaterfallStoreError> {
    let mut object = parse_object(contents, "Tier 1.5 observation")?;
    let schema = take_number(&mut object, "schema")?;
    let kind = take_string(&mut object, "kind")?;
    let open_observed = take_number(&mut object, "open_observed")?;
    let open_deduplicated = take_number(&mut object, "open_deduplicated")?;
    let closed_observed = take_number(&mut object, "closed_observed")?;
    let budget = take_number(&mut object, "budget")?;
    let readiness = take_object(&mut object, "readiness")?;
    let decisions = take_array(&mut object, "decisions")?;
    if !object.is_empty()
        || schema != 1
        || kind != "tier15_observation"
        || open_deduplicated > open_observed
        || decisions.len() as u64 != open_deduplicated
    {
        return invalid("Tier 1.5 observation has an invalid schema or counts");
    }
    let mut numbers = BTreeSet::new();
    let mut rendered = Vec::with_capacity(decisions.len());
    let mut produced = 0u64;
    for decision in decisions {
        let (document, is_produced, number) = validate_decision(decision)?;
        if number == 0 || !numbers.insert(number) {
            return invalid("Tier 1.5 decisions require unique positive issue numbers");
        }
        produced += u64::from(is_produced);
        rendered.push(document);
    }
    let mut rendered_readiness = Vec::with_capacity(readiness.len());
    for (number, value) in readiness {
        let parsed = number.parse::<u64>().map_err(|_| {
            WaterfallStoreError::InvalidReceipt("Tier 1.5 readiness key is invalid".into())
        })?;
        if parsed == 0 || !numbers.contains(&parsed) {
            return invalid("Tier 1.5 readiness must identify a decision");
        }
        let state = value
            .into_string("Tier 1.5 readiness")
            .map_err(WaterfallStoreError::InvalidReceipt)?;
        if !matches!(state.as_str(), "candidate" | "verified" | "safety_reviewed") {
            return invalid("Tier 1.5 readiness value is invalid");
        }
        rendered_readiness.push(format!("\"{number}\":\"{state}\""));
    }
    if rendered_readiness.len() != numbers.len() {
        return invalid("Tier 1.5 readiness must cover every decision");
    }
    let observed = open_observed
        .checked_add(closed_observed)
        .ok_or_else(|| WaterfallStoreError::InvalidReceipt("Tier 1.5 count overflow".into()))?;
    let deduplicated = open_deduplicated
        .checked_add(closed_observed)
        .ok_or_else(|| WaterfallStoreError::InvalidReceipt("Tier 1.5 count overflow".into()))?;
    let expected_funnel =
        FunnelCounts::new(observed, deduplicated, deduplicated, produced, produced)
            .map_err(WaterfallStoreError::InvalidReceipt)?;
    let status_matches = matches!(receipt.status(), TierStatus::Exhausted { reason: DryReason::NoProposalsGenerated } if produced == 0)
        || matches!(receipt.status(), TierStatus::Produced { count } if *count == produced && produced > 0);
    if receipt.funnel() != &expected_funnel || !status_matches {
        return invalid("Tier 1.5 observation does not reconstruct its receipt");
    }
    let expected = format!(
        "{{\"schema\":1,\"kind\":\"tier15_observation\",\"open_observed\":{open_observed},\"open_deduplicated\":{open_deduplicated},\"closed_observed\":{closed_observed},\"budget\":{budget},\"readiness\":{{{}}},\"decisions\":[{}]}}\n",
        rendered_readiness.join(","),
        rendered.join(",")
    );
    if contents != expected {
        return invalid("Tier 1.5 observation is not canonical one-line JSON");
    }
    Ok(())
}

fn validate_decision(value: JsonValue) -> Result<(String, bool, u64), WaterfallStoreError> {
    let mut object = value
        .into_object("Tier 1.5 decision")
        .map_err(WaterfallStoreError::InvalidReceipt)?;
    let number = take_number(&mut object, "number")?;
    let classification = take_string(&mut object, "classification")?;
    if !matches!(
        classification.as_str(),
        "needs_classify" | "needs_template" | "unlabeled"
    ) {
        return invalid("Tier 1.5 decision classification is invalid");
    }
    let decision = take_string(&mut object, "decision")?;
    let mut fields = format!(
        "\"number\":{number},\"classification\":\"{classification}\",\"decision\":\"{decision}\""
    );
    let produced = decision == "produced";
    match decision.as_str() {
        "produced" if object.is_empty() => {}
        "skipped" | "held" | "quarantined" => {
            let reason = take_string(&mut object, "reason")?;
            if !object.is_empty() || !valid_reason(&decision, &reason) {
                return invalid("Tier 1.5 decision reason is invalid");
            }
            fields.push_str(&format!(",\"reason\":\"{reason}\""));
        }
        "routed" => {
            let route = take_string(&mut object, "route")?;
            let reason = take_string(&mut object, "reason")?;
            if !object.is_empty()
                || !matches!(route.as_str(), "split" | "template")
                || !matches!(
                    reason.as_str(),
                    "epic" | "template_required" | "structured_intent" | "broad_scope"
                )
            {
                return invalid("Tier 1.5 routed decision is invalid");
            }
            fields.push_str(&format!(",\"route\":\"{route}\",\"reason\":\"{reason}\""));
        }
        _ => return invalid("Tier 1.5 decision kind is invalid"),
    }
    Ok((format!("{{{fields}}}"), produced, number))
}

fn valid_reason(decision: &str, reason: &str) -> bool {
    match decision {
        "skipped" => matches!(
            reason,
            "excluded_label" | "closed_fingerprint" | "already_groomed" | "budget_exhausted"
        ),
        "held" => matches!(reason, "thin_intent" | "ambiguous_intent" | "dependency"),
        "quarantined" => reason == "existing_security",
        _ => false,
    }
}

fn validate_failure(
    contents: &str,
    receipt: &TierReceipt,
    tier: &str,
) -> Result<(), WaterfallStoreError> {
    let mut object = parse_object(contents, "early-tier failure")?;
    let schema = take_number(&mut object, "schema")?;
    let kind = take_string(&mut object, "kind")?;
    let reason = take_string(&mut object, "reason")?;
    let TierStatus::Failed { reason: status } = receipt.status() else {
        return invalid("failure evidence is linked to a non-failed receipt");
    };
    if !object.is_empty()
        || schema != 1
        || kind != "read_failure"
        || &reason != status
        || receipt.funnel() != &FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel")
        || (tier == "Tier 1" && !reason.starts_with("ready_queue_read_failed: "))
    {
        return invalid(&format!(
            "{tier} failure evidence does not match its receipt"
        ));
    }
    let expected = format!(
        "{{\"schema\":1,\"kind\":\"read_failure\",\"reason\":\"{}\"}}\n",
        escape(&reason)
    );
    (contents == expected).then_some(()).ok_or_else(|| {
        WaterfallStoreError::InvalidReceipt(format!("{tier} failure evidence is not canonical"))
    })
}

fn parse_object(
    contents: &str,
    context: &str,
) -> Result<BTreeMap<String, JsonValue>, WaterfallStoreError> {
    JsonParser::new(contents)
        .parse()
        .map_err(WaterfallStoreError::InvalidReceipt)?
        .into_object(context)
        .map_err(WaterfallStoreError::InvalidReceipt)
}
fn take_number(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<u64, WaterfallStoreError> {
    object
        .remove(key)
        .ok_or_else(|| WaterfallStoreError::InvalidReceipt(format!("missing {key}")))?
        .into_number(key)
        .map_err(WaterfallStoreError::InvalidReceipt)
}
fn take_string(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<String, WaterfallStoreError> {
    object
        .remove(key)
        .ok_or_else(|| WaterfallStoreError::InvalidReceipt(format!("missing {key}")))?
        .into_string(key)
        .map_err(WaterfallStoreError::InvalidReceipt)
}
fn take_object(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<BTreeMap<String, JsonValue>, WaterfallStoreError> {
    object
        .remove(key)
        .ok_or_else(|| WaterfallStoreError::InvalidReceipt(format!("missing {key}")))?
        .into_object(key)
        .map_err(WaterfallStoreError::InvalidReceipt)
}
fn take_array(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<JsonValue>, WaterfallStoreError> {
    match object.remove(key) {
        Some(JsonValue::Array(values)) => Ok(values),
        _ => invalid(&format!("{key} must be an array")),
    }
}
fn number(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<u64, WaterfallStoreError> {
    match object.get(key) {
        Some(JsonValue::Number(value)) => value
            .parse()
            .map_err(|_| WaterfallStoreError::InvalidReceipt(format!("{key} must be unsigned"))),
        _ => invalid(&format!("{key} must be a number")),
    }
}
fn boolean(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<bool, WaterfallStoreError> {
    match object.get(key) {
        Some(JsonValue::Bool(value)) => Ok(*value),
        _ => invalid(&format!("{key} must be a boolean")),
    }
}
fn escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| match c {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            c if c.is_control() => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect()
}
fn invalid<T>(message: &str) -> Result<T, WaterfallStoreError> {
    Err(WaterfallStoreError::InvalidReceipt(message.to_string()))
}
