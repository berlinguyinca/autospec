use std::fs;

use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier4::{
    evaluate_tier4, Tier4GeneratedCandidates, Tier4Input, Tier4RoiPolicy, Tier4StageResult,
    Tier4VerifierVerdicts, TIER4_SCHEMA,
};
use autospec_core::autonomous::waterfall::{FunnelCounts, TierReceipt, TierStatus, WaterfallState};
use autospec_core::coordination::{QueueGateCounts, ReadyQueuePlan, WorkerCap};

use super::resilience::{acquire_test_lifecycle, replace_test_lifecycle_generation};
use super::tier15::Tier15Scan;
use super::tier15_receipts::record_tier15_with_lease;
use super::tier2::Tier2Scan;
use super::tier2_receipts::record_tier2_with_lease;
use super::tier2_receipts_tests::{TempRoot, REPO};
use super::tier3::Tier3Scan;
use super::tier3_receipts::record_tier3_with_lease;
use super::tier4::Tier4Scan;
use super::tier4_receipts::{
    record_tier4, record_tier4_with_lease, record_tier4_with_source_policy, Tier4Progress,
};
use super::tier4_receipts_tests::{seed_tier_four_cursor, source};
use super::waterfall_coordinator::{
    record_tier_one_unfenced_for_test, Tier1Progress, Tier1QueueEvidence,
};
use super::waterfall_policy::WaterfallPolicy;

fn empty_plan() -> ReadyQueuePlan {
    ReadyQueuePlan {
        ready: Vec::new(),
        blocked: Vec::new(),
        claimed: Vec::new(),
        conflicts: Vec::new(),
        worker_cap: WorkerCap {
            max_repo_workers: 1,
            active_count: 0,
            remaining: 1,
            reached: false,
        },
        batch: Vec::new(),
        gate_counts: QueueGateCounts::default(),
    }
}

fn config() -> AutonomousConfig {
    AutonomousConfig::parse(
        "tier4:\n  sources:\n    - id: alpha\n      host: alpha.example.test\n      path: /facts\n      max_bytes: 1024\n      deadline_millis: 1000\n",
    )
    .expect("configured Tier 4")
}

fn changed_config() -> AutonomousConfig {
    AutonomousConfig::parse(
        "tier4:\n  sources:\n    - id: beta\n      host: beta.example.test\n      path: /facts\n      max_bytes: 2048\n      deadline_millis: 1000\n",
    )
    .expect("changed Tier 4")
}

fn completed_tier4() -> TempRoot {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    let policy = WaterfallPolicy::from_config(&config()).expect("waterfall policy");
    let source_policy = policy.tier4_source().expect("Tier 4 source policy").clone();
    let observation = evaluate_tier4(Tier4Input::Enabled {
        source_policy: source_policy.clone(),
        sources: vec![Tier4StageResult::Complete(source(&[]))],
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
    .expect("Tier 4 observation")
    .observation()
    .cloned()
    .expect("completed observation");
    assert_eq!(
        record_tier4_with_source_policy(
            root.path(),
            REPO,
            Tier4Scan::Complete(observation),
            source_policy,
        )
        .expect("complete Tier 4"),
        Tier4Progress::Advanced
    );
    root
}

fn snapshot(root: &TempRoot) -> Vec<(String, Vec<u8>)> {
    fn collect(
        root: &std::path::Path,
        current: &std::path::Path,
        output: &mut Vec<(String, Vec<u8>)>,
    ) {
        let mut entries = fs::read_dir(current)
            .expect("snapshot directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("snapshot entries");
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("snapshot path")
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(path).expect("snapshot file"),
                ));
            }
        }
    }

    let mut output = Vec::new();
    collect(root.path(), root.path(), &mut output);
    output
}

fn seed_tier15_cursor(root: &TempRoot) {
    let store = super::tier2_receipts_tests::store(root);
    let evidence = store
        .persist_tier1_evidence(
            1,
            super::waterfall::Tier1EvidenceArtifact::ReadyPage,
            "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
        )
        .expect("Tier 1 evidence");
    let receipt = TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier1,
        "rust-foreground-tier1-v1",
        1,
        1,
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        FunnelCounts::new(0, 0, 0, 0, 0).expect("empty funnel"),
        vec![evidence],
    )
    .expect("Tier 1 receipt");
    store.persist_receipt(&receipt).expect("persist Tier 1");
    store
        .persist_state(
            &WaterfallState::new(REPO, 1, NoWorkTier::Tier1)
                .expect("Tier 1 state")
                .record_receipt(&receipt)
                .expect("Tier 1.5 cursor"),
        )
        .expect("persist Tier 1.5 cursor");
}

