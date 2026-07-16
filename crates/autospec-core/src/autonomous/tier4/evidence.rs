use super::failure::Tier4Failure;
use super::model::{
    Tier4Candidate, Tier4Deduplication, Tier4DeduplicationGroup, Tier4GeneratedCandidates,
    Tier4Observation, Tier4RankedCandidate, Tier4RoiDecision, Tier4SourceEnvelope, Tier4SourceFact,
    Tier4SourcePolicy, Tier4Verification, Tier4VerifierVerdicts,
};
use super::{TIER4_RANK_LIMIT, TIER4_SCHEMA};

pub struct Tier4EvidenceDocuments<'a> {
    source: DocumentSource<'a>,
}

enum DocumentSource<'a> {
    Observation(&'a Tier4Observation),
    Failure(&'a Tier4Failure),
}

#[derive(Clone, Copy)]
enum EvidenceStage {
    SourcePolicy,
    Sources,
    Generated,
    Dedup,
    Verification,
    RoiRank,
}

impl EvidenceStage {
    fn predecessor(self) -> Option<Self> {
        match self {
            Self::SourcePolicy => None,
            Self::Sources => Some(Self::SourcePolicy),
            Self::Generated => Some(Self::Sources),
            Self::Dedup => Some(Self::Generated),
            Self::Verification => Some(Self::Dedup),
            Self::RoiRank => Some(Self::Verification),
        }
    }
}

impl<'a> Tier4EvidenceDocuments<'a> {
    pub(super) fn observation(observation: &'a Tier4Observation) -> Self {
        Self {
            source: DocumentSource::Observation(observation),
        }
    }

    pub(super) fn failure(failure: &'a Tier4Failure) -> Self {
        Self {
            source: DocumentSource::Failure(failure),
        }
    }

    pub fn source_policy_json(&self) -> Option<String> {
        self.document(EvidenceStage::SourcePolicy)
    }

    pub fn sources_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.checked_document(EvidenceStage::Sources, predecessor_digest)
    }

    pub fn generated_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.checked_document(EvidenceStage::Generated, predecessor_digest)
    }

    pub fn dedup_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.checked_document(EvidenceStage::Dedup, predecessor_digest)
    }

    pub fn verification_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.checked_document(EvidenceStage::Verification, predecessor_digest)
    }

    pub fn roi_rank_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.checked_document(EvidenceStage::RoiRank, predecessor_digest)
    }

    pub fn failure_json(&self, predecessor_digest: Option<&str>) -> Result<String, String> {
        let DocumentSource::Failure(failure) = self.source else {
            return Err("complete Tier 4 evidence does not render a failure document".to_string());
        };
        let predecessor = match (self.failure_predecessor_digest(), predecessor_digest) {
            (None, None) => "null".to_string(),
            (Some(expected), Some(actual)) if expected == actual => text(actual),
            _ => {
                return Err(
                    "failure predecessor digest does not match completed Tier 4 stages".to_string(),
                )
            }
        };
        Ok(format!(
            "{{\"schema\":{TIER4_SCHEMA},\"kind\":\"tier4_failure\",\"predecessor_digest\":{predecessor},\"stage\":{},\"code\":{},\"status_reason\":{},\"detail\":{},\"funnel\":{}}}\n",
            text(failure.stage().as_str()),
            text(failure.code().as_str()),
            text(&failure.status_reason()),
            text(failure.detail()),
            funnel_json(failure.partial_evidence().funnel()),
        ))
    }

    fn checked_document(
        &self,
        stage: EvidenceStage,
        predecessor_digest: &str,
    ) -> Result<Option<String>, String> {
        let Some(document) = self.document(stage) else {
            return Ok(None);
        };
        let expected = self
            .predecessor_digest(stage)
            .ok_or_else(|| "completed evidence stage has no predecessor document".to_string())?;
        if predecessor_digest == expected {
            Ok(Some(document))
        } else {
            Err(
                "predecessor digest does not match the immediately prior Tier 4 document"
                    .to_string(),
            )
        }
    }

    fn failure_predecessor_digest(&self) -> Option<String> {
        matches!(&self.source, DocumentSource::Failure(_))
            .then(|| {
                [
                    EvidenceStage::Verification,
                    EvidenceStage::Dedup,
                    EvidenceStage::Generated,
                    EvidenceStage::Sources,
                    EvidenceStage::SourcePolicy,
                ]
                .into_iter()
                .find_map(|stage| {
                    self.document(stage)
                        .map(|document| document_digest(&document))
                })
            })
            .flatten()
    }

    fn predecessor_digest(&self, stage: EvidenceStage) -> Option<String> {
        stage
            .predecessor()
            .and_then(|predecessor| self.document(predecessor))
            .map(|document| document_digest(&document))
    }

    fn document(&self, stage: EvidenceStage) -> Option<String> {
        match stage {
            EvidenceStage::SourcePolicy => self.source_policy().map(source_policy_json),
            EvidenceStage::Sources => Some(sources_json(
                self.sources()?,
                &self.predecessor_digest(stage)?,
            )),
            EvidenceStage::Generated => Some(generated_json(
                self.generated()?,
                &self.predecessor_digest(stage)?,
            )),
            EvidenceStage::Dedup => Some(dedup_json(
                self.deduplication()?,
                &self.predecessor_digest(stage)?,
            )),
            EvidenceStage::Verification => Some(verification_json(
                self.verification()?,
                &self.predecessor_digest(stage)?,
            )),
            EvidenceStage::RoiRank => match self.source {
                DocumentSource::Observation(observation) => {
                    Some(roi_rank_json(observation, &self.predecessor_digest(stage)?))
                }
                DocumentSource::Failure(_) => None,
            },
        }
    }

    fn source_policy(&self) -> Option<&Tier4SourcePolicy> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.source_policy()),
            DocumentSource::Failure(failure) => failure.partial_evidence().source_policy(),
        }
    }

    fn sources(&self) -> Option<&[Tier4SourceEnvelope]> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.sources()),
            DocumentSource::Failure(failure) => failure.partial_evidence().sources(),
        }
    }

    fn generated(&self) -> Option<&Tier4GeneratedCandidates> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.generated()),
            DocumentSource::Failure(failure) => failure.partial_evidence().generated(),
        }
    }

    fn deduplication(&self) -> Option<&Tier4Deduplication> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.deduplication()),
            DocumentSource::Failure(failure) => failure.partial_evidence().deduplication(),
        }
    }

    fn verification(&self) -> Option<&Tier4VerifierVerdicts> {
        match self.source {
            DocumentSource::Observation(observation) => Some(observation.verification()),
            DocumentSource::Failure(failure) => failure.partial_evidence().verification(),
        }
    }
}

