//! Portfolio-level scope selection and zero-mutation dry-run validation.
//!
//! Two separate guarantees live here.
//!
//! **Scope.** A frozen plan spans several repositories, so it needs one tracker that
//! owns its status. The plan may declare the selector explicitly; without a declaration
//! the primary scope is derived, and a derivation with no candidate or more than one is
//! reported instead of guessed.
//!
//! **Zero mutation.** [`dry_run::validate_plan_dry_run`] proves the read-only property
//! rather than asserting an intent: a witness snapshots the observable state before and
//! after the validation walk and a ledger counts anything the walk tried to write. Any
//! discrepancy — or a witness that could not read what it was asked to — is an error,
//! never a skipped assertion. The types are re-exported below so callers see one flat
//! portfolio API.

// The freeze, scope and dry-run API lands one issue ahead of the materialization step that
// will call it. The unit tests exercise it today; without this the binary build reports the
// whole module as dead.
#![allow(dead_code)]

#[path = "portfolio/dry_run.rs"]
mod dry_run;
#[path = "portfolio/manifest.rs"]
pub mod manifest;
#[path = "portfolio/tests.rs"]
#[cfg(test)]
mod tests;

use self::manifest::{PlanViolationCode, PortfolioPlan};
use autospec_core::managed_project::{PortfolioId, ProductKey};
use std::collections::BTreeSet;
use std::fmt;

// Re-exported so callers see one flat portfolio API; the materialization step that
// consumes them lands in a later issue.
#[allow(unused_imports)]
pub use self::dry_run::{
    validate_plan_dry_run, DryRunError, DryRunReport, DryRunTarget, MutationLedger,
    MutationWitness, NoopWitness, TreeWitness,
};
pub use self::manifest::PrimaryScopeSelector;

/// The resolved primary scope of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryScope {
    Product(ProductKey),
    SpecPortfolio(PortfolioId),
}

impl PrimaryScope {
    pub fn as_str(&self) -> String {
        match self {
            Self::Product(key) => format!("product:{key}"),
            Self::SpecPortfolio(id) => format!("spec-portfolio:{id}"),
        }
    }
}

/// Why no single primary scope could be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeViolationCode {
    /// Nothing could own the tracker: no declaration and no item-hosting repository.
    PrimaryScopeUndeclared,
    /// Several products host items and no selector was declared.
    PrimaryScopeAmbiguous,
    /// The declared product does not host any item of this plan.
    PrimaryScopeUnknown,
}

impl ScopeViolationCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryScopeUndeclared => "PRIMARY_SCOPE_UNDECLARED",
            Self::PrimaryScopeAmbiguous => "PRIMARY_SCOPE_AMBIGUOUS",
            Self::PrimaryScopeUnknown => "PRIMARY_SCOPE_UNKNOWN",
        }
    }

    /// Exit codes 40..42, deliberately in a range disjoint from
    /// [`PlanViolationCode::exit_code`] so a single report can carry either family.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::PrimaryScopeUndeclared => 40,
            Self::PrimaryScopeAmbiguous => 41,
            Self::PrimaryScopeUnknown => 42,
        }
    }
}

/// A rejected scope resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeViolation {
    code: ScopeViolationCode,
    detail: String,
}

impl ScopeViolation {
    pub fn new(code: ScopeViolationCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> ScopeViolationCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub fn exit_code(&self) -> i32 {
        self.code.exit_code()
    }
}

impl fmt::Display for ScopeViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} exit={} {}",
            self.code.as_str(),
            self.exit_code(),
            self.detail
        )
    }
}

impl std::error::Error for ScopeViolation {}

