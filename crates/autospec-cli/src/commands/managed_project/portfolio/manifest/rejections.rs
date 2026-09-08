// Every refusal the portfolio plan schema produces, one test per stable code family.
// Shared fixtures (`draft`, `item`, `code_of`, the object ids) come from the parent
// test module; only the mutated inputs differ here.
use super::*;

#[test]
fn rejects_an_unsupported_schema() {
    let error = PortfolioPlan::check_schema("autospec.portfolio-plan.v2")
        .expect_err("a future schema must not be silently accepted");
    assert_eq!(error.code(), PlanViolationCode::SchemaUnsupported);
    assert_eq!(error.exit_code(), 20);
    PortfolioPlan::check_schema(PORTFOLIO_PLAN_SCHEMA).expect("current schema is accepted");
}

#[test]
fn rejects_missing_owner_unknown_owner_and_empty_repository_set() {
    assert_eq!(
        code_of(PortfolioPlan::freeze(PlanDraft::new(
            Some(spec()),
            None,
            vec![repo_alpha()],
            vec![],
        ))),
        PlanViolationCode::OwnerMissing
    );
    assert_eq!(
        code_of(PortfolioPlan::freeze(PlanDraft::new(
            Some(spec()),
            Some("not a owner slug"),
            vec![repo_alpha()],
            vec![],
        ))),
        PlanViolationCode::OwnerInvalid
    );
    assert_eq!(
        code_of(PortfolioPlan::freeze(PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![],
            vec![],
        ))),
        PlanViolationCode::PortfolioSetEmpty
    );
    assert_eq!(
        canonical_owner(" Acme-Corp ").expect("owner canonicalizes"),
        "acme-corp"
    );
    assert!(canonical_owner("-acme").is_err());
    assert!(canonical_owner(&"a".repeat(40)).is_err());
}

#[test]
fn rejects_malformed_or_duplicate_repositories_and_items() {
    assert!(canonical_repository_id("acme").is_err());
    assert!(canonical_repository_id("acme/tool/extra").is_err());
    assert!(canonical_repository_id("acme/.").is_err());
    assert_eq!(
        canonical_repository_id("Acme/Tool").expect("id canonicalizes"),
        "acme/tool"
    );

    assert_eq!(
        code_of(PortfolioPlan::freeze(PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![repo_alpha(), RepositoryFacts::unprobed("ACME/ALPHA")],
            vec![],
        ))),
        PlanViolationCode::RepositoryDuplicate
    );

    assert_eq!(
        code_of(PortfolioPlan::freeze(PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![repo_alpha()],
            vec![
                item("alpha:build", "acme/alpha", &[]),
                item("alpha:build", "acme/alpha", &[]),
            ],
        ))),
        PlanViolationCode::ItemKeyDuplicate
    );

    assert_eq!(
        code_of(PortfolioPlan::freeze(PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![repo_alpha()],
            vec![item("gamma:build", "acme/gamma", &[])],
        ))),
        PlanViolationCode::ItemRepositoryUndeclared
    );
}

#[test]
fn rejects_a_dependency_cycle() {
    let violation = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![repo_alpha()],
        vec![
            item("cycle:a", "acme/alpha", &["cycle:c"]),
            item("cycle:b", "acme/alpha", &["cycle:a"]),
            item("cycle:c", "acme/alpha", &["cycle:b"]),
        ],
    ))
    .expect_err("a cycle must not freeze");
    assert_eq!(violation.code(), PlanViolationCode::DependencyCycle);
    assert_eq!(violation.exit_code(), 35);
    assert!(
        violation.detail().contains(" -> "),
        "the cycle path is reported: {}",
        violation.detail()
    );
}

#[test]
fn rejects_a_self_dependency() {
    let violation = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![repo_alpha()],
        vec![item("loop:a", "acme/alpha", &["loop:a"])],
    ))
    .expect_err("a self-loop must not freeze");
    assert_eq!(violation.code(), PlanViolationCode::EdgeSelfDependency);
    assert_eq!(violation.exit_code(), 32);
}

#[test]
fn rejects_an_edge_pointing_at_nothing_in_the_plan() {
    let violation = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![repo_alpha()],
        vec![item("alpha:build", "acme/alpha", &["alpha:not-planned"])],
    ))
    .expect_err("a dangling edge must not freeze");
    assert_eq!(violation.code(), PlanViolationCode::EdgeReferenceMissing);
    assert_eq!(violation.exit_code(), 33);
}

#[test]
fn rejects_a_local_parent_hosted_by_another_repository() {
    let cross = PlanItem::new(
        "beta:page",
        "acme/beta",
        PlanItemRole::Implementation,
        PlanCompletionPolicy::SelfClosing,
        &[],
        &["alpha:build"],
    )
    .expect("item builds");
    let violation = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![repo_alpha(), repo_beta()],
        vec![item("alpha:build", "acme/alpha", &[]), cross],
    ))
    .expect_err("a local parent must live in the same repository");
    assert_eq!(
        violation.code(),
        PlanViolationCode::LocalParentCrossRepository
    );
    assert_eq!(violation.exit_code(), 34);
}

#[test]
fn rejects_the_same_edge_declared_twice() {
    let duplicated = PlanItem::new(
        "alpha:test",
        "acme/alpha",
        PlanItemRole::Implementation,
        PlanCompletionPolicy::SelfClosing,
        &["alpha:build"],
        &["alpha:build"],
    )
    .expect("item builds");
    let violation = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![repo_alpha()],
        vec![item("alpha:build", "acme/alpha", &[]), duplicated],
    ))
    .expect_err("a duplicated edge must not freeze");
    assert_eq!(violation.code(), PlanViolationCode::EdgeDuplicate);
    assert_eq!(violation.exit_code(), 31);
}

#[test]
fn unknown_and_unavailable_capabilities_are_distinct_refusals() {
    let violation = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![RepositoryFacts::unprobed("acme/alpha")],
        vec![item("alpha:build", "acme/alpha", &[])],
    ))
    .expect_err("an unprobed repository must not host an item");
    assert_eq!(
        violation.code(),
        PlanViolationCode::RepositoryCapabilityUnknown
    );
    assert_eq!(violation.exit_code(), 26);

    let violation = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![RepositoryFacts::unavailable("acme/alpha")],
        vec![item("alpha:build", "acme/alpha", &[])],
    ))
    .expect_err("an unavailable repository must not host an item");
    assert_eq!(
        violation.code(),
        PlanViolationCode::RepositoryCapabilityUnavailable
    );
    assert_eq!(violation.exit_code(), 27);
}
