//! InferWeave routing, freshness, memory and configuration
//! (spec sections 16, 17, 23, 24, 31, 37, 51).

mod rag_support;

use autospec_core::rag::authority::SourceAuthority;
use autospec_core::rag::config::{RagConfig, SourceAvailability};
use autospec_core::rag::evidence::Privacy;
use autospec_core::rag::freshness::{Freshness, FreshnessInput, FreshnessPolicy, StalenessRule};
use autospec_core::rag::memory::{MemoryCandidate, MemoryRejection, MemoryTier, MemoryWritePolicy};
use autospec_core::rag::metrics::ContextEfficiency;
use autospec_core::rag::policy::AgentRole;
use autospec_core::rag::routing::{
    select_node, LatencyPriority, ModelCapabilities, NodeCandidate, RagModelTask, ReasoningClass,
};
use autospec_core::rag::score::Score;
use autospec_core::rag::source::SourceKind;
use rag_support::{evidence, NOW, REVISION};

fn node(id: &str, free_context: u32, speed: u32) -> NodeCandidate {
    NodeCandidate {
        id: id.to_string(),
        reasoning_class: ReasoningClass::Strong,
        coding: true,
        structured_output: true,
        free_context_tokens: free_context,
        speed_rank: speed,
        available_seats: 2,
    }
}

fn needs(context: u32) -> ModelCapabilities {
    ModelCapabilities {
        reasoning_class: ReasoningClass::Medium,
        coding: false,
        min_context: context,
        structured_output: true,
        latency_priority: LatencyPriority::High,
    }
}

#[test]
fn a_faster_node_without_the_context_capacity_is_not_selected() {
    // Specification section 24's worked example: fast node with 20K free,
    // slower node with 100K free, a request estimated at 60K.
    let fast = node("A", 20_000, 100);
    let slow = node("B", 100_000, 10);

    let decision = select_node(&needs(60_000), &[fast, slow]);

    assert_eq!(decision.selected.expect("a node is eligible").id, "B");
    assert!(decision
        .rejected
        .iter()
        .any(|rejection| rejection.node_id == "A" && rejection.reason.contains("free context")));
}

#[test]
fn eligible_nodes_are_packed_from_the_tightest_fit_upward() {
    let tight = node("tight", 70_000, 10);
    let roomy = node("roomy", 200_000, 100);

    let decision = select_node(&needs(60_000), &[roomy, tight]);

    assert_eq!(
        decision.selected.expect("a node is eligible").id,
        "tight",
        "large contiguous capacity is preserved for later large requests"
    );
}

#[test]
fn routing_reserves_a_margin_above_the_token_estimate() {
    // A node with only the estimated capacity is rejected; one carrying the
    // margin is not. Sizing to the estimate exactly fails whenever the real
    // tokenizer counts higher than the approximation.
    let exact = select_node(&needs(10_000), &[node("exact", 10_500, 10)]);
    let with_margin = select_node(&needs(10_000), &[node("roomy", 11_500, 10)]);

    assert_eq!(exact.required_context_tokens, 11_000);
    assert!(exact.selected.is_none());
    assert_eq!(
        with_margin
            .selected
            .expect("the roomier node is eligible")
            .id,
        "roomy"
    );
}

#[test]
fn a_node_below_the_required_reasoning_class_is_rejected() {
    let mut weak = node("weak", 200_000, 100);
    weak.reasoning_class = ReasoningClass::Small;

    let decision = select_node(&needs(1_000), &[weak]);

    assert!(decision.selected.is_none());
    assert!(decision.rejected[0].reason.contains("reasoning class"));
}

#[test]
fn a_node_with_no_free_seats_is_rejected() {
    let mut full = node("full", 200_000, 100);
    full.available_seats = 0;

    let decision = select_node(&needs(1_000), &[full]);

    assert!(decision.selected.is_none());
    assert!(decision.rejected[0].reason.contains("seats"));
}

#[test]
fn rag_subtasks_declare_capabilities_rather_than_naming_a_model() {
    let rewrite = RagModelTask::QueryRewriting.capabilities(4_000);
    let synthesis = RagModelTask::ArchitectureSynthesis.capabilities(40_000);

    assert_eq!(rewrite.reasoning_class, ReasoningClass::Small);
    assert_eq!(rewrite.latency_priority, LatencyPriority::High);
    assert_eq!(synthesis.reasoning_class, ReasoningClass::Strong);
    assert!(RagModelTask::ImplementationPlan.capabilities(1).coding);
}

