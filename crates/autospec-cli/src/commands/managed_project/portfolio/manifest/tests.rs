//! Unit tests for the frozen plan: digest stability, canonicalization, and every
//! rejection the materialization gate depends on.

use super::facts::{canonical_owner, canonical_repository_id};
use super::{
    PlanCompletionPolicy, PlanDraft, PlanItem, PlanItemRole, PlanViolation, PlanViolationCode,
    PortfolioPlan, PrimaryScopeSelector, RepositoryCapability, RepositoryFacts,
    PORTFOLIO_PLAN_SCHEMA,
};
use autospec_core::managed_project::SourceSpecIdentity;

// Rejection tests live in a sibling file of their own (issue #3431): the acceptance
// criteria name three invalid-graph cases and the schema refuses fourteen more, and each
// refusal is its own test. They are a child module so they share the fixtures below.
#[path = "rejections.rs"]
mod rejections;
const OID_A: &str = "a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3";
const OID_B: &str = "b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4";
const COMMIT_A: &str = "1111111111111111111111111111111111111111";
const COMMIT_B: &str = "2222222222222222222222222222222222222222";

fn spec() -> SourceSpecIdentity {
    SourceSpecIdentity::new("Acme/spec", "docs/specs/portfolio.md", OID_A)
        .expect("test source spec is valid")
}

fn repo_alpha() -> RepositoryFacts {
    RepositoryFacts::available("Acme/Alpha", COMMIT_A)
}

fn repo_beta() -> RepositoryFacts {
    RepositoryFacts::available("acme/beta", COMMIT_B)
}

fn tracker() -> PlanItem {
    PlanItem::new(
        "tracker/source",
        "Acme/spec",
        PlanItemRole::SourceTracker,
        PlanCompletionPolicy::PortfolioGate,
        &[],
        &[],
    )
    .expect("test tracker is valid")
}

fn item(key: &str, repository: &str, depends_on: &[&str]) -> PlanItem {
    PlanItem::new(
        key,
        repository,
        PlanItemRole::Implementation,
        PlanCompletionPolicy::SelfClosing,
        depends_on,
        &[],
    )
    .expect("test item is valid")
}

/// A two-repository plan with one spec-tracker plus a cross-repository dependency.
fn draft() -> PlanDraft {
    let spec_repo = RepositoryFacts::available("acme/spec", COMMIT_A);
    PlanDraft::new(
        Some(spec()),
        Some("Acme"),
        vec![spec_repo, repo_beta(), repo_alpha()],
        vec![
            item("beta:docs", "acme/beta", &["alpha:build"]),
            item("alpha:build", "acme/alpha", &["tracker/source"]),
            tracker(),
        ],
    )
}

fn frozen() -> PortfolioPlan {
    PortfolioPlan::freeze(draft()).expect("fixture plan freezes")
}

fn code_of(result: Result<PortfolioPlan, PlanViolation>) -> PlanViolationCode {
    match result {
        Ok(_) => panic!("plan was accepted but a rejection was expected"),
        Err(violation) => violation.code(),
    }
}

#[test]
fn freeze_stores_canonical_identity_and_sorted_repositories() {
    let plan = frozen();
    assert_eq!(plan.project_owner(), "acme");
    assert_eq!(plan.portfolio_id().as_str().len(), 64);
    assert_eq!(plan.source_spec().source_spec_blob_oid(), OID_A);
    let ids: Vec<&str> = plan.repositories().iter().map(|r| r.repository()).collect();
    assert_eq!(ids, ["acme/alpha", "acme/beta", "acme/spec"]);
    assert_eq!(
        plan.repositories()[0].observed_revision(),
        Some(COMMIT_A),
        "the probe records the revision capability was observed at"
    );
    let keys: Vec<String> = plan
        .items()
        .iter()
        .map(|i| i.item_key().to_string())
        .collect();
    assert_eq!(keys, ["alpha:build", "beta:docs", "tracker/source"]);
}

#[test]
fn plan_digest_is_stable_and_order_insensitive() {
    let first = frozen();
    let second = frozen();
    assert_eq!(
        first.plan_digest(),
        second.plan_digest(),
        "the same facts must produce the same digest"
    );
    assert_eq!(
        first.canonical_yaml(),
        second.canonical_yaml(),
        "canonical rendering must be byte-identical"
    );
    let reordered = {
        let spec_repo = RepositoryFacts::available("acme/spec", COMMIT_A);
        PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![repo_alpha(), repo_beta(), spec_repo],
            vec![
                tracker(),
                item("alpha:build", "acme/alpha", &["tracker/source"]),
                item("beta:docs", "acme/beta", &["alpha:build"]),
            ],
        )
    };
    let reordered = PortfolioPlan::freeze(reordered).expect("reordered fixture freezes");
    assert_eq!(
        reordered.plan_digest(),
        first.plan_digest(),
        "declaration order must not change the digest"
    );
}

