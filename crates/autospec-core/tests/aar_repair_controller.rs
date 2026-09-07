//! AAR spec section 12: finite repair, failure taxonomy and profile
//! escalation. Integration coverage for issue #3322's acceptance criteria,
//! including the impossible-task fixture and the blocked-handoff schema.

use autospec_core::aar::repair_controller::{
    AttemptRecord, BlockedHandoff, BuilderTier, FailureClass, RepairAction, RepairController,
    RepairRequest, REPAIR_SCHEMA_VERSION,
};

fn request(class: FailureClass, sig: &str, node: &str) -> RepairRequest {
    RepairRequest {
        failed_class: class,
        reason: format!("diagnosis recorded for {sig}"),
        changed_input: format!("input patched for {sig}"),
        node_id: node.to_string(),
        input_signature: sig.to_string(),
    }
}

fn run_impossible_fixture(
    controller: &mut RepairController,
) -> (Vec<AttemptRecord>, BlockedHandoff) {
    // FAST: one cycle, then escalate.
    assert!(matches!(
        controller.request_retry(request(FailureClass::Inference, "fast-1", "node-a")),
        Ok(RepairAction::Retry { .. })
    ));
    match controller.request_retry(request(FailureClass::Inference, "fast-2", "node-a")) {
        Ok(RepairAction::Escalate { to, .. }) => {
            assert_eq!(to, BuilderTier::Coding);
        }
        other => panic!("expected escalation to CODING, got {other:?}"),
    }
    controller.adopt(BuilderTier::Coding).unwrap();

    // CODING: two cycles, then escalate.
    for sig in ["code-1", "code-2"] {
        assert!(matches!(
            controller.request_retry(request(FailureClass::Compile, sig, "node-a")),
            Ok(RepairAction::Retry { .. })
        ));
    }
    match controller.request_retry(request(FailureClass::Compile, "code-3", "node-a")) {
        Ok(RepairAction::Escalate { to, .. }) => {
            assert_eq!(to, BuilderTier::Deep);
        }
        other => panic!("expected escalation to DEEP, got {other:?}"),
    }
    controller.adopt(BuilderTier::Deep).unwrap();

    // DEEP: three cycles, then blocked.
    for sig in ["deep-1", "deep-2", "deep-3"] {
        assert!(matches!(
            controller.request_retry(request(FailureClass::Test, sig, "node-a")),
            Ok(RepairAction::Retry { .. })
        ));
    }
    match controller.request_retry(request(FailureClass::Test, "deep-4", "node-a")) {
        Ok(RepairAction::Blocked { handoff }) => {
            let attempts = handoff.attempts.clone();
            (attempts, handoff)
        }
        other => panic!("expected blocked handoff, got {other:?}"),
    }
}

/// The impossible-task fixture: every tier's cap is hit and the run ends with
/// `status: blocked`.
#[test]
fn repair_controller_impossible_fixture_ends_blocked() {
    let mut controller = RepairController::start("impossible-task", BuilderTier::Fast, "node-a");
    let (attempts, handoff) = run_impossible_fixture(&mut controller);

    assert_eq!(handoff.status, "blocked");
    assert_eq!(handoff.schema_version, REPAIR_SCHEMA_VERSION);
    assert_eq!(handoff.task_id, "impossible-task");
    assert_eq!(handoff.total_failed_cycles, 6);
    assert_eq!(attempts.len(), 6);
    let tiers: Vec<BuilderTier> = attempts.iter().map(|a| a.tier).collect();
    assert_eq!(
        tiers,
        vec![
            BuilderTier::Fast,
            BuilderTier::Coding,
            BuilderTier::Coding,
            BuilderTier::Deep,
            BuilderTier::Deep,
            BuilderTier::Deep
        ]
    );
    handoff
        .validate()
        .expect("blocked handoff schema must validate");
}

