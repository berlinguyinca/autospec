use autospec_core::autonomous::premerge::{
    evaluate_premerge, EvidenceAvailability, EvidenceVerdict, PremergeDecision,
    PremergeLaneIdentity, QaEvidence, SecurityAuditEvidence, QA_PRODUCER, SECURITY_AUDIT_PRODUCER,
};
use serde_json::{json, Value};

const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn lane(issue: u64, claim_id: &str, commit: &str) -> PremergeLaneIdentity {
    PremergeLaneIdentity::new(
        "test/repo",
        issue,
        format!("worker-{issue}"),
        claim_id,
        format!("autonomous/issue-{issue}"),
        commit,
    )
    .expect("valid lane")
}

fn evidence_document(
    kind: &str,
    producer: &str,
    lane: &PremergeLaneIdentity,
    verdict: &str,
    finding_codes: &[&str],
    reason: &str,
) -> String {
    json!({
        "schema": 1,
        "kind": kind,
        "producer": producer,
        "repo": lane.repo,
        "issue": lane.issue,
        "worker_id": lane.worker_id,
        "claim_id": lane.claim_id,
        "branch": lane.branch,
        "commit": lane.commit,
        "run_id": format!("{kind}-run-1"),
        "completed_at": 1_800_000_000_u64,
        "verdict": verdict,
        "finding_codes": finding_codes,
        "reason": reason,
    })
    .to_string()
}

fn qa(lane: &PremergeLaneIdentity, verdict: &str, codes: &[&str], reason: &str) -> QaEvidence {
    QaEvidence::parse(&evidence_document(
        "qa",
        QA_PRODUCER,
        lane,
        verdict,
        codes,
        reason,
    ))
    .expect("valid QA evidence")
}

fn security(
    lane: &PremergeLaneIdentity,
    verdict: &str,
    codes: &[&str],
    reason: &str,
) -> SecurityAuditEvidence {
    SecurityAuditEvidence::parse(&evidence_document(
        "security-audit",
        SECURITY_AUDIT_PRODUCER,
        lane,
        verdict,
        codes,
        reason,
    ))
    .expect("valid security evidence")
}

fn decision_digest(decision: &PremergeDecision) -> &str {
    match decision {
        PremergeDecision::Pass {
            evidence_digest, ..
        }
        | PremergeDecision::Blocked {
            evidence_digest, ..
        }
        | PremergeDecision::Failed {
            evidence_digest, ..
        } => evidence_digest,
    }
}

#[test]
fn missing_qa_fails_closed() {
    let lane = lane(1, "claim-1", COMMIT_A);

    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Missing,
        EvidenceAvailability::Present(security(&lane, "pass", &[], "")),
    );

    assert!(matches!(
        decision,
        PremergeDecision::Failed { reason, .. } if reason.contains("QA") && reason.contains("missing")
    ));
}

#[test]
fn missing_security_fails_closed() {
    let lane = lane(2, "claim-2", COMMIT_A);

    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Present(qa(&lane, "pass", &[], "")),
        EvidenceAvailability::Missing,
    );

    assert!(matches!(
        decision,
        PremergeDecision::Failed { reason, .. } if reason.contains("security") && reason.contains("missing")
    ));
}

#[test]
fn malformed_availability_fails_closed() {
    let lane = lane(3, "claim-3", COMMIT_A);

    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Malformed("not JSON".into()),
        EvidenceAvailability::Present(security(&lane, "pass", &[], "")),
    );

    assert!(matches!(
        decision,
        PremergeDecision::Failed { reason, .. } if reason.contains("malformed") && reason.contains("not JSON")
    ));
}

#[test]
fn invalid_typed_evidence_fails_closed() {
    let lane = lane(19, "claim-19", COMMIT_A);
    let mut invalid_qa = qa(&lane, "pass", &[], "");
    invalid_qa.run_id.clear();

    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Present(invalid_qa),
        EvidenceAvailability::Present(security(&lane, "pass", &[], "")),
    );

    assert!(matches!(
        decision,
        PremergeDecision::Failed { reason, .. } if reason.contains("invalid") && reason.contains("run_id")
    ));
}

#[test]
fn malformed_json_is_rejected() {
    assert!(QaEvidence::parse("{not-json}").is_err());
    assert!(SecurityAuditEvidence::parse("[]").is_err());
}

