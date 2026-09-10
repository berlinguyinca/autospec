//! Dry-run planner (spec §25, §36, §48).
//!
//! Pure function: given a list of [`ObservedResource`]s (as produced by the
//! Phase-1 observers in `git.rs`, `docker.rs`, and `process.rs`), `plan`
//! returns a [`DryRunPlan`] of [`DryRunEntry`]s and per-`ResourceType`
//! totals. It emits `WOULD REMOVE` / `WOULD QUARANTINE` / `WOULD REPORT`
//! lines for the CLI report (spec §25) **without performing any mutation**:
//! no deletion, no quarantine move, no filesystem write, no ledger write, no
//! process signal.
//!
//! The action mapping encodes the §36 invariants:
//!
//! * **Invariant 1 + 4 + §48** — a resource AutoSpec cannot establish
//!   ownership of (`OwnershipClass::External`, which is also where legacy
//!   resources without ownership metadata land) MUST be `WOULD REPORT`
//!   (`ownership_unestablished`), never `WOULD REMOVE`.
//! * **Invariant 2 + §13.4** — a `RunExclusive` Git worktree with the
//!   observer's `dirty` reason is `DIRTY -> QUARANTINE`.
//! * **Invariant 3** — a `RunExclusive` Git entry with the observer's
//!   `has_unpushed_commits` reason is `WOULD QUARANTINE`, never destroyed.
//! * **Invariant 5** — a `RunExclusive` entry whose only hold is
//!   `lease_expired` stays `WOULD REPORT`: an expired lease alone is not
//!   proof of abandonment.
//! * **§11** — `RepoShared` / `GlobalShared` entries are `WOULD REPORT`
//!   pending reference-count / lease checks.
//!
//! The dirty/unpushed/lease-expired tokens are matched by exact equality
//! against the reasons emitted by the observers (`git.rs` and `process.rs`);
//! free-form reason text never silently triggers quarantine.
//!
//! `reclaimable_bytes` mirrors `ObservedResource::size_bytes` exactly —
//! `None` (unknown) stays `None`; it is never coerced to `0`, because `0`
//! means "measured, genuinely empty" in this codebase.

use serde::{Deserialize, Serialize};

use super::model::{ObservedResource, OwnershipClass, ResourceType};

/// Observer reason token: dirty Git worktree (emitted by `git.rs`).
pub const REASON_DIRTY: &str = "dirty";
/// Observer reason token: unpushed commits on a Git entry (emitted by `git.rs`).
pub const REASON_UNPUSHED: &str = "has_unpushed_commits";
/// Observer reason token: expired lease (emitted by `process.rs`).
pub const REASON_LEASE_EXPIRED: &str = "lease_expired";
/// Observer reason token: clean Git worktree (emitted by `git.rs`).
pub const REASON_CLEAN: &str = "clean";
/// Observer reason token: branch merged into origin/main (emitted by `git.rs`).
pub const REASON_MERGED: &str = "merged_into_origin_main";
/// Observer reason token: branch not merged into origin/main (emitted by `git.rs`).
pub const REASON_NOT_MERGED: &str = "not_merged_into_origin_main";

/// The proposed action for one observed resource in a dry-run (spec §25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposedAction {
    /// `WOULD REMOVE` — only ever emitted for `RunExclusive` entries with no
    /// dirty/unpushed/lease-expired hold (spec §25, §36 Invariants 1–5).
    WouldRemove,
    /// `WOULD QUARANTINE` — dirty or unpushed `RunExclusive` Git entries
    /// (spec §13.4 `DIRTY -> QUARANTINE`, §36 Invariants 2–3).
    WouldQuarantine,
    /// `WOULD REPORT` — everything the planner must not touch: unestablished
    /// or shared ownership, and expired-lease-alone holds (spec §36
    /// Invariants 1, 4, 5; §48).
    WouldReport,
}

impl ProposedAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposedAction::WouldRemove => "would_remove",
            ProposedAction::WouldQuarantine => "would_quarantine",
            ProposedAction::WouldReport => "would_report",
        }
    }
}

