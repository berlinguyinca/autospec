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

// ---------------------------------------------------------------------------
// Portfolio lease: a repository-hosted optimistic lock that fences concurrent
// portfolio mutations.
//
// Two hosts must not mutate one portfolio at the same time. The lease lives at
// `refs/heads/autospec-state/portfolio/<id>` in the source repository: each
// generation is a git commit whose message carries the lease payload, and the
// commit chain is the audit chain. Advancing the lease is a compare-and-swap
// fast-forward over the observed ref tip, so only one contender can win a
// generation. A stale holder re-reads before every mutation, sees a lease that
// is not its own, and is fenced: it may reconcile read-only but never write.
//
// The manager is generic over [`PortfolioRefStore`] so it stays transport-free
// and unit-testable; the adapter over the shared transport lands in a later
// issue. The coordination ref is never force-updated.
// ---------------------------------------------------------------------------

/// Coordination ref prefix: the lease ref is `refs/heads/autospec-state/portfolio/<id>`.
pub const PORTFOLIO_LEASE_REF_PREFIX: &str = "refs/heads/autospec-state/portfolio/";
/// Commit-message schema tag; the lease JSON payload follows on the next line.
pub const PORTFOLIO_LEASE_SCHEMA: &str = "autospec.portfolio-lease.v1";
/// Lease duration in seconds; a holder renews while more than one-third remain.
pub const PORTFOLIO_LEASE_TTL_SECONDS: u64 = 1_800;
/// The empty git tree object; a lease commit carries no blob content.
pub const PORTFOLIO_LEASE_EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A failure of the portfolio lease protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortfolioLeaseError {
    /// The observed lease is not the caller's (foreign holder) or the caller's own lease has
    /// expired: the caller is fenced and must not mutate.
    Fenced,
    /// The observed generation differs from the caller's: fenced.
    StaleGeneration { observed: u64, expected: u64 },
    /// A takeover was attempted but the lease is still live.
    NotExpired,
    /// The observed plan digest differs from the caller's.
    PlanDigestMismatch { expected: String, observed: String },
    /// The lease payload did not parse or failed validation.
    Integrity(String),
    /// A compare-and-swap update was rejected by the store (ref moved or already exists).
    CasConflict,
    /// The underlying ref store reported a transport error.
    Transport(String),
    /// A local (non-remote) failure, e.g. reading the entropy source.
    Local(String),
}

impl PortfolioLeaseError {
    /// Fenced outcomes: the caller holds a stale view and may only reconcile read-only.
    pub fn is_fenced(&self) -> bool {
        matches!(
            self,
            Self::Fenced | Self::StaleGeneration { .. } | Self::NotExpired | Self::CasConflict
        )
    }
}

impl fmt::Display for PortfolioLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fenced => {
                write!(formatter, "fenced by a live or mismatched portfolio lease")
            }
            Self::StaleGeneration { observed, expected } => write!(
                formatter,
                "stale portfolio lease generation: observed {observed}, expected {expected}"
            ),
            Self::NotExpired => {
                write!(formatter, "portfolio lease is still live; takeover refused")
            }
            Self::PlanDigestMismatch { expected, observed } => write!(
                formatter,
                "portfolio plan digest mismatch: expected {expected}, observed {observed}"
            ),
            Self::Integrity(detail) => {
                write!(formatter, "portfolio lease integrity error: {detail}")
            }
            Self::CasConflict => write!(formatter, "portfolio lease compare-and-swap was rejected"),
            Self::Transport(detail) => {
                write!(formatter, "portfolio lease transport error: {detail}")
            }
            Self::Local(detail) => write!(formatter, "portfolio lease local error: {detail}"),
        }
    }
}

impl std::error::Error for PortfolioLeaseError {}

/// A repository-hosted portfolio lease, stored as a git commit message on
/// `refs/heads/autospec-state/portfolio/<id>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioLease {
    portfolio_id: String,
    plan_digest: String,
    holder: String,
    generation: u64,
    expires_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_checkpoint: Option<String>,
}

impl PortfolioLease {
    /// Build a lease. Callers normally go through the manager functions, which set the
    /// generation, expiry and digest; this is exposed for tests and the transport adapter.
    pub fn new(
        portfolio_id: String,
        plan_digest: String,
        holder: String,
        generation: u64,
        expires_at: u64,
        last_checkpoint: Option<String>,
    ) -> Self {
        Self {
            portfolio_id,
            plan_digest,
            holder,
            generation,
            expires_at,
            last_checkpoint,
        }
    }