#[test]
fn unknown_schema_key_and_verdict_are_rejected() {
    let lane = lane(4, "claim-4", COMMIT_A);
    let valid = evidence_document("qa", QA_PRODUCER, &lane, "pass", &[], "");

    let mut unknown_schema: Value = serde_json::from_str(&valid).unwrap();
    unknown_schema["schema"] = json!(2);
    assert!(QaEvidence::parse(&unknown_schema.to_string()).is_err());

    let mut unknown_key: Value = serde_json::from_str(&valid).unwrap();
    unknown_key["extra"] = json!(true);
    assert!(QaEvidence::parse(&unknown_key.to_string()).is_err());

    let mut unknown_verdict: Value = serde_json::from_str(&valid).unwrap();
    unknown_verdict["verdict"] = json!("warning");
    assert!(QaEvidence::parse(&unknown_verdict.to_string()).is_err());
}

#[test]
fn fixed_kind_and_producer_pairs_are_enforced() {
    let lane = lane(5, "claim-5", COMMIT_A);
    assert!(QaEvidence::parse(&evidence_document(
        "qa",
        SECURITY_AUDIT_PRODUCER,
        &lane,
        "pass",
        &[],
        "",
    ))
    .is_err());
    assert!(SecurityAuditEvidence::parse(&evidence_document(
        "qa",
        SECURITY_AUDIT_PRODUCER,
        &lane,
        "pass",
        &[],
        "",
    ))
    .is_err());
}

#[test]
fn evidence_identity_must_match_every_expected_lane_field() {
    let expected = lane(6, "claim-6", COMMIT_A);
    let passing_qa = qa(&expected, "pass", &[], "");
    let passing_security = security(&expected, "pass", &[], "");
    let mismatches = [
        PremergeLaneIdentity::new(
            "other/repo",
            expected.issue,
            &expected.worker_id,
            &expected.claim_id,
            &expected.branch,
            &expected.commit,
        )
        .unwrap(),
        lane(7, "claim-6", COMMIT_A),
        PremergeLaneIdentity::new(
            &expected.repo,
            expected.issue,
            "other-worker",
            &expected.claim_id,
            &expected.branch,
            &expected.commit,
        )
        .unwrap(),
        lane(6, "other-claim", COMMIT_A),
        PremergeLaneIdentity::new(
            &expected.repo,
            expected.issue,
            &expected.worker_id,
            &expected.claim_id,
            "other/branch",
            &expected.commit,
        )
        .unwrap(),
        lane(6, "claim-6", COMMIT_B),
    ];

    for mismatched in mismatches {
        let decision = evaluate_premerge(
            &mismatched,
            EvidenceAvailability::Present(passing_qa.clone()),
            EvidenceAvailability::Present(passing_security.clone()),
        );
        assert!(matches!(decision, PremergeDecision::Failed { .. }));
    }
}

#[test]
fn two_pass_verdicts_admit_the_lane() {
    let lane = lane(8, "claim-8", COMMIT_A);
    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Present(qa(&lane, "pass", &[], "")),
        EvidenceAvailability::Present(security(&lane, "pass", &[], "")),
    );

    assert!(matches!(decision, PremergeDecision::Pass { lane: actual, .. } if actual == lane));
}

#[test]
fn explicit_failed_verdict_has_precedence_over_blocked() {
    let lane = lane(9, "claim-9", COMMIT_A);
    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Present(qa(&lane, "blocked", &["QA-9"], "")),
        EvidenceAvailability::Present(security(&lane, "failed", &[], "scanner crashed")),
    );

    assert!(matches!(
        decision,
        PremergeDecision::Failed { reason, .. } if reason.contains("scanner crashed")
    ));
}

#[test]
fn qa_blocked_quarantines_the_expected_lane() {
    let lane = lane(10, "claim-10", COMMIT_A);
    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Present(qa(&lane, "blocked", &["QA-10", "QA-11"], "")),
        EvidenceAvailability::Present(security(&lane, "pass", &[], "")),
    );

    assert!(matches!(
        decision,
        PremergeDecision::Blocked { quarantine, .. }
            if quarantine.lane == lane
                && quarantine.finding_codes == ["QA-10", "QA-11"]
                && !quarantine.evidence_digest.is_empty()
    ));
}

#[test]
fn security_blocked_quarantines_the_expected_lane() {
    let lane = lane(11, "claim-11", COMMIT_A);
    let decision = evaluate_premerge(
        &lane,
        EvidenceAvailability::Present(qa(&lane, "pass", &[], "")),
        EvidenceAvailability::Present(security(&lane, "blocked", &["SEC-11"], "")),
    );

    assert!(matches!(
        decision,
        PremergeDecision::Blocked { quarantine, .. }
            if quarantine.lane == lane && quarantine.finding_codes == ["SEC-11"]
    ));
}

#[test]
fn blocked_without_codes_and_failed_without_reason_are_rejected() {
    let lane = lane(12, "claim-12", COMMIT_A);
    assert!(QaEvidence::parse(&evidence_document(
        "qa",
        QA_PRODUCER,
        &lane,
        "blocked",
        &[],
        "",
    ))
    .is_err());
    assert!(SecurityAuditEvidence::parse(&evidence_document(
        "security-audit",
        SECURITY_AUDIT_PRODUCER,
        &lane,
        "failed",
        &[],
        "",
    ))
    .is_err());
}

