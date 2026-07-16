use std::fs;

use autospec_core::autonomous::config::Tier4SourceDescriptor;
use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier3::DISABLED_REASON as TIER3_DISABLED_REASON;
use autospec_core::autonomous::tier4::{
    evaluate_tier4, Tier4Candidate, Tier4GeneratedCandidates, Tier4Input, Tier4Observation,
    Tier4RoiPolicy, Tier4SourceEnvelope, Tier4SourceFact, Tier4SourcePolicy, Tier4StageResult,
    Tier4Verification, Tier4VerifierVerdicts, DISABLED_REASON, TIER4_SCHEMA,
};
use autospec_core::autonomous::waterfall::{FunnelCounts, TierReceipt, TierStatus};

use super::tier2_receipts_tests::{store, TempRoot, REPO};
use super::tier3::Tier3Scan;
use super::tier3_receipts::{record_tier3, Tier3Progress};
use super::tier3_receipts_tests::{observation as tier3_observation, seed_tier_three_cursor};
use super::tier4::{disabled_by_checked_in_policy, Tier4Scan};
use super::tier4_receipts::{record_tier4, record_tier4_with_source_policy, Tier4Progress};
use super::waterfall::{StoreAcquisition, WaterfallStore};

pub(super) fn seed_tier_four_cursor(root: &TempRoot) {
    seed_tier_three_cursor(root);
    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(tier3_observation(Vec::new()))
        )
        .expect("Tier 3 dry receipt"),
        Tier3Progress::Advanced
    );
}

fn descriptor_for(id: &str) -> Tier4SourceDescriptor {
    Tier4SourceDescriptor {
        id: id.to_string(),
        host: format!("{id}.example.test"),
        path: "/facts".to_string(),
        max_bytes: 1_024,
        deadline_millis: 1_000,
    }
}

pub(super) fn expected_source_policy() -> Tier4SourcePolicy {
    Tier4SourcePolicy {
        schema_version: TIER4_SCHEMA,
        policy_identity: "checked-in-policy-v1".to_string(),
        descriptors: vec![descriptor_for("alpha")],
    }
}

pub(super) fn record_tier4_with_expected_policy(
    root: &TempRoot,
    scan: Tier4Scan,
) -> Result<Tier4Progress, String> {
    record_tier4_with_source_policy(root.path(), REPO, scan, expected_source_policy())
}

pub(super) fn tier4_store(root: &TempRoot) -> WaterfallStore {
    match WaterfallStore::acquire_with_tier4_source_policy(
        root.path().join("waterfall"),
        REPO,
        expected_source_policy(),
    )
    .expect("store acquisition")
    {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => panic!("fresh test root must be unlocked"),
    }
}

fn alternate_source_policy() -> Tier4SourcePolicy {
    Tier4SourcePolicy {
        schema_version: TIER4_SCHEMA,
        policy_identity: "alternate-checked-in-policy-v1".to_string(),
        descriptors: vec![descriptor_for("beta")],
    }
}

pub(super) fn source(facts: &[&str]) -> Tier4SourceEnvelope {
    source_for("alpha", facts)
}

fn source_for(id: &str, facts: &[&str]) -> Tier4SourceEnvelope {
    Tier4SourceEnvelope {
        schema_version: TIER4_SCHEMA,
        producer_identity: "test-source-adapter".to_string(),
        producer_protocol_version: "v1".to_string(),
        source_id: id.to_string(),
        byte_length: 128,
        body_sha256: "a".repeat(64),
        facts: facts
            .iter()
            .map(|fact_key| Tier4SourceFact {
                fact_key: (*fact_key).to_string(),
                fact_type: "release".to_string(),
                value: format!("typed fact for {fact_key}"),
            })
            .collect(),
    }
}

fn candidate() -> Tier4Candidate {
    Tier4Candidate {
        stable_key: "candidate-alpha".to_string(),
        source_id: "alpha".to_string(),
        fact_key: "fact-alpha".to_string(),
        title: "Investigate candidate alpha".to_string(),
        rationale: "typed evidence supports the candidate".to_string(),
    }
}

pub(super) fn observation(
    source: Tier4SourceEnvelope,
    candidates: Vec<Tier4Candidate>,
    verdicts: Vec<Tier4Verification>,
) -> Tier4Observation {
    evaluate_tier4(Tier4Input::Enabled {
        source_policy: expected_source_policy(),
        sources: vec![Tier4StageResult::Complete(source)],
        generated: Tier4StageResult::Complete(Tier4GeneratedCandidates {
            schema_version: TIER4_SCHEMA,
            generator_identity: "test-generator".to_string(),
            generator_protocol_version: "v1".to_string(),
            candidates,
        }),
        verifier: Tier4StageResult::Complete(Tier4VerifierVerdicts {
            schema_version: TIER4_SCHEMA,
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts,
        }),
        roi_policy: Tier4RoiPolicy::v1(),
    })
    .expect("valid Tier 4 input")
    .observation()
    .cloned()
    .expect("complete observation")
}