/// The blocked handoff has a stable, structured JSON schema.
#[test]
fn repair_controller_blocked_handoff_schema_round_trips() {
    let mut controller = RepairController::start("impossible-task", BuilderTier::Fast, "node-a");
    let (_, handoff) = run_impossible_fixture(&mut controller);

    let json = serde_json::to_value(&handoff).expect("handoff must serialize");
    assert_eq!(json["schema_version"], REPAIR_SCHEMA_VERSION);
    assert_eq!(json["status"], "blocked");
    assert_eq!(json["task_id"], "impossible-task");
    assert_eq!(json["tier"], "Deep");
    assert_eq!(json["total_failed_cycles"], 6);
    assert_eq!(json["last_failure"], "Test");
    assert_eq!(json["attempts"].as_array().unwrap().len(), 6);
    assert!(json["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a.as_str().is_some_and(|s| !s.is_empty())));
    let first = &json["attempts"][0];
    assert_eq!(first["tier"], "Fast");
    assert_eq!(first["failure_class"], "Inference");
    assert!(!first["reason"].as_str().unwrap().is_empty());
    assert!(!first["changed_input"].as_str().unwrap().is_empty());
    assert_eq!(first["node_id"], "node-a");

    let back: BlockedHandoff =
        serde_json::from_value(json).expect("handoff must deserialize from its own schema");
    assert_eq!(back, handoff);
    back.validate()
        .expect("round-tripped handoff must validate");
}

#[test]
fn repair_controller_coding_builder_stops_after_two_cycles() {
    let mut controller = RepairController::start("coding-stop", BuilderTier::Coding, "node-a");
    for sig in ["c-1", "c-2"] {
        assert!(matches!(
            controller.request_retry(request(FailureClass::Test, sig, "node-a")),
            Ok(RepairAction::Retry {
                tier: BuilderTier::Coding,
                ..
            })
        ));
    }
    // No third retry at CODING: the only action left at this tier is escalate.
    match controller.request_retry(request(FailureClass::Test, "c-3", "node-a")) {
        Ok(RepairAction::Escalate { from, to, .. }) => {
            assert_eq!(from, BuilderTier::Coding);
            assert_eq!(to, BuilderTier::Deep);
        }
        other => panic!("expected escalation after two cycles, got {other:?}"),
    }
}

#[test]
fn repair_controller_identical_retry_is_no_blind_retry() {
    let mut controller = RepairController::start("blind-retry", BuilderTier::Coding, "node-a");
    assert!(controller
        .request_retry(request(FailureClass::Edit, "same-sig", "node-a"))
        .is_ok());
    // Same input signature again — even on a different node, even with
    // different prose.
    let mut blind = request(FailureClass::Edit, "same-sig", "node-b");
    blind.reason = "brand new words".to_string();
    blind.changed_input = "brand new change".to_string();
    let err = controller.request_retry(blind).expect_err("must reject");
    assert_eq!(err.as_str(), "NO_BLIND_RETRY");
    // Rejected attempts are not counted.
    assert_eq!(controller.failed_cycles(), 1);
    assert_eq!(controller.history().len(), 1);
}

#[test]
fn repair_controller_node_failure_must_change_node_id() {
    let mut controller = RepairController::start("node-fail", BuilderTier::Coding, "node-a");
    let err = controller
        .request_retry(request(FailureClass::Node, "n-1", "node-a"))
        .expect_err("same-node retry must be rejected");
    assert_eq!(err.as_str(), "NODE_NOT_CHANGED");
    assert!(matches!(
        controller.request_retry(request(FailureClass::Node, "n-2", "node-b")),
        Ok(RepairAction::Retry { ref node_id, .. }) if node_id == "node-b"
    ));
    // The moved node is now the baseline: staying on node-b is a repeat.
    let err = controller
        .request_retry(request(FailureClass::Node, "n-3", "node-b"))
        .expect_err("retry must move again after a second node failure");
    assert_eq!(err.as_str(), "NODE_NOT_CHANGED");
}
