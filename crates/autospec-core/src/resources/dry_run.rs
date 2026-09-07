//! The dry-run planner (spec `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`
//! §25, with the §13.3/§14.3 safety checks, §36 invariants and §48 backward
//! compatibility rules folded in).
//!
//! This is the LAST read-only layer before any phase is allowed to delete
//! something: a pure function from a slice of [`ObservedResource`] to a
//! [`DryRunPlan`]. It spawns no subprocess, opens no file, and touches no
//! ledger — an executor (spec §24/§38, out of scope here) consumes the plan;
//! producing one cannot mutate the world the plan describes.
//!
//! Two rules dominate every mapping below:
//!
//! - **§36 Invariant 1** — nothing may be proposed for removal unless the
//!   observation establishes ownership. `External` never is, and a claim of
//!   `RunExclusive` backed only by a name prefix (the legacy case, §48) is
//!   not evidence either; both degrade to `WOULD REPORT`.
//! - **§36 Invariants 2/3/5** — dirty state, unique/unpushed commits, and an
//!   expired lease standing alone are never proof for a destructive action.
//!   The first two route to `WOULD QUARANTINE` (§13.4 `DIRTY -> QUARANTINE`);
//!   the last only ever routes to `WOULD REPORT`.

use super::{ObservedResource, OwnershipClass, ResourceState, ResourceType};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// What the planner proposes for a single observed resource. Spec §25 names
/// exactly these three emissions; there is no "delete now" variant at this
/// layer because a dry run cannot act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposedAction {
    /// Reclaim the resource. Reserved for `RunExclusive` observations with no
    /// risk evidence — see [`plan`].
    WouldRemove,
    /// Set the resource aside for inspection instead of reclaiming it
    /// (spec §13.4, §26). The move itself is performed by the executor, not
    /// here.
    WouldQuarantine,
    /// Say nothing destructive; report the resource and why it is not safe
    /// (or not ours) to reclaim.
    WouldReport,
}

impl ProposedAction {
    /// Canonical §25 heading, exactly as it appears in the dry-run report.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposedAction::WouldRemove => "WOULD REMOVE",
            ProposedAction::WouldQuarantine => "WOULD QUARANTINE",
            ProposedAction::WouldReport => "WOULD REPORT",
        }
    }
}

impl fmt::Display for ProposedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable reason codes emitted by [`plan`]. They are part of the report
/// contract (machine-checkable, locale-free), and the observer's own evidence
/// prose is appended after them so a human reading the §25 block sees what
/// the observer actually saw.
mod reason {
    pub const OWNERSHIP_UNESTABLISHED: &str = "ownership_unestablished";
    pub const EXTERNAL_NEVER_DELETED: &str = "external_never_deleted";
    pub const OWNERSHIP_EVIDENCE_INSUFFICIENT: &str = "ownership_evidence_insufficient";
    pub const LEGACY_PREFIX_ONLY: &str = "legacy_prefix_only_ownership";
    pub const DIRTY_NEVER_DELETED: &str = "dirty_state_never_deleted";
    pub const DIRTY_NOT_QUARANTINABLE: &str = "dirty_state_not_quarantinable";
    pub const UNPUSHED_NEVER_DESTROYED: &str = "unique_unpushed_commits_never_destroyed";
    pub const EXPIRED_LEASE_INSUFFICIENT: &str = "expired_lease_insufficient_proof";
    pub const RUN_EXCLUSIVE_CLEAN: &str = "run_exclusive_with_ownership_proof_no_risk_evidence";
    pub const SHARED_REFERENCE_CHECKS: &str = "shared_ownership_requires_reference_checks";
}

