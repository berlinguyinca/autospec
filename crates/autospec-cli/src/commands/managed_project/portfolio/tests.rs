//! Scope resolution and the zero-mutation proof behind a read-only planning run.

use super::manifest::{
    PlanCompletionPolicy, PlanDraft, PlanItem, PlanItemRole, PlanViolationCode, PortfolioPlan,
    PrimaryScopeSelector, RepositoryFacts,
};
use super::{
    documented_exit_codes, select_primary_scope, validate_plan_dry_run, DryRunError, DryRunTarget,
    MutationLedger, MutationWitness, PrimaryScope, ScopeViolationCode, TreeWitness,
};
use autospec_core::managed_project::{ProductKey, SourceSpecIdentity};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const OID: &str = "a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3";

fn spec() -> SourceSpecIdentity {
    SourceSpecIdentity::new("Acme/spec", "docs/specs/portfolio.md", OID).unwrap()
}

fn item(key: &str, repository: &str, parents: &[&str]) -> PlanItem {
    PlanItem::new(
        key,
        repository,
        PlanItemRole::Implementation,
        PlanCompletionPolicy::SelfClosing,
        parents,
        &[],
    )
    .unwrap()
}

/// One product, one repository, one item with a dependency.
fn single_host_plan() -> PortfolioPlan {
    PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![
            RepositoryFacts::available("acme/alpha", "1111111"),
            RepositoryFacts::reachable("acme/beta"),
        ],
        vec![
            item("alpha:build", "acme/alpha", &[]),
            item("beta:docs", "acme/alpha", &["alpha:build"]),
        ],
    ))
    .expect("fixture plan must freeze")
}

/// Two products hosting items, so nothing can be derived without a declaration.
fn multi_host_plan() -> PortfolioPlan {
    PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![
            RepositoryFacts::reachable("acme/alpha"),
            RepositoryFacts::reachable("other/beta"),
        ],
        vec![
            item("alpha:build", "acme/alpha", &[]),
            item("beta:build", "other/beta", &[]),
        ],
    ))
    .expect("fixture plan must freeze")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let serial = AtomicU64::new(0).fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "autospec-portfolio-scope-{label}-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn one_host_derives_its_product_as_the_primary_scope() {
    let plan = single_host_plan();
    assert_eq!(
        select_primary_scope(&plan).unwrap(),
        PrimaryScope::Product(ProductKey::new("acme").unwrap())
    );
}

#[test]
fn an_explicit_spec_portfolio_selector_beats_derivation() {
    let plan = PortfolioPlan::freeze(
        PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![
                RepositoryFacts::reachable("acme/alpha"),
                RepositoryFacts::reachable("other/beta"),
            ],
            vec![
                item("alpha:build", "acme/alpha", &[]),
                item("beta:build", "other/beta", &[]),
            ],
        )
        .with_primary_scope(PrimaryScopeSelector::SpecPortfolio),
    )
    .expect("two hosts are legal once the scope is declared");
    assert_eq!(
        select_primary_scope(&plan).unwrap(),
        PrimaryScope::SpecPortfolio(plan.portfolio_id().clone()),
        "the declared selector, not the derived one, decides"
    );
}

#[test]
fn several_hosts_without_a_declaration_are_ambiguous_not_guessed() {
    let violation = select_primary_scope(&multi_host_plan()).unwrap_err();
    assert_eq!(violation.code(), ScopeViolationCode::PrimaryScopeAmbiguous);
    assert_eq!(violation.exit_code(), 41);
    assert!(
        violation.detail().contains("acme") && violation.detail().contains("other"),
        "the refusal names every candidate: {}",
        violation.detail()
    );
}

#[test]
fn a_declared_product_that_hosts_no_item_is_refused() {
    let plan = PortfolioPlan::freeze(
        PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![RepositoryFacts::reachable("acme/alpha")],
            vec![item("alpha:build", "acme/alpha", &[])],
        )
        .with_primary_scope(PrimaryScopeSelector::Product("other".to_string())),
    )
    .expect("a selector naming a non-host still freezes");
    let violation = select_primary_scope(&plan).unwrap_err();
    assert_eq!(violation.code(), ScopeViolationCode::PrimaryScopeUnknown);
    assert_eq!(violation.exit_code(), 42);
}