fn source_policy_json(policy: &Tier4SourcePolicy) -> String {
    format!(
        "{{\"schema\":{TIER4_SCHEMA},\"kind\":\"tier4_source_policy\",\"policy_identity\":{},\"descriptors\":[{}]}}\n",
        text(&policy.policy_identity),
        policy
            .descriptors
            .iter()
            .map(descriptor_json)
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn sources_json(sources: &[Tier4SourceEnvelope], predecessor_digest: &str) -> String {
    format!(
        "{{\"schema\":{TIER4_SCHEMA},\"kind\":\"tier4_sources\",\"predecessor_digest\":{predecessor},\"sources\":[{}]}}\n",
        sources.iter().map(source_json).collect::<Vec<_>>().join(","),
        predecessor = text(predecessor_digest),
    )
}

fn generated_json(generated: &Tier4GeneratedCandidates, predecessor_digest: &str) -> String {
    let mut candidates = generated.candidates.clone();
    candidates.sort_by(|left, right| {
        left.stable_key
            .cmp(&right.stable_key)
            .then_with(|| left.source_id.cmp(&right.source_id))
            .then_with(|| left.fact_key.cmp(&right.fact_key))
    });
    format!(
        "{{\"schema\":{TIER4_SCHEMA},\"kind\":\"tier4_generated\",\"predecessor_digest\":{predecessor},\"generator_identity\":{},\"generator_protocol_version\":{},\"candidates\":[{}]}}\n",
        text(&generated.generator_identity),
        text(&generated.generator_protocol_version),
        candidates.iter().map(candidate_json).collect::<Vec<_>>().join(","),
        predecessor = text(predecessor_digest),
    )
}

fn dedup_json(deduplication: &Tier4Deduplication, predecessor_digest: &str) -> String {
    format!(
        "{{\"schema\":{TIER4_SCHEMA},\"kind\":\"tier4_dedup\",\"predecessor_digest\":{predecessor},\"groups\":[{}]}}\n",
        deduplication
            .groups
            .iter()
            .map(dedup_group_json)
            .collect::<Vec<_>>()
            .join(","),
        predecessor = text(predecessor_digest),
    )
}

fn verification_json(verification: &Tier4VerifierVerdicts, predecessor_digest: &str) -> String {
    let mut verdicts = verification.verdicts.clone();
    verdicts.sort_by(|left, right| left.stable_key().cmp(right.stable_key()));
    format!(
        "{{\"schema\":{TIER4_SCHEMA},\"kind\":\"tier4_verification\",\"predecessor_digest\":{predecessor},\"verifier_identity\":{},\"verifier_protocol_version\":{},\"verdicts\":[{}]}}\n",
        text(&verification.verifier_identity),
        text(&verification.verifier_protocol_version),
        verdicts.iter().map(verdict_json).collect::<Vec<_>>().join(","),
        predecessor = text(predecessor_digest),
    )
}

fn roi_rank_json(observation: &Tier4Observation, predecessor_digest: &str) -> String {
    format!(
        "{{\"schema\":{TIER4_SCHEMA},\"kind\":\"tier4_roi_rank\",\"predecessor_digest\":{predecessor},\"rank_limit\":{TIER4_RANK_LIMIT},\"terminal\":{},\"funnel\":{},\"candidates\":[{}],\"ranked\":[{}]}}\n",
        terminal_json(observation.terminal()),
        funnel_json(observation.funnel()),
        observation.roi().iter().map(roi_json).collect::<Vec<_>>().join(","),
        observation.ranked().iter().map(ranked_json).collect::<Vec<_>>().join(","),
        predecessor = text(predecessor_digest),
    )
}

fn descriptor_json(descriptor: &crate::autonomous::config::Tier4SourceDescriptor) -> String {
    format!(
        "{{\"id\":{},\"host\":{},\"path\":{},\"max_bytes\":{},\"deadline_millis\":{}}}",
        text(&descriptor.id),
        text(&descriptor.host),
        text(&descriptor.path),
        descriptor.max_bytes,
        descriptor.deadline_millis,
    )
}

fn source_json(source: &Tier4SourceEnvelope) -> String {
    format!(
        "{{\"schema_version\":{},\"producer_identity\":{},\"producer_protocol_version\":{},\"source_id\":{},\"byte_length\":{},\"body_sha256\":{},\"facts\":[{}]}}",
        source.schema_version,
        text(&source.producer_identity),
        text(&source.producer_protocol_version),
        text(&source.source_id),
        source.byte_length,
        text(&source.body_sha256),
        source.facts.iter().map(fact_json).collect::<Vec<_>>().join(","),
    )
}

fn fact_json(fact: &Tier4SourceFact) -> String {
    format!(
        "{{\"fact_key\":{},\"fact_type\":{},\"value\":{}}}",
        text(&fact.fact_key),
        text(&fact.fact_type),
        text(&fact.value),
    )
}

fn candidate_json(candidate: &Tier4Candidate) -> String {
    format!(
        "{{\"stable_key\":{},\"source_id\":{},\"fact_key\":{},\"title\":{},\"rationale\":{}}}",
        text(&candidate.stable_key),
        text(&candidate.source_id),
        text(&candidate.fact_key),
        text(&candidate.title),
        text(&candidate.rationale),
    )
}

fn dedup_group_json(group: &Tier4DeduplicationGroup) -> String {
    format!(
        "{{\"stable_key\":{},\"title\":{},\"rationale\":{},\"references\":[{}]}}",
        text(&group.stable_key),
        text(&group.title),
        text(&group.rationale),
        group
            .references
            .iter()
            .map(|reference| {
                format!(
                    "{{\"source_id\":{},\"fact_key\":{}}}",
                    text(&reference.source_id),
                    text(&reference.fact_key),
                )
            })
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn verdict_json(verdict: &Tier4Verification) -> String {
    match verdict {
        Tier4Verification::Accepted {
            stable_key,
            roi_millis,
            reason,
        } => format!(
            "{{\"stable_key\":{},\"result\":\"accepted\",\"roi_millis\":{},\"reason\":{}}}",
            text(stable_key),
            roi_millis,
            text(reason),
        ),
        Tier4Verification::Rejected { stable_key, reason } => format!(
            "{{\"stable_key\":{},\"result\":\"rejected\",\"reason\":{}}}",
            text(stable_key),
            text(reason),
        ),
    }
}

fn roi_json(decision: &Tier4RoiDecision) -> String {
    let roi_millis = decision
        .roi_millis
        .map_or_else(|| "null".to_string(), |value| value.to_string());
    format!(
        "{{\"stable_key\":{},\"verified\":{},\"roi_millis\":{roi_millis},\"permitted\":{},\"reason\":{}}}",
        text(&decision.stable_key),
        decision.verified,
        decision.permitted,
        text(&decision.reason),
    )
}

fn ranked_json(ranked: &Tier4RankedCandidate) -> String {
    format!(
        "{{\"rank\":{},\"stable_key\":{},\"roi_millis\":{},\"title\":{},\"rationale\":{},\"references\":[{}]}}",
        ranked.rank,
        text(&ranked.stable_key),
        ranked.roi_millis,
        text(&ranked.title),
        text(&ranked.rationale),
        ranked
            .references
            .iter()
            .map(|reference| {
                format!(
                    "{{\"source_id\":{},\"fact_key\":{}}}",
                    text(&reference.source_id),
                    text(&reference.fact_key),
                )
            })
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn terminal_json(terminal: &super::model::Tier4Terminal) -> String {
    match terminal {
        super::model::Tier4Terminal::Exhausted { reason } => {
            format!(
                "{{\"result\":\"exhausted\",\"reason\":{}}}",
                text(reason.as_str())
            )
        }
        super::model::Tier4Terminal::Produced { count } => {
            format!("{{\"result\":\"produced\",\"count\":{count}}}")
        }
    }
}

fn funnel_json(funnel: &crate::autonomous::waterfall::FunnelCounts) -> String {
    format!(
        "{{\"observed\":{},\"deduplicated\":{},\"verified\":{},\"roi_approved\":{},\"ranked\":{}}}",
        funnel.observed, funnel.deduplicated, funnel.verified, funnel.roi_approved, funnel.ranked
    )
}

fn document_digest(document: &str) -> String {
    crate::autonomous::waterfall::sha256_hex(document.as_bytes())
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
