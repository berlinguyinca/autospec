use super::{
    codec, digest, EvidenceAvailability, EvidenceVerdict, LaneQuarantine, PremergeDecision,
    PremergeLaneIdentity, QaEvidence, SecurityAuditEvidence,
};

pub(super) fn evaluate(
    lane: &PremergeLaneIdentity,
    qa: EvidenceAvailability<QaEvidence>,
    security: EvidenceAvailability<SecurityAuditEvidence>,
) -> PremergeDecision {
    let evidence_digest = digest::evidence_digest(lane, &qa, &security);
    if let Err(error) = codec::validate_lane(lane) {
        return failed(
            lane,
            evidence_digest,
            format!("invalid expected lane: {error}"),
        );
    }
    let (qa, security) = match require_valid_evidence(qa, security) {
        Ok(evidence) => evidence,
        Err(reason) => return failed(lane, evidence_digest, reason),
    };
    if qa.lane != *lane {
        return failed(
            lane,
            evidence_digest,
            "QA evidence lane identity mismatch".into(),
        );
    }
    if security.lane != *lane {
        return failed(
            lane,
            evidence_digest,
            "security evidence lane identity mismatch".into(),
        );
    }
    decide_verdicts(lane, evidence_digest, qa.verdict, security.verdict)
}

fn require_valid_evidence(
    qa: EvidenceAvailability<QaEvidence>,
    security: EvidenceAvailability<SecurityAuditEvidence>,
) -> Result<(QaEvidence, SecurityAuditEvidence), String> {
    let qa = require_present("QA", qa)?;
    let security = require_present("security", security)?;
    codec::validate_typed_evidence(&qa.lane, &qa.run_id, qa.completed_at, &qa.verdict)
        .map_err(|error| format!("invalid QA evidence: {error}"))?;
    codec::validate_typed_evidence(
        &security.lane,
        &security.run_id,
        security.completed_at,
        &security.verdict,
    )
    .map_err(|error| format!("invalid security evidence: {error}"))?;
    Ok((qa, security))
}

fn require_present<T>(name: &str, availability: EvidenceAvailability<T>) -> Result<T, String> {
    match availability {
        EvidenceAvailability::Present(evidence) => Ok(evidence),
        EvidenceAvailability::Missing => Err(format!("{name} evidence missing")),
        EvidenceAvailability::Malformed(error) => {
            Err(format!("{name} evidence malformed: {error}"))
        }
    }
}

fn decide_verdicts(
    lane: &PremergeLaneIdentity,
    evidence_digest: String,
    qa: EvidenceVerdict,
    security: EvidenceVerdict,
) -> PremergeDecision {
    if let EvidenceVerdict::Failed { reason } = qa {
        return failed(
            lane,
            evidence_digest,
            format!("QA evidence failed: {reason}"),
        );
    }
    if let EvidenceVerdict::Failed { reason } = security {
        return failed(
            lane,
            evidence_digest,
            format!("security evidence failed: {reason}"),
        );
    }
    let finding_codes = blocked_codes(qa, security);
    if finding_codes.is_empty() {
        return PremergeDecision::Pass {
            lane: lane.clone(),
            evidence_digest,
        };
    }
    blocked(lane, evidence_digest, finding_codes)
}

fn blocked_codes(qa: EvidenceVerdict, security: EvidenceVerdict) -> Vec<String> {
    let mut codes = Vec::new();
    if let EvidenceVerdict::Blocked { finding_codes } = qa {
        codes.extend(finding_codes);
    }
    if let EvidenceVerdict::Blocked { finding_codes } = security {
        codes.extend(finding_codes);
    }
    codes
}

fn blocked(
    lane: &PremergeLaneIdentity,
    evidence_digest: String,
    finding_codes: Vec<String>,
) -> PremergeDecision {
    let quarantine = LaneQuarantine {
        lane: lane.clone(),
        evidence_digest: evidence_digest.clone(),
        finding_codes,
    };
    PremergeDecision::Blocked {
        lane: lane.clone(),
        reason: "premerge evidence contains blocking findings".into(),
        evidence_digest,
        quarantine,
    }
}

fn failed(
    lane: &PremergeLaneIdentity,
    evidence_digest: String,
    reason: String,
) -> PremergeDecision {
    PremergeDecision::Failed {
        lane: lane.clone(),
        reason,
        evidence_digest,
    }
}