/// Resolve the primary scope: the declared selector when present, otherwise the single
/// product whose repositories host items.
pub fn select_primary_scope(plan: &PortfolioPlan) -> Result<PrimaryScope, ScopeViolation> {
    let hosts = item_host_owners(plan);
    match plan.primary_scope_selector() {
        Some(PrimaryScopeSelector::SpecPortfolio) => {
            Ok(PrimaryScope::SpecPortfolio(plan.portfolio_id().clone()))
        }
        Some(PrimaryScopeSelector::Product(key)) if hosts.contains(key) => {
            let product = product_key(key)?;
            Ok(PrimaryScope::Product(product))
        }
        Some(PrimaryScopeSelector::Product(key)) => Err(ScopeViolation::new(
            ScopeViolationCode::PrimaryScopeUnknown,
            format!(
                "declared primary product `{key}` hosts no item of this plan; hosts are {}",
                summarize(&hosts)
            ),
        )),
        None if hosts.len() == 1 => {
            let owner = hosts
                .iter()
                .next()
                .expect("length was checked immediately above");
            Ok(PrimaryScope::Product(product_key(owner)?))
        }
        None if hosts.is_empty() => Err(ScopeViolation::new(
            ScopeViolationCode::PrimaryScopeUndeclared,
            "no repository hosts an item, so nothing can own the portfolio tracker",
        )),
        None => Err(ScopeViolation::new(
            ScopeViolationCode::PrimaryScopeAmbiguous,
            format!(
                "{} products host items and no primary scope was declared: {}",
                hosts.len(),
                summarize(&hosts)
            ),
        )),
    }
}

/// A host owner that is not a legal product key is a scope problem, not a silent skip.
fn product_key(owner: &str) -> Result<ProductKey, ScopeViolation> {
    ProductKey::new(owner.to_string()).map_err(|error| {
        ScopeViolation::new(ScopeViolationCode::PrimaryScopeUnknown, error.to_string())
    })
}

/// Owners of the repositories that host at least one item. `PortfolioPlan::freeze`
/// canonicalizes every repository id to lowercase `owner/name`, so the first segment is
/// the owner; the case-fold here is belt-and-braces for a plan built by `from_parts`.
fn item_host_owners(plan: &PortfolioPlan) -> BTreeSet<String> {
    plan.items()
        .iter()
        .filter_map(|item| item.repository().split('/').next())
        .filter(|owner| !owner.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn summarize(hosts: &BTreeSet<String>) -> String {
    if hosts.is_empty() {
        return "(none)".to_string();
    }
    hosts.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// The codes a caller can expect from a read-only planning run, for docs and CI greps.
pub fn documented_exit_codes() -> Vec<(&'static str, i32)> {
    let plan_codes: [PlanViolationCode; 17] = [
        PlanViolationCode::SchemaUnsupported,
        PlanViolationCode::OwnerMissing,
        PlanViolationCode::OwnerInvalid,
        PlanViolationCode::PortfolioSetEmpty,
        PlanViolationCode::RepositoryInvalid,
        PlanViolationCode::RepositoryDuplicate,
        PlanViolationCode::RepositoryCapabilityUnknown,
        PlanViolationCode::RepositoryCapabilityUnavailable,
        PlanViolationCode::ItemKeyInvalid,
        PlanViolationCode::ItemKeyDuplicate,
        PlanViolationCode::ItemRepositoryUndeclared,
        PlanViolationCode::EdgeDuplicate,
        PlanViolationCode::EdgeSelfDependency,
        PlanViolationCode::EdgeReferenceMissing,
        PlanViolationCode::LocalParentCrossRepository,
        PlanViolationCode::DependencyCycle,
        PlanViolationCode::DigestMismatch,
    ];
    let scope_codes: [ScopeViolationCode; 3] = [
        ScopeViolationCode::PrimaryScopeUndeclared,
        ScopeViolationCode::PrimaryScopeAmbiguous,
        ScopeViolationCode::PrimaryScopeUnknown,
    ];
    let mut codes: Vec<(&'static str, i32)> = plan_codes
        .iter()
        .map(|code| (code.as_str(), code.exit_code()))
        .collect();
    codes.extend(
        scope_codes
            .iter()
            .map(|code| (code.as_str(), code.exit_code())),
    );
    codes
}