/// Evidence vocabulary. Observers (git, Docker, process) record *why* they
/// classified a resource the way they did in `ObservedResource::reasons`; the
/// planner reads that evidence and nothing else. Matching runs per reason on a
/// normalized (lowercased, `_`-joined) form, so free prose ("uncommitted
/// changes detected") and token style (`dirty=true`) are both recognized, and
/// a token immediately preceded by a negation word ("no ledger_row") does not
/// count as evidence.
const DIRTY_EVIDENCE: &[&str] = &["dirty", "uncommitted", "untracked"];
const UNPUSHED_EVIDENCE: &[&str] = &[
    "unpushed",
    "not_pushed",
    "unique_commit",
    "unique_commits",
    "unreachable_commit",
];
const EXPIRED_LEASE_EVIDENCE: &[&str] = &[
    "lease_expired",
    "expired_lease",
    "lease_expires_at",
    "lease_lapsed",
];
/// Evidence that a run is finished, which is what turns "the lease is gone"
/// into something more than an absence of heartbeat (§36 Invariant 5).
const RUN_FINISHED_EVIDENCE: &[&str] = &[
    "run_completed",
    "completed_run",
    "run_finished",
    "finished_run",
    "run_merged",
    "pr_merged",
    "merged_to_main",
    "merged_into_main",
];
/// Positive ownership proof. A name prefix is deliberately NOT in this list:
/// per `model.rs`, a prefix match does not prove ownership.
const OWNERSHIP_PROOF_EVIDENCE: &[&str] = &[
    "created_by_run",
    "owned_by_run",
    "managed_by_run",
    "ledger_row",
    "ledger_record",
    "autospec_label",
    "label_autospec_run",
    "run_id",
    "work_item",
];
/// Evidence that the only thing tying the resource to AutoSpec is its name —
/// the §48 legacy case.
const PREFIX_ONLY_EVIDENCE: &[&str] = &[
    "name_prefix",
    "prefix_match",
    "name_only",
    "legacy",
    "not_in_ledger",
    "absent_from_ledger",
];
/// Words that invert the evidence token right after them. Observer prose says
/// "no run_id label" and "no untracked files"; counting those as positive
/// ownership proof or as dirt would be worse than counting nothing.
const NEGATION_WORDS: &[&str] = &[
    "no", "not", "never", "without", "absent", "missing", "lacks",
];

/// The §25 report row for one observation.
///
/// `owner` is the [`OwnershipClass`], not a run id: `ObservedResource`
/// (spec §11, issue #3186) carries the class the observer established, not
/// the run that owns the resource, and inventing a run-id field here would
/// put words in the observer's mouth.
///
/// `state` is `None` when the observation states no lifecycle state — see
/// [`DryRunEntry::state`]. Unknown is recorded as unknown, never coerced to
/// `Active`, exactly as `size_bytes` keeps unknown out of `0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunEntry {
    pub resource_type: ResourceType,
    pub external_id: String,
    pub owner: OwnershipClass,
    pub state: Option<ResourceState>,
    pub action: ProposedAction,
    /// Never empty: the leading element is always the stable reason code that
    /// decided `action`, followed by any secondary codes and the observer's
    /// evidence verbatim (spec §25 "Safety reason").
    pub safety_reasons: Vec<String>,
    /// Copied from the observation's `size_bytes` (#3190/#3191). `plan` never
    /// stats anything, so an unknown size stays `None` rather than `0`.
    pub reclaimable_bytes: Option<u64>,
}

impl DryRunEntry {
    /// The §25 text block for this entry.
    pub fn render(&self) -> String {
        let state = match self.state {
            Some(state) => state.as_str().to_string(),
            None => "unknown".to_string(),
        };
        let reclaimable = match self.reclaimable_bytes {
            Some(bytes) => bytes.to_string(),
            None => "unknown".to_string(),
        };
        let mut out = String::from(self.action.as_str());
        out.push('\n');
        out.push_str(&format!(
            "  {}: {}\n  owner: {}\n  state: {}\n",
            self.resource_type.as_str(),
            self.external_id,
            self.owner.as_str(),
            state
        ));
        for reason in &self.safety_reasons {
            out.push_str(&format!("  reason: {reason}\n"));
        }
        out.push_str(&format!("  reclaimable_bytes: {reclaimable}\n"));
        out
    }
}

/// Per-`ResourceType` roll-up of a plan (spec §24.4 totals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypeTotals {
    pub remove: usize,
    pub quarantine: usize,
    pub report: usize,
    /// Sum of the *known* `reclaimable_bytes` for the type. `None` only when
    /// no observation of that type reported a size — an unmeasured total is
    /// not a zero total.
    pub reclaimable_bytes: Option<u64>,
}

/// Totals keyed by canonical resource-type string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanTotals {
    pub by_type: BTreeMap<&'static str, TypeTotals>,
}

impl PlanTotals {
    pub fn for_type(&self, resource_type: ResourceType) -> Option<TypeTotals> {
        self.by_type.get(resource_type.as_str()).copied()
    }
}

/// The whole dry run: one [`DryRunEntry`] per input observation, in input
/// order, plus totals.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DryRunPlan {
    pub entries: Vec<DryRunEntry>,
    pub totals: PlanTotals,
}

impl DryRunPlan {
    /// The §25 text report.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&entry.render());
            out.push('\n');
        }
        out
    }
}

/// Turns observations into a plan. Pure: no subprocess, no filesystem access,
/// no ledger write.
pub fn plan(observations: &[ObservedResource]) -> DryRunPlan {
    let mut entries = Vec::with_capacity(observations.len());
    let mut totals = PlanTotals::default();

    for observed in observations {
        let entry = propose(observed);
        let bucket = totals
            .by_type
            .entry(entry.resource_type.as_str())
            .or_default();
        match entry.action {
            ProposedAction::WouldRemove => bucket.remove += 1,
            ProposedAction::WouldQuarantine => bucket.quarantine += 1,
            ProposedAction::WouldReport => bucket.report += 1,
        }
        if let Some(bytes) = entry.reclaimable_bytes {
            bucket.reclaimable_bytes = Some(match bucket.reclaimable_bytes {
                Some(total) => total.saturating_add(bytes),
                None => bytes,
            });
        }
        entries.push(entry);
    }

    DryRunPlan { entries, totals }
}

