//! Integration tests for the repo-local immutable evaluation registry
//! (`EvaluationStore`), plan Task 9 of the evaluator-coevolution slice.
//!
//! Fixtures are generated in a throwaway temp root so the test is
//! self-contained: no committed case files, and the suite's `content_digest`
//! values always match the bytes actually written.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::evaluation::anchor::{
    AccessRole, AnchorCase, AnchorSuite, AnchorVisibility, ProtectedLabel, Provenance, Severity,
};
use autospec_core::evaluation::digest::Digest;
use autospec_core::evaluation::evaluator::{EvaluatorDefinition, EvaluatorKind, Provenance as EvaluatorProvenance};
use autospec_core::evaluation::ids::{
    AnchorCaseId, AnchorSuiteId, EvaluationId, EpochId, EvaluatorSlot, EvaluatorVersionRef,
};
use autospec_core::evaluation::policy::PromotionPolicy;
use autospec_core::evaluation::promotion::PromotionState;
use autospec_core::evaluation::qualification::Verdict;
use autospec_core::evaluation::record::{
    ActiveRankingStatus, EvaluationRecord, Independence, RuntimeProvenance,
};
use autospec_core::evaluation::store::{EvaluationErrorKind, EvaluationStore};
use autospec_core::evaluation::EVALUATION_SCHEMA_VERSION;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TempProjectRoot {
    path: PathBuf,
}

impl TempProjectRoot {
    fn new() -> Self {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-eval-registry-{nonce}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempProjectRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A deterministic evaluator definition for the `architecture` slot.
fn fixture_definition(version: u32) -> EvaluatorDefinition {
    EvaluatorDefinition {
        schema: EVALUATION_SCHEMA_VERSION,
        slot: EvaluatorSlot::Architecture,
        version,
        kind: EvaluatorKind::Deterministic,
        rubric_ref: "docs/rules/evaluator-qualification.rules.yaml#architecture".into(),
        prompt_digest: None,
        skill_pack_digest: None,
        knowledge_base_digest: None,
        runtime_settings_digest: None,
        routing_policy_digest: Digest::of_bytes(b"routing"),
        tool_policy_digest: Digest::of_bytes(b"tools"),
        model_family: None,
        created_at: 1_760_000_000,
        parent_version: None,
        provenance: EvaluatorProvenance {
            created_by: "operator".into(),
            source: "manual".into(),
            notes: None,
        },
    }
}

fn fixture_record(evaluation_id: &str, evaluator_ref: &str) -> EvaluationRecord {
    EvaluationRecord {
        schema: EVALUATION_SCHEMA_VERSION,
        evaluation_id: EvaluationId::parse(evaluation_id).unwrap(),
        subject: "issue-1".into(),
        epoch_id: EpochId(0),
        evaluator: EvaluatorVersionRef::parse(evaluator_ref).unwrap(),
        outcome: Verdict::Accept,
        score: None,
        evidence_refs: Vec::new(),
        runtime: RuntimeProvenance {
            model_id: "claude-x".into(),
            provider_family: "anthropic".into(),
            execution_identity: "sandbox-1".into(),
            independence: Independence::High,
        },
        created_at: 1_760_000_000,
        active_ranking_status: ActiveRankingStatus::Active,
        ranking_history: Vec::new(),
    }
}

/// Write 40 case artifacts under `<root>/fixtures/anchors/architecture-fixture/cases`
/// and return a suite whose `content_digest` values match the bytes written.
///
/// Visibility is varied so the redaction assertions exercise every path:
/// every 11th case (i%11==5) is quarantined, every 7th (i%7==3) is a
/// protected holdout, the rest are development cases.
fn build_fixture_suite(root: &Path) -> AnchorSuite {
    let suite_id = AnchorSuiteId::parse("architecture-fixture").unwrap();
    let cases_dir = root.join("fixtures/anchors/architecture-fixture/cases");
    let mut cases = Vec::new();
    for i in 0..40u32 {
        let content = format!("case-{i} fixture artifact content");
        let file = cases_dir.join(format!("case-{i:02}.md"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, &content).unwrap();

        let visibility = if i % 11 == 5 {
            AnchorVisibility::Quarantine
        } else if i % 7 == 3 {
            AnchorVisibility::ProtectedHoldout
        } else {
            AnchorVisibility::Development
        };
        let label = if i % 2 == 0 {
            ProtectedLabel::Accept
        } else {
            ProtectedLabel::Reject
        };
        cases.push(AnchorCase {
            case_id: AnchorCaseId::parse(&format!("case-{i:02}")).unwrap(),
            artifact_ref: format!("fixtures/anchors/architecture-fixture/cases/case-{i:02}.md"),
            expected_label: Some(label),
            severity: Severity::Medium,
            tags: BTreeSet::new(),
            visibility,
            source: "fixture".into(),
            adjudication: None,
            content_digest: Digest::of_bytes(content.as_bytes()),
        });
    }
    AnchorSuite {
        schema: EVALUATION_SCHEMA_VERSION,
        suite_id,
        version: 1,
        slot: EvaluatorSlot::Architecture,
        cases,
        minimum_case_count: 40,
        required_subsets: Vec::new(),
        provenance: Provenance {
            created_by: "operator".into(),
            source: "fixture".into(),
            notes: None,
        },
    }
}

#[test]
fn init_is_once_and_open_requires_policy_and_pointer() {
    let root = TempProjectRoot::new();
    let store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    assert_eq!(store.current_epoch().unwrap().epoch_id, EpochId(0));
    let err = EvaluationStore::init(root.path(), PromotionPolicy::default(), 101).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::Immutable);
    assert!(EvaluationStore::open(root.path()).is_ok());
    std::fs::remove_file(root.path().join(".autospec/evaluation/current.json")).unwrap();
    assert!(EvaluationStore::open(root.path()).is_err());
}

#[test]
fn evaluator_versions_cannot_be_rewritten() {
    let root = TempProjectRoot::new();
    let mut store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    let def = fixture_definition(1);
    let digest = store.register_evaluator(&def, 100).unwrap();
    assert_eq!(digest, def.definition_digest());

    // A second write to the same slot/version is immutable, even if the
    // behaviour changed in place.
    let mut edited = def.clone();
    edited.prompt_digest = Some(Digest::of_bytes(b"edited in place"));
    let err = store.register_evaluator(&edited, 101).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::Immutable);
    assert!(err.message.contains("evaluators/architecture/v1.json"), "{err}");

    assert_eq!(store.evaluator(def.version_ref()).unwrap(), def);
    assert_eq!(store.list_evaluators().unwrap().len(), 1);
}

#[test]
fn anchor_registration_verifies_artifacts_and_redacts_for_mutation_role() {
    let root = TempProjectRoot::new();
    let suite = build_fixture_suite(root.path());

    let mut store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    let digest = store.register_anchor_suite(&suite, root.path(), 100).unwrap();
    assert_eq!(digest, suite.suite_digest());

    // The mutation role must never see protected-holdout labels.
    let redacted = store
        .anchor_suite(&suite.suite_id, 1, AccessRole::Mutation)
        .unwrap();
    assert!(
        redacted
            .cases
            .iter()
            .filter(|c| c.visibility == AnchorVisibility::ProtectedHoldout)
            .all(|c| c.expected_label.is_none()),
        "protected-holdout labels must be redacted for the mutation role"
    );
    // Quarantine cases are dropped entirely for the mutation role.
    assert!(
        redacted.cases.iter().all(|c| c.visibility != AnchorVisibility::Quarantine),
        "quarantine cases must be dropped for the mutation role"
    );

    // The qualification role sees the full, labeled suite.
    let full = store
        .anchor_suite(&suite.suite_id, 1, AccessRole::Qualification)
        .unwrap();
    assert_eq!(full.labeled_cases().unwrap().len(), 40);

    // Tamper with one artifact, then try to register a new version: the
    // pinned digest no longer matches, so verification fails closed.
    std::fs::write(root.path().join(&suite.cases[3].artifact_ref), b"tampered").unwrap();
    let mut v2 = suite.clone();
    v2.version = 2;
    assert_eq!(
        store.register_anchor_suite(&v2, root.path(), 102).unwrap_err().kind,
        EvaluationErrorKind::Integrity
    );
}

#[test]
fn records_and_trials_are_write_once() {
    let root = TempProjectRoot::new();
    let mut store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    let record = fixture_record("ev-1", "architecture@1");
    store.write_record(&record, 100).unwrap();
    let err = store.write_record(&record, 101).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::Immutable);
    assert_eq!(store.records().unwrap().len(), 1);
    assert_eq!(store.record(&record.evaluation_id).unwrap(), record);
}

