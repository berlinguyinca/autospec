use serde::{Deserialize, Serialize};

use super::{
    EvidenceVerdict, PremergeLaneIdentity, QaEvidence, SecurityAuditEvidence, MAX_FINDING_CODES,
    MAX_FINDING_CODE_LENGTH, MAX_IDENTIFIER_LENGTH, MAX_REASON_LENGTH, QA_PRODUCER,
    SECURITY_AUDIT_PRODUCER,
};

pub(super) fn parse_qa(document: &str) -> Result<QaEvidence, String> {
    let raw = parse_raw(document, "qa", QA_PRODUCER)?;
    Ok(QaEvidence {
        lane: raw.lane()?,
        run_id: raw.run_id.clone(),
        completed_at: raw.completed_at,
        verdict: raw.verdict()?,
    })
}

pub(super) fn parse_security(document: &str) -> Result<SecurityAuditEvidence, String> {
    let raw = parse_raw(document, "security-audit", SECURITY_AUDIT_PRODUCER)?;
    Ok(SecurityAuditEvidence {
        lane: raw.lane()?,
        run_id: raw.run_id.clone(),
        completed_at: raw.completed_at,
        verdict: raw.verdict()?,
    })
}

pub(super) fn qa_to_json(evidence: &QaEvidence) -> String {
    RawEvidence::from_parts(
        "qa",
        QA_PRODUCER,
        &evidence.lane,
        &evidence.run_id,
        evidence.completed_at,
        &evidence.verdict,
    )
    .to_json()
}

pub(super) fn security_to_json(evidence: &SecurityAuditEvidence) -> String {
    RawEvidence::from_parts(
        "security-audit",
        SECURITY_AUDIT_PRODUCER,
        &evidence.lane,
        &evidence.run_id,
        evidence.completed_at,
        &evidence.verdict,
    )
    .to_json()
}

pub(super) fn validate_lane(lane: &PremergeLaneIdentity) -> Result<(), String> {
    validate_bounded_nonempty("repo", &lane.repo, MAX_IDENTIFIER_LENGTH)?;
    if lane.issue == 0 {
        return Err("issue must be positive".into());
    }
    validate_bounded_nonempty("worker_id", &lane.worker_id, MAX_IDENTIFIER_LENGTH)?;
    validate_bounded_nonempty("claim_id", &lane.claim_id, MAX_IDENTIFIER_LENGTH)?;
    validate_bounded_nonempty("branch", &lane.branch, MAX_IDENTIFIER_LENGTH)?;
    validate_commit(&lane.commit)
}

pub(super) fn validate_typed_evidence(
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
    validate_typed_verdict(verdict)
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
            "failed" if self.finding_codes.is_empty() => self.failed_verdict(),
            "pass" => Err("pass requires empty finding_codes and reason".into()),
            "blocked" => Err("blocked requires finding_codes and an empty reason".into()),
            "failed" => Err("failed requires no finding_codes and a nonempty reason".into()),
            _ => Err(format!("unknown evidence verdict: {}", self.verdict)),
        }
    }

    fn failed_verdict(&self) -> Result<EvidenceVerdict, String> {
        validate_bounded_nonempty("reason", &self.reason, MAX_REASON_LENGTH)?;
        Ok(EvidenceVerdict::Failed {
            reason: self.reason.clone(),
        })
    }

    fn from_parts(
        kind: &str,
        producer: &str,
        lane: &PremergeLaneIdentity,
        run_id: &str,
        completed_at: u64,
        verdict: &EvidenceVerdict,
    ) -> Self {
        let (verdict, finding_codes, reason) = verdict_fields(verdict);
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
    validate_producer(&raw, expected_kind, expected_producer)?;
    raw.lane()?;
    validate_bounded_nonempty("run_id", &raw.run_id, MAX_IDENTIFIER_LENGTH)?;
    if raw.completed_at == 0 {
        return Err("completed_at must be positive".into());
    }
    raw.verdict()?;
    Ok(raw)
}

fn validate_producer(
    raw: &RawEvidence,
    expected_kind: &str,
    expected_producer: &str,
) -> Result<(), String> {
    if raw.kind == expected_kind && raw.producer == expected_producer {
        return Ok(());
    }
    Err(format!(
        "expected kind/producer {expected_kind}/{expected_producer}, got {}/{}",
        raw.kind, raw.producer
    ))
}

fn validate_commit(commit: &str) -> Result<(), String> {
    let lowercase_hex = commit
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if matches!(commit.len(), 40 | 64) && lowercase_hex {
        return Ok(());
    }
    Err("commit must be 40 or 64 lowercase hexadecimal characters".into())
}

fn validate_typed_verdict(verdict: &EvidenceVerdict) -> Result<(), String> {
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

fn validate_finding_codes(codes: &[String]) -> Result<(), String> {
    if codes.len() > MAX_FINDING_CODES {
        return Err(format!("finding_codes exceeds {MAX_FINDING_CODES} entries"));
    }
    for code in codes {
        validate_bounded_nonempty("finding code", code, MAX_FINDING_CODE_LENGTH)?;
    }
    Ok(())
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

fn verdict_fields(verdict: &EvidenceVerdict) -> (&str, Vec<String>, String) {
    match verdict {
        EvidenceVerdict::Pass => ("pass", Vec::new(), String::new()),
        EvidenceVerdict::Blocked { finding_codes } => {
            ("blocked", finding_codes.clone(), String::new())
        }
        EvidenceVerdict::Failed { reason } => ("failed", Vec::new(), reason.clone()),
    }
}
