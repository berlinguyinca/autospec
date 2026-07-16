use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use autospec_core::autonomous::config::Tier4SourceDescriptor;
use autospec_core::autonomous::tier4::{
    evaluate_tier4, Tier4Candidate, Tier4GeneratedCandidates, Tier4Input, Tier4RoiPolicy,
    Tier4SourceEnvelope, Tier4SourceFact, Tier4SourcePolicy, Tier4StageResult, Tier4Verification,
    Tier4VerifierVerdicts,
};
use autospec_core::autonomous::waterfall::{sha256_hex, TierReceipt, TierStatus};
use autospec_core::state::json::{JsonParser, JsonValue};

use super::super::WaterfallStoreError;

pub(super) fn verify_completed_facts(
    root: &Path,
    receipt: &TierReceipt,
    expected_source_policy: &Tier4SourcePolicy,
) -> Result<(), WaterfallStoreError> {
    if receipt.evidence().len() != 6 {
        return invalid("Tier 4 completed receipt has an invalid evidence shape");
    }
    let documents = receipt
        .evidence()
        .iter()
        .map(|evidence| read_document(root, &evidence.reference))
        .collect::<Result<Vec<_>, _>>()?;
    let parsed = documents
        .iter()
        .map(|document| parse_object(document))
        .collect::<Result<Vec<_>, _>>()?;
    let policy = parse_policy(&parsed[0])?;
    if &policy != expected_source_policy {
        return invalid("Tier 4 source policy does not match the trusted checked-in policy");
    }
    let sources = parse_sources(&parsed[1])?;
    let generated = parse_generated(&parsed[2])?;
    let verifier = parse_verifier(&parsed[4])?;
    let evaluation = evaluate_tier4(Tier4Input::Enabled {
        source_policy: policy,
        sources: sources
            .into_iter()
            .map(Tier4StageResult::Complete)
            .collect(),
        generated: Tier4StageResult::Complete(generated),
        verifier: Tier4StageResult::Complete(verifier),
        roi_policy: Tier4RoiPolicy::v1(),
    })
    .map_err(|failure| {
        WaterfallStoreError::InvalidReceipt(format!(
            "Tier 4 evidence cannot be replayed by the sealed evaluator: {}",
            failure.status_reason()
        ))
    })?;
    let observation = evaluation.observation().ok_or_else(|| {
        WaterfallStoreError::InvalidReceipt(
            "Tier 4 evidence did not observe candidates".to_string(),
        )
    })?;
    if observation.funnel() != receipt.funnel()
        || !matches_terminal(receipt, observation.terminal())
    {
        return invalid("Tier 4 terminal status or funnel does not match sealed evidence");
    }
    let evidence = observation.documents();
    let source_policy = evidence.source_policy_json().ok_or_else(|| {
        WaterfallStoreError::InvalidReceipt("Tier 4 evidence is missing source policy".to_string())
    })?;
    let sources = evidence
        .sources_json(&sha256_hex(source_policy.as_bytes()))
        .map_err(WaterfallStoreError::InvalidReceipt)?
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt("Tier 4 evidence is missing sources".to_string())
        })?;
    let generated = evidence
        .generated_json(&sha256_hex(sources.as_bytes()))
        .map_err(WaterfallStoreError::InvalidReceipt)?
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt("Tier 4 evidence is missing generation".to_string())
        })?;
    let dedup = evidence
        .dedup_json(&sha256_hex(generated.as_bytes()))
        .map_err(WaterfallStoreError::InvalidReceipt)?
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt(
                "Tier 4 evidence is missing deduplication".to_string(),
            )
        })?;
    let verification = evidence
        .verification_json(&sha256_hex(dedup.as_bytes()))
        .map_err(WaterfallStoreError::InvalidReceipt)?
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt(
                "Tier 4 evidence is missing verification".to_string(),
            )
        })?;
    let roi_rank = evidence
        .roi_rank_json(&sha256_hex(verification.as_bytes()))
        .map_err(WaterfallStoreError::InvalidReceipt)?
        .ok_or_else(|| {
            WaterfallStoreError::InvalidReceipt("Tier 4 evidence is missing ranking".to_string())
        })?;
    let expected = [
        source_policy,
        sources,
        generated,
        dedup,
        verification,
        roi_rank,
    ];
    if documents
        .iter()
        .zip(expected)
        .any(|(actual, expected)| actual != &expected)
    {
        return invalid("Tier 4 evidence is not the canonical sealed evaluator output");
    }
    Ok(())
}

fn read_document(root: &Path, reference: &str) -> Result<String, WaterfallStoreError> {
    fs::read_to_string(root.join(reference)).map_err(|error| {
        WaterfallStoreError::InvalidReceipt(format!(
            "cannot read Tier 4 evidence {reference}: {error}"
        ))
    })
}

fn parse_object(document: &str) -> Result<BTreeMap<String, JsonValue>, WaterfallStoreError> {
    JsonParser::new(document)
        .parse()
        .map_err(|error| {
            WaterfallStoreError::InvalidReceipt(format!("invalid Tier 4 JSON: {error}"))
        })?
        .into_object("Tier 4 evidence")
        .map_err(WaterfallStoreError::InvalidReceipt)
}

