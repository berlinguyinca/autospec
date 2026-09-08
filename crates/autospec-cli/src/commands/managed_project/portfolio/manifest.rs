//! Frozen multi-repository portfolio plan manifest (`autospec.portfolio-plan.v1`).
//!
//! The plan is the artifact the planner freezes before anything durable is written:
//! canonical repository facts with tri-state capability, stable item keys, and the
//! local-parent plus dependency edges that materialization later follows. The plan
//! digest is `sha256(namespace || canonical document)` where the canonical document is
//! the YAML rendering with the `plan_digest` key omitted, so a reader can verify a
//! frozen artifact without trusting the digest it carries.
//!
//! Splitting: [`facts`] holds the declared facts and their canonicalization, [`graph`]
//! holds the edge and capability gate, [`yaml`] holds the deterministic rendering. All
//! three are children of this module, so the plan keeps its fields private.
//!
//! Every rejection carries a stable code and a deterministic exit code so a dry run is
//! reproducible across hosts:
//!
//! | code | exit | | code | exit |
//! |---|---|---|---|---|
//! | `SCHEMA_UNSUPPORTED` | 20 | | `ITEM_KEY_DUPLICATE` | 29 |
//! | `OWNER_MISSING` | 21 | | `ITEM_REPOSITORY_UNDECLARED` | 30 |
//! | `OWNER_INVALID` | 22 | | `EDGE_DUPLICATE` | 31 |
//! | `PORTFOLIO_SET_EMPTY` | 23 | | `EDGE_SELF_DEPENDENCY` | 32 |
//! | `REPOSITORY_INVALID` | 24 | | `EDGE_REFERENCE_MISSING` | 33 |
//! | `REPOSITORY_DUPLICATE` | 25 | | `LOCAL_PARENT_CROSS_REPOSITORY` | 34 |
//! | `REPOSITORY_CAPABILITY_UNKNOWN` | 26 | | `DEPENDENCY_CYCLE` | 35 |
//! | `REPOSITORY_CAPABILITY_UNAVAILABLE` | 27 | | `DIGEST_MISMATCH` | 36 |
//! | `ITEM_KEY_INVALID` | 28 | | | |

#[path = "manifest/facts.rs"]
mod facts;
#[path = "manifest/graph.rs"]
mod graph;
#[path = "manifest/tests.rs"]
#[cfg(test)]
mod tests;
#[path = "manifest/yaml.rs"]
mod yaml;

// Re-exported for the dry-run/scope layer, the unit tests, and the future apply path.
#[allow(unused_imports)]
pub use facts::{
    PlanCompletionPolicy, PlanDraft, PlanItem, PlanItemRole, RepositoryCapability, RepositoryFacts,
};

use autospec_core::autonomous::waterfall::sha256_hex;
use autospec_core::managed_project::{ItemKey, PortfolioId, SourceSpecIdentity};
use std::fmt;

/// Schema identifier of a frozen portfolio plan.
pub const PORTFOLIO_PLAN_SCHEMA: &str = "autospec.portfolio-plan.v1";

/// Namespace prefixed to the canonical document before hashing, so a plan digest can
/// never collide with another SHA-256 consumer in the journal.
const DIGEST_NAMESPACE: &[u8] = b"autospec.portfolio-plan.digest.v1";

/// How the plan says its primary portfolio tracker should be chosen. Declared in the
/// draft, frozen verbatim into the plan, and part of the digest: the same facts with a
/// different selector are a different plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryScopeSelector {
    /// The named product owns the portfolio tracker.
    Product(String),
    /// The spec repository's portfolio owns it; no single product does.
    SpecPortfolio,
}

impl PrimaryScopeSelector {
    /// Canonical text form, which is what the digest covers.
    pub fn as_str(&self) -> String {
        match self {
            Self::Product(key) => format!("product:{key}"),
            Self::SpecPortfolio => "spec-portfolio".to_string(),
        }
    }
}

/// The source spec identity is the plan's anchor: the portfolio id is a hash of it, so a
/// plan without one has no identity to freeze.
fn source_spec_missing() -> PlanViolation {
    PlanViolation::new(
        PlanViolationCode::OwnerMissing,
        "plan declares no source spec identity",
    )
}

