//! Role policies, token budgets and the context package
//! (spec sections 7, 18, 19, 20, 22).

mod rag_support;

use autospec_core::rag::authority::SourceAuthority;
use autospec_core::rag::budget::{RetrievalBudget, StopReason};
use autospec_core::rag::compression::{self, CompressionLevel};
use autospec_core::rag::context_package::ContextPackageBuilder;
use autospec_core::rag::contradiction::{Contradiction, ContradictionSet, ContradictionSeverity};
use autospec_core::rag::evidence::Privacy;
use autospec_core::rag::policy::{AgentRole, PolicySet, RetrievalPolicy, ALL_ROLES};
use autospec_core::rag::source::SourceKind;
use rag_support::evidence;

fn snippet(id: &str, body: &str) -> autospec_core::rag::evidence::Evidence {
    evidence(
        id,
        SourceKind::Repository,
        "src/scheduler.rs",
        body,
        SourceAuthority::Implementation,
        900,
    )
}

#[test]
fn each_role_gets_its_own_source_ordering_not_one_global_top_k() {
    let spec = RetrievalPolicy::for_role(AgentRole::Specification);
    let implementation = RetrievalPolicy::for_role(AgentRole::Implementation);

    assert_eq!(spec.priority_sources()[0], SourceKind::Specification);
    assert_eq!(implementation.priority_sources()[0], SourceKind::Repository);
    assert_ne!(spec.priority_sources(), implementation.priority_sources());
}

#[test]
fn role_context_budgets_match_the_specification_defaults() {
    for (role, tokens) in [
        (AgentRole::Specification, 60_000),
        (AgentRole::Planner, 50_000),
        (AgentRole::Implementation, 30_000),
        (AgentRole::Reviewer, 40_000),
        (AgentRole::Test, 30_000),
        (AgentRole::RetrievalEvaluator, 8_000),
    ] {
        assert_eq!(
            RetrievalPolicy::for_role(role).max_context_tokens(),
            tokens,
            "{}",
            role.as_str()
        );
    }
}

#[test]
fn reviewer_and_test_roles_require_independent_verification() {
    for role in ALL_ROLES {
        let policy = RetrievalPolicy::for_role(role);
        let expected = matches!(role, AgentRole::Reviewer | AgentRole::Test);
        assert_eq!(
            policy.requires_independent_verification(),
            expected,
            "{}",
            role.as_str()
        );
    }
}

#[test]
fn a_role_policy_can_only_tighten_the_administrator_budget() {
    let administrator = RetrievalBudget {
        max_context_tokens: 20_000,
        ..RetrievalBudget::default()
    };
    // The specification role wants 60k; the administrator allows 20k.
    let effective =
        RetrievalPolicy::for_role(AgentRole::Specification).apply_to_budget(&administrator);

    assert_eq!(effective.max_context_tokens, 20_000);
}

#[test]
fn a_deprioritized_source_ranks_after_an_unlisted_one() {
    let policy = RetrievalPolicy::for_role(AgentRole::Implementation);

    assert!(
        policy.source_rank(SourceKind::Repository) < policy.source_rank(SourceKind::Web),
        "the worktree-local role prefers the repository over the web"
    );
    assert!(policy.source_rank(SourceKind::Web) > policy.source_rank(SourceKind::Adr));
}

#[test]
fn required_evidence_is_never_dropped_for_budget() {
    let required = snippet("ev_required", &"x ".repeat(200));
    let supporting = snippet("ev_supporting", &"y ".repeat(2_000));

    let package = ContextPackageBuilder::new("ctx_1", "AS-1", AgentRole::Implementation, 1_000)
        .required(required)
        .supporting(supporting)
        .build(StopReason::SufficientEvidence)
        .expect("required evidence fits");

    assert_eq!(package.required_evidence().len(), 1);
    assert_eq!(package.supporting_evidence().len(), 0);
    assert_eq!(package.omitted_evidence(), ["ev_supporting".to_string()]);
}

