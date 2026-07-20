use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::model::{
    StrictCollectorEvidence, Tier2Deduplication, Tier2Evaluation, Tier2Failure,
    Tier2GeneratedProposals, Tier2Observation, Tier2Proposal, Tier2RankedProposal,
    Tier2Verification, Tier2VerifierVerdicts, TIER2_NORMALIZATION_VERSION, TIER2_RANK_LIMIT,
    TIER2_SCHEMA,
};
use super::partial::PartialEvidenceState;

const EXCLUSION_DIGEST_VERSION: &str = "autospec-tier2-exclusion-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier2PollutionCode {
    ProhibitedVendorPath,
}

impl Tier2PollutionCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProhibitedVendorPath => "prohibited_vendor_path",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2PollutionFinding {
    pub path: String,
    pub excluded_component: String,
    pub finding: Tier2PollutionCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2ExclusionReport {
    pub(super) policy_digest: String,
    pub(super) excluded_path_count: u64,
    pub(super) pollution_findings: Vec<Tier2PollutionFinding>,
}

impl Tier2ExclusionReport {
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }

    pub fn excluded_path_count(&self) -> u64 {
        self.excluded_path_count
    }

    pub fn pollution_findings(&self) -> &[Tier2PollutionFinding] {
        &self.pollution_findings
    }
}

pub(super) fn exclusion_policy_digest(components: &BTreeSet<String>) -> String {
    let mut digest = Sha256::new();
    for value in
        std::iter::once(EXCLUSION_DIGEST_VERSION).chain(components.iter().map(String::as_str))
    {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn valid_exclusion_component(value: &str) -> bool {
    super::model::bounded_text(value, super::model::FIELD_SCALAR_LIMIT)
        && !matches!(value, "." | "..")
        && !value.contains(['/', '\\'])
        && value.trim() == value
}

/// Read-only canonical receipts derived from one evaluator-sealed outcome.
pub struct Tier2EvidenceDocuments<'a> {
    source: DocumentSource<'a>,
}

enum DocumentSource<'a> {
    Observation(&'a Tier2Observation),
    Failure(&'a Tier2Failure),
}

impl Tier2Observation {
    pub fn collector(&self) -> &StrictCollectorEvidence {
        &self.collector
    }

    pub fn generated(&self) -> &Tier2GeneratedProposals {
        &self.generated
    }

    pub fn deduplication(&self) -> &Tier2Deduplication {
        &self.deduplication
    }

    pub fn verification(&self) -> &Tier2VerifierVerdicts {
        &self.verification
    }

    pub fn roi(&self) -> &[super::Tier2RoiDecision] {
        &self.roi
    }

    pub fn ranked(&self) -> &[Tier2RankedProposal] {
        &self.ranked
    }

    pub fn documents(&self) -> Tier2EvidenceDocuments<'_> {
        Tier2EvidenceDocuments {
            source: DocumentSource::Observation(self),
        }
    }
}

impl Tier2Failure {
    pub fn documents(&self) -> Option<Tier2EvidenceDocuments<'_>> {
        self.is_sealed().then_some(Tier2EvidenceDocuments {
            source: DocumentSource::Failure(self),
        })
    }
}

impl Tier2EvidenceDocuments<'_> {
    pub fn collector_json(&self) -> Option<String> {
        match self.source {
            DocumentSource::Observation(observation) => Some(collector_json(
                observation.collector(),
                Some(observation.exclusion_report()),
            )),
            DocumentSource::Failure(failure) => self
                .collector()
                .map(|collector| collector_json(collector, failure.exclusion_report.as_ref())),
        }
    }

    pub fn generated_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.generated()
            .map(|generated| generated_json(generated, predecessor_digest))
            .transpose()
    }

    pub fn deduplication_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.deduplication()
            .map(|deduplication| deduplication_json(deduplication, predecessor_digest))
            .transpose()
    }

    pub fn verification_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.verification()
            .map(|verification| verification_json(verification, predecessor_digest))
            .transpose()
    }

    pub fn roi_rank_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        match self.source {
            DocumentSource::Observation(observation) => {
                roi_rank_json(observation, predecessor_digest).map(Some)
            }
            DocumentSource::Failure(_) => Ok(None),
        }
    }

    pub fn failure_json(&self, predecessor_digest: Option<&str>) -> Result<Option<String>, String> {
        match self.source {
            DocumentSource::Observation(_) => Ok(None),
            DocumentSource::Failure(failure) => failure_json(failure, predecessor_digest).map(Some),
        }
    }

    fn collector(&self) -> Option<&StrictCollectorEvidence> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.collector()),
            DocumentSource::Failure(failure) => match failure.partial_evidence().state() {
                PartialEvidenceState::None { .. } => None,
                PartialEvidenceState::Collector { collector, .. }
                | PartialEvidenceState::Generated { collector, .. }
                | PartialEvidenceState::Deduplicated { collector, .. }
                | PartialEvidenceState::Verified { collector, .. } => Some(collector),
            },
        }
    }

    fn generated(&self) -> Option<&Tier2GeneratedProposals> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.generated()),
            DocumentSource::Failure(failure) => match failure.partial_evidence().state() {
                PartialEvidenceState::Generated { generated, .. }
                | PartialEvidenceState::Deduplicated { generated, .. }
                | PartialEvidenceState::Verified { generated, .. } => Some(generated),
                PartialEvidenceState::None { .. } | PartialEvidenceState::Collector { .. } => None,
            },
        }
    }

    fn deduplication(&self) -> Option<&Tier2Deduplication> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.deduplication()),
            DocumentSource::Failure(failure) => match failure.partial_evidence().state() {
                PartialEvidenceState::Deduplicated { deduplication, .. }
                | PartialEvidenceState::Verified { deduplication, .. } => Some(deduplication),
                PartialEvidenceState::None { .. }
                | PartialEvidenceState::Collector { .. }
                | PartialEvidenceState::Generated { .. } => None,
            },
        }
    }

    fn verification(&self) -> Option<&Tier2VerifierVerdicts> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.verification()),
            DocumentSource::Failure(failure) => match failure.partial_evidence().state() {
                PartialEvidenceState::Verified { verification, .. } => Some(verification),
                PartialEvidenceState::None { .. }
                | PartialEvidenceState::Collector { .. }
                | PartialEvidenceState::Generated { .. }
                | PartialEvidenceState::Deduplicated { .. } => None,
            },
        }
    }
}

