//! Contract tests for the additive learning-integration records
//! (`src/learning/`), the "Shared contracts" section of the observational
//! memory / native sessions / readiness delta design
//! (`docs/specs/2026-09-01-observational-memory-native-sessions-readiness-integration-delta-design.md`).
//!
//! These serde/property tests were written failing first against real
//! contract fixtures (no database, no mocks) and prove:
//!
//! - stable, byte-identical round-trip serialization of `LearningContractV1`;
//! - reuse of the existing work/role lineage, evidence run-id, and
//!   routing-ledger dispatch/outcome identifiers;
//! - explicit, scope-isolated memory scoping;
//! - rejection of unknown authority-changing enum values and unknown
//!   struct fields on deserialization;
//! - the trust and separation-of-duties invariants (observations are
//!   untrusted data, never policy; no self-approval; a preservation
//!   failure cannot be overridden by a quality score);
//! - 0 parallel scheduler, executor, project, ledger, benchmark, role, or
//!   database types in the module source.
//!
//! The module is compiled here via `#[path]` because this issue edits
//! exactly the three named files and does not wire `learning` into
//! `lib.rs`.

#[path = "../src/learning/mod.rs"]
mod learning;

use learning::contracts::*;

/// 64-character lowercase hex — sha256 digest grammar, fixture value only.
const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const COMMIT: &str = "0f1e2d3c4b5a6978";

fn lineage_ref() -> SessionLineageRef {
    SessionLineageRef {
        work_item: "issue-3463".to_string(),
        stage: "implement".to_string(),
        role: "implementer".to_string(),
        worktree: "feat/learning-contract-reconciliation".to_string(),
        branch: "feat/learning-contract-reconciliation".to_string(),
        pull_request: Some("pr-9001".to_string()),
        model: "claude-sonnet-5".to_string(),
        provider: "anthropic".to_string(),
    }
}

fn memory_record() -> MemoryRecord {
    MemoryRecord {
        record_id: "mem-3463-1".to_string(),
        mem_type: MemoryType::Statement,
        statement: "learning contracts reuse existing identifiers".to_string(),
        reason: "issue 3463 contract reconciliation".to_string(),
        scope: MemoryScope::Repository,
        authority: MemoryAuthority::UserApprovedSpec,
        confidence: 90,
        status: MemoryStatus::Active,
        origin_role: "implementer".to_string(),
        origin_model: "claude-sonnet-5".to_string(),
        origin_provider: "anthropic".to_string(),
        created_at: 1_700_000_000,
        updated_at: 1_700_000_060,
        evidence: vec![EvidenceRef {
            run_id: "run-3463".to_string(),
            artifact: Some(".autospec/evidence/run-3463/bundle.json".to_string()),
        }],
        source_commit: Some(COMMIT.to_string()),
        revision: 1,
        content_hash: HASH.to_string(),
        tags: vec!["learning".to_string()],
        relations: vec![MemoryRelation {
            kind: MemoryRelationKind::Supersedes,
            target_id: "mem-3463-0".to_string(),
        }],
        failed_approach: None,
        events: vec![MemoryAuditEvent {
            event: MemoryEvent::Audited,
            actor_role: "independent-reviewer".to_string(),
            at: 1_700_000_120,
        }],
        validated_by: Some("independent-reviewer".to_string()),
    }
}

