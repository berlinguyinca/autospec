//! Capability and edge gate for a frozen plan.
//!
//! Three separate questions, three separate codes: can the repository be used at all
//! ([`validate_capabilities`]), does an edge point at something in the plan
//! ([`validate_edges`] and its helpers), and is the resulting graph acyclic
//! ([`detect_cycle`]). Reporting them apart matters because the operator's fix differs:
//! re-probe a repository, correct a key, or reorder the plan.

use super::facts::{PlanItem, RepositoryCapability, RepositoryFacts};
use super::{PlanViolation, PlanViolationCode};
use autospec_core::managed_project::ItemKey;
use std::collections::{BTreeMap, BTreeSet};

/// Markers for the DFS: a node being visited is on the current path, a visited node is
/// proven cycle-free and never re-entered.
const VISITING: u8 = 1;
const VISITED: u8 = 2;

/// Every item must be hosted by a repository whose capability was observed as available.
/// `Unknown` and `Unavailable` are reported apart: one means the probe never ran, the
/// other means it ran and the repository answered "no".
pub(super) fn validate_capabilities(
    repositories: &[RepositoryFacts],
    items: &[PlanItem],
) -> Result<(), PlanViolation> {
    for item in items {
        let facts = repositories
            .iter()
            .find(|declared| declared.repository() == item.repository());
        check_capability(item, facts)?;
    }
    Ok(())
}

fn check_capability(item: &PlanItem, facts: Option<&RepositoryFacts>) -> Result<(), PlanViolation> {
    let (code, why) = match facts.map(RepositoryFacts::capability) {
        Some(RepositoryCapability::Available) => return Ok(()),
        Some(RepositoryCapability::Unavailable) => (
            PlanViolationCode::RepositoryCapabilityUnavailable,
            "was probed and is unavailable",
        ),
        Some(RepositoryCapability::Unknown) => (
            PlanViolationCode::RepositoryCapabilityUnknown,
            "was never probed",
        ),
        None => (
            PlanViolationCode::ItemRepositoryUndeclared,
            "is not declared in the plan",
        ),
    };
    Err(PlanViolation::new(
        code,
        format!(
            "item `{}` is hosted by `{}` which {}",
            item.item_key(),
            item.repository(),
            why
        ),
    ))
}

/// Self-loops, dangling references, cross-repository local parents, duplicate edges, and
/// cycles. Order of checks is fixed so a plan with several defects always reports the
/// same one.
pub(super) fn validate_edges(items: &[PlanItem]) -> Result<(), PlanViolation> {
    let by_key: BTreeMap<&str, &PlanItem> = items
        .iter()
        .map(|item| (item.item_key().as_str(), item))
        .collect();
    let mut seen: BTreeSet<(&str, &str)> = BTreeSet::new();
    for item in items {
        for target in item.depends_on().iter().chain(item.local_parents().iter()) {
            check_edge(item, target, &by_key, &mut seen)?;
        }
    }
    detect_cycle(items, &by_key)
}

fn check_edge<'a>(
    item: &'a PlanItem,
    target: &'a ItemKey,
    by_key: &BTreeMap<&str, &PlanItem>,
    seen: &mut BTreeSet<(&'a str, &'a str)>,
) -> Result<(), PlanViolation> {
    check_reference(item, target, by_key)?;
    if is_local_parent(item, target) && crosses_repository(item, target, by_key) {
        return Err(PlanViolation::new(
            PlanViolationCode::LocalParentCrossRepository,
            format!(
                "item `{}` declares a local parent hosted by `{}`",
                item.item_key(),
                host_of(target, by_key)
            ),
        ));
    }
    if !seen.insert((item.item_key().as_str(), target.as_str())) {
        return Err(PlanViolation::new(
            PlanViolationCode::EdgeDuplicate,
            format!(
                "item `{}` declares the edge to `{}` more than once",
                item.item_key(),
                target
            ),
        ));
    }
    Ok(())
}

/// A local parent is a same-repository structural parent; a parent in another repository
/// is a dependency and must be declared as one.
fn is_local_parent(item: &PlanItem, target: &ItemKey) -> bool {
    item.local_parents().iter().any(|parent| parent == target)
}

fn crosses_repository(
    item: &PlanItem,
    target: &ItemKey,
    by_key: &BTreeMap<&str, &PlanItem>,
) -> bool {
    by_key
        .get(target.as_str())
        .is_some_and(|parent| parent.repository() != item.repository())
}

