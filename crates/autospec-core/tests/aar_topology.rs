//! AAR spec section 8: agent topology, structured handoffs and separation of
//! duties.

use autospec_core::aar::classify::{classify, ClassificationInput, Complexity, Risk, TaskClass};
use autospec_core::aar::topology::{
    enforce_separation, preserves_separation_after_fallback, select_topology, AgentRole, Handoff,
    HandoffStyle, RoleAssignment, SeparationPolicy,
};

fn classification(
    complexity: Complexity,
    risk: Risk,
    class: TaskClass,
) -> autospec_core::aar::classify::TaskClassification {
    let mut classification = classify(&ClassificationInput::new("placeholder", "placeholder"));
    classification.complexity = complexity;
    classification.risk = risk;
    classification.task_class = class;
    classification.requires_vision = false;
    classification
}

#[test]
fn trivial_low_risk_work_runs_a_single_agent() {
    let topology = select_topology(&classification(
        Complexity::Trivial,
        Risk::Low,
        TaskClass::Docs,
    ));

    assert!(topology.is_single_agent());
    assert!(!topology.isolated_contexts);
}

#[test]
fn anything_above_trivial_gets_an_independent_reviewer() {
    let topology = select_topology(&classification(
        Complexity::Low,
        Risk::Low,
        TaskClass::Bugfix,
    ));

    assert!(topology.contains(AgentRole::Reviewer));
    assert!(topology.contains(AgentRole::Implementer));
    assert!(topology.isolated_contexts);
}

#[test]
fn medium_complexity_adds_an_explorer_and_a_tester() {
    let topology = select_topology(&classification(
        Complexity::Medium,
        Risk::Low,
        TaskClass::Feature,
    ));

    assert!(topology.contains(AgentRole::Explorer));
    assert!(topology.contains(AgentRole::Tester));
}

#[test]
fn high_complexity_adds_a_planner_and_a_coordinator() {
    let topology = select_topology(&classification(
        Complexity::High,
        Risk::Low,
        TaskClass::Feature,
    ));

    assert!(topology.contains(AgentRole::Planner));
    assert!(topology.contains(AgentRole::Coordinator));
}

#[test]
fn critical_risk_adds_a_security_reviewer() {
    let topology = select_topology(&classification(
        Complexity::Medium,
        Risk::Critical,
        TaskClass::Feature,
    ));

    assert!(topology.contains(AgentRole::SecurityReviewer));
}

#[test]
fn documentation_work_uses_a_documentation_writer_not_an_implementer() {
    let topology = select_topology(&classification(Complexity::Low, Risk::Low, TaskClass::Docs));

    assert!(topology.contains(AgentRole::DocumentationWriter));
    assert!(!topology.contains(AgentRole::Implementer));
}

#[test]
fn ui_work_adds_a_ui_evaluator() {
    let mut ui = classification(Complexity::Medium, Risk::Low, TaskClass::Ui);
    ui.requires_vision = true;

    assert!(select_topology(&ui).contains(AgentRole::UiEvaluator));
}

#[test]
fn handoffs_default_to_structured_summaries_not_transcripts() {
    let topology = select_topology(&classification(
        Complexity::High,
        Risk::High,
        TaskClass::Feature,
    ));

    assert_eq!(topology.handoff, HandoffStyle::StructuredSummary);
}

#[test]
fn separation_holds_when_implementer_and_reviewer_are_distinct() {
    let assignments = [
        RoleAssignment::new(
            AgentRole::Implementer,
            "qwen@q4",
            "coding-local",
            "session-a",
        ),
        RoleAssignment::new(
            AgentRole::Reviewer,
            "qwen@bf16",
            "coding-local",
            "session-b",
        ),
    ];

    let verdict = enforce_separation(&assignments, &SeparationPolicy::default());

    assert!(verdict.satisfied, "{:?}", verdict.violations);
}

#[test]
fn one_model_instance_may_not_both_implement_and_review() {
    let assignments = [
        RoleAssignment::new(
            AgentRole::Implementer,
            "qwen@q4",
            "coding-local",
            "session-a",
        ),
        RoleAssignment::new(AgentRole::Reviewer, "qwen@q4", "coding-local", "session-b"),
    ];

    let verdict = enforce_separation(&assignments, &SeparationPolicy::default());

    assert!(!verdict.satisfied);
    assert!(verdict.violations[0].contains("share model instance"));
}

/// Distinct models sharing a session is the same failure wearing a different
/// hat: the reviewer sees the implementer's context.
#[test]
fn a_shared_session_also_breaks_separation() {
    let assignments = [
        RoleAssignment::new(AgentRole::Implementer, "qwen@q4", "coding-local", "shared"),
        RoleAssignment::new(AgentRole::Reviewer, "other@bf16", "coding-local", "shared"),
    ];

    let verdict = enforce_separation(&assignments, &SeparationPolicy::default());

    assert!(!verdict.satisfied);
    assert!(verdict
        .violations
        .iter()
        .any(|violation| violation.contains("share session")));
}

