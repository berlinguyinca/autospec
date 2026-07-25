use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::tier3::{
    evaluate_tier3, Tier3AdapterEvidence, Tier3Finding, Tier3FindingKind, Tier3Input,
    Tier3Severity, Tier3StageResult,
};

use super::tier2::Tier2Scan;
use super::tier2_receipts::record_tier2;
use super::tier2_receipts_tests::{observation as tier2_observation, store, TempRoot, REPO};
use super::tier3::Tier3Scan;
use super::tier3_receipts::{record_tier3, Tier3Progress};

pub(super) fn seed_tier_three_cursor(root: &TempRoot) {
    super::tier2_receipts_tests::seed_tier_two_cursor(root);
    assert!(matches!(
        record_tier2(
            root.path(),
            REPO,
            Tier2Scan::Complete(tier2_observation(Vec::new(), Vec::new()))
        ),
        Ok(super::tier2_receipts::Tier2Progress::Advanced)
    ));
}

fn adapter(kind: Tier3FindingKind, findings: Vec<Tier3Finding>) -> Tier3AdapterEvidence {
    Tier3AdapterEvidence {
        schema_version: 1,
        adapter_version: format!("test-{}-adapter", kind.as_str()),
        rule_version: "rules-v1".to_string(),
        findings,
    }
}

pub(super) fn observation(
    architecture: Vec<Tier3Finding>,
) -> autospec_core::autonomous::tier3::Tier3Observation {
    observation_parts(architecture, Vec::new(), Vec::new())
}

fn observation_parts(
    architecture: Vec<Tier3Finding>,
    coverage: Vec<Tier3Finding>,
    debt: Vec<Tier3Finding>,
) -> autospec_core::autonomous::tier3::Tier3Observation {
    evaluate_tier3(Tier3Input::Enabled {
        architecture: Tier3StageResult::Complete(adapter(
            Tier3FindingKind::Architecture,
            architecture,
        )),
        coverage: Tier3StageResult::Complete(adapter(Tier3FindingKind::Coverage, coverage)),
        debt: Tier3StageResult::Complete(adapter(Tier3FindingKind::Debt, debt)),
    })
    .expect("Tier 3 input")
    .observation()
    .cloned()
    .expect("complete observation")
}

pub(super) fn finding() -> Tier3Finding {
    Tier3Finding {
        kind: Tier3FindingKind::Architecture,
        severity: Tier3Severity::High,
        rule_id: "architecture.boundary".to_string(),
        path: "src/lib.rs".to_string(),
        line: 3,
        message: "boundary is unsealed".to_string(),
    }
}

#[test]
fn tier3_disabled_policy_receipt_advances_waterfall() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);

    assert_eq!(
        record_tier3(root.path(), REPO, Tier3Scan::NotRun).expect("disabled receipt"),
        Tier3Progress::Advanced
    );
    let receipt = store(&root)
        .load_receipt(1, NoWorkTier::Tier3)
        .expect("receipt")
        .expect("sealed receipt");
    assert_eq!(receipt.evidence().len(), 1);
    assert_eq!(
        receipt.evidence()[0].reference,
        "waterfall/1/tier3/policy.json"
    );
    let state = store(&root).load_state().expect("state").expect("cursor");
    assert_eq!(state.current_tier(), NoWorkTier::Tier4);
    assert_eq!(
        state
            .completed_receipts()
            .last()
            .map(|completed| (completed.tier, completed.digest.as_str())),
        Some((NoWorkTier::Tier3, receipt.digest()))
    );
}

#[test]
fn tier3_empty_complete_metadata_advances_only_to_tier_four() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);

    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(observation(Vec::new()))
        )
        .expect("empty metadata receipt"),
        Tier3Progress::Advanced
    );
    assert_eq!(
        store(&root)
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier4
    );
}

#[test]
fn tier3_produced_metadata_replays_without_advancing() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(observation(vec![finding()]))
        )
        .expect("produced receipt"),
        Tier3Progress::Produced(1)
    );
    assert_eq!(
        record_tier3(root.path(), REPO, Tier3Scan::NotRun).expect("sealed replay"),
        Tier3Progress::Produced(1)
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

#[test]
fn tier3_valid_cross_adapter_dedup_order_does_not_need_rank_order() {
    let mut architecture = finding();
    architecture.severity = Tier3Severity::Low;
    let coverage = Tier3Finding {
        kind: Tier3FindingKind::Coverage,
        severity: Tier3Severity::Critical,
        rule_id: "coverage.critical".to_string(),
        path: "tests/lib.rs".to_string(),
        line: 8,
        message: "critical branch is unmeasured".to_string(),
    };
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(observation_parts(
                vec![architecture],
                vec![coverage],
                Vec::new()
            ))
        )
        .expect("valid core evidence must be replayable"),
        Tier3Progress::Produced(2)
    );
}

#[test]
fn tier3_equal_rank_tuples_across_kinds_replay_in_kind_order() {
    let architecture = finding();
    let coverage = Tier3Finding {
        kind: Tier3FindingKind::Coverage,
        severity: architecture.severity,
        rule_id: architecture.rule_id.clone(),
        path: architecture.path.clone(),
        line: architecture.line,
        message: architecture.message.clone(),
    };
    let debt = Tier3Finding {
        kind: Tier3FindingKind::Debt,
        severity: architecture.severity,
        rule_id: architecture.rule_id.clone(),
        path: architecture.path.clone(),
        line: architecture.line,
        message: architecture.message.clone(),
    };
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);

    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(observation_parts(
                vec![architecture],
                vec![coverage],
                vec![debt],
            ))
        )
        .expect("equal rank tuples must persist"),
        Tier3Progress::Produced(3)
    );
    let receipt = store(&root)
        .load_receipt(1, NoWorkTier::Tier3)
        .expect("receipt")
        .expect("sealed receipt");
    let findings = std::fs::read_to_string(
        root.path()
            .join("waterfall")
            .join(&receipt.evidence()[3].reference),
    )
    .expect("findings evidence");
    let ranked = findings
        .split_once("\"ranked\":")
        .expect("ranked findings")
        .1;
    let architecture_index = ranked
        .find("\"kind\":\"architecture\"")
        .expect("architecture");
    let coverage_index = ranked.find("\"kind\":\"coverage\"").expect("coverage");
    let debt_index = ranked.find("\"kind\":\"debt\"").expect("debt");
    assert!(architecture_index < coverage_index && coverage_index < debt_index);
    assert_eq!(
        record_tier3(root.path(), REPO, Tier3Scan::NotRun).expect("sealed replay"),
        Tier3Progress::Produced(3)
    );
}

#[test]
fn tier3_receipts_accept_core_rendered_escaped_finding_text() {
    let mut escaped = finding();
    escaped.message = "quoted \"finding\"\nwith a control marker \u{001b}".to_string();
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(observation(vec![escaped]))
        )
        .expect("core-rendered escaped evidence must replay"),
        Tier3Progress::Produced(1)
    );
}