pub(super) fn render_tier2_evaluation_json(evaluation: &Tier2Evaluation) -> String {
    match evaluation {
        Tier2Evaluation::NotRun(not_run) => format!(
            "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_evaluation\",\"result\":\"not_run\",\"reason\":{}}}\n",
            text(not_run.reason())
        ),
        Tier2Evaluation::Complete(observation) => format!(
            "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_evaluation\",\"result\":\"complete\",\"exclusion_policy\":{},\"funnel\":{},\"ranked\":[{}]}}\n",
            exclusion_report_json(observation.exclusion_report()),
            funnel_json(observation.funnel()),
            observation.ranked().iter().map(ranked_json).collect::<Vec<_>>().join(",")
        ),
    }
}

fn exclusion_report_json(report: &super::Tier2ExclusionReport) -> String {
    let findings = report
        .pollution_findings()
        .iter()
        .map(|finding| {
            format!(
                "{{\"finding\":{},\"path\":{},\"excluded_component\":{}}}",
                text(finding.finding.as_str()),
                text(&finding.path),
                text(&finding.excluded_component),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"policy_digest\":{},\"excluded_path_count\":{},\"pollution_findings\":[{findings}]}}",
        text(report.policy_digest()),
        report.excluded_path_count(),
    )
}

fn collector_json(
    collector: &StrictCollectorEvidence,
    exclusion_report: Option<&Tier2ExclusionReport>,
) -> String {
    let domains = collector
        .domains
        .iter()
        .map(domain_json)
        .collect::<Vec<_>>()
        .join(",");
    let exclusion_policy = exclusion_report
        .map(exclusion_report_json)
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_collector\",\"collector_version\":{},\"canonical_repo_scope\":{},\"exclusion_policy\":{exclusion_policy},\"domains\":[{domains}]}}\n",
        text(&collector.collector_version),
        text(&collector.canonical_repo_scope),
    )
}

fn generated_json(
    generated: &Tier2GeneratedProposals,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let mut proposals = generated.proposals.clone();
    proposals.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_generated\",\"predecessor_digest\":{predecessor},\"generator_identity\":{},\"generator_protocol_version\":{},\"proposals\":[{}]}}\n",
        text(&generated.generator_identity),
        text(&generated.generator_protocol_version),
        proposals.iter().map(proposal_json).collect::<Vec<_>>().join(","),
    ))
}

fn deduplication_json(
    deduplication: &Tier2Deduplication,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let groups = deduplication
        .groups
        .iter()
        .map(dedup_json)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_dedup\",\"predecessor_digest\":{predecessor},\"normalization_version\":{TIER2_NORMALIZATION_VERSION},\"groups\":[{groups}]}}\n"
    ))
}

fn verification_json(
    verification: &Tier2VerifierVerdicts,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let mut verdicts = verification.verdicts.clone();
    verdicts.sort_by(|left, right| left.stable_key().cmp(right.stable_key()));
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_verification\",\"predecessor_digest\":{predecessor},\"verifier_identity\":{},\"verifier_protocol_version\":{},\"verdicts\":[{}]}}\n",
        text(&verification.verifier_identity),
        text(&verification.verifier_protocol_version),
        verdicts.iter().map(verdict_json).collect::<Vec<_>>().join(","),
    ))
}

fn roi_rank_json(
    observation: &Tier2Observation,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    let candidates = observation
        .roi()
        .iter()
        .map(roi_json)
        .collect::<Vec<_>>()
        .join(",");
    let ranked = observation
        .ranked()
        .iter()
        .map(ranked_json)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_roi_rank\",\"predecessor_digest\":{predecessor},\"rank_limit\":{TIER2_RANK_LIMIT},\"funnel\":{},\"candidates\":[{candidates}],\"ranked\":[{ranked}]}}\n",
        funnel_json(observation.funnel()),
    ))
}