fn alternate_valid_dry_observation() -> Tier4Observation {
    evaluate_tier4(Tier4Input::Enabled {
        source_policy: alternate_source_policy(),
        sources: vec![Tier4StageResult::Complete(source_for("beta", &[]))],
        generated: Tier4StageResult::Complete(Tier4GeneratedCandidates {
            schema_version: TIER4_SCHEMA,
            generator_identity: "test-generator".to_string(),
            generator_protocol_version: "v1".to_string(),
            candidates: Vec::new(),
        }),
        verifier: Tier4StageResult::Complete(Tier4VerifierVerdicts {
            schema_version: TIER4_SCHEMA,
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: Vec::new(),
        }),
        roi_policy: Tier4RoiPolicy::v1(),
    })
    .expect("alternate valid Tier 4 input")
    .observation()
    .cloned()
    .expect("alternate complete observation")
}

fn rejected() -> Tier4Verification {
    Tier4Verification::Rejected {
        stable_key: "candidate-alpha".to_string(),
        reason: "not actionable".to_string(),
    }
}

fn accepted(roi_millis: u16) -> Tier4Verification {
    Tier4Verification::Accepted {
        stable_key: "candidate-alpha".to_string(),
        roi_millis,
        reason: "verified against typed source evidence".to_string(),
    }
}

pub(super) fn produced_observation() -> Tier4Observation {
    observation(
        source(&["fact-alpha"]),
        vec![candidate()],
        vec![accepted(900)],
    )
}

pub(super) fn generator_failure() -> autospec_core::autonomous::tier4::Tier4Failure {
    let mut invalid = candidate();
    invalid.fact_key = "missing-fact".to_string();
    evaluate_tier4(Tier4Input::Enabled {
        source_policy: expected_source_policy(),
        sources: vec![Tier4StageResult::Complete(source(&["fact-alpha"]))],
        generated: Tier4StageResult::Complete(Tier4GeneratedCandidates {
            schema_version: TIER4_SCHEMA,
            generator_identity: "test-generator".to_string(),
            generator_protocol_version: "v1".to_string(),
            candidates: vec![invalid],
        }),
        verifier: Tier4StageResult::Complete(Tier4VerifierVerdicts {
            schema_version: TIER4_SCHEMA,
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: Vec::new(),
        }),
        roi_policy: Tier4RoiPolicy::v1(),
    })
    .expect_err("unknown source fact must fail at generation")
}

#[test]
fn fully_sealed_alternate_policy_dry_evidence_is_rejected_by_trusted_policy() {
    let untrusted_root = TempRoot::new();
    seed_tier_four_cursor(&untrusted_root);
    assert!(record_tier4(
        untrusted_root.path(),
        REPO,
        Tier4Scan::Complete(observation(source(&[]), Vec::new(), Vec::new())),
    )
    .is_err());

    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4_with_source_policy(
            root.path(),
            REPO,
            Tier4Scan::Complete(alternate_valid_dry_observation()),
            alternate_source_policy(),
        )
        .expect("alternate-policy dry receipt"),
        Tier4Progress::Advanced
    );
    let alternate_store = store(&root);
    assert_eq!(
        alternate_store
            .load_receipt(1, NoWorkTier::Tier4)
            .expect("alternate receipt")
            .expect("sealed alternate receipt")
            .evidence()
            .len(),
        6,
        "the alternate policy fixture seals and rehashes the complete dry chain"
    );
    assert!(
        alternate_store.load_state().is_err(),
        "retained completed Tier 4 history must not replay without trusted policy"
    );
    drop(alternate_store);
    let mismatched_store = match WaterfallStore::acquire_with_tier4_source_policy(
        root.path().join("waterfall"),
        REPO,
        expected_source_policy(),
    )
    .expect("store acquisition")
    {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => panic!("fresh test root must be unlocked"),
    };
    assert!(mismatched_store.load_state().is_err());
}