/// Why a plan was rejected, with the deterministic exit code the planner reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanViolationCode {
    SchemaUnsupported,
    OwnerMissing,
    OwnerInvalid,
    PortfolioSetEmpty,
    RepositoryInvalid,
    RepositoryDuplicate,
    RepositoryCapabilityUnknown,
    RepositoryCapabilityUnavailable,
    ItemKeyInvalid,
    ItemKeyDuplicate,
    ItemRepositoryUndeclared,
    EdgeDuplicate,
    EdgeSelfDependency,
    EdgeReferenceMissing,
    LocalParentCrossRepository,
    DependencyCycle,
    DigestMismatch,
}

impl PlanViolationCode {
    /// Stable machine-readable identifier, independent of the Rust variant name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SchemaUnsupported => "SCHEMA_UNSUPPORTED",
            Self::OwnerMissing => "OWNER_MISSING",
            Self::OwnerInvalid => "OWNER_INVALID",
            Self::PortfolioSetEmpty => "PORTFOLIO_SET_EMPTY",
            Self::RepositoryInvalid => "REPOSITORY_INVALID",
            Self::RepositoryDuplicate => "REPOSITORY_DUPLICATE",
            Self::RepositoryCapabilityUnknown => "REPOSITORY_CAPABILITY_UNKNOWN",
            Self::RepositoryCapabilityUnavailable => "REPOSITORY_CAPABILITY_UNAVAILABLE",
            Self::ItemKeyInvalid => "ITEM_KEY_INVALID",
            Self::ItemKeyDuplicate => "ITEM_KEY_DUPLICATE",
            Self::ItemRepositoryUndeclared => "ITEM_REPOSITORY_UNDECLARED",
            Self::EdgeDuplicate => "EDGE_DUPLICATE",
            Self::EdgeSelfDependency => "EDGE_SELF_DEPENDENCY",
            Self::EdgeReferenceMissing => "EDGE_REFERENCE_MISSING",
            Self::LocalParentCrossRepository => "LOCAL_PARENT_CROSS_REPOSITORY",
            Self::DependencyCycle => "DEPENDENCY_CYCLE",
            Self::DigestMismatch => "DIGEST_MISMATCH",
        }
    }

    /// Exit code a read-only planning run returns for this violation. Written out per
    /// variant on purpose: numbering by declaration order would silently renumber every
    /// code when a variant is inserted.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::SchemaUnsupported => 20,
            Self::OwnerMissing => 21,
            Self::OwnerInvalid => 22,
            Self::PortfolioSetEmpty => 23,
            Self::RepositoryInvalid => 24,
            Self::RepositoryDuplicate => 25,
            Self::RepositoryCapabilityUnknown => 26,
            Self::RepositoryCapabilityUnavailable => 27,
            Self::ItemKeyInvalid => 28,
            Self::ItemKeyDuplicate => 29,
            Self::ItemRepositoryUndeclared => 30,
            Self::EdgeDuplicate => 31,
            Self::EdgeSelfDependency => 32,
            Self::EdgeReferenceMissing => 33,
            Self::LocalParentCrossRepository => 34,
            Self::DependencyCycle => 35,
            Self::DigestMismatch => 36,
        }
    }
}

/// A single rejected plan rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanViolation {
    code: PlanViolationCode,
    detail: String,
}