#[test]
fn pin_seeds_an_empty_slot_and_rejects_repin_unknown_and_empty_actor() {
    let root = TempProjectRoot::new();
    let mut store = EvaluationStore::init(root.path(), PromotionPolicy::default(), 100).unwrap();
    let def = fixture_definition(1);
    store.register_evaluator(&def, 100).unwrap();

    // Pinning the registered version seeds a committed, human-approved epoch.
    let reference = def.version_ref();
    let event = store.pin(reference, "operator", 101).unwrap();
    assert_eq!(event.to, reference);
    assert_eq!(event.epoch.epoch_id, EpochId(1));
    assert_eq!(event.state, PromotionState::Committed);
    assert_eq!(event.approvals.len(), 1);
    assert_eq!(event.approvals[0].by, "operator");
    // The successor epoch records the pin; the store's current pointer moved.
    let current = store.current_epoch().unwrap();
    assert_eq!(current.epoch_id, EpochId(1));
    assert_eq!(current.version_of(def.slot), Some(1));
    // The promotion event round-trips from disk.
    assert_eq!(store.promotion(&event.id).unwrap(), event);
    assert_eq!(store.promotions().unwrap().len(), 1);
    assert_eq!(store.epoch_history().unwrap().len(), 2);

    // Repinning the same slot is a promotion, not a pin.
    let err = store.pin(reference, "operator", 102).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::Invariant);
    assert!(err.message.contains("already pins"), "{err}");

    // Pinning an unregistered version fails (the definition must exist).
    let mut unknown = def.clone();
    unknown.version = 2;
    let err = store.pin(unknown.version_ref(), "operator", 103).unwrap_err();
    assert!(err.message.contains("evaluators/architecture/v2.json"), "{err}");

    // A registered version pinned with a blank actor is rejected.
    store.register_evaluator(&unknown, 103).unwrap();
    let err = store.pin(unknown.version_ref(), "  ", 104).unwrap_err();
    assert_eq!(err.kind, EvaluationErrorKind::Invariant);
    assert!(err.message.contains("actor"), "{err}");
}