/// The single-resource decision, ordered most-conservative first.
fn propose(observed: &ObservedResource) -> DryRunEntry {
    let evidence = normalize(&observed.reasons);
    let codes: &[&str] = if observed.ownership == OwnershipClass::External {
        // §36 Invariant 4 / Invariant 1 — never ours, whatever the evidence says.
        &[
            reason::OWNERSHIP_UNESTABLISHED,
            reason::EXTERNAL_NEVER_DELETED,
        ]
    } else if !any_match(&evidence, OWNERSHIP_PROOF_EVIDENCE) {
        // §36 Invariant 1, §48 — a class claim with no evidence behind it
        // (legacy prefix match, empty reasons) is not ownership.
        if any_match(&evidence, PREFIX_ONLY_EVIDENCE) {
            &[
                reason::OWNERSHIP_EVIDENCE_INSUFFICIENT,
                reason::LEGACY_PREFIX_ONLY,
            ]
        } else {
            &[reason::OWNERSHIP_EVIDENCE_INSUFFICIENT]
        }
    } else if any_match(&evidence, DIRTY_EVIDENCE) {
        // §13.4 DIRTY -> QUARANTINE, §36 Invariant 2.
        if is_git(observed.resource_type) {
            &[reason::DIRTY_NEVER_DELETED]
        } else {
            // Quarantine is specified for git worktrees (§26); a Docker or
            // process resource with dirty-ish evidence is reported instead.
            &[reason::DIRTY_NOT_QUARANTINABLE]
        }
    } else if any_match(&evidence, UNPUSHED_EVIDENCE) {
        // §36 Invariant 3 — set aside, never destroy.
        if is_git(observed.resource_type) {
            &[reason::UNPUSHED_NEVER_DESTROYED]
        } else {
            &[reason::DIRTY_NOT_QUARANTINABLE]
        }
    } else if any_match(&evidence, EXPIRED_LEASE_EVIDENCE)
        && !any_match(&evidence, RUN_FINISHED_EVIDENCE)
    {
        // §36 Invariant 5 — an expired lease alone proves nothing.
        &[reason::EXPIRED_LEASE_INSUFFICIENT]
    } else if observed.ownership == OwnershipClass::RunExclusive {
        &[reason::RUN_EXCLUSIVE_CLEAN]
    } else {
        // §11 — shared classes need reference-count/lease checks this layer
        // cannot perform, so they are reported, never removed.
        &[reason::SHARED_REFERENCE_CHECKS]
    };

    let action = match codes[0] {
        reason::DIRTY_NEVER_DELETED | reason::UNPUSHED_NEVER_DESTROYED => {
            ProposedAction::WouldQuarantine
        }
        reason::RUN_EXCLUSIVE_CLEAN => ProposedAction::WouldRemove,
        _ => ProposedAction::WouldReport,
    };

    let mut safety_reasons: Vec<String> = codes.iter().map(|code| (*code).to_string()).collect();
    // The observer's evidence prose follows the codes verbatim: §25 shows a
    // human-readable `reason:` line ("run completed 3h ago"), and the plan is
    // the artifact the report is printed from.
    safety_reasons.extend(observed.reasons.iter().cloned());

    DryRunEntry {
        resource_type: observed.resource_type,
        external_id: observed.external_id.clone(),
        owner: observed.ownership,
        state: stated_state(&observed.reasons),
        action,
        safety_reasons,
        reclaimable_bytes: observed.size_bytes,
    }
}

/// Quarantine is initially required for git worktrees only (spec §26);
/// branches carry the same unpushed-commit hazard (§14.3).
fn is_git(resource_type: ResourceType) -> bool {
    matches!(
        resource_type,
        ResourceType::GitWorktree | ResourceType::GitBranch
    )
}

/// Lowercases each reason and collapses separators to `_` so `uncommitted
/// changes` and `uncommitted_changes` match the same token. Kept one string
/// per reason: negation is judged within a single observation reason, never
/// across two unrelated ones.
fn normalize(reasons: &[String]) -> Vec<String> {
    reasons
        .iter()
        .map(|reason| reason.to_lowercase().replace(['-', '/', ':', ' '], "_"))
        .collect()
}

fn any_match(evidence: &[String], needles: &[&str]) -> bool {
    evidence
        .iter()
        .any(|reason| needles.iter().any(|n| appears_positive(reason, n)))
}

