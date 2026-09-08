//! Portfolio status reconciliation (spec
//! `2026-08-31-automatic-spec-projects-design.md`, "Status reconciliation
//! and completion").
//!
//! Every item derives one of the ten `Autospec delivery` values as a total
//! function of authoritative lifecycle facts: the issue, its PR, post-merge
//! CI checks, review activity, typed dependency edges, parent
//! reconciliation, and audit/external receipts. Project fields are a
//! projection of these facts and never authoritative.
//!
//! Precedence (highest first): `Unknown` for missing, stale, or
//! contradictory identity facts; `Failed` for a current terminal failure;
//! `Blocked` for safety/pause/inaccessible state or a dependency that is
//! Blocked, Failed, or Unknown; `Done` when the issue is closed and its
//! completion policy is satisfied; `Verifying` after merge while checks,
//! receipt, or closure remain; `Review`; `PR Open`; `Running`; `Ready` when
//! admitted and all dependencies are Done; otherwise `Planned`, including
//! healthy unfinished dependencies. A CI failure alone does not make an
//! active item Failed; only an exhausted-retry terminal failure does.
//! Reopen or retry moves a terminal item backward because `Done` requires
//! the issue to be closed and the policy to stay satisfied.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use autospec_core::managed_project::ItemKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The ten `Autospec delivery` values, declared in derivation precedence
/// order (highest first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ItemStatus {
    #[serde(rename = "Unknown")]
    Unknown,
    #[serde(rename = "Failed")]
    Failed,
    #[serde(rename = "Blocked")]
    Blocked,
    #[serde(rename = "Done")]
    Done,
    #[serde(rename = "Verifying")]
    Verifying,
    #[serde(rename = "Review")]
    Review,
    #[serde(rename = "PR Open")]
    PrOpen,
    #[serde(rename = "Running")]
    Running,
    #[serde(rename = "Ready")]
    Ready,
    #[serde(rename = "Planned")]
    Planned,
}

impl ItemStatus {
    /// Every delivery value; the per-status README counts always carry all ten.
    pub const ALL: [ItemStatus; 10] = [
        ItemStatus::Unknown,
        ItemStatus::Failed,
        ItemStatus::Blocked,
        ItemStatus::Done,
        ItemStatus::Verifying,
        ItemStatus::Review,
        ItemStatus::PrOpen,
        ItemStatus::Running,
        ItemStatus::Ready,
        ItemStatus::Planned,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ItemStatus::Unknown => "Unknown",
            ItemStatus::Failed => "Failed",
            ItemStatus::Blocked => "Blocked",
            ItemStatus::Done => "Done",
            ItemStatus::Verifying => "Verifying",
            ItemStatus::Review => "Review",
            ItemStatus::PrOpen => "PR Open",
            ItemStatus::Running => "Running",
            ItemStatus::Ready => "Ready",
            ItemStatus::Planned => "Planned",
        }
    }
}

/// Portfolio item roles from the frozen recovery capsule. Both tracker
/// roles count as the single `tracker` outstanding kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemRole {
    SourceTracker,
    RepoTracker,
    Prerequisite,
    Implementation,
    Audit,
}

impl ItemRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemRole::SourceTracker => "source-tracker",
            ItemRole::RepoTracker => "repo-tracker",
            ItemRole::Prerequisite => "prerequisite",
            ItemRole::Implementation => "implementation",
            ItemRole::Audit => "audit",
        }
    }

    fn is_tracker(self) -> bool {
        matches!(self, ItemRole::SourceTracker | ItemRole::RepoTracker)
    }

    /// The four outstanding item kinds that gate portfolio completion.
    pub fn outstanding_kind(self) -> &'static str {
        match self {
            ItemRole::SourceTracker | ItemRole::RepoTracker => "tracker",
            ItemRole::Prerequisite => "prerequisite",
            ItemRole::Implementation => "implementation",
            ItemRole::Audit => "audit",
        }
    }

    /// The one completion policy this role may carry; any other pairing is
    /// a contradictory fact and fails closed.
    pub fn completion_policy(self) -> CompletionPolicy {
        match self {
            ItemRole::SourceTracker | ItemRole::RepoTracker => CompletionPolicy::ClosedTracker,
            ItemRole::Prerequisite => CompletionPolicy::ExternalPrerequisite,
            ItemRole::Implementation => CompletionPolicy::MergedPr,
            ItemRole::Audit => CompletionPolicy::AuditReceipt,
        }
    }
}

/// Completion policies declared by the frozen plan; an item is `Done` only
/// when its issue is closed AND its policy is satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompletionPolicy {
    MergedPr,
    ClosedTracker,
    AuditReceipt,
    ExternalPrerequisite,
}