#[test]
fn plan_digest_covers_revision_capability_and_edges() {
    let baseline = frozen();

    let moved_revision = {
        let mut repositories: Vec<RepositoryFacts> = vec![
            RepositoryFacts::available("acme/spec", COMMIT_B),
            repo_beta(),
            RepositoryFacts::available("acme/alpha", COMMIT_B),
        ];
        repositories.sort_by(|left, right| left.repository().cmp(right.repository()));
        PlanDraft::new(
            Some(spec()),
            Some("acme"),
            repositories,
            vec![
                tracker(),
                item("alpha:build", "acme/alpha", &["tracker/source"]),
                item("beta:docs", "acme/beta", &["alpha:build"]),
            ],
        )
    };
    let moved_revision = PortfolioPlan::freeze(moved_revision).expect("fixture freezes");
    assert_ne!(
        moved_revision.plan_digest(),
        baseline.plan_digest(),
        "a different observed revision is a different plan"
    );

    let extra_edge = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![
            RepositoryFacts::available("acme/spec", COMMIT_A),
            repo_beta(),
            repo_alpha(),
        ],
        vec![
            tracker(),
            item("alpha:build", "acme/alpha", &["tracker/source"]),
            item("beta:docs", "acme/beta", &["alpha:build", "tracker/source"]),
        ],
    ))
    .expect("fixture freezes");
    assert_ne!(
        extra_edge.plan_digest(),
        baseline.plan_digest(),
        "an added dependency edge is a different plan"
    );
}

#[test]
fn tampering_with_a_frozen_plan_breaks_its_digest() {
    let mut plan = frozen();
    plan.project_owner = "somebody-else".to_string();
    let error = plan
        .verify_digest()
        .expect_err("an edited owner must not verify");
    assert_eq!(error.code(), PlanViolationCode::DigestMismatch);
    assert_eq!(error.exit_code(), 36);
}

#[test]
fn execution_order_runs_parents_before_dependents() {
    let plan = frozen();
    let order: Vec<String> = plan
        .execution_order()
        .expect("acyclic plan has an order")
        .iter()
        .map(|key| key.to_string())
        .collect();
    assert_eq!(
        order,
        ["tracker/source", "alpha:build", "beta:docs"],
        "parents are emitted before their dependents"
    );
    let position = |key: &str| order.iter().position(|k| k == key).expect("item ordered");
    assert!(position("tracker/source") < position("alpha:build"));
    assert!(position("alpha:build") < position("beta:docs"));
}

#[test]
fn canonical_yaml_is_fully_quoted_and_digest_bearing() {
    let document = frozen().canonical_yaml();
    assert!(document.contains(&format!("schema: \"{PORTFOLIO_PLAN_SCHEMA}\"\n")));
    assert!(document.contains("capability: \"available\""));
    assert!(document.contains("depends_on:\n"));
    assert!(document.contains("local_parents: []\n"));
    assert!(document.contains(&format!("plan_digest: \"{}", frozen().plan_digest())));
    // The only unquoted values are YAML's own null and empty list, both of which stay
    // distinct from the quoted strings `"null"` and `"[]"`.
    for line in document.lines().filter(|line| !line.trim().is_empty()) {
        let value = line
            .split_once(": ")
            .map(|parts| parts.1)
            .unwrap_or_default();
        assert!(
            value.is_empty()
                || value == "[]"
                || value == "null"
                || value.starts_with('"')
                || line.ends_with(':'),
            "unquoted scalar in canonical document: {line}"
        );
    }
}

#[test]
fn an_undeclared_primary_scope_renders_as_yaml_null() {
    // A draft is born with no declared scope; freeze accepts that and records YAML null.
    let plan = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![repo_alpha()],
        vec![item("alpha:build", "acme/alpha", &[])],
    ))
    .expect("scope may be left undeclared at freeze time");
    let document = plan.canonical_yaml();
    assert!(
        document.contains("primary_scope: null\n"),
        "absence is YAML null, not the empty string: {document}"
    );

    let declared = PortfolioPlan::freeze(
        PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![repo_alpha()],
            vec![item("alpha:build", "acme/alpha", &[])],
        )
        .with_primary_scope(PrimaryScopeSelector::Product("acme".to_string())),
    )
    .expect("declared scope resolves");
    assert!(declared
        .canonical_yaml()
        .contains("primary_scope: \"product:acme\"\n"));
    assert_ne!(
        declared.plan_digest(),
        plan.plan_digest(),
        "declaring the scope changes the frozen plan"
    );
}