fn contract() -> LearningContractV1 {
    let artifact = ArtifactIdentity {
        artifact_id: "spec-3463".to_string(),
        revision: 1,
        target_role: "implementer".to_string(),
        target_model: "claude-sonnet-5".to_string(),
        content_hash: HASH.to_string(),
    };
    let observation = ObservationCandidate {
        candidate_id: "obs-3463-1".to_string(),
        statement: "observer saw the contract round-trip".to_string(),
        authority: MemoryAuthority::RuntimeObservation,
        source_harness: "pi".to_string(),
        provenance_commit: COMMIT.to_string(),
        observed_at: 1_700_000_030,
        redacted: true,
    };
    let promotion = PromotionResult {
        candidate_id: "obs-3463-1".to_string(),
        outcome: PromotionOutcome::Promoted,
        decided_at: 1_700_000_040,
        reason: "normalized by AutoSpec-owned promotion".to_string(),
    };
    let providers = vec![QualityProviderResult {
        provider: "deterministic-rules".to_string(),
        dimension: "readiness".to_string(),
        available: true,
        score: Some(90),
    }];
    let quality = QualityReport {
        report_id: "qr-3463-1".to_string(),
        artifact: artifact.clone(),
        policy_mode: QualityPolicyMode::Shadow,
        providers: providers.clone(),
        aggregate: Some(90),
        hard_failure: false,
        decision: QualityDecision::Ready,
        policy_fingerprint: policy_fingerprint(QualityPolicyMode::Shadow, &providers),
    };
    let fingerprint = ContextFingerprint {
        fingerprint: HASH.to_string(),
        source_commit: COMMIT.to_string(),
        fresh: true,
        compiled_at: 1_700_000_050,
    };
    let preservation = PreservationResult {
        before: artifact.clone(),
        after: ArtifactIdentity {
            revision: 2,
            ..artifact.clone()
        },
        preserved: true,
        failures: vec![],
        attempts: 1,
    };
    let correlation = OutcomeCorrelation {
        dispatch_id: "dispatch-3463-1".to_string(),
        evidence_run_id: "run-3463".to_string(),
        routing_outcome: RoutingOutcome::MergedClean,
        benchmark_id: Some("realwork-42".to_string()),
        calibration: CalibrationBand::Uncalibrated,
    };
    LearningContractV1 {
        schema_version: LEARNING_CONTRACT_SCHEMA,
        session: lineage_ref(),
        artifact,
        observation: Some(observation),
        promotion: Some(promotion),
        memory: Some(memory_record()),
        quality: Some(quality),
        fingerprint: Some(fingerprint),
        preservation: Some(preservation),
        correlation: Some(correlation),
    }
}

// ── stable serialization ──────────────────────────────────────────────────

#[test]
fn full_contract_serializes_stably_and_round_trips() {
    let contract = contract();
    let first = serde_json::to_string(&contract).expect("contract serializes");
    let parsed: LearningContractV1 = serde_json::from_str(&first).expect("contract parses");
    let second = serde_json::to_string(&parsed).expect("parsed contract serializes");
    assert_eq!(
        first, second,
        "re-serialization must be byte-identical (stable serde)"
    );
    assert_eq!(parsed, contract);
    parsed.validate().expect("round-tripped contract validates");
}

// ── reuse of existing identifiers ─────────────────────────────────────────

#[test]
fn contract_reuses_existing_work_role_evidence_and_ledger_identifiers() {
    let contract = contract();
    assert_eq!(contract.work_item(), "issue-3463");
    assert_eq!(contract.role(), "implementer");
    assert_eq!(contract.ledger_dispatch_id(), Some("dispatch-3463-1"));
    assert_eq!(contract.evidence_run_id(), Some("run-3463"));
    assert_eq!(
        contract.routing_outcome(),
        Some(RoutingOutcome::MergedClean)
    );

    // Evidence run-id grammar is the evidence authority's grammar.
    assert!(is_valid_evidence_run_id("run-3463"));
    assert!(is_valid_evidence_run_id("run.3463_a-b"));
    assert!(!is_valid_evidence_run_id(""));
    assert!(!is_valid_evidence_run_id("-run"));
    assert!(!is_valid_evidence_run_id("run/../x"));

    // Routing outcome vocabulary is the routing ledger's vocabulary,
    // string for string.
    let vocabulary: Vec<&str> = [
        RoutingOutcome::Pending,
        RoutingOutcome::MergedClean,
        RoutingOutcome::LgtmFirstPass,
        RoutingOutcome::RetriedOk,
        RoutingOutcome::Escalated,
        RoutingOutcome::QaFailed,
        RoutingOutcome::Reverted,
        RoutingOutcome::Abandoned,
    ]
    .iter()
    .map(|outcome| outcome.as_str())
    .collect();
    assert_eq!(
        vocabulary,
        [
            "pending",
            "merged_clean",
            "lgtm_first_pass",
            "retried_ok",
            "escalated",
            "qa_failed",
            "reverted",
            "abandoned"
        ]
    );
    for outcome in [
        RoutingOutcome::Pending,
        RoutingOutcome::MergedClean,
        RoutingOutcome::LgtmFirstPass,
        RoutingOutcome::RetriedOk,
        RoutingOutcome::Escalated,
        RoutingOutcome::QaFailed,
        RoutingOutcome::Reverted,
        RoutingOutcome::Abandoned,
    ] {
        assert_eq!(RoutingOutcome::parse(outcome.as_str()), Ok(outcome));
    }
    assert!(RoutingOutcome::parse("whitelisted").is_err());
}