    pub fn portfolio_id(&self) -> &str {
        &self.portfolio_id
    }

    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }

    pub fn holder(&self) -> &str {
        &self.holder
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub fn last_checkpoint(&self) -> Option<&str> {
        self.last_checkpoint.as_deref()
    }

    /// A lease is expired once `now` reaches its `expires_at` bound.
    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    /// Renewal is due once less than one-third of the lease duration remains.
    pub fn renewal_due(&self, now: u64) -> bool {
        now >= self
            .expires_at
            .saturating_sub(PORTFOLIO_LEASE_TTL_SECONDS / 3)
    }

    /// Whether this lease is held by the given holder at the given generation.
    pub fn holds(&self, holder: &str, generation: u64) -> bool {
        self.holder == holder && self.generation == generation
    }

    /// The commit message that carries this lease: the schema tag, then the JSON payload.
    pub fn commit_message(&self) -> String {
        format!(
            "{PORTFOLIO_LEASE_SCHEMA}\n{}",
            serde_json::to_string(self).expect("lease fields are serializable primitives")
        )
    }
}

/// The git-data-plane operations the lease protocol needs, abstracted so the manager stays
/// transport-free and unit-testable. A real adapter over the shared transport lands in a
/// later issue; the manager never names a concrete transport.
pub trait PortfolioRefStore {
    /// Read the sha of the ref tip; `None` when the ref is absent.
    fn read_ref(
        &mut self,
        repository: &str,
        ref_name: &str,
    ) -> Result<Option<String>, PortfolioLeaseError>;
    /// Read a commit's message body.
    fn read_commit_message(
        &mut self,
        repository: &str,
        sha: &str,
    ) -> Result<String, PortfolioLeaseError>;
    /// Create a commit with the given message, tree and parents; returns the new sha.
    fn create_commit(
        &mut self,
        repository: &str,
        message: &str,
        tree: &str,
        parents: &[String],
    ) -> Result<String, PortfolioLeaseError>;
    /// Create a ref (valid only while the ref is absent).
    fn create_ref(
        &mut self,
        repository: &str,
        ref_name: &str,
        sha: &str,
    ) -> Result<(), PortfolioLeaseError>;
    /// Fast-forward a ref to `sha`; rejected unless the current tip is an ancestor of it.
    fn fast_forward_ref(
        &mut self,
        repository: &str,
        ref_name: &str,
        sha: &str,
    ) -> Result<(), PortfolioLeaseError>;
}

/// What `acquire_lease` observed: the caller now holds the lease, or it is held or expired by
/// the ref's current lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseObservation {
    /// The caller created the lease and now holds it.
    Acquired(PortfolioLease),
    /// A live lease is held (by the caller or a foreign host); the caller must not mutate.
    Held { observed: PortfolioLease },
    /// The lease has expired; a caller may take it over with [`take_over_expired_lease`].
    Expired { observed: PortfolioLease },
}

/// The coordination ref for a portfolio id.
pub fn portfolio_lease_ref_name(portfolio_id: &str) -> String {
    format!("{PORTFOLIO_LEASE_REF_PREFIX}{portfolio_id}")
}

/// Classify a parsed lease against `now`: expired, otherwise held.
fn classify_observed(observed: PortfolioLease, now: u64) -> LeaseObservation {
    if observed.is_expired(now) {
        LeaseObservation::Expired { observed }
    } else {
        LeaseObservation::Held { observed }
    }
}

/// The read-only fence. The observed lease must still be the caller's holder, at the caller's
/// generation, and not expired; any mismatch fences with zero mutations.
fn check_fence(
    observed: &PortfolioLease,
    expected: &PortfolioLease,
    now: u64,
) -> Result<(), PortfolioLeaseError> {
    if observed.holder != expected.holder {
        return Err(PortfolioLeaseError::Fenced);
    }
    if observed.generation != expected.generation {
        return Err(PortfolioLeaseError::StaleGeneration {
            observed: observed.generation,
            expected: expected.generation,
        });
    }
    if observed.is_expired(now) {
        return Err(PortfolioLeaseError::Fenced);
    }
    Ok(())
}