#[test]
fn security_and_performance_reviewers_are_independent_reviewers_too() {
    for reviewer in [AgentRole::SecurityReviewer, AgentRole::PerformanceReviewer] {
        let assignments = [
            RoleAssignment::new(AgentRole::Implementer, "qwen@q4", "coding-local", "a"),
            RoleAssignment::new(reviewer, "qwen@q4", "coding-local", "b"),
        ];

        assert!(
            !enforce_separation(&assignments, &SeparationPolicy::default()).satisfied,
            "{} must not share the implementer's model",
            reviewer.as_str()
        );
    }
}

#[test]
fn planner_and_reviewer_may_share_a_model_when_policy_permits() {
    let assignments = [
        RoleAssignment::new(AgentRole::Implementer, "small@q4", "coding-local", "a"),
        RoleAssignment::new(AgentRole::Planner, "large@bf16", "coding-local-large", "b"),
        RoleAssignment::new(AgentRole::Reviewer, "large@bf16", "coding-local-large", "c"),
    ];

    let permitted = enforce_separation(&assignments, &SeparationPolicy::default());
    let forbidden = enforce_separation(
        &assignments,
        &SeparationPolicy {
            allow_planner_reviewer_sharing: false,
        },
    );

    assert!(permitted.satisfied, "{:?}", permitted.violations);
    assert!(!forbidden.satisfied);
}

#[test]
fn a_multi_agent_topology_without_a_reviewer_is_a_violation() {
    let assignments = [
        RoleAssignment::new(AgentRole::Implementer, "qwen@q4", "coding-local", "a"),
        RoleAssignment::new(AgentRole::Tester, "qwen@q4", "coding-local", "b"),
    ];

    let verdict = enforce_separation(&assignments, &SeparationPolicy::default());

    assert!(!verdict.satisfied);
    assert!(verdict
        .violations
        .iter()
        .any(|violation| violation.contains("no independent reviewer")));
}

/// The failure mode the spec singles out: a quota failure that quietly moves
/// the reviewer onto the implementer's model.
#[test]
fn a_fallback_that_collapses_the_reviewer_onto_the_implementer_is_rejected() {
    let before = [
        RoleAssignment::new(AgentRole::Implementer, "qwen@q4", "coding-local", "a"),
        RoleAssignment::new(AgentRole::Reviewer, "other@bf16", "coding-local", "b"),
    ];
    let after = [
        RoleAssignment::new(AgentRole::Implementer, "qwen@q4", "coding-local", "a"),
        RoleAssignment::new(AgentRole::Reviewer, "qwen@q4", "coding-local", "b"),
    ];

    let verdict =
        preserves_separation_after_fallback(&before, &after, &SeparationPolicy::default());

    assert!(!verdict.satisfied);
    assert!(verdict.violations[0].contains("fallback weakened separation"));
}

#[test]
fn a_fallback_onto_a_different_model_preserves_separation() {
    let before = [
        RoleAssignment::new(AgentRole::Implementer, "qwen@q4", "coding-local", "a"),
        RoleAssignment::new(AgentRole::Reviewer, "other@bf16", "coding-local", "b"),
    ];
    let after = [
        RoleAssignment::new(AgentRole::Implementer, "qwen@q4", "coding-local", "a"),
        RoleAssignment::new(AgentRole::Reviewer, "third@bf16", "coding-cloud", "c"),
    ];

    assert!(
        preserves_separation_after_fallback(&before, &after, &SeparationPolicy::default())
            .satisfied
    );
}

#[test]
fn a_handoff_carries_summaries_and_artifacts_not_a_transcript() {
    let handoff = Handoff::new(
        AgentRole::Explorer,
        AgentRole::Implementer,
        "The panic originates in queue_parser::parse on an empty document.",
    )
    .with_artifacts([".autospec/findings.md"])
    .with_open_questions(["Should an empty document be an error or an empty queue?"]);

    let markdown = handoff.to_markdown();

    assert!(markdown.starts_with("# Handoff: explorer -> implementer"));
    assert!(markdown.contains(".autospec/findings.md"));
    assert!(markdown.contains("Should an empty document"));
}

#[test]
fn an_empty_handoff_renders_explicit_none_sections() {
    let handoff = Handoff::new(
        AgentRole::Planner,
        AgentRole::Implementer,
        "Plan is in plan.md.",
    );

    let markdown = handoff.to_markdown();

    assert_eq!(markdown.matches("_none_").count(), 2);
}

#[test]
fn every_role_round_trips_through_its_string_form() {
    for role in [
        AgentRole::Coordinator,
        AgentRole::Explorer,
        AgentRole::Planner,
        AgentRole::Implementer,
        AgentRole::Tester,
        AgentRole::Reviewer,
        AgentRole::DocumentationWriter,
        AgentRole::UiEvaluator,
        AgentRole::SecurityReviewer,
        AgentRole::PerformanceReviewer,
    ] {
        assert_eq!(AgentRole::parse(role.as_str()), Some(role));
    }
}
