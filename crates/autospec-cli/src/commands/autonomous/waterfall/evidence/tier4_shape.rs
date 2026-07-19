use std::collections::BTreeMap;

use autospec_core::state::json::JsonValue;

use super::{canonical, Tier4EvidenceArtifact};

const DESCRIPTOR_KEYS: [&str; 5] = ["id", "host", "path", "max_bytes", "deadline_millis"];
const SOURCE_KEYS: [&str; 7] = [
    "schema_version",
    "producer_identity",
    "producer_protocol_version",
    "source_id",
    "byte_length",
    "body_sha256",
    "facts",
];
const FACT_KEYS: [&str; 3] = ["fact_key", "fact_type", "value"];
const CANDIDATE_KEYS: [&str; 5] = ["stable_key", "source_id", "fact_key", "title", "rationale"];
const GROUP_KEYS: [&str; 4] = ["stable_key", "title", "rationale", "references"];
const REFERENCE_KEYS: [&str; 2] = ["source_id", "fact_key"];
const ACCEPTED_VERDICT_KEYS: [&str; 4] = ["stable_key", "result", "roi_millis", "reason"];
const REJECTED_VERDICT_KEYS: [&str; 3] = ["stable_key", "result", "reason"];
const TERMINAL_EXHAUSTED_KEYS: [&str; 2] = ["result", "reason"];
const TERMINAL_PRODUCED_KEYS: [&str; 2] = ["result", "count"];
const FUNNEL_KEYS: [&str; 5] = [
    "observed",
    "deduplicated",
    "verified",
    "roi_approved",
    "ranked",
];
const ROI_KEYS: [&str; 5] = [
    "stable_key",
    "verified",
    "roi_millis",
    "permitted",
    "reason",
];
const RANKED_KEYS: [&str; 6] = [
    "rank",
    "stable_key",
    "roi_millis",
    "title",
    "rationale",
    "references",
];

pub(super) fn kind(artifact: Tier4EvidenceArtifact) -> &'static str {
    match artifact {
        Tier4EvidenceArtifact::Policy => "tier4_policy",
        Tier4EvidenceArtifact::SourcePolicy => "tier4_source_policy",
        Tier4EvidenceArtifact::Sources => "tier4_sources",
        Tier4EvidenceArtifact::Generated => "tier4_generated",
        Tier4EvidenceArtifact::Dedup => "tier4_dedup",
        Tier4EvidenceArtifact::Verification => "tier4_verification",
        Tier4EvidenceArtifact::RoiRank => "tier4_roi_rank",
        Tier4EvidenceArtifact::Failure => "tier4_failure",
    }
}

