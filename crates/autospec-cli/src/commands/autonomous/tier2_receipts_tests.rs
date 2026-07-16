use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier2::{
    evaluate_tier2, Tier2Complexity, Tier2Failure, Tier2GeneratedProposals, Tier2Input,
    Tier2Observation, Tier2Proposal, Tier2RoiPolicy, Tier2Severity, Tier2Source, Tier2StageResult,
    Tier2Verification, Tier2VerifierVerdicts, DISABLED_REASON,
};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};
use autospec_core::explore::specialists::{
    DetectedDomain, FileLineEvidence, StrictCollectorEvidence,
};

use super::tier2::Tier2Scan;
use super::tier2_receipts::{record_tier2, Tier2Progress};
use super::waterfall::{
    StoreAcquisition, Tier15EvidenceArtifact, Tier1EvidenceArtifact, WaterfallStore,
};

pub(super) const REPO: &str = "owner/repo";
static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct TempRoot(PathBuf);

impl TempRoot {
    pub(super) fn new() -> Self {
        let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time is after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("autospec-tier2-receipts-{nanos}-{sequence}"));
        fs::create_dir_all(&path).expect("temporary root");
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn store(root: &TempRoot) -> WaterfallStore {
    match WaterfallStore::acquire(root.path().join("waterfall"), REPO).expect("store acquisition") {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => panic!("fresh test root must be unlocked"),
    }
}

pub(super) fn seed_tier_two_cursor(root: &TempRoot) {
    let store = store(root);
    let tier_one_evidence = store
        .persist_tier1_evidence(
            1,
            Tier1EvidenceArtifact::ReadyPage,
            "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
        )
        .expect("Tier 1 evidence");
    let tier_one = sealed_receipt(
        NoWorkTier::Tier1,
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel"),
        vec![tier_one_evidence],
    );
    store.persist_receipt(&tier_one).expect("Tier 1 receipt");
    let state = WaterfallState::new(REPO, 1, NoWorkTier::Tier1)
        .expect("Tier 1 state")
        .record_receipt(&tier_one)
        .expect("Tier 1.5 cursor");
    let tier_fifteen_evidence = store
        .persist_tier15_evidence(
            1,
            Tier15EvidenceArtifact::Observation,
            "{\"schema\":1,\"kind\":\"tier15_observation\",\"open_observed\":0,\"open_deduplicated\":0,\"closed_observed\":0,\"budget\":0,\"decisions\":[]}\n",
        )
        .expect("Tier 1.5 evidence");
    let tier_fifteen = sealed_receipt(
        NoWorkTier::Tier1_5,
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel"),
        vec![tier_fifteen_evidence],
    );
    store
        .persist_receipt(&tier_fifteen)
        .expect("Tier 1.5 receipt");
    let state = state.record_receipt(&tier_fifteen).expect("Tier 2 cursor");
    store.persist_state(&state).expect("persist Tier 2 cursor");
}

fn sealed_receipt(
    tier: NoWorkTier,
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<SealedEvidence>,
) -> TierReceipt {
    let producer = match tier {
        NoWorkTier::Tier1 => "rust-foreground-tier1-v1",
        NoWorkTier::Tier1_5 => "rust-tier1_5-read-only-v1",
        _ => "tier2-test",
    };
    TierReceipt::new(REPO, 1, tier, producer, 1, 1, status, funnel, evidence)
        .expect("sealed receipt")
}

fn row() -> FileLineEvidence {
    FileLineEvidence {
        file: "Cargo.toml".to_string(),
        line: 1,
        r#match: "trading".to_string(),
    }
}

pub(super) fn collector() -> StrictCollectorEvidence {
    StrictCollectorEvidence {
        schema_version: 1,
        collector_version: "strict-local-v1".to_string(),
        canonical_repo_scope: "/repo".to_string(),
        domains: vec![DetectedDomain {
            name: "trading".to_string(),
            score: 1,
            evidence: vec![row()],
        }],
    }
}

pub(super) fn proposal(key: &str) -> Tier2Proposal {
    Tier2Proposal {
        stable_key: key.to_string(),
        title: format!("feat: {key}"),
        source: Tier2Source::StrictLocalSpecialist,
        evidence: vec![row()],
        severity: Tier2Severity::Medium,
        confidence_millis: 800,
        complexity: Tier2Complexity::Small,
        named_consumer: "maintainer".to_string(),
    }
}

pub(super) fn observation(
    proposals: Vec<Tier2Proposal>,
    verdicts: Vec<Tier2Verification>,
) -> Tier2Observation {
    let evaluation = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(Tier2GeneratedProposals {
            generator_identity: "test-generator".to_string(),
            generator_protocol_version: "v1".to_string(),
            proposals,
        }),
        verifier: Tier2StageResult::Complete(Tier2VerifierVerdicts {
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts,
        }),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect("complete Tier 2 evaluation");
    evaluation
        .observation()
        .cloned()
        .expect("complete observation")
}

pub(super) fn survives(key: &str) -> Tier2Verification {
    Tier2Verification::Survived {
        stable_key: key.to_string(),
        reason: "bounded evidence remains actionable".to_string(),
    }
}

fn refutes(key: &str) -> Tier2Verification {
    Tier2Verification::Refuted {
        stable_key: key.to_string(),
        reason: "evidence does not establish a gap".to_string(),
    }
}

pub(super) fn deduplicator_failure() -> Tier2Failure {
    let first = proposal("first");
    let mut second = proposal("second");
    second.title = first.title.clone();
    second.named_consumer = "different consumer".to_string();
    evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(Tier2GeneratedProposals {
            generator_identity: "test-generator".to_string(),
            generator_protocol_version: "v1".to_string(),
            proposals: vec![first, second],
        }),
        verifier: Tier2StageResult::Complete(Tier2VerifierVerdicts {
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: vec![survives("first")],
        }),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("conflicting duplicate must fail at deduplication")
}

#[test]
fn tier2_disabled_policy_seals_only_policy_and_retains_cursor() {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);

