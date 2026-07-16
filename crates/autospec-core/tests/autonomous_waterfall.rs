use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

const EVIDENCE_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn counts() -> FunnelCounts {
    FunnelCounts::new(8, 5, 3, 2, 1).expect("monotonic funnel counts")
}

fn evidence() -> Vec<SealedEvidence> {
    vec![
        SealedEvidence::new("evidence/ready-queue.json", EVIDENCE_DIGEST)
            .expect("sealed evidence reference"),
    ]
}

fn receipt(status: TierStatus) -> TierReceipt {
    TierReceipt::new(
        "owner/repo",
        9,
        NoWorkTier::Tier2,
        "native-discovery-v1",
        100,
        101,
        status,
        counts(),
        evidence(),
    )
    .expect("valid receipt")
}

#[test]
fn receipt_schema_binds_exact_scope_pass_tier_and_sealed_evidence() {
    let receipt = receipt(TierStatus::Exhausted {
        reason: DryReason::VerificationRejected,
    });
    let json = receipt.to_json();

    assert_eq!(
        TierReceipt::parse_json(&json, "owner/repo", 9, NoWorkTier::Tier2)
            .expect("matching receipt parses"),
        receipt
    );
    assert!(
        TierReceipt::parse_json(&json, "other/repo", 9, NoWorkTier::Tier2)
            .expect_err("foreign repository fails closed")
            .contains("repository")
    );
    assert!(
        TierReceipt::parse_json(&json, "owner/repo", 10, NoWorkTier::Tier2)
            .expect_err("wrong pass fails closed")
            .contains("pass")
    );
    assert!(
        TierReceipt::parse_json(&json, "owner/repo", 9, NoWorkTier::Tier3)
            .expect_err("wrong tier fails closed")
            .contains("tier")
    );
    assert!(TierReceipt::parse_json(
        &json.replace(receipt.digest(), "not-a-sealed-digest"),
        "owner/repo",
        9,
        NoWorkTier::Tier2,
    )
    .expect_err("tampered receipt digest fails closed")
    .contains("digest"));
    assert!(TierReceipt::parse_json(
        &json.replace("evidence/ready-queue.json", "../outside.json"),
        "owner/repo",
        9,
        NoWorkTier::Tier2,
    )
    .expect_err("unsafe evidence path fails closed")
    .contains("reference"));
    assert!(TierReceipt::parse_json(
        &json.replace("native-discovery-v1", "tampered-producer-v1"),
        "owner/repo",
        9,
        NoWorkTier::Tier2,
    )
    .expect_err("payload tampering cannot retain the original digest")
    .contains("digest"));
}

#[test]
fn receipt_schema_rejects_unknown_fields_and_statuses() {
    let json = receipt(TierStatus::Produced { count: 1 }).to_json();

    assert!(TierReceipt::parse_json(
        &json.replace('}', ",\"unexpected\":true}"),
        "owner/repo",
        9,
        NoWorkTier::Tier2,
    )
    .expect_err("unknown receipt field fails closed")
    .contains("unexpected"));
    assert!(TierReceipt::parse_json(
        &json.replace("\"kind\":\"produced\"", "\"kind\":\"invented\""),
        "owner/repo",
        9,
        NoWorkTier::Tier2,
    )
    .expect_err("unknown status fails closed")
    .contains("status"));
}

#[test]
fn later_tier_one_cursor_requires_retained_prior_pass_history() {
    assert!(
        WaterfallState::new("owner/repo", 2, NoWorkTier::Tier1).is_err(),
        "a rollover cursor must retain every prior-pass receipt"
    );
}

#[test]
fn state_preserves_failed_and_not_run_receipts_and_derives_paths() {
    let failed = TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1,
        "queue-v1",
        100,
        101,
        TierStatus::Failed {
            reason: "queue transport unavailable".to_string(),
        },
        counts(),
        evidence(),
    )
    .expect("failed receipt remains valid");
    let not_run = TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1_5,
        "architecture-v1",
        102,
        103,
        TierStatus::NotRun {
            reason: "policy disabled".to_string(),
        },
        counts(),
        evidence(),
    )
    .expect("not-run receipt remains valid");
    let state = WaterfallState::new("owner/repo", 1, NoWorkTier::Tier1)
        .expect("new state")
        .record_receipt(&failed)
        .expect("failed receipt is retained")
        .record_receipt(&not_run)
        .expect("not-run receipt is retained");

    assert_eq!(
        TierReceipt::parse_json(&failed.to_json(), "owner/repo", 1, NoWorkTier::Tier1)
            .expect("failed receipt parses"),
        failed
    );
    assert_eq!(
        TierReceipt::parse_json(&not_run.to_json(), "owner/repo", 1, NoWorkTier::Tier1_5)
            .expect("not-run receipt parses"),
        not_run
    );
    assert_eq!(state.completed_receipts().len(), 2);
    assert_eq!(
        state.completed_receipts()[0].reference,
        "waterfall/1/tier1.json"
    );
    assert_eq!(
        state.completed_receipts()[1].reference,
        "waterfall/1/tier1_5.json"
    );
    let json = state.to_json();
    assert_eq!(
        WaterfallState::parse_json(&json, "owner/repo").expect("state parses"),
        state
    );
    assert!(WaterfallState::parse_json(
        &json.replace("waterfall/1/tier1.json", "waterfall/1/tier4.json"),
        "owner/repo",
    )
    .expect_err("tampered receipt path fails closed")
    .contains("derived"));
    assert!(WaterfallState::parse_json(
        &json.replace(failed.digest(), "not-a-sealed-digest"),
        "owner/repo",
    )
    .expect_err("tampered completed digest fails closed")
    .contains("digest"));
}