/// One dry-run line for one observed resource (spec §25 fields: resource,
/// owner, current state, proposed action, safety reason, estimated
/// reclaimed space when available).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunEntry {
    pub resource_type: ResourceType,
    pub external_id: String,
    /// The observation's ownership class — the "owner" column.
    pub owner: OwnershipClass,
    /// Deterministic one-token summary of the observation's current state,
    /// derived from its reason tokens (`dirty` | `unpushed` | `lease_expired`
    /// | `clean` | `merged` | `unmerged` | `unknown`).
    pub state: String,
    pub action: ProposedAction,
    /// Non-empty for every entry, by construction (spec §25: every line
    /// carries a safety reason).
    pub safety_reasons: Vec<String>,
    /// Estimated reclaimable space; `None` (unknown) stays `None`, never 0.
    pub reclaimable_bytes: Option<u64>,
}

/// Per-`ResourceType` rollup for the dry-run report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTypeTotals {
    pub count: usize,
    pub would_remove: usize,
    pub would_quarantine: usize,
    pub would_report: usize,
    /// Sum of `reclaimable_bytes` over entries with a known size; `None` if
    /// any entry's size is unknown (unknown stays unknown, never 0).
    pub reclaimable_bytes: Option<u64>,
}

/// The dry-run plan: one [`DryRunEntry`] per input observation (same order)
/// plus one totals row per `ResourceType` present, in `ResourceType::ALL`
/// order (deterministic).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunPlan {
    pub entries: Vec<DryRunEntry>,
    pub totals: Vec<(ResourceType, ResourceTypeTotals)>,
}

/// Plan a dry-run over `observations` without mutating anything.
///
/// Pure: reads only the observations; the output order is the input order,
/// so a caller can zip inputs and entries. See the module docs for the
/// action mapping and the invariants it encodes.
pub fn plan(observations: &[ObservedResource]) -> DryRunPlan {
    let mut entries: Vec<DryRunEntry> = Vec::with_capacity(observations.len());
    // One totals slot per ResourceType, addressed via ALL (exhaustive), so
    // no `Ord` bound is needed and the output order is deterministic.
    let mut slots: Vec<Option<(ResourceType, ResourceTypeTotals)>> =
        ResourceType::ALL.iter().map(|_| None).collect();

    for obs in observations {
        let (action, safety_reasons) = propose(obs);
        let entry = DryRunEntry {
            resource_type: obs.resource_type,
            external_id: obs.external_id.clone(),
            owner: obs.ownership,
            state: state_summary(obs).to_string(),
            action,
            safety_reasons,
            // Mirrors size_bytes exactly: None (unknown) never becomes 0.
            reclaimable_bytes: obs.size_bytes,
        };

        let idx = ResourceType::ALL
            .iter()
            .position(|t| *t == entry.resource_type)
            .expect("ResourceType::ALL is exhaustive");
        let slot = slots[idx].get_or_insert_with(|| {
            (
                entry.resource_type,
                ResourceTypeTotals {
                    count: 0,
                    would_remove: 0,
                    would_quarantine: 0,
                    would_report: 0,
                    reclaimable_bytes: Some(0),
                },
            )
        });
        slot.1.count += 1;
        match entry.action {
            ProposedAction::WouldRemove => slot.1.would_remove += 1,
            ProposedAction::WouldQuarantine => slot.1.would_quarantine += 1,
            ProposedAction::WouldReport => slot.1.would_report += 1,
        }
        slot.1.reclaimable_bytes = match (slot.1.reclaimable_bytes, entry.reclaimable_bytes) {
            // Any unknown size makes the whole type total unknown.
            (_, None) => None,
            (Some(accumulated), Some(bytes)) => Some(accumulated.saturating_add(bytes)),
            (None, Some(_)) => unreachable!("the (_, None) arm above catches it"),
        };

        entries.push(entry);
    }

    let totals = slots.into_iter().flatten().collect();
    DryRunPlan { entries, totals }
}