pub(super) fn keys(artifact: Tier4EvidenceArtifact) -> &'static [&'static str] {
    match artifact {
        Tier4EvidenceArtifact::Policy => &["schema", "kind", "mode", "reason", "policy_source"],
        Tier4EvidenceArtifact::SourcePolicy => {
            &["schema", "kind", "policy_identity", "descriptors"]
        }
        Tier4EvidenceArtifact::Sources => &["schema", "kind", "predecessor_digest", "sources"],
        Tier4EvidenceArtifact::Generated => &[
            "schema",
            "kind",
            "predecessor_digest",
            "generator_identity",
            "generator_protocol_version",
            "candidates",
        ],
        Tier4EvidenceArtifact::Dedup => &["schema", "kind", "predecessor_digest", "groups"],
        Tier4EvidenceArtifact::Verification => &[
            "schema",
            "kind",
            "predecessor_digest",
            "verifier_identity",
            "verifier_protocol_version",
            "verdicts",
        ],
        Tier4EvidenceArtifact::RoiRank => &[
            "schema",
            "kind",
            "predecessor_digest",
            "rank_limit",
            "terminal",
            "funnel",
            "candidates",
            "ranked",
        ],
        Tier4EvidenceArtifact::Failure => &[
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

pub(super) fn has_canonical_nested_keys(
    artifact: Tier4EvidenceArtifact,
    contents: &str,
    object: &BTreeMap<String, JsonValue>,
) -> bool {
    let mut expected = vec![keys(artifact)];
    match artifact {
        Tier4EvidenceArtifact::Policy => {}
        Tier4EvidenceArtifact::SourcePolicy => {
            if !push_objects(&mut expected, object, "descriptors", &DESCRIPTOR_KEYS) {
                return false;
            }
        }
        Tier4EvidenceArtifact::Sources => {
            let Some(sources) = array(object, "sources") else {
                return false;
            };
            for source in sources {
                expected.push(&SOURCE_KEYS);
                let JsonValue::Object(source) = source else {
                    return false;
                };
                if !push_objects(&mut expected, source, "facts", &FACT_KEYS) {
                    return false;
                }
            }
        }
        Tier4EvidenceArtifact::Generated => {
            if !push_objects(&mut expected, object, "candidates", &CANDIDATE_KEYS) {
                return false;
            }
        }
        Tier4EvidenceArtifact::Dedup => {
            let Some(groups) = array(object, "groups") else {
                return false;
            };
            for group in groups {
                expected.push(&GROUP_KEYS);
                let JsonValue::Object(group) = group else {
                    return false;
                };
                if !push_objects(&mut expected, group, "references", &REFERENCE_KEYS) {
                    return false;
                }
            }
        }
        Tier4EvidenceArtifact::Verification => {
            let Some(verdicts) = array(object, "verdicts") else {
                return false;
            };
            for verdict in verdicts {
                let JsonValue::Object(verdict) = verdict else {
                    return false;
                };
                match string(verdict, "result") {
                    Some("accepted") => expected.push(&ACCEPTED_VERDICT_KEYS),
                    Some("rejected") => expected.push(&REJECTED_VERDICT_KEYS),
                    _ => return false,
                }
            }
        }
        Tier4EvidenceArtifact::RoiRank => {
            let Some(JsonValue::Object(terminal)) = object.get("terminal") else {
                return false;
            };
            match string(terminal, "result") {
                Some("exhausted") => expected.push(&TERMINAL_EXHAUSTED_KEYS),
                Some("produced") => expected.push(&TERMINAL_PRODUCED_KEYS),
                _ => return false,
            }
            expected.push(&FUNNEL_KEYS);
            if !push_objects(&mut expected, object, "candidates", &ROI_KEYS) {
                return false;
            }
            let Some(ranked) = array(object, "ranked") else {
                return false;
            };
            for row in ranked {
                expected.push(&RANKED_KEYS);
                let JsonValue::Object(row) = row else {
                    return false;
                };
                if !push_objects(&mut expected, row, "references", &REFERENCE_KEYS) {
                    return false;
                }
            }
        }
        Tier4EvidenceArtifact::Failure => expected.push(&FUNNEL_KEYS),
    }
    canonical::matches_object_key_orders(contents, &expected)
}

fn push_objects(
    expected: &mut Vec<&'static [&'static str]>,
    object: &BTreeMap<String, JsonValue>,
    key: &str,
    keys: &'static [&'static str],
) -> bool {
    let Some(values) = array(object, key) else {
        return false;
    };
    if values
        .iter()
        .any(|value| !matches!(value, JsonValue::Object(_)))
    {
        return false;
    }
    expected.extend(std::iter::repeat_n(keys, values.len()));
    true
}

fn array<'a>(object: &'a BTreeMap<String, JsonValue>, key: &str) -> Option<&'a [JsonValue]> {
    match object.get(key) {
        Some(JsonValue::Array(values)) => Some(values),
        _ => None,
    }
}

fn string<'a>(object: &'a BTreeMap<String, JsonValue>, key: &str) -> Option<&'a str> {
    match object.get(key) {
        Some(JsonValue::String(value)) => Some(value),
        _ => None,
    }
}