#[test]
fn source_code_evidence_goes_stale_when_the_revision_moves() {
    let policy = FreshnessPolicy::default();
    let input = FreshnessInput {
        captured_revision: REVISION.to_string(),
        current_revision: "def456".to_string(),
        retrieved_at: NOW,
        now: NOW,
        superseded: false,
    };

    assert_eq!(
        policy.assess(SourceKind::Repository, &input),
        Freshness::Stale
    );
}

#[test]
fn runtime_telemetry_goes_stale_after_its_window() {
    let policy = FreshnessPolicy::default();
    let make = |elapsed: u64| FreshnessInput {
        captured_revision: REVISION.to_string(),
        current_revision: REVISION.to_string(),
        retrieved_at: NOW,
        now: NOW + elapsed,
        superseded: false,
    };

    assert_eq!(
        policy.assess(SourceKind::Runtime, &make(10)),
        Freshness::Current
    );
    assert_eq!(
        policy.assess(SourceKind::Runtime, &make(50)),
        Freshness::Aging
    );
    assert_eq!(
        policy.assess(SourceKind::Runtime, &make(61)),
        Freshness::Stale
    );
}

#[test]
fn an_adr_goes_stale_only_when_superseded() {
    let policy = FreshnessPolicy::default();
    let mut input = FreshnessInput {
        captured_revision: REVISION.to_string(),
        current_revision: "def456".to_string(),
        retrieved_at: NOW,
        now: NOW + 400 * 86_400,
        superseded: false,
    };

    assert_eq!(policy.assess(SourceKind::Adr, &input), Freshness::Current);
    input.superseded = true;
    assert_eq!(policy.assess(SourceKind::Adr, &input), Freshness::Stale);
}

#[test]
fn a_project_can_override_one_sources_staleness_rule() {
    let policy = FreshnessPolicy::default()
        .with_rule(SourceKind::Runtime, StalenessRule::After { seconds: 600 });
    let input = FreshnessInput {
        captured_revision: REVISION.to_string(),
        current_revision: REVISION.to_string(),
        retrieved_at: NOW,
        now: NOW + 120,
        superseded: false,
    };

    assert_eq!(
        policy.assess(SourceKind::Runtime, &input),
        Freshness::Current
    );
}

#[test]
fn a_memory_candidate_without_provenance_is_rejected() {
    let policy = MemoryWritePolicy::new();
    let candidate = MemoryCandidate {
        id: "mem_1".to_string(),
        tier: MemoryTier::Project,
        content: "the scheduler is fair-share".to_string(),
        provenance: Vec::new(),
        confidence: Score::ONE,
        privacy: Privacy::Internal,
        durable: true,
    };

    assert_eq!(
        policy.evaluate(&candidate),
        Err(MemoryRejection::NoProvenance)
    );
}

#[test]
fn private_evidence_cannot_enter_global_autospec_memory() {
    let supporting = evidence(
        "ev_1",
        SourceKind::Repository,
        "src/private.rs",
        "internal detail",
        SourceAuthority::Implementation,
        900,
    );
    let candidate = MemoryCandidate::from_evidence(
        "mem_1",
        MemoryTier::GlobalAutospec,
        "prefer fair-share scheduling for multi-tenant queues",
        &[supporting],
        true,
    );

    let rejection = MemoryWritePolicy::new()
        .evaluate(&candidate)
        .expect_err("internal evidence must not reach global memory");

    assert!(matches!(
        rejection,
        MemoryRejection::PrivacyViolation { .. }
    ));
}

#[test]
fn a_transient_observation_is_not_written_to_durable_memory() {
    let supporting = evidence(
        "ev_1",
        SourceKind::Repository,
        "src/a.rs",
        "detail",
        SourceAuthority::Implementation,
        900,
    );
    let mut candidate = MemoryCandidate::from_evidence(
        "mem_1",
        MemoryTier::Project,
        "the current branch has three failing tests",
        &[supporting],
        false,
    );
    candidate.durable = false;

    assert_eq!(
        MemoryWritePolicy::new().evaluate(&candidate),
        Err(MemoryRejection::NotDurable)
    );
}

#[test]
fn a_well_sourced_durable_candidate_is_admitted() {
    let supporting = evidence(
        "ev_1",
        SourceKind::Repository,
        "src/a.rs",
        "detail",
        SourceAuthority::Implementation,
        900,
    );
    let candidate = MemoryCandidate::from_evidence(
        "mem_1",
        MemoryTier::Project,
        "this project routes every scheduler change through the gateway tests",
        &[supporting],
        true,
    );

    let entry = MemoryWritePolicy::new()
        .evaluate(&candidate)
        .expect("a sourced, durable, confident candidate passes");

    assert_eq!(entry.provenance, ["ev_1".to_string()]);
}

