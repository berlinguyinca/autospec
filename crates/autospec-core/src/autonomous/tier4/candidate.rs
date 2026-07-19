use std::collections::{BTreeMap, BTreeSet};

use crate::autonomous::config::Tier4SourceDescriptor;

use super::failure::Tier4FailureCode;
use super::model::{
    bounded_text, Tier4Candidate, Tier4CandidateReference, Tier4Deduplication,
    Tier4DeduplicationGroup, Tier4GeneratedCandidates, Tier4RoiPolicy, Tier4SourceEnvelope,
    Tier4SourcePolicy, Tier4Verification, Tier4VerifierVerdicts, FIELD_LIMIT,
    MAX_GENERATED_REFERENCES, MAX_SOURCES, MAX_SOURCE_FACTS, ROI_SCALE_MILLIS,
    ROI_THRESHOLD_MILLIS,
};
use super::{Tier4Failure, Tier4Stage, TIER4_SCHEMA};

pub(super) fn validate_source_policy(policy: &Tier4SourcePolicy) -> Result<(), Tier4Failure> {
    if policy.schema_version != TIER4_SCHEMA
        || !bounded_text(&policy.policy_identity, FIELD_LIMIT)
        || !(1..=MAX_SOURCES).contains(&policy.descriptors.len())
    {
        return Err(failure(
            Tier4Stage::SourcePolicy,
            Tier4FailureCode::InvalidSourcePolicy,
            "source policy schema, identity, or descriptor count is invalid",
        ));
    }

    let mut ids = BTreeSet::new();
    let mut hosts = BTreeSet::new();
    for descriptor in &policy.descriptors {
        if !valid_descriptor(descriptor)
            || !ids.insert(descriptor.id.clone())
            || !hosts.insert(descriptor.host.clone())
        {
            return Err(failure(
                Tier4Stage::SourcePolicy,
                Tier4FailureCode::InvalidSourcePolicy,
                "source policy descriptor is not unique or is outside checked-in limits",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_sources(
    policy: &Tier4SourcePolicy,
    sources: &mut [Tier4SourceEnvelope],
) -> Result<BTreeSet<(String, String)>, Tier4Failure> {
    if sources.len() != policy.descriptors.len() {
        return Err(failure(
            Tier4Stage::Sources,
            Tier4FailureCode::InvalidSourceCoverage,
            "source envelope count does not match the checked-in policy",
        ));
    }

    let mut references = BTreeSet::new();
    for (descriptor, envelope) in policy.descriptors.iter().zip(sources) {
        if descriptor.id != envelope.source_id {
            return Err(failure(
                Tier4Stage::Sources,
                Tier4FailureCode::InvalidSourceCoverage,
                "source envelopes must match descriptor order exactly",
            ));
        }
        validate_envelope(descriptor, envelope)?;
        envelope
            .facts
            .sort_by(|left, right| left.fact_key.cmp(&right.fact_key));
        for fact in &envelope.facts {
            if !references.insert((envelope.source_id.clone(), fact.fact_key.clone())) {
                return Err(failure(
                    Tier4Stage::Sources,
                    Tier4FailureCode::InvalidSourceFact,
                    "typed source facts must be unique within each source",
                ));
            }
        }
    }
    Ok(references)
}

pub(super) fn validate_generated(
    generated: &mut Tier4GeneratedCandidates,
    facts: &BTreeSet<(String, String)>,
) -> Result<(), Tier4Failure> {
    if generated.schema_version != TIER4_SCHEMA
        || !bounded_text(&generated.generator_identity, FIELD_LIMIT)
        || !bounded_text(&generated.generator_protocol_version, FIELD_LIMIT)
        || generated.candidates.len() > MAX_GENERATED_REFERENCES
    {
        return Err(failure(
            Tier4Stage::Generator,
            Tier4FailureCode::InvalidGeneratedCandidates,
            "generated candidate envelope is invalid or exceeds the reference cap",
        ));
    }

    let mut instances = BTreeSet::new();
    for candidate in &generated.candidates {
        let reference = (candidate.source_id.clone(), candidate.fact_key.clone());
        let instance = (
            candidate.stable_key.clone(),
            candidate.source_id.clone(),
            candidate.fact_key.clone(),
        );
        if !valid_candidate(candidate) || !facts.contains(&reference) || !instances.insert(instance)
        {
            return Err(failure(
                Tier4Stage::Generator,
                Tier4FailureCode::InvalidCandidate,
                "candidate must uniquely reference one observed typed source fact",
            ));
        }
    }
    generated.candidates.sort_by(|left, right| {
        left.stable_key
            .cmp(&right.stable_key)
            .then_with(|| left.source_id.cmp(&right.source_id))
            .then_with(|| left.fact_key.cmp(&right.fact_key))
    });
    Ok(())
}

pub(super) fn deduplicate(
    candidates: &[Tier4Candidate],
) -> Result<Tier4Deduplication, Tier4Failure> {
    let mut grouped = BTreeMap::<String, Vec<&Tier4Candidate>>::new();
    for candidate in candidates {
        grouped
            .entry(candidate.stable_key.clone())
            .or_default()
            .push(candidate);
    }

    let mut groups = Vec::with_capacity(grouped.len());
    for (stable_key, candidates) in grouped {
        let first = candidates
            .first()
            .expect("BTreeMap groups are never empty after insertion");
        if candidates.iter().any(|candidate| {
            candidate.title != first.title || candidate.rationale != first.rationale
        }) {
            return Err(failure(
                Tier4Stage::Deduplicator,
                Tier4FailureCode::DuplicateConflict,
                "equal stable keys must preserve the same candidate semantics",
            ));
        }
        let mut references = candidates
            .iter()
            .map(|candidate| Tier4CandidateReference {
                source_id: candidate.source_id.clone(),
                fact_key: candidate.fact_key.clone(),
            })
            .collect::<Vec<_>>();
        references.sort();
        groups.push(Tier4DeduplicationGroup {
            stable_key,
            title: first.title.clone(),
            rationale: first.rationale.clone(),
            references,
        });
    }
    Ok(Tier4Deduplication { groups })
}

pub(super) fn validate_verifier(
    verifier: &mut Tier4VerifierVerdicts,
    expected: &BTreeSet<String>,
) -> Result<BTreeMap<String, Tier4Verification>, Tier4Failure> {
    if verifier.schema_version != TIER4_SCHEMA
        || !bounded_text(&verifier.verifier_identity, FIELD_LIMIT)
        || !bounded_text(&verifier.verifier_protocol_version, FIELD_LIMIT)
    {
        return Err(failure(
            Tier4Stage::Verifier,
            Tier4FailureCode::InvalidVerdictCoverage,
            "verifier schema or identity is invalid",
        ));
    }

    let mut coverage = BTreeMap::new();
    for verdict in &verifier.verdicts {
        let valid = bounded_text(verdict.stable_key(), FIELD_LIMIT)
            && bounded_text(verdict.reason(), FIELD_LIMIT)
            && expected.contains(verdict.stable_key())
            && verdict
                .roi_millis()
                .is_none_or(|roi| roi <= ROI_SCALE_MILLIS);
        if !valid
            || coverage
                .insert(verdict.stable_key().to_string(), verdict.clone())
                .is_some()
        {
            return Err(failure(
                Tier4Stage::Verifier,
                Tier4FailureCode::InvalidVerdictCoverage,
                "verdict coverage must contain one bounded result for every deduplicated key",
            ));
        }
    }
    if coverage.len() != expected.len() {
        Err(failure(
            Tier4Stage::Verifier,
            Tier4FailureCode::InvalidVerdictCoverage,
            "verdict coverage is incomplete",
        ))
    } else {
        verifier
            .verdicts
            .sort_by(|left, right| left.stable_key().cmp(right.stable_key()));
        Ok(coverage)
    }
}

pub(super) fn validate_roi_policy(policy: Tier4RoiPolicy) -> Result<(), Tier4Failure> {
    if policy.threshold_millis == ROI_THRESHOLD_MILLIS && policy.scale_millis == ROI_SCALE_MILLIS {
        Ok(())
    } else {
        Err(failure(
            Tier4Stage::RoiRank,
            Tier4FailureCode::InvalidRoiPolicy,
            "Tier 4 requires the fixed 500 of 1000 ROI policy",
        ))
    }
}

pub(super) fn failure(
    stage: Tier4Stage,
    code: Tier4FailureCode,
    detail: impl Into<String>,
) -> Tier4Failure {
    Tier4Failure::initial(stage, code, detail.into())
}

fn validate_envelope(
    descriptor: &Tier4SourceDescriptor,
    envelope: &Tier4SourceEnvelope,
) -> Result<(), Tier4Failure> {
    if envelope.facts.len() > MAX_SOURCE_FACTS {
        return Err(failure(
            Tier4Stage::Sources,
            Tier4FailureCode::InvalidSourceFact,
            "typed source facts exceed the per-source fact cap",
        ));
    }
    let valid = envelope.schema_version == TIER4_SCHEMA
        && bounded_text(&envelope.producer_identity, FIELD_LIMIT)
        && bounded_text(&envelope.producer_protocol_version, FIELD_LIMIT)
        && envelope.byte_length <= descriptor.max_bytes
        && sealed_digest(&envelope.body_sha256);
    if valid {
        for fact in &envelope.facts {
            if !bounded_text(&fact.fact_key, FIELD_LIMIT)
                || !bounded_text(&fact.fact_type, FIELD_LIMIT)
                || !bounded_text(&fact.value, FIELD_LIMIT)
            {
                return Err(failure(
                    Tier4Stage::Sources,
                    Tier4FailureCode::InvalidSourceFact,
                    "typed source fact fields must be bounded scalars",
                ));
            }
        }
        Ok(())
    } else {
        Err(failure(
            Tier4Stage::Sources,
            Tier4FailureCode::InvalidSourceEnvelope,
            "source envelope metadata, digest, byte length, or fact cap is invalid",
        ))
    }
}

fn valid_descriptor(descriptor: &Tier4SourceDescriptor) -> bool {
    valid_id(&descriptor.id)
        && valid_host(&descriptor.host)
        && valid_path(&descriptor.path)
        && (1..=1_048_576).contains(&descriptor.max_bytes)
        && (100..=30_000).contains(&descriptor.deadline_millis)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.is_ascii()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.is_ascii()
        && value.contains('.')
        && !ipv4_like_host(value)
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn ipv4_like_host(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    (1..=4).contains(&parts.len()) && parts.iter().all(|part| ipv4_number(part))
}

fn ipv4_number(value: &str) -> bool {
    if let Some(hex) = value.strip_prefix("0x") {
        return !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 256
        && !value.contains(['?', '#', '\\'])
        && (value == "/"
            || value[1..]
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != ".."))
}

fn valid_candidate(candidate: &Tier4Candidate) -> bool {
    bounded_text(&candidate.stable_key, FIELD_LIMIT)
        && bounded_text(&candidate.source_id, FIELD_LIMIT)
        && bounded_text(&candidate.fact_key, FIELD_LIMIT)
        && bounded_text(&candidate.title, FIELD_LIMIT)
        && bounded_text(&candidate.rationale, FIELD_LIMIT)
}

fn sealed_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