fn host_of(target: &ItemKey, by_key: &BTreeMap<&str, &PlanItem>) -> String {
    by_key
        .get(target.as_str())
        .map(|host| host.repository().to_string())
        .unwrap_or_else(|| format!("<undeclared item {target}>"))
}

fn check_reference(
    item: &PlanItem,
    target: &ItemKey,
    by_key: &BTreeMap<&str, &PlanItem>,
) -> Result<(), PlanViolation> {
    if target.as_str() == item.item_key().as_str() {
        return Err(PlanViolation::new(
            PlanViolationCode::EdgeSelfDependency,
            format!("item `{}` depends on itself", item.item_key()),
        ));
    }
    if !by_key.contains_key(target.as_str()) {
        return Err(PlanViolation::new(
            PlanViolationCode::EdgeReferenceMissing,
            format!(
                "item `{}` references `{}` which is not an item of this plan",
                item.item_key(),
                target
            ),
        ));
    }
    Ok(())
}

/// Depth-first walk over the sorted item list. Because both the node order and each
/// node's edge list are sorted, the cycle reported for a given plan is always the same
/// path, which keeps the message usable as a regression assertion.
pub(super) fn detect_cycle(
    items: &[PlanItem],
    by_key: &BTreeMap<&str, &PlanItem>,
) -> Result<(), PlanViolation> {
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    let mut path: Vec<&str> = Vec::new();
    for item in items {
        visit(item.item_key().as_str(), by_key, &mut state, &mut path)?;
    }
    Ok(())
}

fn visit<'a>(
    key: &'a str,
    by_key: &BTreeMap<&'a str, &'a PlanItem>,
    state: &mut BTreeMap<&'a str, u8>,
    path: &mut Vec<&'a str>,
) -> Result<(), PlanViolation> {
    match state.get(key).copied() {
        Some(VISITED) => return Ok(()),
        Some(VISITING) => {
            path.push(key);
            return Err(PlanViolation::new(
                PlanViolationCode::DependencyCycle,
                format!("dependency cycle: {}", path.join(" -> ")),
            ));
        }
        Some(_) | None => {}
    }
    state.insert(key, VISITING);
    path.push(key);
    if let Some(item) = by_key.get(key) {
        for target in item.depends_on().iter().chain(item.local_parents()) {
            visit(target.as_str(), by_key, state, path)?;
        }
    }
    path.pop();
    state.insert(key, VISITED);
    Ok(())
}

/// Kahn's algorithm over dependencies and local parents, ties broken by item key so two
/// runs over the same plan produce the same order.
pub(super) fn execution_order(items: &[PlanItem]) -> Result<Vec<ItemKey>, PlanViolation> {
    let mut blocked: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for item in items {
        let mut parents: BTreeSet<&str> = BTreeSet::new();
        parents.extend(item.depends_on().iter().map(|key| key.as_str()));
        parents.extend(item.local_parents().iter().map(|key| key.as_str()));
        // A self-edge would stall the queue forever; the gate rejects it earlier, and
        // dropping it here keeps this function usable on an ungated plan.
        parents.remove(item.item_key().as_str());
        blocked.insert(item.item_key().as_str(), parents);
    }
    // An ordered ready set is what makes the result deterministic: among items whose
    // parents are all emitted, the lexicographically smallest key goes first.
    let mut ready: BTreeSet<&str> = blocked
        .iter()
        .filter(|(_, parents)| parents.is_empty())
        .map(|(key, _)| *key)
        .collect();
    let mut emitted: BTreeSet<&str> = BTreeSet::new();
    let mut order: Vec<ItemKey> = Vec::with_capacity(items.len());
    while let Some(key) = ready.iter().next().copied() {
        ready.take(key);
        emitted.insert(key);
        order.push(ItemKey::new(key).expect("every key came from a validated item"));
        for parents in blocked.values_mut() {
            parents.remove(key);
        }
        for (candidate, pending) in &blocked {
            if pending.is_empty() && !emitted.contains(candidate) {
                ready.insert(*candidate);
            }
        }
    }
    if order.len() == items.len() {
        Ok(order)
    } else {
        Err(PlanViolation::new(
            PlanViolationCode::DependencyCycle,
            format!(
                "no execution order: {} of {} items remain blocked by a cycle",
                items.len() - order.len(),
                items.len()
            ),
        ))
    }
}