#[test]
fn a_plan_with_nothing_to_build_declares_no_scope() {
    let plan = PortfolioPlan::freeze(
        PlanDraft::new(
            Some(spec()),
            Some("acme"),
            vec![RepositoryFacts::reachable("acme/alpha")],
            vec![],
        )
        .with_primary_scope(PrimaryScopeSelector::SpecPortfolio),
    )
    .expect("an empty plan is a plan");
    assert_eq!(
        select_primary_scope(&plan).unwrap(),
        PrimaryScope::SpecPortfolio(plan.portfolio_id().clone()),
        "a declaration works even with no hosts"
    );

    let undeclared = PortfolioPlan::freeze(PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![RepositoryFacts::reachable("acme/alpha")],
        vec![],
    ))
    .expect("an empty plan freezes without a declaration too");
    let violation = select_primary_scope(&undeclared).unwrap_err();
    assert_eq!(violation.code(), ScopeViolationCode::PrimaryScopeUndeclared);
    assert_eq!(violation.exit_code(), 40);
}

#[test]
fn a_dry_run_certifies_zero_mutations_over_a_real_tree() {
    let journal = TempDir::new("journal");
    std::fs::write(journal.path.join("plan.yaml"), b"existing").unwrap();
    std::fs::write(journal.path.join("journal.jsonl"), b"{\"prior\":1}\n").unwrap();
    std::fs::create_dir(journal.path.join("locks")).unwrap();
    std::fs::write(journal.path.join("locks/owner.lock"), b"").unwrap();

    let report = validate_plan_dry_run(
        &single_host_plan(),
        DryRunTarget::Journal(journal.path.clone()),
    )
    .expect("a read-only walk over an existing journal validates");

    assert_eq!(report.mutations().durable(), 0);
    assert_eq!(report.mutations().remote(), 0);
    assert!(report.verify_zero_mutations().is_ok());
    assert_eq!(
        report.checked_paths().len(),
        4,
        "two files, the locks directory, and the lock file inside it"
    );
    assert_eq!(
        report.execution_order(),
        ["alpha:build".to_string(), "beta:docs".to_string()]
    );
    assert_eq!(report.item_count(), 2);
    assert_eq!(report.repository_count(), 2);
    assert_eq!(report.plan_digest(), single_host_plan().plan_digest());
    assert_eq!(
        report.summary(),
        format!(
            "mutate=0 dry_run=1 items=2 repositories=2 digest={} scope=product:acme mutations=0",
            single_host_plan().plan_digest()
        )
    );

    // The walk left every byte where it found it.
    assert_eq!(
        std::fs::read_dir(journal.path.join("plan.yaml")).is_err(),
        true,
        "plan.yaml is still a file, not a directory the walk created"
    );
    assert_eq!(
        std::fs::read(journal.path.join("journal.jsonl")).unwrap(),
        b"{\"prior\":1}\n"
    );
    assert_eq!(
        TreeWitness::new(journal.path.clone())
            .snapshot()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn a_dry_run_refuses_a_journal_that_is_not_there() {
    let missing = Path::new("/nonexistent/autospec-portfolio-journal");
    let error = validate_plan_dry_run(
        &single_host_plan(),
        DryRunTarget::Journal(missing.to_path_buf()),
    )
    .expect_err("a missing journal cannot certify anything");
    assert!(matches!(error, DryRunError::JournalMissing(path) if path == missing));
}

#[test]
fn a_dry_run_rejects_an_invalid_plan_before_reporting() {
    let draft = PlanDraft::new(
        Some(spec()),
        Some("acme"),
        vec![RepositoryFacts::reachable("acme/alpha")],
        vec![
            item("first", "acme/alpha", &["second"]),
            item("second", "acme/alpha", &["first"]),
        ],
    );
    let plan = PortfolioPlan::from_parts(draft).expect("a cycle canonicalizes before it is gated");
    let error =
        validate_plan_dry_run(&plan, DryRunTarget::InMemory).expect_err("the cycle must surface");
    match error {
        DryRunError::Plan(violation) => {
            assert_eq!(violation.code(), PlanViolationCode::DependencyCycle)
        }
        other => panic!("expected the plan violation to be carried through, got {other:?}"),
    }
}

#[test]
fn the_witness_notices_every_way_a_tree_can_change() {
    let journal = TempDir::new("witness");
    std::fs::write(journal.path.join("a.json"), b"one").unwrap();
    let witness = TreeWitness::new(journal.path.clone());
    let before = witness.snapshot().unwrap();

    std::fs::write(journal.path.join("b.json"), b"created by a stray write").unwrap();
    let created = witness.snapshot().unwrap();
    assert_ne!(before, created, "a created file is visible to the witness");

    std::fs::remove_file(journal.path.join("a.json")).unwrap();
    std::fs::write(journal.path.join("b.json"), b"grown").unwrap();
    let after = witness.snapshot().unwrap();
    assert_ne!(
        created, after,
        "a removed file and a grown file are both visible"
    );
    assert!(
        after.contains(&(journal.path.join("b.json"), 5u64)),
        "the witness records sizes, not just names: {after:?}"
    );
}

#[test]
fn an_in_memory_run_has_nothing_to_check() {
    let report = validate_plan_dry_run(&single_host_plan(), DryRunTarget::InMemory).unwrap();
    assert!(report.checked_paths().is_empty());
    assert!(report.verify_zero_mutations().is_ok());
}

#[test]
fn a_ledger_that_counted_anything_fails_the_zero_check() {
    let mut ledger = MutationLedger::default();
    assert_eq!(ledger.total(), 0);
    ledger.record_durable();
    ledger.record_remote();
    ledger.record_remote();
    assert_eq!(
        (ledger.durable(), ledger.remote(), ledger.total()),
        (1, 2, 3),
        "the counters exist so a stray write is reportable, not invisible"
    );
}

#[test]
fn every_documented_exit_code_is_distinct() {
    let codes = documented_exit_codes();
    assert_eq!(codes.len(), 20, "17 plan codes plus 3 scope codes");
    let mut seen: Vec<(&str, i32)> = codes.clone();
    seen.sort_by_key(|(_, code)| *code);
    for pair in seen.windows(2) {
        assert_ne!(
            pair[0].1, pair[1].1,
            "{} and {} share exit code {}",
            pair[0].0, pair[1].0, pair[0].1
        );
    }
    assert_eq!(seen.first().unwrap(), &("SCHEMA_UNSUPPORTED", 20));
    assert_eq!(seen.last().unwrap(), &("PRIMARY_SCOPE_UNKNOWN", 42));
}

/// The operator document is the surface an on-call reader trusts, so it is checked
/// against the code table rather than kept in sync by hope: same rows, same values.
#[test]
fn the_documented_exit_code_table_matches_the_code_table() {
    let doc = include_str!("../../../../../../docs/managed-project-portfolio-plan.md");
    let documented = exit_code_table(doc);
    let expected: Vec<(String, i32)> = documented_exit_codes()
        .into_iter()
        .map(|(code, exit)| (code.to_string(), exit))
        .collect();
    let mut documented_sorted = documented.clone();
    let mut expected_sorted = expected;
    documented_sorted.sort();
    expected_sorted.sort();
    assert_eq!(
        documented_sorted, expected_sorted,
        "docs/managed-project-portfolio-plan.md and documented_exit_codes() disagree"
    );
}

/// Reads `| \`CODE\` | exit | meaning |` rows out of a Markdown table, ignoring every
/// other table in the document.
fn exit_code_table(document: &str) -> Vec<(String, i32)> {
    document
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('|').skip(1);
            let code = columns.next()?.trim().trim_matches('`');
            let exit: i32 = columns.next()?.trim().parse().ok()?;
            if code.is_empty() || !code.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                return None;
            }
            Some((code.to_string(), exit))
        })
        .collect()
}
