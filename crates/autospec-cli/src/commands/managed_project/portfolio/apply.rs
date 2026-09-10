//! Resumable `portfolio apply` ordering engine.
//!
//! This is the pure checkpoint logic behind the `portfolio apply` transaction: it
//! drives issue creation in deterministic item-key order, records Project
//! membership and canonical cross-repository graph edges, and — critically —
//! survives an interrupted run without re-creating issues. It is transport-free:
//! the GitHub GraphQL transport feeds it outcomes, and the durable store persists
//! the resulting [`Checkpoint`]. No remote call happens here, which is what makes
//! the ordering and recovery rules unit-testable without a live host.
//!
//! The invariants enforced here are the ones the spec's "canonical lifecycle and
//! admission ordering" section depends on: a lost create response enters
//! `create_unknown` and is never auto-retried; duplicate item markers fail closed;
//! and a parent admits zero children to `auto-implement` until every recorded
//! child is terminal-success.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The logical role of a portfolio item. Roles fix the apply order: the primary
/// umbrella is filed first, then repository-local trackers, then the
/// implementation/prerequisite children, and the Phase 5.5 audit last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Umbrella,
    Tracker,
    Implementation,
    Audit,
}

impl Role {
    fn precedence(&self) -> u8 {
        match self {
            Role::Umbrella => 0,
            Role::Tracker => 1,
            Role::Implementation => 2,
            Role::Audit => 3,
        }
    }
}

/// A single item in a frozen portfolio plan, addressed by a stable logical key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioItem {
    /// Stable logical key, e.g. `source-tracker` or `issue:beta`.
    pub item_key: String,
    /// Canonical `OWNER/REPO` identity of the repository that owns the item.
    pub repo: String,
    pub role: Role,
    /// Item keys of this item's hard predecessors (logical edges).
    pub depends_on: Vec<String>,
}

/// The outcome of a single issue-create attempt, as observed by the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateOutcome {
    /// GitHub acknowledged the create and returned a canonical issue URL.
    Acknowledged(String),
    /// The create response was lost; no matching marker is visible yet.
    Lost,
}

/// A resumable apply checkpoint: which items are bound to a canonical issue URL,
/// which are in `create_unknown`, which have Project membership, and the canonical
/// dependency edges. It is the serializable projection the durable store keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checkpoint {
    bound: BTreeMap<String, String>,
    create_unknown: BTreeSet<String>,
    project_members: BTreeSet<String>,
    edges: BTreeMap<String, Vec<String>>,
}

/// Failures the apply engine can surface while driving a frozen plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// More than one issue carries the exact item marker; the create is ambiguous.
    DuplicateMarkers { key: String, matches: usize },
    /// The item is not in `create_unknown`, so there is nothing to reconcile.
    NotUnknown { key: String },
    /// The item is already bound; binding it again would create a duplicate.
    AlreadyBound { key: String },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateMarkers { key, matches } => write!(
                f,
                "portfolio item {key} has {matches} issues carrying its exact marker; refusing to guess"
            ),
            Self::NotUnknown { key } => {
                write!(f, "portfolio item {key} is not in create_unknown")
            }
            Self::AlreadyBound { key } => {
                write!(f, "portfolio item {key} is already bound to an issue URL")
            }
        }
    }
}

impl std::error::Error for ApplyError {}

impl Checkpoint {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_bound(&self, key: &str) -> bool {
        self.bound.contains_key(key)
    }

    pub fn is_unknown(&self, key: &str) -> bool {
        self.create_unknown.contains(key)
    }

    pub fn is_member(&self, key: &str) -> bool {
        self.project_members.contains(key)
    }

    pub fn url(&self, key: &str) -> Option<&str> {
        self.bound.get(key).map(String::as_str)
    }

    pub fn bound_count(&self) -> usize {
        self.bound.len()
    }

    pub fn unknown_count(&self) -> usize {
        self.create_unknown.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.values().map(Vec::len).sum()
    }

