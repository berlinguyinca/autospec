mod codec;
mod decision;
mod digest;

pub const QA_PRODUCER: &str = "autospec-qa";
pub const SECURITY_AUDIT_PRODUCER: &str = "autospec-secaudit";

pub(super) const MAX_IDENTIFIER_LENGTH: usize = 256;
pub(super) const MAX_REASON_LENGTH: usize = 4_096;
pub(super) const MAX_FINDING_CODE_LENGTH: usize = 128;
pub(super) const MAX_FINDING_CODES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PremergeLaneIdentity {
    pub repo: String,
    pub issue: u64,
    pub worker_id: String,
    pub claim_id: String,
    pub branch: String,
    pub commit: String,
}

impl PremergeLaneIdentity {
    pub fn new(
        repo: impl Into<String>,
        issue: u64,
        worker_id: impl Into<String>,
        claim_id: impl Into<String>,
        branch: impl Into<String>,
        commit: impl Into<String>,
    ) -> Result<Self, String> {
        let lane = Self {
            repo: repo.into(),
            issue,
            worker_id: worker_id.into(),
            claim_id: claim_id.into(),
            branch: branch.into(),
            commit: commit.into(),
        };
        codec::validate_lane(&lane)?;
        Ok(lane)
    }

    pub fn lane_digest(&self) -> String {
        digest::lane_digest(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PremergeDecisionKind {
    Pass,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PremergeDecisionReceipt {
    pub decision: PremergeDecisionKind,
    pub lane: PremergeLaneIdentity,
    pub lane_digest: String,
    pub evidence_digest: String,
}

impl PremergeDecisionReceipt {
    pub fn parse(document: &str) -> Result<Self, String> {
        codec::parse_decision_receipt(document)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceVerdict {
    Pass,
    Blocked { finding_codes: Vec<String> },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaEvidence {
    pub lane: PremergeLaneIdentity,
    pub run_id: String,
    pub completed_at: u64,
    pub verdict: EvidenceVerdict,
}

impl QaEvidence {
    pub fn parse(document: &str) -> Result<Self, String> {
        codec::parse_qa(document)
    }

    pub fn to_json(&self) -> String {
        codec::qa_to_json(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityAuditEvidence {
    pub lane: PremergeLaneIdentity,
    pub run_id: String,
    pub completed_at: u64,
    pub verdict: EvidenceVerdict,
}

impl SecurityAuditEvidence {
    pub fn parse(document: &str) -> Result<Self, String> {
        codec::parse_security(document)
    }

    pub fn to_json(&self) -> String {
        codec::security_to_json(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceAvailability<T> {
    Present(T),
    Missing,
    Malformed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneQuarantine {
    pub lane: PremergeLaneIdentity,
    pub evidence_digest: String,
    pub finding_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PremergeDecision {
    Pass {
        lane: PremergeLaneIdentity,
        evidence_digest: String,
    },
    Blocked {
        lane: PremergeLaneIdentity,
        reason: String,
        evidence_digest: String,
        quarantine: LaneQuarantine,
    },
    Failed {
        lane: PremergeLaneIdentity,
        reason: String,
        evidence_digest: String,
    },
}

pub fn evaluate_premerge(
    lane: &PremergeLaneIdentity,
    qa: EvidenceAvailability<QaEvidence>,
    security: EvidenceAvailability<SecurityAuditEvidence>,
) -> PremergeDecision {
    decision::evaluate(lane, qa, security)
}