#[test]
fn a_package_reports_what_it_omitted_rather_than_dropping_it_silently() {
    let package = ContextPackageBuilder::new("ctx_1", "AS-1", AgentRole::Implementation, 400)
        .supporting(snippet("ev_1", &"a ".repeat(1_000)))
        .supporting(snippet("ev_2", "small"))
        .build(StopReason::SufficientEvidence)
        .expect("package builds");

    assert!(package.render().contains("Omitted for budget"));
    assert_eq!(package.omitted_evidence(), ["ev_1".to_string()]);
}

#[test]
fn required_evidence_over_the_ceiling_is_an_error_not_a_silent_truncation() {
    let error = ContextPackageBuilder::new("ctx_1", "AS-1", AgentRole::Implementation, 100)
        .required(snippet("ev_1", &"z ".repeat(5_000)))
        .build(StopReason::SufficientEvidence)
        .expect_err("an oversized required set must be reported");

    assert!(error.contains("above the 100 ceiling"), "{error}");
}

#[test]
fn the_package_reports_actual_tokens_against_the_request() {
    let package = ContextPackageBuilder::new("ctx_1", "AS-1", AgentRole::Planner, 50_000)
        .summary_line("the scheduler picks a node")
        .required(snippet("ev_1", "fn select_node()"))
        .build(StopReason::SufficientEvidence)
        .expect("package builds");

    assert_eq!(package.requested_tokens(), 50_000);
    assert!(package.actual_tokens() > 0);
    assert!(package.actual_tokens() < package.requested_tokens());
}

#[test]
fn contradictions_reach_the_caller_instead_of_being_resolved() {
    let spec = evidence(
        "ev_spec",
        SourceKind::Specification,
        "docs/specs/gateway.md",
        "timeout = 30s",
        SourceAuthority::AcceptedSpecification,
        950,
    );
    let code = snippet("ev_code", "timeout = 60s");
    let mut contradictions = ContradictionSet::new();
    contradictions.record(Contradiction::new(
        "con_1",
        "gateway_timeout",
        &spec,
        &code,
        "30s versus 60s",
        ContradictionSeverity::High,
    ));

    let package = ContextPackageBuilder::new("ctx_1", "AS-1", AgentRole::Planner, 50_000)
        .required(spec)
        .required(code)
        .contradictions(contradictions)
        .build(StopReason::SufficientEvidence)
        .expect("package builds");

    assert!(package.blocks_autonomous_implementation());
    assert!(package.render().contains("Contradictions"));
}

#[test]
fn a_package_inherits_the_strictest_privacy_of_its_evidence() {
    let package = ContextPackageBuilder::new("ctx_1", "AS-1", AgentRole::Planner, 50_000)
        .required(snippet("ev_1", "internal detail"))
        .build(StopReason::SufficientEvidence)
        .expect("package builds");

    assert_eq!(package.privacy(), Privacy::Internal);
}

#[test]
fn compression_chooses_a_coarser_level_as_the_budget_tightens() {
    let generous = compression::level_for_budget(4, 4_000).expect("a level fits");
    let tight = compression::level_for_budget(40, 4_000).expect("a level fits");

    assert!(
        tight > generous,
        "{tight:?} should be coarser than {generous:?}"
    );
}

#[test]
fn compression_reports_when_no_level_fits() {
    assert!(compression::level_for_budget(1_000, 100).is_none());
}

#[test]
fn compression_levels_climb_to_architecture_and_stop() {
    assert_eq!(
        CompressionLevel::Raw.coarser(),
        Some(CompressionLevel::Symbol)
    );
    assert_eq!(CompressionLevel::Architecture.coarser(), None);
}

#[test]
fn every_role_has_a_policy_and_a_named_strategy() {
    let policies = PolicySet::default();

    for role in ALL_ROLES {
        let policy = policies.policy(role);
        assert_eq!(policy.role(), role);
        assert!(!policy.name().is_empty(), "{}", role.as_str());
    }
}

#[test]
fn a_role_policy_override_replaces_only_that_role() {
    let mut policies = PolicySet::default();
    let narrowed = policies
        .policy(AgentRole::Planner)
        .clone()
        .with_max_context_tokens(1_000);
    policies.set(narrowed);

    assert_eq!(
        policies.policy(AgentRole::Planner).max_context_tokens(),
        1_000
    );
    assert_eq!(
        policies.policy(AgentRole::Reviewer).max_context_tokens(),
        40_000
    );
}