/// True when `needle` occurs in `reason` without being negated immediately
/// before it (`no_ledger_row`, `not created_by_run`, …).
fn appears_positive(reason: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = reason[from..].find(needle) {
        let start = from + offset;
        if !negated(&reason[..start]) {
            return true;
        }
        from = start + needle.len();
    }
    false
}

fn negated(prefix: &str) -> bool {
    let word = prefix
        .trim_end_matches('_')
        .rsplit('_')
        .next()
        .unwrap_or("");
    NEGATION_WORDS.contains(&word)
}

/// Reads an explicit `state=<canonical>` reason, if the observer stated one.
/// `ObservedResource` carries no ledger state, so anything unstated stays
/// `None`; an unrecognized token is unknown too, never a coerced default.
fn stated_state(reasons: &[String]) -> Option<ResourceState> {
    reasons.iter().find_map(|reason| {
        let rest = reason.strip_prefix("state=")?;
        ResourceState::from_str(rest).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(
        resource_type: ResourceType,
        external_id: &str,
        ownership: OwnershipClass,
        reasons: &[&str],
        size_bytes: Option<u64>,
    ) -> ObservedResource {
        ObservedResource {
            resource_type,
            external_id: external_id.to_string(),
            ownership,
            reasons: reasons.iter().map(|reason| reason.to_string()).collect(),
            size_bytes,
        }
    }

    fn action_of(observation: &ObservedResource) -> ProposedAction {
        plan(std::slice::from_ref(observation)).entries[0].action
    }

    // ── AC: one entry per observation, input order preserved ─────────────

    #[test]
    fn plan_returns_one_entry_per_input_observation_in_order() {
        let observations = vec![
            observed(
                ResourceType::GitWorktree,
                "/tmp/wt-a",
                OwnershipClass::RunExclusive,
                &["created_by_run: as-1"],
                Some(1),
            ),
            observed(
                ResourceType::DockerContainer,
                "container-b",
                OwnershipClass::External,
                &["no autospec label present"],
                None,
            ),
            observed(
                ResourceType::BuildCache,
                "buildx-cache-c",
                OwnershipClass::GlobalShared,
                &["shared across repositories"],
                Some(3),
            ),
        ];

        let plan = plan(&observations);
        assert_eq!(plan.entries.len(), observations.len());
        assert_eq!(plan.entries[0].external_id, "/tmp/wt-a");
        assert_eq!(plan.entries[1].external_id, "container-b");
        assert_eq!(plan.entries[2].external_id, "buildx-cache-c");
    }

    #[test]
    fn plan_of_no_observations_is_empty() {
        let plan = plan(&[]);
        assert!(plan.entries.is_empty());
        assert!(plan.totals.by_type.is_empty());
        assert_eq!(plan.render(), "");
    }

    // ── AC: External never yields WouldRemove ────────────────────────────

    #[test]
    fn external_observation_never_yields_would_remove() {
        for resource_type in ResourceType::ALL {
            // Even the most favorable evidence cannot lift External into a
            // removal proposal (§36 Invariant 4).
            for reasons in [
                vec![
                    "run completed 3h ago".to_string(),
                    "run_id=as-1".to_string(),
                ],
                vec!["ledger_row present; run_finished".to_string()],
                vec![],
            ] {
                let observation = observed(
                    *resource_type,
                    "external-thing",
                    OwnershipClass::External,
                    &reasons.iter().map(String::as_str).collect::<Vec<_>>(),
                    Some(1024),
                );
                let entry = &plan(&[observation]).entries[0];
                assert_ne!(entry.action, ProposedAction::WouldRemove);
                assert_eq!(entry.action, ProposedAction::WouldReport);
                assert!(entry
                    .safety_reasons
                    .contains(&reason::OWNERSHIP_UNESTABLISHED.to_string()));
                assert!(entry
                    .safety_reasons
                    .contains(&reason::EXTERNAL_NEVER_DELETED.to_string()));
            }
        }
    }

    // ── AC: non-empty safety_reasons on every entry ──────────────────────

    #[test]
    fn every_entry_carries_non_empty_safety_reasons() {
        for ownership in OwnershipClass::ALL {
            for reasons in [
                vec![],
                vec!["dirty worktree".to_string()],
                vec!["lease expired".to_string()],
                vec!["created_by_run as-1; run completed 3h ago".to_string()],
                vec!["legacy branch matching autospec/ prefix".to_string()],
            ] {
                for resource_type in ResourceType::ALL {
                    let observation = observed(
                        *resource_type,
                        "x",
                        *ownership,
                        &reasons.iter().map(String::as_str).collect::<Vec<_>>(),
                        None,
                    );
                    let entry = &plan(&[observation]).entries[0];
                    assert!(
                        !entry.safety_reasons.is_empty(),
                        "{resource_type:?}/{ownership:?} with {reasons:?} produced no reason"
                    );
                    assert!(!entry.safety_reasons[0].is_empty());
                }
            }
        }
    }

    // ── WOULD REMOVE mapping (RunExclusive only) ─────────────────────────

    #[test]
    fn run_exclusive_with_proof_and_no_risk_evidence_yields_would_remove() {
        let observation = observed(
            ResourceType::DockerContainer,
            "autospec-test-a31f",
            OwnershipClass::RunExclusive,
            &["ledger_row present", "run completed 3h ago"],
            Some(4096),
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldRemove);
        assert_eq!(entry.safety_reasons[0], reason::RUN_EXCLUSIVE_CLEAN);
    }

    #[test]
    fn would_remove_is_never_proposed_for_a_shared_class() {
        for ownership in [OwnershipClass::RepoShared, OwnershipClass::GlobalShared] {
            let observation = observed(
                ResourceType::DockerVolume,
                "shared-cache",
                ownership,
                &["created_by_run as-1", "run_finished"],
                Some(8),
            );
            let entry = &plan(&[observation]).entries[0];
            assert_ne!(entry.action, ProposedAction::WouldRemove);
            assert_eq!(entry.action, ProposedAction::WouldReport);
            assert!(entry
                .safety_reasons
                .contains(&reason::SHARED_REFERENCE_CHECKS.to_string()));
        }
    }

    // ── Misattribution / legacy (§48) ────────────────────────────────────

    #[test]
    fn legacy_branch_without_ownership_metadata_yields_would_report() {
        // The counter-team's misattribution case: a branch that merely looks
        // like ours. A name prefix is not ownership, so it is reported.
        let observation = observed(
            ResourceType::GitBranch,
            "refs/heads/autospec/legacy/thing",
            OwnershipClass::RunExclusive,
            &["legacy branch matching the autospec/ name_prefix only"],
            Some(2048),
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert_eq!(
            entry.safety_reasons[0],
            reason::OWNERSHIP_EVIDENCE_INSUFFICIENT
        );
        assert!(entry
            .safety_reasons
            .contains(&reason::LEGACY_PREFIX_ONLY.to_string()));
    }

    #[test]
    fn run_exclusive_claim_with_no_evidence_at_all_yields_would_report() {
        let observation = observed(
            ResourceType::GitWorktree,
            "/tmp/wt-claim",
            OwnershipClass::RunExclusive,
            &[],
            None,
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert_eq!(
            entry.safety_reasons[0],
            reason::OWNERSHIP_EVIDENCE_INSUFFICIENT
        );
    }

    // ── Negated evidence is not evidence ──────────────────────────────

    #[test]
    fn negated_ownership_evidence_yields_would_report() {
        let observation = observed(
            ResourceType::GitWorktree,
            "/tmp/wt-nolabel",
            OwnershipClass::RunExclusive,
            &["no run_id label present", "no ledger_row"],
            None,
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert_eq!(
            entry.safety_reasons[0],
            reason::OWNERSHIP_EVIDENCE_INSUFFICIENT
        );
    }

    #[test]
    fn negated_dirt_does_not_trigger_quarantine() {
        let observation = observed(
            ResourceType::GitWorktree,
            "/tmp/wt-clean",
            OwnershipClass::RunExclusive,
            &[
                "created_by_run as-1",
                "no untracked files",
                "worktree is not dirty",
            ],
            Some(12),
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldRemove);
    }

    #[test]
    fn appears_positive_skips_only_negated_occurrences() {
        assert!(!appears_positive("no_ledger_row", "ledger_row"));
        assert!(appears_positive("ledger_row_present", "ledger_row"));
        assert!(!appears_positive("worktree_is_not_dirty", "dirty"));
        assert!(!appears_positive("no_untracked_files", "untracked"));
        assert!(appears_positive("untracked_files_detected", "untracked"));
        assert!(appears_positive("worktree_is_dirty", "dirty"));
        // A negation word two tokens away does not invert the token.
        assert!(appears_positive("no_label_but_ledger_row", "ledger_row"));
        // The needle that spells out its own negation still matches (§14.3
        // "not pushed" is itself the risk signal).
        assert!(appears_positive("commits_not_pushed", "not_pushed"));
    }

    // ── Dirty / unpushed (§13.4, §36 Invariants 2 and 3) ─────────────────

    #[test]
    fn dirty_worktree_yields_would_quarantine() {
        let observation = observed(
            ResourceType::GitWorktree,
            "/repo/.autospec/worktrees/as-1/implement",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1", "uncommitted changes detected"],
            Some(10),
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldQuarantine);
        assert_eq!(entry.safety_reasons[0], reason::DIRTY_NEVER_DELETED);
    }

    #[test]
    fn unpushed_branch_yields_would_quarantine() {
        let observation = observed(
            ResourceType::GitBranch,
            "refs/heads/feat/thing",
            OwnershipClass::RunExclusive,
            &["ledger_row present", "2 unpushed commits"],
            None,
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldQuarantine);
        assert_eq!(entry.safety_reasons[0], reason::UNPUSHED_NEVER_DESTROYED);
    }

    #[test]
    fn dirty_state_on_a_non_git_resource_is_reported_not_quarantined() {
        // §26 scopes quarantine to git worktrees initially.
        let observation = observed(
            ResourceType::DockerVolume,
            "vol-1",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1", "dirty state"],
            None,
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert_eq!(entry.safety_reasons[0], reason::DIRTY_NOT_QUARANTINABLE);
    }

    // ── Expired lease alone (§36 Invariant 5) ────────────────────────────

    #[test]
    fn expired_lease_alone_yields_would_report() {
        let observation = observed(
            ResourceType::GitWorktree,
            "/repo/.autospec/worktrees/as-2/implement",
            OwnershipClass::RunExclusive,
            &["created_by_run as-2", "lease expired 40m ago"],
            Some(7),
        );
        let entry = &plan(&[observation]).entries[0];
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert_eq!(entry.safety_reasons[0], reason::EXPIRED_LEASE_INSUFFICIENT);
    }

    #[test]
    fn expired_lease_plus_completed_run_yields_would_remove() {
        // The lease is no longer the sole proof: the observer stated the run
        // finished.
        let observation = observed(
            ResourceType::DockerContainer,
            "ctr-9",
            OwnershipClass::RunExclusive,
            &[
                "ledger_row present",
                "lease expired",
                "run completed 3h ago",
            ],
            Some(7),
        );
        assert_eq!(action_of(&observation), ProposedAction::WouldRemove);
    }

    // ── AC: size passthrough (#3190/#3191) ───────────────────────────────

    #[test]
    fn known_size_yields_matching_non_zero_reclaimable_bytes() {
        let observation = observed(
            ResourceType::DockerImage,
            "sha256:abc",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1"],
            Some(123_456_789),
        );
        let plan = plan(&[observation]);
        let entry = &plan.entries[0];
        assert_eq!(entry.reclaimable_bytes, Some(123_456_789));
        assert_ne!(entry.reclaimable_bytes, Some(0));
        assert_eq!(
            plan.totals
                .for_type(ResourceType::DockerImage)
                .expect("docker_image totals")
                .reclaimable_bytes,
            Some(123_456_789)
        );
    }

    #[test]
    fn unknown_size_stays_unknown_never_zero() {
        let observation = observed(
            ResourceType::TempFile,
            "/tmp/scratch",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1"],
            None,
        );
        let plan = plan(&[observation]);
        assert_eq!(plan.entries[0].reclaimable_bytes, None);
        assert_eq!(
            plan.totals
                .for_type(ResourceType::TempFile)
                .unwrap()
                .reclaimable_bytes,
            None
        );
        assert!(plan.render().contains("reclaimable_bytes: unknown"));
    }

    #[test]
    fn measured_zero_size_is_kept_as_zero() {
        let observation = observed(
            ResourceType::TempFile,
            "/tmp/empty",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1"],
            Some(0),
        );
        let plan = plan(&[observation]);
        assert_eq!(plan.entries[0].reclaimable_bytes, Some(0));
        assert_eq!(
            plan.totals
                .for_type(ResourceType::TempFile)
                .unwrap()
                .reclaimable_bytes,
            Some(0)
        );
    }

    // ── Totals ───────────────────────────────────────────────────────────

    #[test]
    fn totals_are_keyed_by_resource_type_and_count_every_entry() {
        let observations = vec![
            observed(
                ResourceType::GitWorktree,
                "/wt/dirty",
                OwnershipClass::RunExclusive,
                &["created_by_run as-1", "uncommitted changes"],
                Some(100),
            ),
            observed(
                ResourceType::GitWorktree,
                "/wt/clean",
                OwnershipClass::RunExclusive,
                &["created_by_run as-1", "run completed"],
                Some(50),
            ),
            observed(
                ResourceType::DockerContainer,
                "ctr-external",
                OwnershipClass::External,
                &["no autospec label"],
                None,
            ),
        ];

        let plan = plan(&observations);
        let worktrees = plan
            .totals
            .for_type(ResourceType::GitWorktree)
            .expect("worktrees");
        assert_eq!(
            (worktrees.remove, worktrees.quarantine, worktrees.report),
            (1, 1, 0)
        );
        assert_eq!(worktrees.reclaimable_bytes, Some(150));

        let containers = plan
            .totals
            .for_type(ResourceType::DockerContainer)
            .expect("containers");
        assert_eq!(
            (containers.remove, containers.quarantine, containers.report),
            (0, 0, 1)
        );
        assert_eq!(containers.reclaimable_bytes, None);

        assert!(plan
            .totals
            .for_type(ResourceType::PortReservation)
            .is_none());

        let counted: usize = plan
            .totals
            .by_type
            .values()
            .map(|totals| totals.remove + totals.quarantine + totals.report)
            .sum();
        assert_eq!(counted, plan.entries.len());
    }

    // ── State passthrough ────────────────────────────────────────────────

    #[test]
    fn stated_state_is_parsed_and_unstated_state_stays_none() {
        let stated = observed(
            ResourceType::GitWorktree,
            "/wt/1",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1", "state=orphaned"],
            None,
        );
        assert_eq!(
            plan(&[stated]).entries[0].state,
            Some(ResourceState::Orphaned)
        );

        let unstated = observed(
            ResourceType::GitWorktree,
            "/wt/2",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1"],
            None,
        );
        assert_eq!(plan(&[unstated]).entries[0].state, None);

        // An unrecognized state token is unknown, never a coerced default.
        let bogus = observed(
            ResourceType::GitWorktree,
            "/wt/3",
            OwnershipClass::RunExclusive,
            &["created_by_run as-1", "state=definitely_not_a_state"],
            None,
        );
        assert_eq!(plan(&[bogus]).entries[0].state, None);
    }

    // ── §25 rendering ────────────────────────────────────────────────────

    #[test]
    fn render_emits_the_section_25_block_shape() {
        let observations = vec![
            observed(
                ResourceType::DockerContainer,
                "autospec-test-a31f",
                OwnershipClass::RunExclusive,
                &["ledger_row present", "run completed 3h ago"],
                Some(4096),
            ),
            observed(
                ResourceType::GitWorktree,
                "/repo/.autospec/worktrees/as-1/implement",
                OwnershipClass::RunExclusive,
                &["created_by_run as-1", "uncommitted changes detected"],
                None,
            ),
        ];

        let rendered = plan(&observations).render();
        assert!(rendered.starts_with("WOULD REMOVE\n"));
        assert!(rendered.contains("  docker_container: autospec-test-a31f\n"));
        assert!(rendered.contains("  owner: run_exclusive\n"));
        assert!(rendered.contains("  reason: run completed 3h ago\n"));
        assert!(rendered.contains("  reclaimable_bytes: 4096\n"));
        assert!(rendered.contains("WOULD QUARANTINE\n"));
        assert!(rendered.contains("  git_worktree: /repo/.autospec/worktrees/as-1/implement\n"));
    }

    // ── AC: purity — no subprocess, no filesystem, no ledger write ───────

    #[test]
    fn module_source_contains_no_process_command_or_filesystem_access() {
        let source = include_str!("dry_run.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("test module marker present");

        // Built by concatenation so this test's own literals cannot satisfy
        // (or defeat) the scan.
        let forbidden = [
            concat!("std::process", "::Command"),
            concat!("std::", "process"),
            concat!("std::", "fs::"),
            concat!("std::", "fs,"),
            "File::create",
            "OpenOptions",
            "Ledger",
            "sqlite",
        ];
        for token in forbidden {
            assert!(
                !production.contains(token),
                "dry_run.rs must not reference {token:?}: found in the production section"
            );
        }
        // The AC's literal count.
        let command_token = format!("{}::{}", "std::process", "Command");
        assert_eq!(source.matches(command_token.as_str()).count(), 0);
    }

    #[test]
    fn plan_performs_no_filesystem_or_ledger_write() {
        // Snapshot a private scratch directory, run the planner over a mixed
        // batch, and require the directory to be untouched, with nothing new
        // created beside it.
        let dir = std::env::temp_dir().join(format!(
            "autospec-dry-run-purity-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        std::fs::write(dir.join("sentinel.txt"), b"unchanged").expect("write sentinel");

        let before = snapshot(&dir);
        let observations = generated_batch(200);
        let produced = plan(&observations);
        let after = snapshot(&dir);

        assert_eq!(before, after, "plan() mutated the filesystem");
        assert!(!produced.entries.is_empty());
        // Nothing new is created beside our own scratch directory.
        let own_prefix = format!("autospec-dry-run-purity-{}-", std::process::id());
        let siblings: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .expect("read temp dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&own_prefix))
            .collect();
        assert_eq!(
            siblings.len(),
            1,
            "plan() created a sibling scratch directory under {own_prefix}*"
        );

        std::fs::remove_dir_all(&dir).expect("remove scratch dir");
    }

    // ── AC: 6,500-observation property test ──────────────────────────────

    #[test]
    fn property_6500_generated_observations_never_propose_removing_external() {
        const COUNT: usize = 6_500;
        let observations = generated_batch(COUNT);
        assert_eq!(observations.len(), COUNT);

        let plan = plan(&observations);
        assert_eq!(plan.entries.len(), COUNT);

        let mut external = 0usize;
        let mut removed = 0usize;
        let mut quarantined = 0usize;
        for (observation, entry) in observations.iter().zip(plan.entries.iter()) {
            // No External entry yields WouldRemove — the headline invariant.
            if observation.ownership == OwnershipClass::External {
                external += 1;
                assert_ne!(
                    entry.action,
                    ProposedAction::WouldRemove,
                    "External {} was proposed for removal with reasons {:?}",
                    observation.external_id,
                    observation.reasons
                );
            }
            // WOULD REMOVE is reserved for RunExclusive (spec §11).
            if entry.action == ProposedAction::WouldRemove {
                removed += 1;
                assert_eq!(entry.owner, OwnershipClass::RunExclusive);
                assert!(observation.ownership.is_reclaimable());
            }
            if entry.action == ProposedAction::WouldQuarantine {
                quarantined += 1;
                assert!(
                    matches!(
                        entry.resource_type,
                        ResourceType::GitWorktree | ResourceType::GitBranch
                    ),
                    "quarantine proposed for a non-git resource"
                );
            }
            // Every entry explains itself.
            assert!(!entry.safety_reasons.is_empty());
            assert!(!entry.safety_reasons[0].is_empty());
            // Identity and size are copied, never re-derived.
            assert_eq!(entry.resource_type, observation.resource_type);
            assert_eq!(entry.external_id, observation.external_id);
            assert_eq!(entry.owner, observation.ownership);
            assert_eq!(entry.reclaimable_bytes, observation.size_bytes);
        }

        // The generator must actually exercise the classes it claims to.
        assert!(external > 0, "generator produced no External observations");
        assert!(removed > 0, "generator produced no removals");
        assert!(quarantined > 0, "generator produced no quarantines");

        // Totals agree with the entries, per type and in aggregate.
        let mut accounted = 0usize;
        let mut bytes_known = 0u64;
        for totals in plan.totals.by_type.values() {
            accounted += totals.remove + totals.quarantine + totals.report;
            bytes_known += totals.reclaimable_bytes.unwrap_or(0);
        }
        assert_eq!(accounted, COUNT);
        let declared: u64 = observations
            .iter()
            .filter_map(|observation| observation.size_bytes)
            .sum();
        assert_eq!(bytes_known, declared);
    }

    // ── generators ───────────────────────────────────────────────────────

    /// Deterministic pseudo-random observations over every type, ownership
    /// class, size and evidence shape. A plain LCG: the property must be
    /// reproducible, and no new dependency is added for it.
    fn generated_batch(count: usize) -> Vec<ObservedResource> {
        const REASONS: &[&[&str]] = &[
            &[],
            &["no autospec label present"],
            &["legacy branch matching the autospec/ name_prefix only"],
            &["created_by_run as-20260816-221904-a31f"],
            &["ledger_row present", "run completed 3h ago"],
            &["created_by_run as-1", "uncommitted changes detected"],
            &["created_by_run as-1", "3 unpushed commits"],
            &["ledger_row present", "lease expired 40m ago"],
            &[
                "ledger_row present",
                "lease expired",
                "run completed 3h ago",
            ],
            &["autospec_label io.autospec.run_id=as-1", "state=orphaned"],
            &["shared across repositories", "run_id=as-1"],
            &["name_prefix match on autospec-"],
        ];

        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        (0..count)
            .map(|index| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let pick = |shift: u32, len: usize| ((state >> shift) % (len as u64)) as usize;
                let reasons: Vec<String> = REASONS[pick(7, REASONS.len())]
                    .iter()
                    .map(|reason| reason.to_string())
                    .collect();
                let size_bytes = match pick(13, 4) {
                    0 => None,
                    1 => Some(0),
                    n => Some(n as u64 * 4096 + index as u64),
                };
                ObservedResource {
                    resource_type: ResourceType::ALL[pick(17, ResourceType::ALL.len())],
                    external_id: format!("gen-{index:05}"),
                    ownership: OwnershipClass::ALL[pick(23, OwnershipClass::ALL.len())],
                    reasons,
                    size_bytes,
                }
            })
            .collect()
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    }

    fn snapshot(dir: &std::path::Path) -> Vec<(String, u64)> {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current).expect("read_dir") {
                let entry = entry.expect("dir entry");
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let len = entry.metadata().expect("metadata").len();
                    files.push((path.to_string_lossy().into_owned(), len));
                }
            }
        }
        files.sort();
        files
    }
}