impl CompletionPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CompletionPolicy::MergedPr => "merged-pr",
            CompletionPolicy::ClosedTracker => "closed-tracker",
            CompletionPolicy::AuditReceipt => "audit-receipt",
            CompletionPolicy::ExternalPrerequisite => "external-prerequisite",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrState {
    None,
    Open,
    Merged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckState {
    Unknown,
    Running,
    Passing,
    Failing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewState {
    None,
    Active,
    ChangesRequested,
    Approved,
}

/// Acknowledged authoritative lifecycle facts for one item. `fresh: false`
/// means the identity/state facts are missing, stale, or contradictory and
/// the item reconciles to `Unknown` regardless of every other observation.
/// Field-closed: untrusted metadata cannot smuggle extra fields in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleObservation {
    pub fresh: bool,
    /// the tracked issue is closed
    pub issue_closed: bool,
    /// closed manually, i.e. not by a merged PR or parent reconciliation
    pub manually_closed: bool,
    /// safety quarantine, pause, or inaccessible state
    pub quarantined: bool,
    /// terminal implementation/audit failure (retries exhausted); a CI
    /// failure alone never sets this
    pub terminal_failure: bool,
    /// admitted to the queue
    pub admitted: bool,
    /// currently claimed by an implementation worker
    pub claimed: bool,
    pub pr: PrState,
    pub post_merge_checks: CheckState,
    pub review: ReviewState,
    /// closed by portfolio-mode parent reconciliation (trackers)
    pub parent_reconciled: bool,
    /// a matching audit receipt (audit items) or external receipt
    /// (prerequisite items) was observed
    pub receipt: bool,
}

/// One item's frozen plan identity (from the recovery capsule) plus its
/// acknowledged authoritative observations. Field-closed for the same
/// untrusted-metadata reason as [`LifecycleObservation`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemFacts {
    pub item_key: String,
    pub repository: String,
    pub role: ItemRole,
    pub completion_policy: CompletionPolicy,
    /// item keys of local parent records (trackers)
    pub local_parents: Vec<String>,
    /// typed hard dependency edges (item keys)
    pub dependencies: Vec<String>,
    /// date of the latest acknowledged authoritative issue, PR, CI,
    /// review, parent, or audit event; ISO-8601 so lexicographic order is
    /// chronological. Projection retries never contribute.
    pub last_event_at: Option<String>,
    pub observation: LifecycleObservation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum PortfolioStatus {
    Active,
    Blocked,
    Done,
}

impl PortfolioStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PortfolioStatus::Active => "Active",
            PortfolioStatus::Blocked => "Blocked",
            PortfolioStatus::Done => "Done",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutstandingItem {
    pub item_key: String,
    pub role: ItemRole,
    pub status: ItemStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reconciliation {
    pub portfolio: PortfolioStatus,
    /// true only when every tracker, prerequisite, and implementation item
    /// is Done AND the audit item is Done
    pub complete: bool,
    pub items: BTreeMap<String, ItemStatus>,
    /// per-status counts across all ten delivery values (zero-filled)
    pub counts: BTreeMap<ItemStatus, u64>,
    /// latest acknowledged authoritative event date across all items
    pub last_activity: Option<String>,
    /// every item that is not Done, in item-key order
    pub outstanding: Vec<OutstandingItem>,
}

impl Reconciliation {
    /// The projected managed-field / README payload for this reconciliation:
    /// portfolio status, completion, per-status counts, `Last activity`,
    /// per-item `Autospec delivery` values, and the outstanding items.
    pub fn projection_payload(&self) -> Value {
        let mut counts = serde_json::Map::new();
        for status in ItemStatus::ALL {
            counts.insert(status.as_str().to_owned(), json!(self.counts[&status]));
        }
        let mut items = serde_json::Map::new();
        for (key, status) in &self.items {
            items.insert(key.clone(), json!(status.as_str()));
        }
        let outstanding = self
            .outstanding
            .iter()
            .map(|item| {
                json!({
                    "item_key": item.item_key,
                    "kind": item.role.outstanding_kind(),
                    "status": item.status.as_str(),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "status": self.portfolio.as_str(),
            "complete": self.complete,
            "counts": counts,
            "last_activity": self.last_activity,
            "items": items,
            "outstanding": outstanding,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileError {
    DuplicateItemKey(String),
    InvalidItemKey(String),
    InvalidRepository(String),
    RolePolicyMismatch(String),
}

impl fmt::Display for ReconcileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateItemKey(key) => {
                write!(formatter, "duplicate portfolio item key: {key}")
            }
            Self::InvalidItemKey(key) => {
                write!(formatter, "portfolio item key has an unsafe grammar: {key}")
            }
            Self::InvalidRepository(key) => {
                write!(formatter, "portfolio item has no repository: {key}")
            }
            Self::RolePolicyMismatch(key) => {
                write!(
                    formatter,
                    "portfolio item role and completion policy conflict: {key}"
                )
            }
        }
    }
}

impl std::error::Error for ReconcileError {}

/// Derive every item status and the portfolio status from typed facts.
/// The derivation is total: it fails only on contradictory fact shapes
/// (duplicated or unsafe keys, role/policy mismatches), never on lifecycle
/// state, which always resolves to one of the ten delivery values.
pub fn reconcile(items: &[ItemFacts]) -> Result<Reconciliation, ReconcileError> {
    let mut seen = BTreeSet::new();
    for item in items {
        validate_fact_shape(item)?;
        if !seen.insert(item.item_key.clone()) {
            return Err(ReconcileError::DuplicateItemKey(item.item_key.clone()));
        }
    }
    let local_children = local_children_by_parent(items);
    let cyclical = cyclical_dependency_members(items);
    let statuses = settle_statuses(items, &local_children, &cyclical);
    let outstanding = outstanding_items(items, &statuses);
    let complete = outstanding.is_empty() && items.iter().any(|item| item.role == ItemRole::Audit);
    let portfolio = if complete {
        PortfolioStatus::Done
    } else if items.iter().any(|item| {
        matches!(
            statuses[&item.item_key],
            ItemStatus::Blocked | ItemStatus::Failed
        )
    }) {
        PortfolioStatus::Blocked
    } else {
        PortfolioStatus::Active
    };
    let last_activity = items
        .iter()
        .filter_map(|item| item.last_event_at.clone())
        .max();
    let counts = status_counts(&statuses);
    Ok(Reconciliation {
        portfolio,
        complete,
        items: statuses,
        counts,
        last_activity,
        outstanding,
    })
}

fn status_counts(statuses: &BTreeMap<String, ItemStatus>) -> BTreeMap<ItemStatus, u64> {
    let mut counts = ItemStatus::ALL
        .iter()
        .copied()
        .map(|status| (status, 0u64))
        .collect::<BTreeMap<_, _>>();
    for status in statuses.values() {
        *counts
            .get_mut(status)
            .expect("every status is present in the count table") += 1;
    }
    counts
}

fn outstanding_items(
    items: &[ItemFacts],
    statuses: &BTreeMap<String, ItemStatus>,
) -> Vec<OutstandingItem> {
    items
        .iter()
        .filter(|item| statuses[&item.item_key] != ItemStatus::Done)
        .map(|item| OutstandingItem {
            item_key: item.item_key.clone(),
            role: item.role,
            status: statuses[&item.item_key],
        })
        .collect()
}

fn validate_fact_shape(item: &ItemFacts) -> Result<(), ReconcileError> {
    ItemKey::new(item.item_key.clone())
        .map_err(|_| ReconcileError::InvalidItemKey(item.item_key.clone()))?;
    if item.repository.trim().is_empty() {
        return Err(ReconcileError::InvalidRepository(item.item_key.clone()));
    }
    if item.role.completion_policy() != item.completion_policy {
        return Err(ReconcileError::RolePolicyMismatch(item.item_key.clone()));
    }
    for key in item.local_parents.iter().chain(item.dependencies.iter()) {
        ItemKey::new(key.clone()).map_err(|_| ReconcileError::InvalidItemKey(key.clone()))?;
    }
    Ok(())
}

fn local_children_by_parent<'a>(items: &'a [ItemFacts]) -> BTreeMap<&'a str, Vec<&'a ItemFacts>> {
    let mut children: BTreeMap<&'a str, Vec<&'a ItemFacts>> = BTreeMap::new();
    for item in items {
        for parent in &item.local_parents {
            children.entry(parent.as_str()).or_default().push(item);
        }
    }
    children
}

fn cyclical_dependency_members(items: &[ItemFacts]) -> BTreeSet<String> {
    let successors: BTreeMap<&str, Vec<&str>> = items
        .iter()
        .map(|item| {
            (
                item.item_key.as_str(),
                item.dependencies.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    items
        .iter()
        .filter(|item| {
            reachable_from(item.item_key.as_str(), &successors).contains(item.item_key.as_str())
        })
        .map(|item| item.item_key.clone())
        .collect()
}

fn reachable_from<'a>(
    start: &'a str,
    successors: &'a BTreeMap<&'a str, Vec<&'a str>>,
) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<&'a str> = successors.get(start).cloned().unwrap_or_default();
    while let Some(node) = stack.pop() {
        if seen.insert(node) {
            if let Some(next) = successors.get(node) {
                stack.extend(next.iter().copied());
            }
        }
    }
    seen
}

fn settle_statuses(
    items: &[ItemFacts],
    local_children: &BTreeMap<&str, Vec<&ItemFacts>>,
    cyclical: &BTreeSet<String>,
) -> BTreeMap<String, ItemStatus> {
    let mut settled: BTreeMap<String, ItemStatus> = items
        .iter()
        .map(|item| {
            (
                item.item_key.clone(),
                if cyclical.contains(&item.item_key) {
                    ItemStatus::Unknown
                } else {
                    ItemStatus::Planned
                },
            )
        })
        .collect();
    // Each item changes value only finitely often (intrinsic fact state,
    // dependency-blocked propagation, Ready admission), so this converges
    // far before the fail-closed bound below.
    for _ in 0..=items.len().saturating_mul(4) + 4 {
        let mut next: BTreeMap<String, ItemStatus> = BTreeMap::new();
        for item in items {
            let status = if cyclical.contains(&item.item_key) {
                ItemStatus::Unknown
            } else {
                derive_item_status(item, &settled, local_children)
            };
            next.insert(item.item_key.clone(), status);
        }
        if next == settled {
            return next;
        }
        settled = next;
    }
    // Unreachable for a well-formed fact set; fail closed as contradictory.
    for status in settled.values_mut() {
        *status = ItemStatus::Unknown;
    }
    settled
}

fn derive_item_status(
    item: &ItemFacts,
    settled: &BTreeMap<String, ItemStatus>,
    local_children: &BTreeMap<&str, Vec<&ItemFacts>>,
) -> ItemStatus {
    let observation = &item.observation;
    if !observation.fresh {
        return ItemStatus::Unknown;
    }
    if observation.terminal_failure {
        return ItemStatus::Failed;
    }
    if observation.quarantined || dependency_blocked(item, settled) {
        return ItemStatus::Blocked;
    }
    if observation.issue_closed && policy_satisfied(item, observation, settled, local_children) {
        return ItemStatus::Done;
    }
    if observation.pr == PrState::Merged {
        return ItemStatus::Verifying;
    }
    if matches!(
        observation.review,
        ReviewState::Active | ReviewState::ChangesRequested
    ) {
        return ItemStatus::Review;
    }
    if observation.pr == PrState::Open {
        return ItemStatus::PrOpen;
    }
    if observation.claimed {
        return ItemStatus::Running;
    }
    if observation.admitted && dependencies_done(item, settled) {
        return ItemStatus::Ready;
    }
    ItemStatus::Planned
}

fn dependency_blocked(item: &ItemFacts, settled: &BTreeMap<String, ItemStatus>) -> bool {
    item.dependencies
        .iter()
        .any(|dependency| match settled.get(dependency) {
            // A predecessor that is missing, Blocked, Failed, or Unknown
            // fails closed: the dependent cannot be admitted.
            None => true,
            Some(ItemStatus::Blocked | ItemStatus::Failed | ItemStatus::Unknown) => true,
            Some(_) => false,
        })
}

fn dependencies_done(item: &ItemFacts, settled: &BTreeMap<String, ItemStatus>) -> bool {
    item.dependencies
        .iter()
        .all(|dependency| settled.get(dependency) == Some(&ItemStatus::Done))
}

fn policy_satisfied(
    item: &ItemFacts,
    observation: &LifecycleObservation,
    settled: &BTreeMap<String, ItemStatus>,
    local_children: &BTreeMap<&str, Vec<&ItemFacts>>,
) -> bool {
    if observation.manually_closed {
        // Manual closure is never a success: implementation items need a
        // merged implementation PR and a manually closed tracker child
        // stays pending.
        return false;
    }
    match item.completion_policy {
        CompletionPolicy::MergedPr => {
            observation.pr == PrState::Merged
                && observation.post_merge_checks == CheckState::Passing
        }
        CompletionPolicy::ClosedTracker => {
            item.role.is_tracker()
                && observation.parent_reconciled
                && local_children
                    .get(item.item_key.as_str())
                    .is_none_or(|children| {
                        children
                            .iter()
                            .all(|child| settled.get(&child.item_key) == Some(&ItemStatus::Done))
                    })
        }
        CompletionPolicy::AuditReceipt => item.role == ItemRole::Audit && observation.receipt,
        CompletionPolicy::ExternalPrerequisite => {
            item.role == ItemRole::Prerequisite && observation.receipt
        }
    }
}