impl PlanViolation {
    pub fn new(code: PlanViolationCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> PlanViolationCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub fn exit_code(&self) -> i32 {
        self.code.exit_code()
    }
}

impl fmt::Display for PlanViolation {
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

impl std::error::Error for PlanViolation {}

/// A frozen `autospec.portfolio-plan.v1` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioPlan {
    portfolio_id: PortfolioId,
    source_spec: SourceSpecIdentity,
    project_owner: String,
    repositories: Vec<RepositoryFacts>,
    items: Vec<PlanItem>,
    primary_scope: Option<PrimaryScopeSelector>,
    plan_digest: String,
}

impl PortfolioPlan {
    /// Canonicalize the draft and compute its digest. Structural rules (owner, key
    /// grammar, declared repository set, duplicate keys) are enforced here; graph rules
    /// are not, so a reconstructed artifact can be inspected before it is gated by
    /// [`Self::validate`].
    pub fn from_parts(draft: PlanDraft) -> Result<Self, PlanViolation> {
        let source_spec = draft
            .source_spec()
            .cloned()
            .ok_or_else(source_spec_missing)?;
        let owner = draft.project_owner().unwrap_or_default();
        let project_owner = facts::canonical_owner(owner)?;
        let repositories = facts::canonical_repositories(draft.repositories())?;
        let items = facts::canonical_items(draft.items(), &repositories)?;
        let mut plan = Self {
            portfolio_id: source_spec.portfolio_id(),
            source_spec,
            project_owner,
            repositories,
            items,
            primary_scope: draft.primary_scope().cloned(),
            plan_digest: String::new(),
        };
        plan.plan_digest = sha256_hex(&plan.digest_input());
        Ok(plan)
    }

    /// Freeze = canonicalize and gate. Materialization may only consume a plan that
    /// froze, or one reconstructed and then passed through [`Self::validate`] explicitly.
    pub fn freeze(draft: PlanDraft) -> Result<Self, PlanViolation> {
        let plan = Self::from_parts(draft)?;
        plan.validate()?;
        Ok(plan)
    }

    /// Every capability and graph rule that must hold before durable work is planned.
    pub fn validate(&self) -> Result<(), PlanViolation> {
        self.verify_digest()?;
        graph::validate_capabilities(&self.repositories, &self.items)?;
        graph::validate_edges(&self.items)
    }

    /// Recompute the digest from the canonical document and compare it with the digest
    /// the plan carries. Any edit to a frozen field invalidates it.
    pub fn verify_digest(&self) -> Result<(), PlanViolation> {
        let recomputed = sha256_hex(&self.digest_input());
        if recomputed == self.plan_digest {
            return Ok(());
        }
        let detail = format!(
            "frozen digest {} does not match recomputed {}",
            self.plan_digest, recomputed
        );
        Err(PlanViolation::new(
            PlanViolationCode::DigestMismatch,
            detail,
        ))
    }

    /// Reject an artifact declaring a schema this build does not understand.
    pub fn check_schema(schema: &str) -> Result<(), PlanViolation> {
        if schema != PORTFOLIO_PLAN_SCHEMA {
            return Err(PlanViolation::new(
                PlanViolationCode::SchemaUnsupported,
                format!("plan schema `{schema}` is not `{PORTFOLIO_PLAN_SCHEMA}`"),
            ));
        }
        Ok(())
    }

    /// The canonical YAML document, digest key included. Byte-identical for equal plans
    /// regardless of the order the facts were supplied in.
    pub fn canonical_yaml(&self) -> String {
        yaml::render_document(self, true)
    }

    /// Canonical document bytes fed to the digest: the rendering without the digest key.
    fn digest_input(&self) -> Vec<u8> {
        let mut bytes = DIGEST_NAMESPACE.to_vec();
        bytes.push(b'\n');
        bytes.extend_from_slice(yaml::render_document(self, false).as_bytes());
        bytes
    }

    /// Deterministic execution order: dependencies and local parents before dependents.
    /// Ties break on the item key, so the order is stable across runs and hosts.
    pub fn execution_order(&self) -> Result<Vec<ItemKey>, PlanViolation> {
        graph::execution_order(&self.items)
    }

    pub fn portfolio_id(&self) -> &PortfolioId {
        &self.portfolio_id
    }

    pub fn source_spec(&self) -> &SourceSpecIdentity {
        &self.source_spec
    }

    pub fn project_owner(&self) -> &str {
        &self.project_owner
    }

    pub fn repositories(&self) -> &[RepositoryFacts] {
        &self.repositories
    }

    pub fn items(&self) -> &[PlanItem] {
        &self.items
    }

    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }

    /// The declared scope selector, verbatim. `None` means the primary tracker is to be
    /// derived from the item hosts (see `super::select_primary_scope`).
    pub fn primary_scope_selector(&self) -> Option<&PrimaryScopeSelector> {
        self.primary_scope.as_ref()
    }
}
