use std::collections::BTreeSet;

use crate::explore::specialists::{DetectedDomain, FileLineEvidence};

use super::model::{
    bounded_text, StrictCollectorEvidence, Tier2Failure, Tier2FailureCode, Tier2GeneratedProposals,
    Tier2Proposal, Tier2Stage, FIELD_SCALAR_LIMIT, TIER2_SCHEMA,
};

pub(super) type EvidenceKey = (String, usize, String);

pub(super) fn validate_collector(
    collector: &StrictCollectorEvidence,
) -> Result<BTreeSet<EvidenceKey>, Tier2Failure> {
    if collector.schema_version != TIER2_SCHEMA
        || collector.collector_version != "strict-local-v1"
        || !bounded_text(&collector.collector_version, FIELD_SCALAR_LIMIT)
        || !bounded_text(&collector.canonical_repo_scope, FIELD_SCALAR_LIMIT)
    {
        return Err(invalid_collector(
            "collector header is not the strict local schema",
        ));
    }
    let mut rows = BTreeSet::new();
    let mut names = BTreeSet::new();
    for pair in collector.domains.windows(2) {
        if !domain_before(&pair[0], &pair[1]) {
            return Err(invalid_collector("domains are not strictly canonical"));
        }
    }
    for domain in &collector.domains {
        if !bounded_text(&domain.name, FIELD_SCALAR_LIMIT)
            || domain.score == 0
            || domain.evidence.is_empty()
            || !names.insert(domain.name.as_str())
        {
            return Err(invalid_collector("domain is invalid"));
        }
        for pair in domain.evidence.windows(2) {
            if evidence_key(&pair[0]) >= evidence_key(&pair[1]) {
                return Err(invalid_collector(
                    "collector evidence is not sorted and unique",
                ));
            }
        }
        for evidence in &domain.evidence {
            validate_evidence(evidence).map_err(invalid_collector)?;
            rows.insert(evidence_key(evidence));
        }
    }
    Ok(rows)
}

pub(super) fn validate_generator(
    generator: &Tier2GeneratedProposals,
    rows: &BTreeSet<EvidenceKey>,
) -> Result<(), Tier2Failure> {
    if !bounded_text(&generator.generator_identity, FIELD_SCALAR_LIMIT)
        || !bounded_text(&generator.generator_protocol_version, FIELD_SCALAR_LIMIT)
    {
        return Err(invalid_proposal("generator identity is invalid"));
    }
    let mut keys = BTreeSet::new();
    for proposal in &generator.proposals {
        if !bounded_text(&proposal.stable_key, FIELD_SCALAR_LIMIT)
            || !bounded_text(&proposal.title, FIELD_SCALAR_LIMIT)
            || !bounded_text(&proposal.named_consumer, FIELD_SCALAR_LIMIT)
            || proposal.confidence_millis > 1000
            || proposal.evidence.is_empty()
            || normalize_title(&proposal.title).is_empty()
            || !keys.insert(proposal.stable_key.as_str())
        {
            return Err(invalid_proposal("proposal fields are invalid"));
        }
        for pair in proposal.evidence.windows(2) {
            if evidence_key(&pair[0]) >= evidence_key(&pair[1]) {
                return Err(invalid_proposal(
                    "proposal evidence is not sorted and unique",
                ));
            }
        }
        for evidence in &proposal.evidence {
            validate_evidence(evidence).map_err(invalid_proposal)?;
            if !rows.contains(&evidence_key(evidence)) {
                return Err(invalid_proposal(
                    "proposal evidence is outside the collector",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_verifier_metadata(
    verifier: &super::Tier2VerifierVerdicts,
) -> Result<(), Tier2Failure> {
    if bounded_text(&verifier.verifier_identity, FIELD_SCALAR_LIMIT)
        && bounded_text(&verifier.verifier_protocol_version, FIELD_SCALAR_LIMIT)
    {
        Ok(())
    } else {
        Err(invalid_coverage("verifier identity is invalid"))
    }
}

pub(super) fn evidence_key(evidence: &FileLineEvidence) -> EvidenceKey {
    (
        evidence.file.clone(),
        evidence.line,
        evidence.r#match.clone(),
    )
}

pub(super) fn normalize_title(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    let content = ["feat:", "fix:", "docs:", "refactor:", "test:", "chore:"]
        .iter()
        .find_map(|prefix| lower.strip_prefix(prefix))
        .unwrap_or(&lower);
    let mut normalized = String::new();
    let mut separator = false;
    for character in content.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    normalized
}

pub(super) fn score(proposal: &Tier2Proposal) -> u64 {
    u64::from(proposal.confidence_millis) / proposal.complexity.units()
}

pub(super) fn invalid_collector(detail: impl Into<String>) -> Tier2Failure {
    failure(
        Tier2Stage::Collector,
        Tier2FailureCode::InvalidCollectorSchema,
        detail,
    )
}

pub(super) fn invalid_proposal(detail: impl Into<String>) -> Tier2Failure {
    failure(
        Tier2Stage::Generator,
        Tier2FailureCode::InvalidProposal,
        detail,
    )
}

pub(super) fn invalid_coverage(detail: impl Into<String>) -> Tier2Failure {
    failure(
        Tier2Stage::Verifier,
        Tier2FailureCode::InvalidVerdictCoverage,
        detail,
    )
}

fn validate_evidence(evidence: &FileLineEvidence) -> Result<(), &'static str> {
    let relative = !evidence.file.starts_with('/')
        && evidence
            .file
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if relative
        && evidence.line > 0
        && bounded_text(&evidence.file, FIELD_SCALAR_LIMIT)
        && bounded_text(&evidence.r#match, 120)
    {
        Ok(())
    } else {
        Err("evidence row is invalid")
    }
}

fn domain_before(left: &DetectedDomain, right: &DetectedDomain) -> bool {
    left.score > right.score || (left.score == right.score && left.name < right.name)
}

fn failure(stage: Tier2Stage, code: Tier2FailureCode, detail: impl Into<String>) -> Tier2Failure {
    Tier2Failure::initial(stage, code, detail)
}