fn failure_json(
    failure: &Tier2Failure,
    predecessor_digest: Option<&str>,
) -> Result<String, String> {
    let expected_predecessor = !matches!(
        failure.partial_evidence().state(),
        PartialEvidenceState::None { .. }
    );
    let predecessor = match (expected_predecessor, predecessor_digest) {
        (false, None) => "null".to_string(),
        (true, Some(value)) => digest(value)?,
        _ => return Err("failure predecessor digest does not match completed stages".to_string()),
    };
    Ok(format!(
        "{{\"schema\":{TIER2_SCHEMA},\"kind\":\"tier2_failure\",\"predecessor_digest\":{predecessor},\"stage\":{},\"code\":{},\"status_reason\":{},\"detail\":{},\"funnel\":{}}}\n",
        text(failure.stage().as_str()), text(failure.code().as_str()), text(&failure.status_reason()),
        text(failure.detail()), funnel_json(failure.partial_evidence().funnel()),
    ))
}

fn domain_json(domain: &crate::explore::specialists::DetectedDomain) -> String {
    format!(
        "{{\"name\":{},\"score\":{},\"evidence\":[{}]}}",
        text(&domain.name),
        domain.score,
        domain
            .evidence
            .iter()
            .map(evidence_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn proposal_json(proposal: &Tier2Proposal) -> String {
    format!(
        "{{\"stable_key\":{},\"title\":{},\"source\":{},\"evidence\":[{}],\"severity\":{},\"confidence_millis\":{},\"complexity\":{},\"named_consumer\":{}}}",
        text(&proposal.stable_key), text(&proposal.title), text(proposal.source.as_str()),
        proposal.evidence.iter().map(evidence_json).collect::<Vec<_>>().join(","),
        text(proposal.severity.as_str()), proposal.confidence_millis, text(proposal.complexity.as_str()), text(&proposal.named_consumer),
    )
}

fn evidence_json(evidence: &crate::explore::specialists::FileLineEvidence) -> String {
    format!(
        "{{\"file\":{},\"line\":{},\"match\":{}}}",
        text(&evidence.file),
        evidence.line,
        text(&evidence.r#match)
    )
}

fn dedup_json(group: &super::Tier2DeduplicationGroup) -> String {
    let candidates = group
        .candidate_keys
        .iter()
        .map(|key| text(key))
        .collect::<Vec<_>>()
        .join(",");
    let suppressed = group
        .suppressed_keys
        .iter()
        .map(|key| text(key))
        .collect::<Vec<_>>()
        .join(",");
    let scores = group
        .score_quotients
        .iter()
        .map(|score| {
            format!(
                "{{\"stable_key\":{},\"score_quotient\":{}}}",
                text(&score.stable_key),
                score.score_quotient
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"key\":{},\"candidate_keys\":[{candidates}],\"winner_key\":{},\"suppressed_keys\":[{suppressed}],\"score_quotients\":[{scores}]}}", text(&group.key), text(&group.winner_key))
}

fn verdict_json(verdict: &Tier2Verification) -> String {
    format!(
        "{{\"stable_key\":{},\"result\":{},\"reason\":{}}}",
        text(verdict.stable_key()),
        text(verdict.as_str()),
        text(verdict.reason())
    )
}

fn roi_json(decision: &super::Tier2RoiDecision) -> String {
    format!(
        "{{\"stable_key\":{},\"source\":{},\"permitted\":{},\"score_numerator\":{},\"complexity_units\":{},\"score_quotient\":{},\"severity_rank\":{},\"proposal\":{}}}",
        text(&decision.stable_key), text(decision.source.as_str()), decision.permitted,
        decision.score_numerator, decision.complexity_units, decision.score_quotient,
        decision.severity_rank, proposal_json(&decision.proposal),
    )
}

fn ranked_json(ranked: &Tier2RankedProposal) -> String {
    format!(
        "{{\"rank\":{},\"stable_key\":{},\"severity_rank\":{},\"score_numerator\":{},\"complexity_units\":{},\"score_quotient\":{},\"named_consumer\":{},\"proposal\":{}}}",
        ranked.rank, text(&ranked.stable_key), ranked.severity_rank, ranked.score_numerator,
        ranked.complexity_units, ranked.score_quotient, text(&ranked.named_consumer), proposal_json(&ranked.proposal),
    )
}

fn funnel_json(funnel: &crate::autonomous::waterfall::FunnelCounts) -> String {
    format!(
        "{{\"observed\":{},\"deduplicated\":{},\"verified\":{},\"roi_approved\":{},\"ranked\":{}}}",
        funnel.observed, funnel.deduplicated, funnel.verified, funnel.roi_approved, funnel.ranked
    )
}

fn digest(value: &str) -> Result<String, String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(text(value))
    } else {
        Err("predecessor digest must be a sealed lowercase SHA-256 value".to_string())
    }
}

fn text(value: &str) -> String {
    let mut rendered = String::from("\"");
    for character in value.chars() {
        match character {
            '\"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            character if character.is_control() => {
                rendered.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => rendered.push(character),
        }
    }
    rendered.push('\"');
    rendered
}