    /// The next item, in deterministic apply order, that is neither bound nor in
    /// `create_unknown`. This is the resume cursor: an interrupted run picks up
    /// exactly where it stopped and never re-creates an item it already bound.
    pub fn next_pending<'a>(&self, items: &'a [PortfolioItem]) -> Option<&'a PortfolioItem> {
        apply_order(items)
            .into_iter()
            .find(|item| !self.is_bound(&item.item_key) && !self.is_unknown(&item.item_key))
    }

    /// Bind an item to its canonical URL after a definitive acknowledged create.
    /// Fails if the item is already bound, so a duplicate create can never win.
    pub fn record_acknowledged(&mut self, key: &str, url: &str) -> Result<(), ApplyError> {
        if self.is_bound(key) {
            return Err(ApplyError::AlreadyBound {
                key: key.to_string(),
            });
        }
        self.create_unknown.remove(key);
        self.bound.insert(key.to_string(), url.to_string());
        Ok(())
    }

    /// Mark an item `create_unknown` after a lost create response. Idempotent: the
    /// item is parked, never auto-retried, and only cleared by reconciliation.
    pub fn record_lost(&mut self, key: &str) {
        self.create_unknown.insert(key.to_string());
    }

    /// Reconcile a `create_unknown` item against the issue URLs that carry its
    /// exact hidden marker. One candidate binds the item; zero leaves it blocked
    /// until GitHub exposes the item; more than one fails closed.
    pub fn reconcile_lost(&mut self, key: &str, candidates: &[String]) -> Result<(), ApplyError> {
        if !self.is_unknown(key) {
            return Err(ApplyError::NotUnknown {
                key: key.to_string(),
            });
        }
        match candidates.len() {
            0 => Ok(()),
            1 => {
                self.create_unknown.remove(key);
                self.bound.insert(key.to_string(), candidates[0].clone());
                Ok(())
            }
            matches => Err(ApplyError::DuplicateMarkers {
                key: key.to_string(),
                matches,
            }),
        }
    }

    /// Record that an item was added to the primary Project. Idempotent: re-adding
    /// an already-present item is a no-op, never a duplicate item.
    pub fn record_membership(&mut self, key: &str) {
        self.project_members.insert(key.to_string());
    }

    /// Bind a canonical cross-repository dependency edge from an item to a
    /// predecessor's canonical issue URL. Duplicate edges are not re-stored.
    pub fn record_edge(&mut self, from: &str, predecessor_url: &str) {
        let entry = self.edges.entry(from.to_string()).or_default();
        if !entry.contains(&predecessor_url.to_string()) {
            entry.push(predecessor_url.to_string());
        }
    }
}

/// Deterministic apply order: lifecycle role precedence first, then item key. This
/// is stable across runs, so resume and the frozen plan always agree on sequence.
pub fn apply_order(items: &[PortfolioItem]) -> Vec<&PortfolioItem> {
    let mut ordered: Vec<&PortfolioItem> = items.iter().collect();
    ordered.sort_by(|left, right| {
        (left.role.precedence(), left.item_key.as_str())
            .cmp(&(right.role.precedence(), right.item_key.as_str()))
    });
    ordered
}

/// The canonical HTTPS issue URL for a repository-local issue, in the form the
/// shared contracts require for durable cross-repository references.
pub fn canonical_issue_url(owner_repo: &str, number: u64) -> String {
    format!("https://github.com/{owner_repo}/issues/{number}")
}

