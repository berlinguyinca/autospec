//! Contradiction detection and authority precedence
//! (spec sections 9, 30, 55.3).

mod rag_support;

use autospec_core::rag::authority::{AuthorityLadder, SourceAuthority};
use autospec_core::rag::contradiction::{
    Contradiction, ContradictionSeverity, ContradictionSet,
};
use autospec_core::rag::source::SourceKind;
use rag_support::evidence;

/// The worked example from specification section 9: the spec says 30s, the code
/// says 60s, telemetry shows requests lasting 45s.
fn timeout_disagreement() -> (
    autospec_core::rag::evidence::Evidence,
    autospec_core::rag::evidence::Evidence,
) {
    let spec = evidence(
        "ev_spec_32",
        SourceKind::Specification,
        "docs/specs/gateway.md",
        "request timeout = 30s",
        SourceAuthority::AcceptedSpecification,
        950,
    );
    let code = evidence(
        "ev_code_54",
        SourceKind::Repository,
        "src/gateway.rs",
        "const TIMEOUT: Duration = Duration::from_secs(60);",
        SourceAuthority::Implementation,
        960,
    );
    (spec, code)
}

#[test]
fn a_spec_code_conflict_is_surfaced_rather_than_resolved() {
    let (spec, code) = timeout_disagreement();
    let mut set = ContradictionSet::new();

    set.record(Contradiction::new(
        "con_123",
        "gateway_request_timeout",
        &spec,
        &code,
        "specification says 30s, implementation uses 60s",
        ContradictionSeverity::High,
    ));

    assert_eq!(set.records().len(), 1);
    assert!(
        set.blocking().is_some(),
        "a high-severity behavioral conflict blocks autonomous implementation"
    );
}

#[test]
fn authority_names_a_preference_without_discarding_the_other_side() {
    let (spec, code) = timeout_disagreement();
    let contradiction = Contradiction::new(
        "con_123",
        "gateway_request_timeout",
        &spec,
        &code,
        "30s versus 60s",
        ContradictionSeverity::High,
    );
    let ladder = AuthorityLadder::default();

    assert_eq!(
        contradiction.preferred_evidence(&ladder),
        Some("ev_spec_32"),
        "the accepted specification outranks the implementation"
    );
    assert_eq!(contradiction.left_evidence(), "ev_spec_32");
    assert_eq!(
        contradiction.right_evidence(),
        "ev_code_54",
        "the lower-authority side is still recorded"
    );
}

#[test]
fn a_project_ladder_override_changes_which_side_is_preferred() {
    let (spec, code) = timeout_disagreement();
    let contradiction = Contradiction::new(
        "con_123",
        "gateway_request_timeout",
        &spec,
        &code,
        "30s versus 60s",
        ContradictionSeverity::High,
    );
    let mut ordered = SourceAuthority::precedence().to_vec();
    ordered.swap(1, 2);
    let ladder = AuthorityLadder::new(ordered).expect("complete ladder");

    assert_eq!(
        contradiction.preferred_evidence(&ladder),
        Some("ev_code_54"),
        "a project that trusts running code first prefers the implementation"
    );
}

#[test]
fn equal_authority_leaves_the_conflict_unresolved() {
    let left = evidence(
        "ev_a",
        SourceKind::Repository,
        "src/a.rs",
        "timeout 30",
        SourceAuthority::Implementation,
        900,
    );
    let right = evidence(
        "ev_b",
        SourceKind::Repository,
        "src/b.rs",
        "timeout 60",
        SourceAuthority::Implementation,
        900,
    );
    let contradiction = Contradiction::new(
        "con_1",
        "timeout",
        &left,
        &right,
        "two call sites disagree",
        ContradictionSeverity::Medium,
    );

    assert_eq!(contradiction.preferred_evidence(&AuthorityLadder::default()), None);
}

#[test]
fn a_docs_versus_code_conflict_is_recorded_at_the_severity_given() {
    let docs = evidence(
        "ev_docs",
        SourceKind::Documentation,
        "README.md",
        "the flag defaults to off",
        SourceAuthority::OfficialDocumentation,
        800,
    );
    let code = evidence(
        "ev_code",
        SourceKind::Repository,
        "src/config.rs",
        "default: true",
        SourceAuthority::Implementation,
        900,
    );
    let mut set = ContradictionSet::new();

    set.record(Contradiction::new(
        "con_docs",
        "flag_default",
        &docs,
        &code,
        "documentation contradicts the implementation",
        ContradictionSeverity::Medium,
    ));

    assert!(set.blocking().is_none(), "medium severity does not block");
    assert_eq!(set.at_least(ContradictionSeverity::Medium).len(), 1);
    assert_eq!(set.at_least(ContradictionSeverity::High).len(), 0);
}

#[test]
fn the_same_conflict_recorded_twice_is_held_once() {
    let (spec, code) = timeout_disagreement();
    let mut set = ContradictionSet::new();
    let make = |id: &str, left, right| {
        Contradiction::new(id, "timeout", left, right, "30 vs 60", ContradictionSeverity::High)
    };

    set.record(make("con_1", &spec, &code));
    set.record(make("con_2", &code, &spec));

    assert_eq!(
        set.records().len(),
        1,
        "the same pair in either order is one contradiction"
    );
}

#[test]
fn contradictions_are_ordered_most_severe_first() {
    let (spec, code) = timeout_disagreement();
    let mut set = ContradictionSet::new();
    set.record(Contradiction::new(
        "con_low",
        "naming",
        &spec,
        &code,
        "a wording difference",
        ContradictionSeverity::Low,
    ));
    set.record(Contradiction::new(
        "con_high",
        "timeout",
        &code,
        &spec,
        "30 versus 60",
        ContradictionSeverity::High,
    ));

    let ordered = set.at_least(ContradictionSeverity::Low);

    assert_eq!(ordered[0].id(), "con_high");
}

#[test]
fn a_contradiction_summary_names_both_sides() {
    let (spec, code) = timeout_disagreement();
    let contradiction = Contradiction::new(
        "con_123",
        "gateway_request_timeout",
        &spec,
        &code,
        "30s versus 60s",
        ContradictionSeverity::High,
    );

    let summary = contradiction.summary();

    assert!(summary.contains("ev_spec_32"), "{summary}");
    assert!(summary.contains("ev_code_54"), "{summary}");
    assert!(summary.contains("high"), "{summary}");
}