#[test]
fn codecs_round_trip_all_verdicts() {
    let lane = lane(13, "claim-13", COMMIT_A);
    let qa_values = [
        qa(&lane, "pass", &[], ""),
        qa(&lane, "blocked", &["QA-13"], ""),
        qa(&lane, "failed", &[], "browser unavailable"),
    ];
    for evidence in qa_values {
        assert_eq!(QaEvidence::parse(&evidence.to_json()).unwrap(), evidence);
    }

    let security_values = [
        security(&lane, "pass", &[], ""),
        security(&lane, "blocked", &["SEC-13"], ""),
        security(&lane, "failed", &[], "scanner unavailable"),
    ];
    for evidence in security_values {
        assert_eq!(
            SecurityAuditEvidence::parse(&evidence.to_json()).unwrap(),
            evidence
        );
    }
}

#[test]
fn lane_and_evidence_digests_are_stable_and_identity_bound() {
    let expected = lane(14, "claim-14", COMMIT_A);
    assert_eq!(expected.lane_digest(), expected.clone().lane_digest());
    assert_ne!(
        expected.lane_digest(),
        lane(14, "claim-other", COMMIT_A).lane_digest()
    );
    assert_ne!(
        expected.lane_digest(),
        lane(14, "claim-14", COMMIT_B).lane_digest()
    );

    let first = evaluate_premerge(
        &expected,
        EvidenceAvailability::Present(qa(&expected, "pass", &[], "")),
        EvidenceAvailability::Present(security(&expected, "pass", &[], "")),
    );
    let repeated = evaluate_premerge(
        &expected,
        EvidenceAvailability::Present(qa(&expected, "pass", &[], "")),
        EvidenceAvailability::Present(security(&expected, "pass", &[], "")),
    );
    assert_eq!(decision_digest(&first), decision_digest(&repeated));

    let changed = evaluate_premerge(
        &expected,
        EvidenceAvailability::Present(qa(&expected, "blocked", &["QA-14"], "")),
        EvidenceAvailability::Present(security(&expected, "pass", &[], "")),
    );
    assert_ne!(decision_digest(&first), decision_digest(&changed));
}

#[test]
fn a_blocked_lane_does_not_affect_an_independent_passing_lane() {
    let lane_a = lane(15, "claim-15", COMMIT_A);
    let lane_b = lane(16, "claim-16", COMMIT_B);

    let blocked = evaluate_premerge(
        &lane_a,
        EvidenceAvailability::Present(qa(&lane_a, "blocked", &["QA-15"], "")),
        EvidenceAvailability::Present(security(&lane_a, "pass", &[], "")),
    );
    let passing = evaluate_premerge(
        &lane_b,
        EvidenceAvailability::Present(qa(&lane_b, "pass", &[], "")),
        EvidenceAvailability::Present(security(&lane_b, "pass", &[], "")),
    );

    assert!(matches!(blocked, PremergeDecision::Blocked { .. }));
    assert!(matches!(passing, PremergeDecision::Pass { lane, .. } if lane == lane_b));
}

#[test]
fn lane_and_document_scalar_constraints_fail_closed() {
    assert!(PremergeLaneIdentity::new("", 1, "worker", "claim", "branch", COMMIT_A).is_err());
    assert!(PremergeLaneIdentity::new("repo", 0, "worker", "claim", "branch", COMMIT_A).is_err());
    assert!(PremergeLaneIdentity::new("repo", 1, "worker", "claim", "branch", "ABC").is_err());

    let lane = lane(17, "claim-17", COMMIT_A);
    let mut invalid: Value = serde_json::from_str(&evidence_document(
        "qa",
        QA_PRODUCER,
        &lane,
        "pass",
        &[],
        "",
    ))
    .unwrap();
    invalid["completed_at"] = json!(0);
    assert!(QaEvidence::parse(&invalid.to_string()).is_err());

    invalid["completed_at"] = json!(1);
    invalid["run_id"] = json!("");
    assert!(QaEvidence::parse(&invalid.to_string()).is_err());
}

#[test]
fn parsed_verdict_uses_the_typed_representation() {
    let lane = lane(18, "claim-18", COMMIT_A);
    assert_eq!(qa(&lane, "pass", &[], "").verdict, EvidenceVerdict::Pass);
    assert_eq!(
        security(&lane, "blocked", &["SEC-18"], "").verdict,
        EvidenceVerdict::Blocked {
            finding_codes: vec!["SEC-18".into()]
        }
    );
}