#[test]
fn configured_tier4_rollover_replays_before_next_tier_one_scan() {
    let root = completed_tier4();
    let plan = empty_plan();

    let policy = WaterfallPolicy::from_config(&config()).expect("waterfall policy");
    let result = record_tier_one_unfenced_for_test(
        root.path(),
        REPO,
        &policy,
        Tier1QueueEvidence::EmptyPage(&plan),
    );

    assert_eq!(result, Ok(Tier1Progress::Advanced));
    let store = match super::waterfall::WaterfallStore::acquire_with_policy(
        root.path().join("waterfall"),
        REPO,
        &policy,
    )
    .expect("policy-aware store")
    {
        super::waterfall::StoreAcquisition::Acquired(store) => store,
        super::waterfall::StoreAcquisition::Held => panic!("test store is not contended"),
    };
    let state = store.load_state().expect("state").expect("cursor");
    assert_eq!(
        (state.next_pass_id(), state.current_tier()),
        (2, NoWorkTier::Tier1_5)
    );
    assert_eq!(state.completed_receipts().len(), 1);
    assert_eq!(state.completed_receipts()[0].tier, NoWorkTier::Tier1);
    assert!(store
        .load_receipt(2, NoWorkTier::Tier1)
        .expect("next-pass Tier 1 receipt")
        .is_some());

    drop(store);
    let before_repeat = snapshot(&root);
    assert_eq!(
        record_tier_one_unfenced_for_test(
            root.path(),
            REPO,
            &policy,
            Tier1QueueEvidence::EmptyPage(&plan),
        ),
        Ok(Tier1Progress::Pending)
    );
    assert_eq!(snapshot(&root), before_repeat);
}

#[test]
fn mismatched_configured_tier4_policy_writes_no_cursor_or_evidence() {
    let root = completed_tier4();
    let before = snapshot(&root);
    let plan = empty_plan();

    let policy = WaterfallPolicy::from_config(&changed_config()).expect("changed policy");
    let result = record_tier_one_unfenced_for_test(
        root.path(),
        REPO,
        &policy,
        Tier1QueueEvidence::EmptyPage(&plan),
    );

    let error = result.expect_err("policy drift must reject configured Tier 4 evidence");
    assert!(error.contains("trusted source policy"), "{error}");
    assert_eq!(snapshot(&root), before);
}

#[test]
fn disabled_tier4_history_replays_without_source_authority() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4(root.path(), REPO, Tier4Scan::NotRun).expect("disabled Tier 4"),
        Tier4Progress::Advanced
    );
    let policy = WaterfallPolicy::from_config(&AutonomousConfig::default()).expect("policy");

    assert_eq!(
        match super::waterfall::WaterfallStore::acquire_with_policy(
            root.path().join("waterfall"),
            REPO,
            &policy,
        )
        .expect("default policy store")
        {
            super::waterfall::StoreAcquisition::Acquired(store) => store,
            super::waterfall::StoreAcquisition::Held => panic!("test store is not contended"),
        }
        .load_state()
        .expect("disabled replay")
        .expect("Tier 4 cursor")
        .current_tier(),
        NoWorkTier::Tier1
    );
}

#[test]
fn configured_policy_has_stable_schema_one_identity_and_exact_descriptors() {
    let config = config();
    let first = WaterfallPolicy::from_config(&config).expect("first policy");
    let second = WaterfallPolicy::from_config(&config).expect("second policy");
    let source = first.tier4_source().expect("configured source policy");

    assert_eq!(first, second);
    assert_eq!(source.schema_version, TIER4_SCHEMA);
    assert_eq!(source.descriptors, config.tier4.sources);
    assert!(source
        .policy_identity
        .starts_with("autospec-tier4-policy-v1:"));
    assert_ne!(
        first,
        WaterfallPolicy::from_config(&changed_config()).expect("changed policy")
    );
}

#[test]
fn replaced_lease_cannot_record_any_later_tier() {
    for tier in [
        NoWorkTier::Tier1_5,
        NoWorkTier::Tier2,
        NoWorkTier::Tier3,
        NoWorkTier::Tier4,
    ] {
        let root = TempRoot::new();
        match tier {
            NoWorkTier::Tier1_5 => seed_tier15_cursor(&root),
            NoWorkTier::Tier2 => super::tier2_receipts_tests::seed_tier_two_cursor(&root),
            NoWorkTier::Tier3 => super::tier3_receipts_tests::seed_tier_three_cursor(&root),
            NoWorkTier::Tier4 => seed_tier_four_cursor(&root),
            NoWorkTier::Tier1 => unreachable!(),
        }
        let lease = acquire_test_lifecycle(root.path(), REPO).expect("lifecycle lease");
        replace_test_lifecycle_generation(&lease).expect("replace lease generation");
        let before = snapshot(&root);
        let policy = WaterfallPolicy::from_config(&AutonomousConfig::default()).expect("policy");
        let result = match tier {
            NoWorkTier::Tier1_5 => record_tier15_with_lease(
                root.path(),
                REPO,
                &lease,
                Tier15Scan::Failed("stale lease probe".to_string()),
            )
            .map(|_| ()),
            NoWorkTier::Tier2 => {
                record_tier2_with_lease(root.path(), REPO, &lease, Tier2Scan::NotRun).map(|_| ())
            }
            NoWorkTier::Tier3 => {
                record_tier3_with_lease(root.path(), REPO, &lease, Tier3Scan::NotRun).map(|_| ())
            }
            NoWorkTier::Tier4 => {
                record_tier4_with_lease(root.path(), REPO, &lease, &policy, Tier4Scan::NotRun)
                    .map(|_| ())
            }
            NoWorkTier::Tier1 => unreachable!(),
        };
        assert!(result.is_err(), "stale lease unexpectedly wrote {tier:?}");
        assert_eq!(snapshot(&root), before, "stale lease mutated {tier:?}");
    }
}