#[test]
fn session_lineage_is_reused_and_preserved() {
    let base = contract();
    assert!(base.validate().is_ok());
    let session = &base.session;
    assert_eq!(
        (
            session.work_item.as_str(),
            session.stage.as_str(),
            session.role.as_str()
        ),
        ("issue-3463", "implement", "implementer")
    );
    assert_eq!(session.pull_request.as_deref(), Some("pr-9001"));
    assert_eq!(
        (
            session.worktree.as_str(),
            session.branch.as_str(),
            session.model.as_str(),
            session.provider.as_str()
        ),
        (
            "feat/learning-contract-reconciliation",
            "feat/learning-contract-reconciliation",
            "claude-sonnet-5",
            "anthropic"
        )
    );

    let mut empty_role = contract();
    empty_role.session.role = String::new();
    assert!(empty_role.validate().is_err(), "empty role must fail");

    let mut empty_pr = contract();
    empty_pr.session.pull_request = Some(String::new());
    assert!(empty_pr.validate().is_err(), "empty pull_request must fail");

    let mut no_pr = contract();
    no_pr.session.pull_request = None;
    assert!(no_pr.validate().is_ok(), "absent pull_request is valid");
}

// ── unknown authority-changing values are rejected ───────────────────────

fn round_trip_and_reject<T, const N: usize>(variants: [T; N], unknown: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug,
{
    for variant in variants {
        let serialized = serde_json::to_string(&variant).expect("variant serializes");
        let parsed: T = serde_json::from_str(&serialized).expect("variant parses");
        assert_eq!(parsed, variant, "stable serde for {serialized}");
    }
    assert!(
        serde_json::from_str::<T>(unknown).is_err(),
        "unknown authority-changing value {unknown:?} must be rejected"
    );
}

#[test]
fn every_control_relevant_enum_rejects_unknown_values() {
    round_trip_and_reject(
        [
            MemoryStatus::Active,
            MemoryStatus::Challenged,
            MemoryStatus::Stale,
            MemoryStatus::Superseded,
            MemoryStatus::Archived,
            MemoryStatus::Quarantined,
        ],
        "\"hacked\"",
    );
    round_trip_and_reject(
        [
            MemoryAuthority::UserApprovedSpec,
            MemoryAuthority::AuthenticatedDirective,
            MemoryAuthority::RuntimeObservation,
            MemoryAuthority::InferredMemory,
        ],
        "\"god_mode\"",
    );
    round_trip_and_reject(
        [
            MemoryType::Statement,
            MemoryType::FailedApproach,
            MemoryType::Constraint,
            MemoryType::Preference,
        ],
        "\"policy\"",
    );
    round_trip_and_reject(
        [
            MemoryScope::Repository,
            MemoryScope::Branch,
            MemoryScope::Worktree,
            MemoryScope::CrossRepository,
        ],
        "\"global\"",
    );
    round_trip_and_reject(
        [
            PromotionOutcome::Promoted,
            PromotionOutcome::Rejected,
            PromotionOutcome::Quarantined,
        ],
        "\"auto_promote\"",
    );
    round_trip_and_reject(
        [
            QualityPolicyMode::Disabled,
            QualityPolicyMode::Shadow,
            QualityPolicyMode::Warn,
            QualityPolicyMode::Enforce,
        ],
        "\"override\"",
    );
    round_trip_and_reject(
        [
            QualityDecision::Ready,
            QualityDecision::Repairable,
            QualityDecision::Blocked,
            QualityDecision::Clarification,
        ],
        "\"trust\"",
    );
    round_trip_and_reject(
        [
            RoutingOutcome::Pending,
            RoutingOutcome::MergedClean,
            RoutingOutcome::LgtmFirstPass,
            RoutingOutcome::RetriedOk,
            RoutingOutcome::Escalated,
            RoutingOutcome::QaFailed,
            RoutingOutcome::Reverted,
            RoutingOutcome::Abandoned,
        ],
        "\"whitelisted\"",
    );
    round_trip_and_reject(
        [CalibrationBand::Uncalibrated, CalibrationBand::Calibrated],
        "\"guessed\"",
    );
    round_trip_and_reject(
        [
            PreservationFailure::RequirementInvented,
            PreservationFailure::ArchitectureChanged,
            PreservationFailure::ConstraintWeakened,
            PreservationFailure::ScopeChanged,
            PreservationFailure::AcceptanceCriteriaInvented,
        ],
        "\"fine\"",
    );
    round_trip_and_reject(
        [
            MemoryEvent::Challenged,
            MemoryEvent::Validated,
            MemoryEvent::Superseded,
            MemoryEvent::MarkedStale,
            MemoryEvent::Archived,
            MemoryEvent::Audited,
        ],
        "\"blessed\"",
    );
    round_trip_and_reject(
        [
            MemoryRelationKind::Supports,
            MemoryRelationKind::Contradicts,
            MemoryRelationKind::Supersedes,
            MemoryRelationKind::DerivedFrom,
        ],
        "\"ignores\"",
    );
}

#[test]
fn unknown_authority_fields_fail_deserialization() {
    let mut value = serde_json::to_value(contract()).expect("contract serializes");
    value
        .as_object_mut()
        .expect("object")
        .insert("secret_authority".to_string(), serde_json::json!(true));
    assert!(
        serde_json::from_value::<LearningContractV1>(value).is_err(),
        "unknown contract field must be rejected (deny_unknown_fields)"
    );

    let mut memory = serde_json::to_value(memory_record()).expect("memory serializes");
    memory
        .as_object_mut()
        .expect("object")
        .insert("policy".to_string(), serde_json::json!("self-approve"));
    assert!(
        serde_json::from_value::<MemoryRecord>(memory).is_err(),
        "unknown memory field must be rejected (deny_unknown_fields)"
    );
}

#[test]
fn schema_version_must_be_v1() {
    let mut base = contract();
    base.schema_version = 2;
    let serialized = serde_json::to_string(&base).expect("serializes");
    let parsed: LearningContractV1 = serde_json::from_str(&serialized).expect("parses as a record");
    assert!(
        parsed.validate().is_err(),
        "a future schema version must fail closed, not silently validate"
    );
    assert_eq!(contract().schema_version, LEARNING_CONTRACT_SCHEMA);
}

// ── scope isolation ───────────────────────────────────────────────────────

#[test]
fn memory_scope_is_explicit_and_isolated_per_variant() {
    for scope in [
        MemoryScope::Repository,
        MemoryScope::Branch,
        MemoryScope::Worktree,
        MemoryScope::CrossRepository,
    ] {
        let mut memory = memory_record();
        memory.scope = scope;
        let serialized = serde_json::to_string(&memory).expect("memory serializes");
        let parsed: MemoryRecord = serde_json::from_str(&serialized).expect("memory parses");
        assert_eq!(
            parsed.scope, scope,
            "scope must round-trip without collapsing"
        );
        parsed.validate().expect("scoped memory validates");
    }
    // Cross-repository memory is a distinct, named scope — never silently
    // merged into repository scope.
    assert_eq!(
        serde_json::to_string(&MemoryScope::CrossRepository).unwrap(),
        "\"cross_repository\""
    );
    assert_ne!(
        serde_json::to_string(&MemoryScope::Repository).unwrap(),
        serde_json::to_string(&MemoryScope::CrossRepository).unwrap()
    );
}

// ── trust and separation of duties ────────────────────────────────────────

#[test]
fn observations_are_untrusted_data_never_policy() {
    for policy_authority in [
        MemoryAuthority::UserApprovedSpec,
        MemoryAuthority::AuthenticatedDirective,
    ] {
        let mut base = contract();
        base.observation
            .as_mut()
            .expect("observation present")
            .authority = policy_authority;
        assert!(
            base.validate().is_err(),
            "observations must never carry policy authority {policy_authority:?}"
        );
    }
    for untrusted in [
        MemoryAuthority::RuntimeObservation,
        MemoryAuthority::InferredMemory,
    ] {
        let mut base = contract();
        base.observation
            .as_mut()
            .expect("observation present")
            .authority = untrusted;
        assert!(base.validate().is_ok(), "{untrusted:?} is allowed");
    }
    let mut not_redacted = contract();
    not_redacted
        .observation
        .as_mut()
        .expect("observation present")
        .redacted = false;
    assert!(
        not_redacted.validate().is_err(),
        "un-redacted observations must not be stored"
    );
}

#[test]
fn separation_of_duties_forbids_self_approval() {
    let mut self_validated = contract();
    self_validated
        .memory
        .as_mut()
        .expect("memory present")
        .validated_by = Some("implementer".to_string());
    assert!(
        self_validated.validate().is_err(),
        "the originating role cannot validate its own memory"
    );

    let mut self_event = contract();
    self_event
        .memory
        .as_mut()
        .expect("memory present")
        .events
        .push(MemoryAuditEvent {
            event: MemoryEvent::Validated,
            actor_role: "implementer".to_string(),
            at: 1_700_000_200,
        });
    assert!(
        self_event.validate().is_err(),
        "a validation event by the originating role must fail"
    );
}

#[test]
fn promotion_must_reference_the_observed_candidate() {
    let mut base = contract();
    base.promotion
        .as_mut()
        .expect("promotion present")
        .candidate_id = "obs-other".to_string();
    assert!(
        base.validate().is_err(),
        "promotion must reference the observed candidate id"
    );
}

// ── quality, preservation, and correlation invariants ─────────────────────

#[test]
fn policy_fingerprint_is_deterministic_and_policy_sensitive() {
    let providers = vec![QualityProviderResult {
        provider: "deterministic-rules".to_string(),
        dimension: "readiness".to_string(),
        available: true,
        score: Some(90),
    }];
    let first = policy_fingerprint(QualityPolicyMode::Shadow, &providers);
    let second = policy_fingerprint(QualityPolicyMode::Shadow, &providers);
    assert_eq!(first, second, "same policy => same fingerprint");
    assert_ne!(
        first,
        policy_fingerprint(QualityPolicyMode::Enforce, &providers),
        "different policy mode => different fingerprint"
    );
    assert!(first.starts_with("fp1-"));

    let mut tampered = contract();
    tampered
        .quality
        .as_mut()
        .expect("quality present")
        .policy_fingerprint = "fp1-0000000000000000".to_string();
    assert!(
        tampered.validate().is_err(),
        "tampered policy fingerprint must fail"
    );
}

#[test]
fn unavailable_provider_cannot_carry_score_and_hard_failure_blocks_ready() {
    let unavailable = QualityProviderResult {
        provider: "prompt-model".to_string(),
        dimension: "semantics".to_string(),
        available: false,
        score: Some(80),
    };
    assert!(
        unavailable.validate().is_err(),
        "an unavailable provider must never carry a score"
    );
    let available = QualityProviderResult {
        provider: "prompt-model".to_string(),
        dimension: "semantics".to_string(),
        available: true,
        score: None,
    };
    assert!(
        available.validate().is_err(),
        "an available provider must carry a score"
    );

    let mut hard_failure = contract();
    {
        let quality = hard_failure.quality.as_mut().expect("quality present");
        quality.hard_failure = true;
        quality.aggregate = Some(10);
    }
    assert!(
        hard_failure.validate().is_err(),
        "hard failure must not aggregate a score"
    );
    hard_failure
        .quality
        .as_mut()
        .expect("quality present")
        .aggregate = None;
    assert!(
        hard_failure.validate().is_err(),
        "hard failure must not allow a Ready decision"
    );
    hard_failure
        .quality
        .as_mut()
        .expect("quality present")
        .decision = QualityDecision::Blocked;
    assert!(hard_failure.validate().is_ok());
}

#[test]
fn preservation_failure_cannot_be_overridden_by_quality_score() {
    let mut base = contract();
    base.preservation
        .as_mut()
        .expect("preservation present")
        .preserved = false;
    base.preservation
        .as_mut()
        .expect("preservation present")
        .failures = vec![PreservationFailure::RequirementInvented];
    base.quality.as_mut().expect("quality present").aggregate = Some(100);
    base.quality.as_mut().expect("quality present").decision = QualityDecision::Ready;
    assert!(
        base.validate().is_err(),
        "a perfect quality score cannot override a preservation failure"
    );
    base.quality.as_mut().expect("quality present").decision = QualityDecision::Blocked;
    assert!(base.validate().is_ok(), "Blocked is honest");

    let mut inconsistent = contract();
    let preservation = inconsistent
        .preservation
        .as_mut()
        .expect("preservation present");
    preservation.preserved = true;
    preservation.failures = vec![PreservationFailure::ScopeChanged];
    assert!(
        inconsistent.validate().is_err(),
        "preserved=true with failures listed is inconsistent"
    );
}

#[test]
fn repair_attempts_are_bounded_at_two() {
    let mut base = contract();
    base.preservation
        .as_mut()
        .expect("preservation present")
        .attempts = 3;
    assert!(
        base.validate().is_err(),
        "repair attempts are bounded at two"
    );
    base.preservation
        .as_mut()
        .expect("preservation present")
        .attempts = 2;
    assert!(base.validate().is_ok());
}

#[test]
fn failed_approaches_are_first_class_and_exportable() {
    let mut typed_without_detail = memory_record();
    typed_without_detail.mem_type = MemoryType::FailedApproach;
    assert!(
        typed_without_detail.validate().is_err(),
        "a failed-approach memory requires its detail"
    );

    let mut untyped_with_detail = memory_record();
    untyped_with_detail.failed_approach = Some(FailedApproachDetail {
        approach: "single giant PR".to_string(),
        failure_conditions: vec!["reviewer context overflow".to_string()],
        replacement: Some("stacked PRs".to_string()),
        retry_conditions: None,
        do_not_retry: true,
    });
    assert!(
        untyped_with_detail.validate().is_err(),
        "detail without the failed-approach type is inconsistent"
    );

    let mut memory = memory_record();
    memory.mem_type = MemoryType::FailedApproach;
    memory.failed_approach = Some(FailedApproachDetail {
        approach: "single giant PR".to_string(),
        failure_conditions: vec!["reviewer context overflow".to_string()],
        replacement: Some("stacked PRs".to_string()),
        retry_conditions: None,
        do_not_retry: true,
    });
    assert!(memory.validate().is_ok());
    let markdown = memory.to_markdown();
    assert!(markdown.contains("failed_approach"));
    assert!(markdown.contains(memory.statement.as_str()));
    assert!(markdown.contains("mem-3463-1"));
}

#[test]
fn content_and_artifact_grammars_are_enforced() {
    assert!(is_sha256_hex_digest(HASH));
    assert!(!is_sha256_hex_digest("short"));
    assert!(!is_sha256_hex_digest(&"g".repeat(64)));

    let mut bad_hash = contract();
    bad_hash.artifact.content_hash = "not-a-hash".to_string();
    assert!(bad_hash.validate().is_err());

    let mut bad_revision = contract();
    bad_revision.artifact.revision = 0;
    assert!(
        bad_revision.validate().is_err(),
        "artifact revisions are 1-based"
    );
}

// ── no parallel authorities ───────────────────────────────────────────────

#[test]
fn no_parallel_scheduler_executor_project_ledger_benchmark_role_or_database_types() {
    let sources = [
        include_str!("../src/learning/mod.rs"),
        include_str!("../src/learning/contracts.rs"),
    ];
    const FORBIDDEN_PREFIXES: &[&str] = &[
        "Scheduler",
        "Executor",
        "Project",
        "Ledger",
        "Benchmark",
        "Role",
        "Database",
        "Db",
        "Store",
        "Queue",
    ];
    for source in sources {
        for line in source.lines() {
            let line = line.trim();
            let remainder = line
                .strip_prefix("pub struct ")
                .or_else(|| line.strip_prefix("pub enum "))
                .or_else(|| line.strip_prefix("pub trait "))
                .or_else(|| line.strip_prefix("pub type "))
                .unwrap_or("");
            if remainder.is_empty() {
                continue;
            }
            let name: String = remainder
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            for prefix in FORBIDDEN_PREFIXES {
                assert!(
                    !name.starts_with(prefix),
                    "parallel authority type {name} (prefix {prefix}) must not exist in the learning module"
                );
            }
        }
    }
    // The module exports exactly one submodule: the additive contracts.
    let declared: Vec<&str> = include_str!("../src/learning/mod.rs")
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("pub mod "))
        .map(|line| {
            line.trim_start_matches("pub mod ")
                .trim_end_matches(';')
                .trim()
        })
        .collect();
    assert_eq!(declared, ["contracts"]);
}
