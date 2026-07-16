use std::collections::BTreeMap;

use autospec_core::state::json::JsonValue;

use super::{canonical, Tier3EvidenceArtifact};

pub(super) const FUNNEL_KEYS: [&str; 5] = [
    "observed",
    "deduplicated",
    "verified",
    "roi_approved",
    "ranked",
];
const FINDING_KEYS: [&str; 6] = ["kind", "severity", "rule_id", "path", "line", "message"];

pub(super) fn kind(artifact: Tier3EvidenceArtifact) -> &'static str {
    match artifact {
        Tier3EvidenceArtifact::Policy => "tier3_policy",
        Tier3EvidenceArtifact::Architecture => "tier3_architecture",
        Tier3EvidenceArtifact::Coverage => "tier3_coverage",
        Tier3EvidenceArtifact::Debt => "tier3_debt",
        Tier3EvidenceArtifact::Findings => "tier3_findings",
        Tier3EvidenceArtifact::Failure => "tier3_failure",
    }
}

pub(super) fn keys(artifact: Tier3EvidenceArtifact) -> &'static [&'static str] {
    match artifact {
        Tier3EvidenceArtifact::Policy => &["schema", "kind", "mode", "reason", "policy_source"],
        Tier3EvidenceArtifact::Architecture => &[
            "schema",
            "kind",
            "adapter_version",
            "rule_version",
            "findings",
        ],
        Tier3EvidenceArtifact::Coverage | Tier3EvidenceArtifact::Debt => &[
            "schema",
            "kind",
            "predecessor_digest",
            "adapter_version",
            "rule_version",
            "findings",
        ],
        Tier3EvidenceArtifact::Findings => &[
            "schema",
            "kind",
            "predecessor_digest",
            "rank_limit",
            "funnel",
            "deduplicated",
            "ranked",
        ],
        Tier3EvidenceArtifact::Failure => &[
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
    artifact: Tier3EvidenceArtifact,
    contents: &str,
    object: &BTreeMap<String, JsonValue>,
) -> bool {
    let mut expected = vec![keys(artifact)];
    match artifact {
        Tier3EvidenceArtifact::Architecture
        | Tier3EvidenceArtifact::Coverage
        | Tier3EvidenceArtifact::Debt => {
            let Some(count) = array_len(object, "findings") else {
                return false;
            };
            for _ in 0..count {
                expected.push(&FINDING_KEYS);
            }
        }
        Tier3EvidenceArtifact::Findings => {
            expected.push(&FUNNEL_KEYS);
            for field in ["deduplicated", "ranked"] {
                let Some(count) = array_len(object, field) else {
                    return false;
                };
                for _ in 0..count {
                    expected.push(&FINDING_KEYS);
                }
            }
        }
        Tier3EvidenceArtifact::Failure => expected.push(&FUNNEL_KEYS),
        Tier3EvidenceArtifact::Policy => {}
    }
    canonical::matches_object_key_orders(contents, &expected)
}

fn array_len(object: &BTreeMap<String, JsonValue>, key: &str) -> Option<usize> {
    match object.get(key) {
        Some(JsonValue::Array(values)) => Some(values.len()),
        _ => None,
    }
}