#[test]
fn a_duplicate_memory_candidate_is_rejected() {
    let supporting = evidence(
        "ev_1",
        SourceKind::Repository,
        "src/a.rs",
        "detail",
        SourceAuthority::Implementation,
        900,
    );
    let candidate = MemoryCandidate::from_evidence(
        "mem_1",
        MemoryTier::Project,
        "scheduler changes go through the gateway tests",
        &[supporting],
        true,
    );
    let mut policy = MemoryWritePolicy::new();
    let entry = policy.evaluate(&candidate).expect("first write passes");
    policy.admit(entry);

    let second = MemoryCandidate {
        id: "mem_2".to_string(),
        ..candidate
    };

    assert!(matches!(
        policy.evaluate(&second),
        Err(MemoryRejection::Duplicate { .. })
    ));
}

#[test]
fn configuration_defaults_match_the_specification_block() {
    let config = RagConfig::default();

    assert!(config.enabled);
    assert_eq!(config.default_budget.max_iterations, 8);
    assert_eq!(config.default_budget.max_queries, 24);
    assert_eq!(config.default_budget.max_evidence_items, 80);
    assert_eq!(config.default_budget.max_context_tokens, 40_000);
    assert_eq!(config.graph.max_depth, 4);
    assert_eq!(
        config.availability(SourceKind::Web),
        SourceAvailability::PolicyGated
    );
    config.validate().expect("the defaults are valid");
}

#[test]
fn a_disabled_source_cannot_be_re_enabled_by_a_role_policy_or_by_the_task() {
    let mut config = RagConfig::default();
    config
        .apply_override("sources.github", "false")
        .expect("override applies");

    assert!(
        !config.source_allowed(SourceKind::GitHub, AgentRole::Reviewer, true),
        "the reviewer lists GitHub and the task asked for it, but the administrator disabled it"
    );
}

#[test]
fn a_policy_gated_source_needs_the_task_to_ask_for_it() {
    let config = RagConfig::default();

    assert!(
        !config.source_allowed(SourceKind::Web, AgentRole::Specification, false),
        "listing the web in a role policy is not a standing licence to browse"
    );
    assert!(config.source_allowed(SourceKind::Web, AgentRole::Specification, true));
}

#[test]
fn a_source_a_role_does_not_list_is_not_reachable_for_that_role() {
    let config = RagConfig::default();

    assert!(
        !config.source_allowed(SourceKind::Runtime, AgentRole::RetrievalEvaluator, true),
        "the evaluator's policy names no runtime source"
    );
    assert!(config.source_allowed(SourceKind::Repository, AgentRole::Implementation, false));
}

#[test]
fn a_cache_that_is_enabled_but_revision_blind_is_refused() {
    let mut config = RagConfig::default();
    config
        .apply_override("cache.revision_aware", "false")
        .expect("override applies");

    let error = config
        .validate()
        .expect_err("a revision-blind cache serves stale evidence");

    assert!(error.contains("revision_aware"), "{error}");
}

#[test]
fn an_unknown_configuration_key_is_an_error_not_a_no_op() {
    let mut config = RagConfig::default();

    let error = config
        .apply_override("default.max_iteratons", "4")
        .expect_err("a typo must not read as a default");

    assert!(error.contains("unknown budget field"), "{error}");
}

#[test]
fn the_rendered_configuration_names_every_role_and_source() {
    let yaml = RagConfig::default().render_yaml();

    for role in autospec_core::rag::policy::ALL_ROLES {
        assert!(
            yaml.contains(role.as_str()),
            "missing role {}",
            role.as_str()
        );
    }
    for kind in autospec_core::rag::source::ALL_SOURCE_KINDS {
        assert!(
            yaml.contains(kind.as_str()),
            "missing source {}",
            kind.as_str()
        );
    }
}

#[test]
fn context_efficiency_reports_a_tiny_supplied_fraction_without_rounding_to_zero() {
    let efficiency = ContextEfficiency {
        searchable_tokens: 400_000_000,
        retrieved_tokens: 120_000,
        supplied_tokens: 17_340,
    };

    assert!(efficiency.supplied_fraction_ppm() > 0);
    assert_eq!(efficiency.retrieval_utilization_permille(), 144);
}