    assert_eq!(
        record_tier2(root.path(), REPO, Tier2Scan::NotRun).expect("disabled receipt"),
        Tier2Progress::NotRun(DISABLED_REASON.to_string())
    );
    let store = store(&root);
    let receipt = store
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt")
        .expect("sealed receipt");
    assert_eq!(
        receipt.evidence()[0].reference,
        "waterfall/1/tier2/policy.json"
    );
    assert_eq!(receipt.evidence().len(), 1);
    assert_eq!(
        fs::read_to_string(root.path().join("waterfall/waterfall/1/tier2/policy.json"))
            .expect("policy document"),
        format!(
            "{{\"schema\":1,\"kind\":\"tier2_policy\",\"mode\":\"disabled\",\"reason\":\"{DISABLED_REASON}\",\"policy_source\":\"checked_in\"}}\n"
        )
    );
    assert_eq!(
        store
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier2
    );
}

#[test]
fn tier2_exhausted_zero_and_refuted_observations_advance_to_tier_three() {
    for scan in [
        Tier2Scan::Complete(observation(Vec::new(), Vec::new())),
        Tier2Scan::Complete(observation(
            vec![proposal("refuted")],
            vec![refutes("refuted")],
        )),
    ] {
        let root = TempRoot::new();
        seed_tier_two_cursor(&root);
        assert_eq!(
            record_tier2(root.path(), REPO, scan).expect("exhausted receipt"),
            Tier2Progress::Advanced
        );
        assert_eq!(
            store(&root)
                .load_state()
                .expect("state")
                .expect("cursor")
                .current_tier(),
            NoWorkTier::Tier3
        );
    }
}

#[test]
fn tier2_produced_and_deduplicator_failed_results_replay_without_advancing() {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);
    let produced = Tier2Scan::Complete(observation(
        vec![proposal("produced")],
        vec![survives("produced")],
    ));
    assert_eq!(
        record_tier2(root.path(), REPO, produced).expect("produced receipt"),
        Tier2Progress::Produced(1)
    );
    assert_eq!(
        record_tier2(root.path(), REPO, Tier2Scan::NotRun).expect("replay"),
        Tier2Progress::Produced(1)
    );
    assert_eq!(
        store(&root)
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier2
    );

    let failure_root = TempRoot::new();
    seed_tier_two_cursor(&failure_root);
    assert_eq!(
        record_tier2(
            &failure_root.0,
            REPO,
            Tier2Scan::Failed(deduplicator_failure())
        )
        .expect("failed receipt"),
        Tier2Progress::Failed("tier2_deduplicator_duplicate_conflict".to_string())
    );
    let receipt = store(&failure_root)
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt")
        .expect("sealed receipt");
    assert_eq!(
        receipt
            .evidence()
            .iter()
            .map(|item| item.reference.as_str())
            .collect::<Vec<_>>(),
        vec![
            "waterfall/1/tier2/collector.json",
            "waterfall/1/tier2/generated.json",
            "waterfall/1/tier2/failure.json"
        ]
    );
}

#[test]
fn tier2_unsealed_failure_cannot_be_persisted_as_a_receipt() {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);
    let failure = Tier2Failure::new(
        autospec_core::autonomous::tier2::Tier2Stage::Collector,
        autospec_core::autonomous::tier2::Tier2FailureCode::ReadFile,
        "raw failure has no evaluated partial evidence",
    )
    .expect("raw failure");
    assert!(record_tier2(root.path(), REPO, Tier2Scan::Failed(failure)).is_err());
    assert!(store(&root)
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt lookup")
        .is_none());
}