/// The §36-invariant action mapping for one observation. The returned
/// `safety_reasons` is non-empty for every branch (spec §25).
fn propose(obs: &ObservedResource) -> (ProposedAction, Vec<String>) {
    match obs.ownership {
        // Invariant 1 + 4, §48: no established ownership → report only.
        // Legacy resources without ownership metadata land here too.
        OwnershipClass::External => (
            ProposedAction::WouldReport,
            vec![
                "ownership_unestablished".to_string(),
                "spec §36 Invariant 1: never delete a resource whose ownership cannot be reasonably established".to_string(),
                "spec §48: legacy resources without ownership metadata are reported, never auto-deleted".to_string(),
            ],
        ),
        // §11: shared ownership needs reference-count / lease checks first.
        OwnershipClass::RepoShared => (
            ProposedAction::WouldReport,
            vec![
                "shared_ownership".to_string(),
                "spec §11: repo_shared reclaim requires reference-count and lease checks before any destructive action".to_string(),
            ],
        ),
        OwnershipClass::GlobalShared => (
            ProposedAction::WouldReport,
            vec![
                "shared_ownership".to_string(),
                "spec §11: global_shared reclaim requires an explicit operator policy".to_string(),
            ],
        ),
        OwnershipClass::RunExclusive => {
            let dirty = has_reason(obs, REASON_DIRTY);
            let unpushed = has_reason(obs, REASON_UNPUSHED);
            if dirty || unpushed {
                let mut reasons = vec!["ownership established (run_exclusive)".to_string()];
                if dirty {
                    reasons.push(REASON_DIRTY.to_string());
                    reasons.push(
                        "spec §13.4 / §36 Invariant 2: DIRTY -> QUARANTINE, never deleted".to_string(),
                    );
                }
                if unpushed {
                    reasons.push(REASON_UNPUSHED.to_string());
                    reasons.push(
                        "spec §36 Invariant 3: unpushed commits must not be silently destroyed".to_string(),
                    );
                }
                (ProposedAction::WouldQuarantine, reasons)
            } else if has_reason(obs, REASON_LEASE_EXPIRED) {
                // Invariant 5: expired lease alone is insufficient proof.
                (
                    ProposedAction::WouldReport,
                    vec![
                        "ownership established (run_exclusive)".to_string(),
                        "lease_expired".to_string(),
                        "spec §36 Invariant 5: an expired lease alone is insufficient proof for destructive cleanup".to_string(),
                    ],
                )
            } else {
                (
                    ProposedAction::WouldRemove,
                    vec![
                        "ownership established (run_exclusive)".to_string(),
                        "no dirty, unpushed, or expired-lease hold on record".to_string(),
                        "dry-run only: nothing is removed until the cleanup executor runs (spec §25)".to_string(),
                    ],
                )
            }
        }
    }
}

/// Deterministic one-token state summary for the report, derived from the
/// observation's reason tokens in fixed priority order.
fn state_summary(obs: &ObservedResource) -> &'static str {
    if has_reason(obs, REASON_DIRTY) {
        "dirty"
    } else if has_reason(obs, REASON_UNPUSHED) {
        "unpushed"
    } else if has_reason(obs, REASON_LEASE_EXPIRED) {
        "lease_expired"
    } else if has_reason(obs, REASON_CLEAN) {
        "clean"
    } else if has_reason(obs, REASON_MERGED) {
        "merged"
    } else if has_reason(obs, REASON_NOT_MERGED) {
        "unmerged"
    } else {
        "unknown"
    }
}