fn parse_policy(
    object: &BTreeMap<String, JsonValue>,
) -> Result<Tier4SourcePolicy, WaterfallStoreError> {
    Ok(Tier4SourcePolicy {
        schema_version: number(object, "schema")?,
        policy_identity: string(object, "policy_identity")?.to_string(),
        descriptors: array(object, "descriptors")?
            .iter()
            .map(|value| {
                let object = as_object(value)?;
                Ok(Tier4SourceDescriptor {
                    id: string(object, "id")?.to_string(),
                    host: string(object, "host")?.to_string(),
                    path: string(object, "path")?.to_string(),
                    max_bytes: u32::try_from(number(object, "max_bytes")?)
                        .map_err(|_| invalid_error("Tier 4 descriptor max bytes overflows"))?,
                    deadline_millis: u32::try_from(number(object, "deadline_millis")?)
                        .map_err(|_| invalid_error("Tier 4 descriptor deadline overflows"))?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_sources(
    object: &BTreeMap<String, JsonValue>,
) -> Result<Vec<Tier4SourceEnvelope>, WaterfallStoreError> {
    array(object, "sources")?
        .iter()
        .map(|value| {
            let object = as_object(value)?;
            Ok(Tier4SourceEnvelope {
                schema_version: number(object, "schema_version")?,
                producer_identity: string(object, "producer_identity")?.to_string(),
                producer_protocol_version: string(object, "producer_protocol_version")?.to_string(),
                source_id: string(object, "source_id")?.to_string(),
                byte_length: u32::try_from(number(object, "byte_length")?)
                    .map_err(|_| invalid_error("Tier 4 source byte length overflows"))?,
                body_sha256: string(object, "body_sha256")?.to_string(),
                facts: array(object, "facts")?
                    .iter()
                    .map(|value| {
                        let object = as_object(value)?;
                        Ok(Tier4SourceFact {
                            fact_key: string(object, "fact_key")?.to_string(),
                            fact_type: string(object, "fact_type")?.to_string(),
                            value: string(object, "value")?.to_string(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            })
        })
        .collect()
}

fn parse_generated(
    object: &BTreeMap<String, JsonValue>,
) -> Result<Tier4GeneratedCandidates, WaterfallStoreError> {
    Ok(Tier4GeneratedCandidates {
        schema_version: number(object, "schema")?,
        generator_identity: string(object, "generator_identity")?.to_string(),
        generator_protocol_version: string(object, "generator_protocol_version")?.to_string(),
        candidates: array(object, "candidates")?
            .iter()
            .map(|value| {
                let object = as_object(value)?;
                Ok(Tier4Candidate {
                    stable_key: string(object, "stable_key")?.to_string(),
                    source_id: string(object, "source_id")?.to_string(),
                    fact_key: string(object, "fact_key")?.to_string(),
                    title: string(object, "title")?.to_string(),
                    rationale: string(object, "rationale")?.to_string(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_verifier(
    object: &BTreeMap<String, JsonValue>,
) -> Result<Tier4VerifierVerdicts, WaterfallStoreError> {
    Ok(Tier4VerifierVerdicts {
        schema_version: number(object, "schema")?,
        verifier_identity: string(object, "verifier_identity")?.to_string(),
        verifier_protocol_version: string(object, "verifier_protocol_version")?.to_string(),
        verdicts: array(object, "verdicts")?
            .iter()
            .map(|value| {
                let object = as_object(value)?;
                match string(object, "result")? {
                    "accepted" => Ok(Tier4Verification::Accepted {
                        stable_key: string(object, "stable_key")?.to_string(),
                        roi_millis: u16::try_from(number(object, "roi_millis")?)
                            .map_err(|_| invalid_error("Tier 4 verdict ROI overflows"))?,
                        reason: string(object, "reason")?.to_string(),
                    }),
                    "rejected" => Ok(Tier4Verification::Rejected {
                        stable_key: string(object, "stable_key")?.to_string(),
                        reason: string(object, "reason")?.to_string(),
                    }),
                    _ => invalid("Tier 4 verifier result is invalid"),
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn matches_terminal(
    receipt: &TierReceipt,
    terminal: &autospec_core::autonomous::tier4::Tier4Terminal,
) -> bool {
    match (receipt.status(), terminal) {
        (
            TierStatus::Exhausted { reason: left },
            autospec_core::autonomous::tier4::Tier4Terminal::Exhausted { reason: right },
        ) => left == right,
        (
            TierStatus::Produced { count: left },
            autospec_core::autonomous::tier4::Tier4Terminal::Produced { count: right },
        ) => left == right,
        _ => false,
    }
}

fn as_object(value: &JsonValue) -> Result<&BTreeMap<String, JsonValue>, WaterfallStoreError> {
    match value {
        JsonValue::Object(object) => Ok(object),
        _ => invalid("Tier 4 nested evidence must be an object"),
    }
}

fn array<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<&'a [JsonValue], WaterfallStoreError> {
    match object.get(key) {
        Some(JsonValue::Array(values)) => Ok(values),
        _ => invalid("Tier 4 evidence array is invalid"),
    }
}

fn string<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<&'a str, WaterfallStoreError> {
    match object.get(key) {
        Some(JsonValue::String(value)) => Ok(value),
        _ => invalid("Tier 4 evidence string is invalid"),
    }
}

fn number(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<u64, WaterfallStoreError> {
    match object.get(key) {
        Some(JsonValue::Number(value)) => value
            .parse()
            .map_err(|_| invalid_error("Tier 4 evidence number is invalid")),
        _ => invalid("Tier 4 evidence number is invalid"),
    }
}

fn invalid_error(message: &str) -> WaterfallStoreError {
    WaterfallStoreError::InvalidReceipt(message.to_string())
}

fn invalid<T>(message: &str) -> Result<T, WaterfallStoreError> {
    Err(invalid_error(message))
}
