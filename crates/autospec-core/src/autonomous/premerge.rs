use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const QA_PRODUCER: &str = "autospec-qa";
pub const SECURITY_AUDIT_PRODUCER: &str = "autospec-secaudit";

const LANE_DIGEST_VERSION: &str = "autospec-premerge-lane-v1";
const EVIDENCE_DIGEST_VERSION: &str = "autospec-premerge-evidence-v1";
const MAX_IDENTIFIER_LENGTH: usize = 256;
const MAX_REASON_LENGTH: usize = 4_096;
const MAX_FINDING_CODE_LENGTH: usize = 128;
const MAX_FINDING_CODES: usize = 256;

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
        validate_lane(&lane)?;
        Ok(lane)
    }

    pub fn lane_digest(&self) -> String {
        hash_parts(&[
            LANE_DIGEST_VERSION.as_bytes(),
            self.repo.as_bytes(),
            self.issue.to_string().as_bytes(),
            self.worker_id.as_bytes(),
            self.claim_id.as_bytes(),
            self.branch.as_bytes(),
            self.commit.as_bytes(),
        ])
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityAuditEvidence {
    pub lane: PremergeLaneIdentity,
    pub run_id: String,
    pub completed_at: u64,
    pub verdict: EvidenceVerdict,
}

impl QaEvidence {
    pub fn parse(document: &str) -> Result<Self, String> {
        let raw = parse_raw(document, "qa", QA_PRODUCER)?;
        let lane = raw.lane()?;
        let verdict = raw.verdict()?;
        Ok(Self {
            lane,
            run_id: raw.run_id,
            completed_at: raw.completed_at,
            verdict,
        })
    }

    pub fn to_json(&self) -> String {
        RawEvidence::from_qa(self).to_json()
    }
}

impl SecurityAuditEvidence {
    pub fn parse(document: &str) -> Result<Self, String> {
        let raw = parse_raw(document, "security-audit", SECURITY_AUDIT_PRODUCER)?;
        let lane = raw.lane()?;
        let verdict = raw.verdict()?;
        Ok(Self {
            lane,
            run_id: raw.run_id,
            completed_at: raw.completed_at,
            verdict,
        })
    }

    pub fn to_json(&self) -> String {
        RawEvidence::from_security(self).to_json()
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
    let evidence_digest = evidence_digest(lane, &qa, &security);
    let failed = |reason: String| PremergeDecision::Failed {
        lane: lane.clone(),
        reason,
        evidence_digest: evidence_digest.clone(),
    };

    if let Err(error) = validate_lane(lane) {
        return failed(format!("invalid expected lane: {error}"));
    }
    if let Some(reason) = availability_failure("QA", &qa) {
        return failed(reason);
    }
    if let Some(reason) = availability_failure("security", &security) {
        return failed(reason);
    }

    let EvidenceAvailability::Present(qa) = qa else {
        unreachable!("availability failures returned above")
    };
    let EvidenceAvailability::Present(security) = security else {
        unreachable!("availability failures returned above")
    };

    if let Err(error) = validate_typed_evidence(&qa.lane, &qa.run_id, qa.completed_at, &qa.verdict)
    {
        return failed(format!("invalid QA evidence: {error}"));
    }
    if let Err(error) = validate_typed_evidence(
        &security.lane,
        &security.run_id,
        security.completed_at,
        &security.verdict,
    ) {
        return failed(format!("invalid security evidence: {error}"));
    }

    if qa.lane != *lane {
        return failed("QA evidence lane identity mismatch".into());
    }
    if security.lane != *lane {
        return failed("security evidence lane identity mismatch".into());
    }

    if let EvidenceVerdict::Failed { reason } = &qa.verdict {
        return failed(format!("QA evidence failed: {reason}"));
    }
    if let EvidenceVerdict::Failed { reason } = &security.verdict {
        return failed(format!("security evidence failed: {reason}"));
    }

    let mut finding_codes = Vec::new();
    if let EvidenceVerdict::Blocked {
        finding_codes: codes,
    } = &qa.verdict
    {
        finding_codes.extend(codes.iter().cloned());
    }
    if let EvidenceVerdict::Blocked {
        finding_codes: codes,
    } = &security.verdict
    {
        finding_codes.extend(codes.iter().cloned());
    }

    if !finding_codes.is_empty() {
        let quarantine = LaneQuarantine {
            lane: lane.clone(),
            evidence_digest: evidence_digest.clone(),
            finding_codes,
        };
        return PremergeDecision::Blocked {
            lane: lane.clone(),
            reason: "premerge evidence contains blocking findings".into(),
            evidence_digest,
            quarantine,
        };
    }

    PremergeDecision::Pass {
        lane: lane.clone(),
        evidence_digest,
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawEvidence {
    schema: u64,
    kind: String,
    producer: String,
    repo: String,
    issue: u64,
    worker_id: String,
    claim_id: String,
    branch: String,
    commit: String,
    run_id: String,
    completed_at: u64,
    verdict: String,
    finding_codes: Vec<String>,
    reason: String,
}

impl RawEvidence {
    fn lane(&self) -> Result<PremergeLaneIdentity, String> {
        PremergeLaneIdentity::new(
            &self.repo,
            self.issue,
            &self.worker_id,
            &self.claim_id,
            &self.branch,
            &self.commit,
        )
    }

    fn verdict(&self) -> Result<EvidenceVerdict, String> {
        validate_finding_codes(&self.finding_codes)?;
        match self.verdict.as_str() {
            "pass" if self.finding_codes.is_empty() && self.reason.is_empty() => {
                Ok(EvidenceVerdict::Pass)
            }
            "blocked" if !self.finding_codes.is_empty() && self.reason.is_empty() => {
                Ok(EvidenceVerdict::Blocked {
                    finding_codes: self.finding_codes.clone(),
                })
            }
            "failed" if self.finding_codes.is_empty() => {
                validate_bounded_nonempty("reason", &self.reason, MAX_REASON_LENGTH)?;
                Ok(EvidenceVerdict::Failed {
                    reason: self.reason.clone(),
                })
            }
            "pass" => Err("pass requires empty finding_codes and reason".into()),
            "blocked" => Err("blocked requires finding_codes and an empty reason".into()),
            "failed" => Err("failed requires no finding_codes and a nonempty reason".into()),
            _ => Err(format!("unknown evidence verdict: {}", self.verdict)),
        }
    }

    fn from_qa(evidence: &QaEvidence) -> Self {
        Self::from_parts(
            "qa",
            QA_PRODUCER,
            &evidence.lane,
            &evidence.run_id,
            evidence.completed_at,
            &evidence.verdict,
        )
    }

    fn from_security(evidence: &SecurityAuditEvidence) -> Self {
        Self::from_parts(
            "security-audit",
            SECURITY_AUDIT_PRODUCER,
            &evidence.lane,
            &evidence.run_id,
            evidence.completed_at,
            &evidence.verdict,
        )
    }

    fn from_parts(
        kind: &str,
        producer: &str,
        lane: &PremergeLaneIdentity,
        run_id: &str,
        completed_at: u64,
        verdict: &EvidenceVerdict,
    ) -> Self {
        let (verdict, finding_codes, reason) = match verdict {
            EvidenceVerdict::Pass => ("pass", Vec::new(), String::new()),
            EvidenceVerdict::Blocked { finding_codes } => {
                ("blocked", finding_codes.clone(), String::new())
            }
            EvidenceVerdict::Failed { reason } => ("failed", Vec::new(), reason.clone()),
        };
        Self {
            schema: 1,
            kind: kind.into(),
            producer: producer.into(),
            repo: lane.repo.clone(),
            issue: lane.issue,
            worker_id: lane.worker_id.clone(),
            claim_id: lane.claim_id.clone(),
            branch: lane.branch.clone(),
            commit: lane.commit.clone(),
            run_id: run_id.into(),
            completed_at,
            verdict: verdict.into(),
            finding_codes,
            reason,
        }
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("evidence fields always serialize")
    }
}

fn parse_raw(
    document: &str,
    expected_kind: &str,
    expected_producer: &str,
) -> Result<RawEvidence, String> {
    let raw: RawEvidence = serde_json::from_str(document)
        .map_err(|error| format!("malformed evidence JSON: {error}"))?;
    if raw.schema != 1 {
        return Err(format!("unsupported evidence schema: {}", raw.schema));
    }
    if raw.kind != expected_kind || raw.producer != expected_producer {
        return Err(format!(
            "expected kind/producer {expected_kind}/{expected_producer}, got {}/{}",
            raw.kind, raw.producer
        ));
    }
    raw.lane()?;
    validate_bounded_nonempty("run_id", &raw.run_id, MAX_IDENTIFIER_LENGTH)?;
    if raw.completed_at == 0 {
        return Err("completed_at must be positive".into());
    }
    raw.verdict()?;
    Ok(raw)
}

fn validate_lane(lane: &PremergeLaneIdentity) -> Result<(), String> {
    validate_bounded_nonempty("repo", &lane.repo, MAX_IDENTIFIER_LENGTH)?;
    if lane.issue == 0 {
        return Err("issue must be positive".into());
    }
    validate_bounded_nonempty("worker_id", &lane.worker_id, MAX_IDENTIFIER_LENGTH)?;
    validate_bounded_nonempty("claim_id", &lane.claim_id, MAX_IDENTIFIER_LENGTH)?;
    validate_bounded_nonempty("branch", &lane.branch, MAX_IDENTIFIER_LENGTH)?;
    if !matches!(lane.commit.len(), 40 | 64)
        || !lane
            .commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("commit must be 40 or 64 lowercase hexadecimal characters".into());
    }
    Ok(())
}

fn validate_finding_codes(codes: &[String]) -> Result<(), String> {
    if codes.len() > MAX_FINDING_CODES {
        return Err(format!("finding_codes exceeds {MAX_FINDING_CODES} entries"));
    }
    for code in codes {
        validate_bounded_nonempty("finding code", code, MAX_FINDING_CODE_LENGTH)?;
    }
    Ok(())
}

fn validate_typed_evidence(
    lane: &PremergeLaneIdentity,
    run_id: &str,
    completed_at: u64,
    verdict: &EvidenceVerdict,
) -> Result<(), String> {
    validate_lane(lane)?;
    validate_bounded_nonempty("run_id", run_id, MAX_IDENTIFIER_LENGTH)?;
    if completed_at == 0 {
        return Err("completed_at must be positive".into());
    }
    match verdict {
        EvidenceVerdict::Pass => Ok(()),
        EvidenceVerdict::Blocked { finding_codes } => {
            validate_finding_codes(finding_codes)?;
            if finding_codes.is_empty() {
                return Err("blocked requires finding_codes".into());
            }
            Ok(())
        }
        EvidenceVerdict::Failed { reason } => {
            validate_bounded_nonempty("reason", reason, MAX_REASON_LENGTH)
        }
    }
}

fn validate_bounded_nonempty(name: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{name} must be nonempty"));
    }
    if value.len() > maximum {
        return Err(format!("{name} exceeds {maximum} bytes"));
    }
    Ok(())
}

fn availability_failure<T>(name: &str, availability: &EvidenceAvailability<T>) -> Option<String> {
    match availability {
        EvidenceAvailability::Present(_) => None,
        EvidenceAvailability::Missing => Some(format!("{name} evidence missing")),
        EvidenceAvailability::Malformed(error) => {
            Some(format!("{name} evidence malformed: {error}"))
        }
    }
}

fn evidence_digest(
    lane: &PremergeLaneIdentity,
    qa: &EvidenceAvailability<QaEvidence>,
    security: &EvidenceAvailability<SecurityAuditEvidence>,
) -> String {
    let qa_payload = qa_payload(qa);
    let security_payload = security_payload(security);
    hash_parts(&[
        EVIDENCE_DIGEST_VERSION.as_bytes(),
        lane.lane_digest().as_bytes(),
        &qa_payload,
        &security_payload,
    ])
}

fn qa_payload(availability: &EvidenceAvailability<QaEvidence>) -> Vec<u8> {
    match availability {
        EvidenceAvailability::Present(evidence) => {
            canonical_evidence("present", &RawEvidence::from_qa(evidence))
        }
        EvidenceAvailability::Missing => canonical_parts(&["missing"]),
        EvidenceAvailability::Malformed(error) => canonical_parts(&["malformed", error]),
    }
}

fn security_payload(availability: &EvidenceAvailability<SecurityAuditEvidence>) -> Vec<u8> {
    match availability {
        EvidenceAvailability::Present(evidence) => {
            canonical_evidence("present", &RawEvidence::from_security(evidence))
        }
        EvidenceAvailability::Missing => canonical_parts(&["missing"]),
        EvidenceAvailability::Malformed(error) => canonical_parts(&["malformed", error]),
    }
}

fn canonical_evidence(availability: &str, evidence: &RawEvidence) -> Vec<u8> {
    let issue = evidence.issue.to_string();
    let completed_at = evidence.completed_at.to_string();
    let codes = canonical_parts(
        &evidence
            .finding_codes
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    canonical_bytes(&[
        availability.as_bytes(),
        evidence.schema.to_string().as_bytes(),
        evidence.kind.as_bytes(),
        evidence.producer.as_bytes(),
        evidence.repo.as_bytes(),
        issue.as_bytes(),
        evidence.worker_id.as_bytes(),
        evidence.claim_id.as_bytes(),
        evidence.branch.as_bytes(),
        evidence.commit.as_bytes(),
        evidence.run_id.as_bytes(),
        completed_at.as_bytes(),
        evidence.verdict.as_bytes(),
        &codes,
        evidence.reason.as_bytes(),
    ])
}

fn canonical_parts(parts: &[&str]) -> Vec<u8> {
    canonical_bytes(&parts.iter().map(|part| part.as_bytes()).collect::<Vec<_>>())
}

fn canonical_bytes(parts: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    for part in parts {
        output.extend_from_slice(&(part.len() as u64).to_be_bytes());
        output.extend_from_slice(part);
    }
    output
}

fn hash_parts(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(canonical_bytes(parts));
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