/// Exact-equality reason-token match — free-form text never triggers a
/// quarantine.
fn has_reason(obs: &ObservedResource, token: &str) -> bool {
    obs.reasons.iter().any(|reason| reason == token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::ledger::ResourceLedger;

    fn obs(
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
            reasons: reasons.iter().map(|s| s.to_string()).collect(),
            size_bytes,
        }
    }

    fn entry_for(
        resource_type: ResourceType,
        external_id: &str,
        ownership: OwnershipClass,
        reasons: &[&str],
        size_bytes: Option<u64>,
    ) -> DryRunEntry {
        plan(&[obs(
            resource_type,
            external_id,
            ownership,
            reasons,
            size_bytes,
        )])
        .entries[0]
            .clone()
    }

    #[test]
    fn plan_returns_one_entry_per_observation() {
        let observations = vec![
            obs(
                ResourceType::GitWorktree,
                "/repo/.autospec/worktrees/r1/implement",
                OwnershipClass::RunExclusive,
                &[REASON_CLEAN],
                Some(2048),
            ),
            obs(
                ResourceType::DockerContainer,
                "as-r1-a1b2",
                OwnershipClass::External,
                &["no autospec ownership label"],
                None,
            ),
            obs(
                ResourceType::GitBranch,
                "feat/spec-x",
                OwnershipClass::RunExclusive,
                &[REASON_MERGED],
                None,
            ),
            obs(
                ResourceType::ChildProcess,
                "1234",
                OwnershipClass::RunExclusive,
                &["lease valid", "pid live"],
                None,
            ),
        ];
        let plan = plan(&observations);
        assert_eq!(plan.entries.len(), observations.len());
        // Same order as the input, ids preserved.
        assert_eq!(
            plan.entries
                .iter()
                .map(|e| e.external_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/repo/.autospec/worktrees/r1/implement",
                "as-r1-a1b2",
                "feat/spec-x",
                "1234",
            ]
        );
        // Every entry carries a non-empty safety reason.
        assert!(plan.entries.iter().all(|e| !e.safety_reasons.is_empty()));
    }

    #[test]
    fn plan_on_empty_input_is_empty() {
        let plan = plan(&[]);
        assert!(plan.entries.is_empty());
        assert!(plan.totals.is_empty());
    }

    #[test]
    fn dirty_worktree_yields_would_quarantine() {
        let entry = entry_for(
            ResourceType::GitWorktree,
            "/repo/.autospec/worktrees/r1/implement",
            OwnershipClass::RunExclusive,
            &[
                "autospec worktree path .autospec/worktrees/<run>/<branch>",
                "lease valid (run r1, expires 3600s from 2026-08-16T00:00:00Z)",
                REASON_DIRTY,
            ],
            Some(4096),
        );
        assert_eq!(entry.action, ProposedAction::WouldQuarantine);
        assert_eq!(entry.state, "dirty");
        assert!(entry
            .safety_reasons
            .iter()
            .any(|r| r.contains("Invariant 2")));
    }

    #[test]
    fn unpushed_worktree_yields_would_quarantine() {
        let entry = entry_for(
            ResourceType::GitWorktree,
            "/repo/.autospec/worktrees/r2/implement",
            OwnershipClass::RunExclusive,
            &[REASON_UNPUSHED],
            None,
        );
        assert_eq!(entry.action, ProposedAction::WouldQuarantine);
        assert_eq!(entry.state, "unpushed");
        assert!(entry
            .safety_reasons
            .iter()
            .any(|r| r.contains("Invariant 3")));
    }

    #[test]
    fn clean_run_exclusive_worktree_yields_would_remove() {
        let entry = entry_for(
            ResourceType::GitWorktree,
            "/repo/.autospec/worktrees/r3/implement",
            OwnershipClass::RunExclusive,
            &[REASON_CLEAN],
            Some(4096),
        );
        assert_eq!(entry.action, ProposedAction::WouldRemove);
        assert_eq!(entry.state, "clean");
        assert!(entry
            .safety_reasons
            .iter()
            .all(|r| !r.contains("ownership_unestablished")));
    }

    #[test]
    fn legacy_branch_without_ownership_metadata_yields_would_report() {
        // §48: a legacy branch AutoSpec cannot establish ownership of.
        let entry = entry_for(
            ResourceType::GitBranch,
            "hotfix/legacy-fix",
            OwnershipClass::External,
            &["branch hotfix/legacy-fix (no autospec branch namespace)"],
            None,
        );
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert!(entry
            .safety_reasons
            .iter()
            .any(|r| r == "ownership_unestablished"));
    }

    #[test]
    fn dirty_external_worktree_still_yields_would_report() {
        // Invariant 1 outranks quarantine: without ownership, even a dirty
        // worktree is only reported.
        let entry = entry_for(
            ResourceType::GitWorktree,
            "/repo/.autospec/worktrees/unknown/implement",
            OwnershipClass::External,
            &[
                "path looks autospec-owned but the lease file is missing",
                REASON_DIRTY,
            ],
            None,
        );
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert!(entry
            .safety_reasons
            .iter()
            .any(|r| r == "ownership_unestablished"));
    }

    #[test]
    fn expired_lease_alone_keeps_would_report() {
        // Invariant 5: expired lease alone is insufficient proof of
        // abandonment, even with established ownership.
        let entry = entry_for(
            ResourceType::DockerContainer,
            "as-r4-a1b2",
            OwnershipClass::RunExclusive,
            &["autospec label present", REASON_LEASE_EXPIRED],
            None,
        );
        assert_eq!(entry.action, ProposedAction::WouldReport);
        assert_eq!(entry.state, "lease_expired");
        assert!(entry
            .safety_reasons
            .iter()
            .any(|r| r.contains("Invariant 5")));
    }

    #[test]
    fn shared_ownership_yields_would_report() {
        for ownership in [OwnershipClass::RepoShared, OwnershipClass::GlobalShared] {
            let entry = entry_for(
                ResourceType::DockerVolume,
                "autospec_shared_data",
                ownership,
                &["shared volume used by multiple runs"],
                Some(1024),
            );
            assert_eq!(entry.action, ProposedAction::WouldReport, "{ownership:?}");
            assert!(entry.safety_reasons.iter().any(|r| r == "shared_ownership"));
        }
    }

    #[test]
    fn known_size_yields_matching_nonzero_reclaimable_bytes() {
        let entry = entry_for(
            ResourceType::DockerImage,
            "autospec-r5-a1b2:latest",
            OwnershipClass::RunExclusive,
            &["autospec label present"],
            Some(4096),
        );
        assert_eq!(entry.reclaimable_bytes, Some(4096));
    }

    #[test]
    fn unknown_size_stays_none_never_zero() {
        let entry = entry_for(
            ResourceType::GitWorktree,
            "/repo/.autospec/worktrees/r6/implement",
            OwnershipClass::RunExclusive,
            &[REASON_CLEAN],
            None,
        );
        assert_eq!(entry.reclaimable_bytes, None);
    }

    #[test]
    fn totals_are_keyed_by_resource_type_and_deterministic() {
        let observations = vec![
            obs(
                ResourceType::GitBranch,
                "feat/a",
                OwnershipClass::RunExclusive,
                &[REASON_MERGED],
                None,
            ),
            obs(
                ResourceType::GitWorktree,
                "/wt/1",
                OwnershipClass::RunExclusive,
                &[REASON_CLEAN],
                Some(1000),
            ),
            obs(
                ResourceType::GitWorktree,
                "/wt/2",
                OwnershipClass::RunExclusive,
                &[REASON_DIRTY],
                Some(500),
            ),
            obs(
                ResourceType::GitWorktree,
                "/wt/3",
                OwnershipClass::External,
                &["no autospec worktree path"],
                None,
            ),
        ];
        let plan = plan(&observations);
        assert_eq!(plan.totals.len(), 2, "one row per present type");
        assert_eq!(
            plan.totals.iter().map(|(t, _)| t).collect::<Vec<_>>(),
            // ALL order: GitWorktree before GitBranch.
            vec![&ResourceType::GitWorktree, &ResourceType::GitBranch]
        );
        let worktree = plan.totals[0].1;
        assert_eq!(worktree.count, 3);
        assert_eq!(worktree.would_remove, 1);
        assert_eq!(worktree.would_quarantine, 1);
        assert_eq!(worktree.would_report, 1);
        // One unknown size makes the type total unknown.
        assert_eq!(worktree.reclaimable_bytes, None);
        let branch = plan.totals[1].1;
        assert_eq!(branch.count, 1);
        assert_eq!(branch.would_remove, 1);
        assert_eq!(branch.reclaimable_bytes, None); // size unknown stays unknown
    }

    #[test]
    fn totals_sum_known_sizes() {
        let observations = vec![
            obs(
                ResourceType::DockerImage,
                "img-a",
                OwnershipClass::RunExclusive,
                &["autospec label present"],
                Some(1000),
            ),
            obs(
                ResourceType::DockerImage,
                "img-b",
                OwnershipClass::RunExclusive,
                &["autospec label present"],
                Some(500),
            ),
        ];
        let plan = plan(&observations);
        assert_eq!(plan.totals.len(), 1);
        assert_eq!(plan.totals[0].1.reclaimable_bytes, Some(1500));
    }

    #[test]
    fn state_summary_covers_all_branches_and_action_as_str() {
        let merged = entry_for(
            ResourceType::GitBranch,
            "feat/a",
            OwnershipClass::RunExclusive,
            &[REASON_MERGED],
            None,
        );
        assert_eq!(merged.state, "merged");

        let unmerged = entry_for(
            ResourceType::GitBranch,
            "feat/b",
            OwnershipClass::RunExclusive,
            &[REASON_NOT_MERGED],
            None,
        );
        assert_eq!(unmerged.state, "unmerged");
        // No hold reasons: a clean run-exclusive branch is removable.
        assert_eq!(unmerged.action, ProposedAction::WouldRemove);

        let unknown = entry_for(
            ResourceType::DockerContainer,
            "mystery",
            OwnershipClass::External,
            &["no autospec ownership label"],
            None,
        );
        assert_eq!(unknown.state, "unknown");

        assert_eq!(ProposedAction::WouldRemove.as_str(), "would_remove");
        assert_eq!(ProposedAction::WouldQuarantine.as_str(), "would_quarantine");
        assert_eq!(ProposedAction::WouldReport.as_str(), "would_report");
    }

    /// Deterministic 6,500-observation generator. Index-derived fields (no
    /// randomness, no dependencies) so the property run is reproducible:
    /// every (type, ownership) pair recurs, and the dirty/unpushed/
    /// lease-expired reason tokens and known/unknown sizes cycle through
    /// the full space.
    fn generated_observations(count: usize) -> Vec<ObservedResource> {
        let types = ResourceType::ALL;
        let owners = OwnershipClass::ALL;
        (0..count)
            .map(|i| {
                let resource_type = types[i % types.len()];
                let ownership = owners[(i / types.len()) % owners.len()];
                let mut reasons: Vec<&str> = vec![];
                if resource_type == ResourceType::GitWorktree {
                    if i % 2 == 0 {
                        reasons.push(REASON_DIRTY);
                    } else {
                        reasons.push(REASON_CLEAN);
                    }
                    if i % 7 == 0 {
                        reasons.push(REASON_UNPUSHED);
                    }
                }
                if i % 5 == 0 {
                    reasons.push(REASON_LEASE_EXPIRED);
                }
                if reasons.is_empty() {
                    reasons.push("synthetic observation");
                }
                let size_bytes = if i % 4 == 0 {
                    Some(1000 + (i as u64) % 100_000)
                } else {
                    None
                };
                obs(
                    resource_type,
                    &format!("obs-{i}"),
                    ownership,
                    &reasons,
                    size_bytes,
                )
            })
            .collect()
    }

    /// Property test (spec §25/§36): over 6,500 generated observations the
    /// planner never removes an `External` resource, always keeps
    /// `safety_reasons` non-empty, mirrors `size_bytes` exactly, and only
    /// emits `WouldRemove` for clean `RunExclusive` entries.
    #[test]
    fn property_test_over_6500_observations() {
        const COUNT: usize = 6500;
        let observations = generated_observations(COUNT);

        // Generator sanity: the space actually exercised the property.
        assert!(observations
            .iter()
            .any(|o| o.ownership == OwnershipClass::External));
        assert!(observations.iter().any(|o| {
            o.ownership == OwnershipClass::RunExclusive
                && o.reasons.iter().any(|r| r == REASON_DIRTY)
        }));
        assert!(observations
            .iter()
            .any(|o| o.reasons.iter().any(|r| r == REASON_LEASE_EXPIRED)));
        assert!(observations.iter().any(|o| o.size_bytes.is_some()));
        assert!(observations.iter().any(|o| o.size_bytes.is_none()));

        let plan = plan(&observations);
        assert_eq!(plan.entries.len(), COUNT, "one entry per observation");

        for (input, entry) in observations.iter().zip(plan.entries.iter()) {
            // Non-empty safety reasons, always.
            assert!(
                !entry.safety_reasons.is_empty(),
                "entry {}",
                entry.external_id
            );
            // Mirrors size_bytes: None stays None, Some matches.
            assert_eq!(
                entry.reclaimable_bytes, input.size_bytes,
                "entry {}",
                entry.external_id
            );
            // THE property: External is never WOULD REMOVE.
            if input.ownership == OwnershipClass::External {
                assert_ne!(
                    entry.action,
                    ProposedAction::WouldRemove,
                    "External {} was proposed for removal",
                    entry.external_id
                );
                assert!(entry
                    .safety_reasons
                    .iter()
                    .any(|r| r == "ownership_unestablished"));
            }
            // WouldRemove is reserved for clean RunExclusive entries.
            if entry.action == ProposedAction::WouldRemove {
                assert_eq!(input.ownership, OwnershipClass::RunExclusive);
                assert!(!has_reason(input, REASON_DIRTY));
                assert!(!has_reason(input, REASON_UNPUSHED));
                assert!(!has_reason(input, REASON_LEASE_EXPIRED));
            }
            // Dirty/unpushed RunExclusive Git entries are quarantined.
            if input.ownership == OwnershipClass::RunExclusive
                && (has_reason(input, REASON_DIRTY) || has_reason(input, REASON_UNPUSHED))
            {
                assert_eq!(
                    entry.action,
                    ProposedAction::WouldQuarantine,
                    "entry {}",
                    entry.external_id
                );
            }
        }

        // Totals reconcile with the entries.
        let total_entries: usize = plan.totals.iter().map(|(_, t)| t.count).sum();
        assert_eq!(total_entries, COUNT);
        let by_action: (usize, usize, usize) =
            plan.entries
                .iter()
                .fold((0, 0, 0), |(r, q, p), e| match e.action {
                    ProposedAction::WouldRemove => (r + 1, q, p),
                    ProposedAction::WouldQuarantine => (r, q + 1, p),
                    ProposedAction::WouldReport => (r, q, p + 1),
                });
        let from_totals: (usize, usize, usize) =
            plan.totals.iter().fold((0, 0, 0), |(r, q, p), (_, t)| {
                (
                    r + t.would_remove,
                    q + t.would_quarantine,
                    p + t.would_report,
                )
            });
        assert_eq!(by_action, from_totals);
        // There is at least one removal, so the plan is not trivially empty.
        assert!(by_action.0 > 0);
    }

    #[test]
    fn module_source_contains_no_process_command() {
        // The planner must not be able to signal/kill anything: zero
        // occurrences of the process-Command type in this file. Tokens are
        // concatenated (and the comment above is split) so this test does not
        // count as its own occurrence.
        let source = include_str!("dry_run.rs");
        let needle = ["std::process::", "Command"].concat();
        assert_eq!(
            source.matches(&needle).count(),
            0,
            "dry_run.rs must not use the process Command type"
        );
    }

    /// Recursively snapshot (relative path, size, mtime) of a tree.
    fn snapshot_tree(root: &std::path::Path) -> Vec<(String, u64, std::time::SystemTime)> {
        let mut out: Vec<(String, u64, std::time::SystemTime)> = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                let meta = std::fs::metadata(&path).unwrap();
                if meta.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(root).unwrap().display().to_string();
                    let mtime = meta.modified().unwrap();
                    out.push((rel, meta.len(), mtime));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn plan_performs_no_filesystem_or_ledger_writes() {
        use std::io::Write as _;
        use std::time::SystemTime;

        // Scratch tree standing in for the observed resources (worktrees,
        // docker dirs) and the ledger's database file.
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let scratch = std::env::temp_dir().join(format!("autospec-dryrun-{nonce}"));
        let wt = scratch.join("worktree");
        std::fs::create_dir_all(&wt).unwrap();
        let mut f = std::fs::File::create(wt.join("tracked.txt")).unwrap();
        f.write_all(b"payload").unwrap();
        drop(f);
        let ledger_dir = scratch.join("state");
        std::fs::create_dir_all(&ledger_dir).unwrap();

        // Open a real ledger (bootstrap writes happen here, before the
        // snapshot, so only plan()'s behaviour is measured).
        let ledger =
            ResourceLedger::open(&format!("sqlite://{}/autospec.db", ledger_dir.display()))
                .unwrap();

        let observations = vec![
            obs(
                ResourceType::GitWorktree,
                wt.to_string_lossy().as_ref(),
                OwnershipClass::RunExclusive,
                &[REASON_DIRTY],
                Some(4096),
            ),
            obs(
                ResourceType::DockerVolume,
                scratch.to_string_lossy().as_ref(),
                OwnershipClass::External,
                &["no autospec ownership label"],
                None,
            ),
        ];

        let before_tree = snapshot_tree(&scratch);
        let ledger_path = ledger_dir.join("autospec.db");
        let before_ledger = std::fs::metadata(&ledger_path).unwrap();
        let before_ledger_mtime = before_ledger.modified().unwrap();

        let plan = plan(&observations);
        assert_eq!(plan.entries.len(), 2);
        // The plan still proposes quarantine/report — it just did not do it.
        assert_eq!(plan.entries[0].action, ProposedAction::WouldQuarantine);
        assert_eq!(plan.entries[1].action, ProposedAction::WouldReport);

        let after_tree = snapshot_tree(&scratch);
        assert_eq!(
            before_tree, after_tree,
            "plan() must not create, modify, or delete files"
        );

        let after_ledger = std::fs::metadata(&ledger_path).unwrap();
        assert_eq!(
            before_ledger.len(),
            after_ledger.len(),
            "plan() must not write to the ledger database"
        );
        assert_eq!(
            before_ledger_mtime,
            after_ledger.modified().unwrap(),
            "plan() must not touch the ledger database file"
        );

        // Ledger is still queryable and empty (plan inserted nothing).
        assert_eq!(ledger.list_all().unwrap().len(), 0);

        let _ = std::fs::remove_dir_all(&scratch);
    }
}