#[test]
fn tier4_disabled_policy_seals_only_checked_in_policy_and_retains_cursor() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);

    assert_eq!(
        record_tier4(root.path(), REPO, disabled_by_checked_in_policy())
            .expect("disabled Tier 4 receipt"),
        Tier4Progress::NotRun(DISABLED_REASON.to_string())
    );
    let store = store(&root);
    let receipt = store
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt")
        .expect("sealed receipt");
    assert_eq!(receipt.producer_version(), "rust-tier4-disabled-policy-v1");
    assert!(matches!(
        receipt.status(),
        TierStatus::NotRun { reason } if reason == DISABLED_REASON
    ));
    assert_eq!(receipt.evidence().len(), 1);
    assert_eq!(
        receipt.evidence()[0].reference,
        "waterfall/1/tier4/policy.json"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("waterfall/waterfall/1/tier4/policy.json"))
            .expect("policy document"),
        format!(
            "{{\"schema\":1,\"kind\":\"tier4_policy\",\"mode\":\"disabled\",\"reason\":\"{DISABLED_REASON}\",\"policy_source\":\"checked_in\"}}\n"
        )
    );
    assert!(receipt.evidence().iter().all(|evidence| {
        !evidence.reference.contains("source_policy")
            && !evidence.reference.contains("sources")
            && !evidence.reference.contains("generated")
            && !evidence.reference.contains("dedup")
            && !evidence.reference.contains("verification")
            && !evidence.reference.contains("roi_rank")
            && !evidence.reference.contains("failure")
    }));
    assert_eq!(
        store
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier4
    );
    assert_ne!(TIER3_DISABLED_REASON, DISABLED_REASON);
}

#[test]
fn tier4_closed_dry_results_roll_over_with_full_receipt_audit_history() {
    let scans = [
        Tier4Scan::Complete(observation(source(&[]), Vec::new(), Vec::new())),
        Tier4Scan::Complete(observation(
            source(&["fact-alpha"]),
            vec![candidate()],
            vec![rejected()],
        )),
        Tier4Scan::Complete(observation(
            source(&["fact-alpha"]),
            vec![candidate()],
            vec![accepted(499)],
        )),
    ];
    for scan in scans {
        let root = TempRoot::new();
        seed_tier_four_cursor(&root);
        assert_eq!(
            record_tier4_with_expected_policy(&root, scan).expect("Tier 4 dry receipt"),
            Tier4Progress::Advanced
        );
        let store = tier4_store(&root);
        let state = store.load_state().expect("state").expect("cursor");
        assert_eq!(
            (state.next_pass_id(), state.current_tier()),
            (2, NoWorkTier::Tier1)
        );
        assert_eq!(
            state
                .completed_receipts()
                .iter()
                .map(|receipt| (&receipt.tier, receipt.reference.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (&NoWorkTier::Tier1, "waterfall/1/tier1.json"),
                (&NoWorkTier::Tier1_5, "waterfall/1/tier1_5.json"),
                (&NoWorkTier::Tier2, "waterfall/1/tier2.json"),
                (&NoWorkTier::Tier3, "waterfall/1/tier3.json"),
                (&NoWorkTier::Tier4, "waterfall/1/tier4.json"),
            ]
        );
        let receipt = store
            .load_receipt(1, NoWorkTier::Tier4)
            .expect("Tier 4 receipt")
            .expect("sealed Tier 4 receipt");
        assert_eq!(
            receipt
                .evidence()
                .iter()
                .map(|item| item.reference.as_str())
                .collect::<Vec<_>>(),
            vec![
                "waterfall/1/tier4/source_policy.json",
                "waterfall/1/tier4/sources.json",
                "waterfall/1/tier4/generated.json",
                "waterfall/1/tier4/dedup.json",
                "waterfall/1/tier4/verification.json",
                "waterfall/1/tier4/roi_rank.json",
            ]
        );
        assert!(matches!(
            receipt.status(),
            TierStatus::Exhausted {
                reason: DryReason::NoProposalsGenerated
                    | DryReason::VerificationRejected
                    | DryReason::RoiFiltered,
            }
        ));

        let tier_one_evidence = store
            .persist_tier1_evidence(
                2,
                super::waterfall::Tier1EvidenceArtifact::ReadyPage,
                "{\"schema\":1,\"kind\":\"ready_page\"}\n",
            )
            .expect("pass two Tier 1 evidence");
        let tier_one = TierReceipt::new(
            REPO,
            2,
            NoWorkTier::Tier1,
            "test-tier1-receipt",
            1,
            1,
            TierStatus::Exhausted {
                reason: DryReason::NoProposalsGenerated,
            },
            FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel"),
            vec![tier_one_evidence],
        )
        .expect("Tier 1 receipt");
        store
            .persist_receipt(&tier_one)
            .expect("persist Tier 1 receipt");
        let next = state
            .record_receipt(&tier_one)
            .expect("advance Tier 1 cursor");
        store.persist_state(&next).expect("persist pass two state");
        assert_eq!(next.completed_receipts().len(), 1);
        assert_eq!(next.completed_receipts()[0].tier, NoWorkTier::Tier1);
    }
}