/// Parse and strictly validate a commit message into a lease.
fn parse_lease(message: &str) -> Result<PortfolioLease, PortfolioLeaseError> {
    let body = message
        .strip_prefix(PORTFOLIO_LEASE_SCHEMA)
        .and_then(|rest| rest.strip_prefix('\n'))
        .ok_or_else(|| {
            PortfolioLeaseError::Integrity("commit message is not a portfolio lease".to_string())
        })?;
    let lease: PortfolioLease = serde_json::from_str(body).map_err(|error| {
        PortfolioLeaseError::Integrity(format!("lease payload is not valid JSON: {error}"))
    })?;
    validate_lease(&lease)?;
    Ok(lease)
}

fn validate_lease(lease: &PortfolioLease) -> Result<(), PortfolioLeaseError> {
    if lease.portfolio_id.is_empty() {
        return Err(PortfolioLeaseError::Integrity(
            "empty portfolio_id".to_string(),
        ));
    }
    if !is_sha256_hex(&lease.plan_digest) {
        return Err(PortfolioLeaseError::Integrity(
            "plan_digest is not a 64-character hex digest".to_string(),
        ));
    }
    if !is_holder_hex(&lease.holder) {
        return Err(PortfolioLeaseError::Integrity(
            "holder is not a 32-character hex id".to_string(),
        ));
    }
    if lease.generation == 0 {
        return Err(PortfolioLeaseError::Integrity(
            "generation must be greater than zero".to_string(),
        ));
    }
    if lease.expires_at == 0 {
        return Err(PortfolioLeaseError::Integrity(
            "expires_at must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

/// Read and parse the current lease on the coordination ref, if any.
pub fn read_current_lease<S: PortfolioRefStore>(
    store: &mut S,
    repository: &str,
    portfolio: &PortfolioId,
) -> Result<Option<PortfolioLease>, PortfolioLeaseError> {
    let ref_name = portfolio_lease_ref_name(portfolio.as_str());
    match store.read_ref(repository, &ref_name)? {
        None => Ok(None),
        Some(sha) => {
            let message = store.read_commit_message(repository, &sha)?;
            Ok(Some(parse_lease(&message)?))
        }
    }
}

/// Try to acquire the lease. When the ref is absent, create the generation-1 lease and point
/// the ref at it. When the ref exists, parse and classify the current lease (held or
/// expired). On a compare-and-swap conflict (a second host created the ref in the same
/// instant) re-read and classify the foreign lease rather than forcing.
pub fn acquire_lease<S: PortfolioRefStore>(
    store: &mut S,
    repository: &str,
    portfolio: &PortfolioId,
    plan_digest: &str,
    holder: &str,
    now: u64,
) -> Result<LeaseObservation, PortfolioLeaseError> {
    let ref_name = portfolio_lease_ref_name(portfolio.as_str());
    match store.read_ref(repository, &ref_name)? {
        Some(sha) => {
            let message = store.read_commit_message(repository, &sha)?;
            let observed = parse_lease(&message)?;
            Ok(classify_observed(observed, now))
        }
        None => {
            let lease = PortfolioLease::new(
                portfolio.as_str().to_string(),
                plan_digest.to_string(),
                holder.to_string(),
                1,
                now + PORTFOLIO_LEASE_TTL_SECONDS,
                None,
            );
            let sha = store.create_commit(
                repository,
                &lease.commit_message(),
                PORTFOLIO_LEASE_EMPTY_TREE,
                &[],
            )?;
            match store.create_ref(repository, &ref_name, &sha) {
                Ok(()) => Ok(LeaseObservation::Acquired(lease)),
                Err(original) => match store.read_ref(repository, &ref_name)? {
                    None => Err(original),
                    Some(tip) if tip == sha => Ok(LeaseObservation::Acquired(lease)),
                    Some(tip) => {
                        let message = store.read_commit_message(repository, &tip)?;
                        let observed = parse_lease(&message)?;
                        Ok(classify_observed(observed, now))
                    }
                },
            }
        }
    }
}

/// Renew a lease the caller believes it holds. Re-read the coordination ref, fence on any
/// mismatch (foreign holder, different generation, or expiry) with zero mutations, then
/// advance to the next generation with a fresh expiry and an optional new checkpoint.
pub fn renew_lease<S: PortfolioRefStore>(
    store: &mut S,
    repository: &str,
    ours: &PortfolioLease,
    now: u64,
    new_checkpoint: Option<&str>,
) -> Result<PortfolioLease, PortfolioLeaseError> {
    let ref_name = portfolio_lease_ref_name(ours.portfolio_id());
    let base_tip = match store.read_ref(repository, &ref_name)? {
        Some(sha) => sha,
        None => return Err(PortfolioLeaseError::Fenced),
    };
    let message = store.read_commit_message(repository, &base_tip)?;
    let observed = parse_lease(&message)?;
    check_fence(&observed, ours, now)?;
    let next = PortfolioLease::new(
        ours.portfolio_id().to_string(),
        ours.plan_digest().to_string(),
        ours.holder().to_string(),
        ours.generation + 1,
        now + PORTFOLIO_LEASE_TTL_SECONDS,
        new_checkpoint
            .map(str::to_string)
            .or_else(|| ours.last_checkpoint().map(str::to_string)),
    );
    install_generation(store, repository, &ref_name, &base_tip, next)
}

/// Take over an expired lease. Refuse when the lease is still live (`NotExpired`) or the ref
/// is absent. On success the caller's holder and plan digest replace the expired holder's,
/// the generation advances, and the checkpoint is dropped (a takeover is a fresh start, not a
/// continuation).
pub fn take_over_expired_lease<S: PortfolioRefStore>(
    store: &mut S,
    repository: &str,
    portfolio: &PortfolioId,
    plan_digest: &str,
    holder: &str,
    now: u64,
) -> Result<PortfolioLease, PortfolioLeaseError> {
    let ref_name = portfolio_lease_ref_name(portfolio.as_str());
    let base_tip = match store.read_ref(repository, &ref_name)? {
        Some(sha) => sha,
        None => {
            return Err(PortfolioLeaseError::Integrity(
                "no lease to take over".to_string(),
            ))
        }
    };
    let message = store.read_commit_message(repository, &base_tip)?;
    let observed = parse_lease(&message)?;
    if !observed.is_expired(now) {
        return Err(PortfolioLeaseError::NotExpired);
    }
    let next = PortfolioLease::new(
        portfolio.as_str().to_string(),
        plan_digest.to_string(),
        holder.to_string(),
        observed.generation + 1,
        now + PORTFOLIO_LEASE_TTL_SECONDS,
        None,
    );
    install_generation(store, repository, &ref_name, &base_tip, next)
}

/// Create the commit for `next` (parented at `base_tip`) and fast-forward the ref to it. On a
/// compare-and-swap rejection, re-read and classify: our sha at the tip means we won, an
/// absent ref means the store vanished, anything else means a foreign host advanced the
/// generation and the caller is fenced.
fn install_generation<S: PortfolioRefStore>(
    store: &mut S,
    repository: &str,
    ref_name: &str,
    base_tip: &str,
    next: PortfolioLease,
) -> Result<PortfolioLease, PortfolioLeaseError> {
    let sha = store.create_commit(
        repository,
        &next.commit_message(),
        PORTFOLIO_LEASE_EMPTY_TREE,
        &[base_tip.to_string()],
    )?;
    if let Err(original) = store.fast_forward_ref(repository, ref_name, &sha) {
        return match store.read_ref(repository, ref_name)? {
            None => Err(original),
            Some(tip) if tip == sha => Ok(next),
            Some(_) => Err(PortfolioLeaseError::Fenced),
        };
    }
    Ok(next)
}

/// A 32-character hex random holder id, read from the platform entropy source.
pub fn random_portfolio_holder() -> Result<String, PortfolioLeaseError> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let mut source = std::fs::File::open("/dev/urandom")
        .map_err(|error| PortfolioLeaseError::Local(format!("open /dev/urandom: {error}")))?;
    source
        .read_exact(&mut bytes)
        .map_err(|error| PortfolioLeaseError::Local(format!("read /dev/urandom: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn is_sha256_hex(value: &str) -> bool {
    is_hex_len(value, 64)
}

fn is_holder_hex(value: &str) -> bool {
    is_hex_len(value, 32)
}

fn is_hex_len(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F'))
}
