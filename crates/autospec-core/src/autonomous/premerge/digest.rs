use sha2::{Digest, Sha256};

use super::{
    EvidenceAvailability, EvidenceVerdict, PremergeLaneIdentity, QaEvidence, SecurityAuditEvidence,
    QA_PRODUCER, SECURITY_AUDIT_PRODUCER,
};

const LANE_DIGEST_VERSION: &str = "autospec-premerge-lane-v1";
const EVIDENCE_DIGEST_VERSION: &str = "autospec-premerge-evidence-v1";

pub(super) fn lane_digest(lane: &PremergeLaneIdentity) -> String {
    hash_parts(&[
        LANE_DIGEST_VERSION.as_bytes(),
        lane.repo.as_bytes(),
        lane.issue.to_string().as_bytes(),
        lane.worker_id.as_bytes(),
        lane.claim_id.as_bytes(),
        lane.branch.as_bytes(),
        lane.commit.as_bytes(),
    ])
}

pub(super) fn evidence_digest(
    lane: &PremergeLaneIdentity,
    qa: &EvidenceAvailability<QaEvidence>,
    security: &EvidenceAvailability<SecurityAuditEvidence>,
) -> String {
    let qa_payload = qa_payload(qa);
    let security_payload = security_payload(security);
    hash_parts(&[
        EVIDENCE_DIGEST_VERSION.as_bytes(),
        lane_digest(lane).as_bytes(),
        &qa_payload,
        &security_payload,
    ])
}

fn qa_payload(availability: &EvidenceAvailability<QaEvidence>) -> Vec<u8> {
    match availability {
        EvidenceAvailability::Present(evidence) => canonical_evidence(
            "qa",
            QA_PRODUCER,
            &evidence.lane,
            &evidence.run_id,
            evidence.completed_at,
            &evidence.verdict,
        ),
        EvidenceAvailability::Missing => canonical_parts(&["missing"]),
        EvidenceAvailability::Malformed(error) => canonical_parts(&["malformed", error]),
    }
}

fn security_payload(availability: &EvidenceAvailability<SecurityAuditEvidence>) -> Vec<u8> {
    match availability {
        EvidenceAvailability::Present(evidence) => canonical_evidence(
            "security-audit",
            SECURITY_AUDIT_PRODUCER,
            &evidence.lane,
            &evidence.run_id,
            evidence.completed_at,
            &evidence.verdict,
        ),
        EvidenceAvailability::Missing => canonical_parts(&["missing"]),
        EvidenceAvailability::Malformed(error) => canonical_parts(&["malformed", error]),
    }
}

fn canonical_evidence(
    kind: &str,
    producer: &str,
    lane: &PremergeLaneIdentity,
    run_id: &str,
    completed_at: u64,
    verdict: &EvidenceVerdict,
) -> Vec<u8> {
    let issue = lane.issue.to_string();
    let completed_at = completed_at.to_string();
    let (verdict, finding_codes, reason) = verdict_fields(verdict);
    let codes = canonical_parts(&finding_codes);
    canonical_bytes(&[
        b"present",
        b"1",
        kind.as_bytes(),
        producer.as_bytes(),
        lane.repo.as_bytes(),
        issue.as_bytes(),
        lane.worker_id.as_bytes(),
        lane.claim_id.as_bytes(),
        lane.branch.as_bytes(),
        lane.commit.as_bytes(),
        run_id.as_bytes(),
        completed_at.as_bytes(),
        verdict.as_bytes(),
        &codes,
        reason.as_bytes(),
    ])
}

fn verdict_fields(verdict: &EvidenceVerdict) -> (&str, Vec<&str>, &str) {
    match verdict {
        EvidenceVerdict::Pass => ("pass", Vec::new(), ""),
        EvidenceVerdict::Blocked { finding_codes } => (
            "blocked",
            finding_codes.iter().map(String::as_str).collect(),
            "",
        ),
        EvidenceVerdict::Failed { reason } => ("failed", Vec::new(), reason),
    }
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
