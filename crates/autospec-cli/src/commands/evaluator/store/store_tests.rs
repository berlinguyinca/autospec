//! Inline tests for the evaluation store (kept in their own module file so
//! `store.rs` stays under the 500-line complexity budget).

use super::tests_support::temp_base;
use super::{load_json, EvaluationStore};
use crate::commands::evaluator::types::{
    EpochId, EvaluationErrorKind, EvaluatorSlot, EvaluatorVersionRef, PromotionEvent,
};
use std::fs;
use std::path::{Path, PathBuf};

fn definition_file(base: &Path, slot: &str, version: u32) -> PathBuf {
    let digest = "a".repeat(64);
    let document = serde_json::json!({
        "schema": 1,
        "slot": slot,
        "version": version,
        "kind": "deterministic",
        "rubric_ref": format!("rubrics/{slot}.md"),
        "routing_policy_digest": digest,
        "tool_policy_digest": digest,
        "created_at": 1_757_217_600
    });
    let path = base.join(format!("{slot}v{version}.json"));
    fs::write(&path, document.to_string()).unwrap();
    path
}

fn store(base: &Path) -> EvaluationStore {
    EvaluationStore::new(base.join(".autospec").join("evaluation"))
}

#[test]
fn init_creates_policy_genesis_epoch_and_current_pointer() {
    let base = temp_base();
    let store = store(&base);
    let report = store.init(None).unwrap();
    assert_eq!(report.epoch, EpochId::genesis());
    assert!(store.policy_path().is_file());
    assert!(store.current_path().is_file());
    assert!(store.epoch_path(EpochId::genesis()).is_file());

    let current = store.epoch_current().unwrap();
    assert!(current.slot_versions.is_empty());
    assert_eq!(current.policy_digest, report.policy_digest);

    let error = store.init(None).unwrap_err();
    assert_eq!(error.kind, EvaluationErrorKind::Immutable);
    assert!(error.to_string().contains("policy.json"));
}

#[test]
fn register_is_immutable_and_list_is_sorted() {
    let base = temp_base();
    let store = store(&base);
    store.init(None).unwrap();

    let second = definition_file(&base, "architecture", 2);
    let first = definition_file(&base, "architecture", 1);
    let spec = definition_file(&base, "spec_compliance", 1);
    let first_report = store.register(&first).unwrap();
    store.register(&second).unwrap();
    store.register(&spec).unwrap();

    let error = store.register(&first).unwrap_err();
    assert_eq!(error.kind, EvaluationErrorKind::Immutable);

    let entries = store.list().unwrap();
    let references: Vec<String> = entries
        .iter()
        .map(|entry| entry.reference.to_string())
        .collect();
    assert_eq!(
        references,
        vec!["architecture@1", "architecture@2", "spec_compliance@1"]
    );
    assert_eq!(first_report.digest.len(), 64);
}

#[test]
fn pin_seeds_an_empty_slot_and_bumps_the_epoch() {
    let base = temp_base();
    let store = store(&base);
    store.init(None).unwrap();
    let file = definition_file(&base, "architecture", 1);
    store.register(&file).unwrap();

    let reference: EvaluatorVersionRef = "architecture@1".parse().unwrap();
    let report = store.pin(&reference, "operator").unwrap();
    assert_eq!(report.epoch, EpochId(1));
    assert!(report.promotion_id.starts_with("promo-"));
    assert_eq!(report.promotion_id.len(), 6 + 16);

    let current = store.epoch_current().unwrap();
    assert_eq!(current.epoch_id, EpochId(1));
    assert_eq!(current.slot_versions[&EvaluatorSlot::Architecture], 1);
    assert_eq!(
        current.promotion.as_deref(),
        Some(report.promotion_id.as_str())
    );

    let history = store.epoch_history().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].epoch_id, EpochId::genesis());
    assert_eq!(history[1].epoch_id, EpochId(1));

    let event: PromotionEvent = load_json(
        &store
            .promotions_dir()
            .join(format!("{}.json", report.promotion_id)),
    )
    .unwrap();
    assert_eq!(event.state, "committed");
    assert_eq!(event.from, None);
    assert_eq!(event.to, 1);
    assert_eq!(event.approval.actor, "operator");

    let error = store.pin(&reference, "operator").unwrap_err();
    assert_eq!(error.kind, EvaluationErrorKind::FailClosed);
}

#[test]
fn pin_requires_a_registered_evaluator() {
    let base = temp_base();
    let store = store(&base);
    store.init(None).unwrap();
    let reference: EvaluatorVersionRef = "maintainability@1".parse().unwrap();
    let error = store.pin(&reference, "operator").unwrap_err();
    assert_eq!(error.kind, EvaluationErrorKind::FailClosed);
}

#[test]
fn unregistered_commands_fail_closed_without_a_store() {
    let base = temp_base();
    let store = store(&base);
    let error = store.list().unwrap_err();
    assert_eq!(error.kind, EvaluationErrorKind::FailClosed);
    assert!(error.to_string().contains("not initialized"));
}