/// The children admitted to `auto-implement` for a portfolio parent. Admission is
/// all-or-nothing: a parent admits its children only once every recorded child is
/// terminal-success. A failed or partially-ready parent admits zero children, so an
/// external runner can never claim a partially-prepared portfolio.
pub fn admitted_children(all: &[u64], terminal: &[u64]) -> Vec<u64> {
    let complete = !all.is_empty() && all.iter().all(|child| terminal.contains(child));
    if !complete {
        return Vec::new();
    }
    all.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, repo: &str, role: Role) -> PortfolioItem {
        PortfolioItem {
            item_key: key.to_string(),
            repo: repo.to_string(),
            role,
            depends_on: Vec::new(),
        }
    }

    fn five_items() -> Vec<PortfolioItem> {
        vec![
            item("audit:phase-5.5", "org/source", Role::Audit),
            item("repo:org/target:tracker", "org/target", Role::Tracker),
            item("issue:beta", "org/source", Role::Implementation),
            item("source-tracker", "org/source", Role::Umbrella),
            item("issue:alpha", "org/source", Role::Implementation),
        ]
    }

    #[test]
    fn apply_order_is_lifecycle_role_then_stable_key() {
        let items = five_items();
        let ordered = apply_order(&items);
        assert_eq!(
            ordered
                .iter()
                .map(|it| it.item_key.as_str())
                .collect::<Vec<_>>(),
            [
                "source-tracker",
                "repo:org/target:tracker",
                "issue:alpha",
                "issue:beta",
                "audit:phase-5.5",
            ]
        );
    }

    #[test]
    fn resumed_five_item_manifest_creates_zero_duplicate_issues() {
        let items = five_items();

        // First run files the umbrella and the secondary tracker, then is interrupted.
        let mut first = Checkpoint::new();
        first
            .record_acknowledged("source-tracker", &canonical_issue_url("org/source", 100))
            .unwrap();
        first
            .record_acknowledged(
                "repo:org/target:tracker",
                &canonical_issue_url("org/target", 101),
            )
            .unwrap();

        // Resume starts from a checkpoint that already carries those two bindings.
        let resumed_from: Vec<&str> = vec!["source-tracker", "repo:org/target:tracker"];
        let mut resumed = first.clone();
        let mut created_on_resume = 0;
        let mut next_number = 102;
        while let Some(next) = resumed.next_pending(&items) {
            let already_bound = resumed_from.contains(&next.item_key.as_str());
            assert!(
                !already_bound,
                "a bound item must never be re-created on resume"
            );
            let url = canonical_issue_url(&next.repo, next_number);
            resumed.record_acknowledged(&next.item_key, &url).unwrap();
            created_on_resume += 1;
            next_number += 1;
        }

        assert_eq!(
            created_on_resume, 3,
            "only the three unfiled items are created on resume"
        );
        assert_eq!(
            resumed.bound_count(),
            5,
            "every planned item is bound exactly once"
        );
        // 2 (first run) + 3 (resume) = 5 creates for 5 items: zero duplicates.
        assert_eq!(first.bound_count() + created_on_resume, 5);
        // Re-binding an already-bound item is rejected, so a duplicate can never win.
        let duplicate = resumed
            .record_acknowledged("source-tracker", "https://github.com/org/source/issues/999");
        assert!(matches!(duplicate, Err(ApplyError::AlreadyBound { .. })));
    }

    #[test]
    fn lost_create_response_enters_unknown_without_retry() {
        let items = five_items();
        let mut checkpoint = Checkpoint::new();
        checkpoint
            .record_acknowledged("source-tracker", &canonical_issue_url("org/source", 100))
            .unwrap();

        let next = checkpoint.next_pending(&items).expect("a pending item");
        assert_eq!(next.item_key, "repo:org/target:tracker");
        checkpoint.record_lost(&next.item_key);
        assert!(checkpoint.is_unknown(&next.item_key));

        // Resume skips the unknown item: no automatic retry of an ambiguous create.
        let resumed_next = checkpoint
            .next_pending(&items)
            .expect("another pending item");
        assert_ne!(resumed_next.item_key, next.item_key);

        // Zero reconciliation matches keeps the item blocked, not bound.
        assert!(checkpoint.reconcile_lost(&next.item_key, &[]).is_ok());
        assert!(checkpoint.is_unknown(&next.item_key));
        assert!(!checkpoint.is_bound(&next.item_key));
    }

    #[test]
    fn duplicate_markers_fail_closed_and_single_match_binds() {
        let mut checkpoint = Checkpoint::new();
        checkpoint.record_lost("issue:alpha");

        let ambiguous = checkpoint.reconcile_lost(
            "issue:alpha",
            &[
                "https://github.com/org/source/issues/1".to_string(),
                "https://github.com/org/source/issues/2".to_string(),
            ],
        );
        assert!(matches!(
            ambiguous,
            Err(ApplyError::DuplicateMarkers { matches: 2, .. })
        ));
        assert!(
            checkpoint.is_unknown("issue:alpha"),
            "ambiguity leaves the item blocked"
        );

        let single = "https://github.com/org/source/issues/1".to_string();
        assert!(checkpoint
            .reconcile_lost("issue:alpha", &[single.clone()])
            .is_ok());
        assert_eq!(checkpoint.url("issue:alpha"), Some(single.as_str()));

        // Reconciling an item that is no longer unknown is a contract violation.
        assert!(matches!(
            checkpoint.reconcile_lost("issue:alpha", &[single]),
            Err(ApplyError::NotUnknown { .. })
        ));
    }

    #[test]
    fn cross_repository_edges_store_canonical_urls_without_duplicates() {
        let mut checkpoint = Checkpoint::new();
        let source_url = canonical_issue_url("org/source", 100);
        let target_url = canonical_issue_url("org/target", 200);

        checkpoint.record_edge("repo:org/target:tracker", &source_url);
        checkpoint.record_edge("issue:beta", &target_url);
        checkpoint.record_edge("issue:beta", &target_url);

        assert_eq!(
            canonical_issue_url("org/target", 200),
            "https://github.com/org/target/issues/200"
        );
        assert_eq!(
            checkpoint.edge_count(),
            2,
            "duplicate edges are not re-stored"
        );
    }

    #[test]
    fn parent_failure_admits_zero_children_until_all_are_terminal() {
        let all = [11, 12, 13];
        // Two of three terminal: the parent is not complete, so zero are admitted.
        assert_eq!(admitted_children(&all, &[11, 12]), Vec::<u64>::new());
        // All terminal: the parent is complete, so every child is admitted.
        assert_eq!(admitted_children(&all, &[11, 12, 13]), all.to_vec());
        // A parent with no recorded children admits nothing.
        assert_eq!(admitted_children(&[], &[11]), Vec::<u64>::new());
    }

    #[test]
    fn project_membership_checkpoint_is_idempotent() {
        let mut checkpoint = Checkpoint::new();
        checkpoint.record_membership("source-tracker");
        checkpoint.record_membership("source-tracker");

        assert!(checkpoint.is_member("source-tracker"));
        assert!(!checkpoint.is_member("issue:alpha"));
    }
}
