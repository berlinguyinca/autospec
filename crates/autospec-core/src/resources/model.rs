//! Resource model: the shared vocabulary the resource-lifecycle subsystem
//! (spec `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`) uses to
//! describe anything AutoSpec creates, shares, or merely references —
//! worktrees, branches, Docker objects, processes, temp files, ports, and
//! build caches.
//!
//! This module is pure in-memory data: no I/O, no persistence, no driver
//! type, and no new dependency (`Cargo.toml` stays at its 5 pinned deps).
//! It owns the *shape* of an observation, not how one is produced — see
//! `ObservedResource` below, which is the ONE type both future observers
//! (git, Docker) return and both future consumers (ledger sync, dry-run
//! planner) read. Do not add per-domain variants (e.g. `ObservedGitResource`)
//! on top of it.
//!
//! `ResourceId` / `RunId` (spec §10) and `CleanupPolicy` (spec §12/§38) are
//! not defined by this issue — no persistence or handler issue has landed
//! yet to own them. Until they do, `ManagedResource` carries their slots as
//! `String` / `serde_json::Value` so the shape matches the spec's ledger
//! schema (`resources.id TEXT`, `resources.cleanup_policy_json TEXT`)
//! without inventing a type this issue does not own.

use crate::AutospecError;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// The kinds of resource AutoSpec can create, share, or reference.
///
/// Spec §9 — exactly these 12 variants for the initial implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    GitWorktree,
    GitBranch,
    DockerContainer,
    DockerImage,
    DockerVolume,
    DockerNetwork,
    DockerComposeProject,
    ChildProcess,
    TempDirectory,
    TempFile,
    PortReservation,
    BuildCache,
}

impl ResourceType {
    /// Every variant, in declaration order. Tests drive round-trip coverage
    /// off this slice instead of a hand-written list, so the exact-count
    /// acceptance criteria (spec §9: 12 variants) cannot silently drift from
    /// the enum definition.
    pub const ALL: &'static [ResourceType] = &[
        ResourceType::GitWorktree,
        ResourceType::GitBranch,
        ResourceType::DockerContainer,
        ResourceType::DockerImage,
        ResourceType::DockerVolume,
        ResourceType::DockerNetwork,
        ResourceType::DockerComposeProject,
        ResourceType::ChildProcess,
        ResourceType::TempDirectory,
        ResourceType::TempFile,
        ResourceType::PortReservation,
        ResourceType::BuildCache,
    ];

    /// Canonical lowercase-snake-case string for ledger TEXT columns
    /// (spec §12 `resources.resource_type TEXT NOT NULL`). Matches the
    /// derived serde representation (`#[serde(rename_all = "snake_case")]`)
    /// so the two encodings never drift apart.
    ///
    /// An exhaustive match with no wildcard arm: adding a variant without
    /// adding its string here fails the build.
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceType::GitWorktree => "git_worktree",
            ResourceType::GitBranch => "git_branch",
            ResourceType::DockerContainer => "docker_container",
            ResourceType::DockerImage => "docker_image",
            ResourceType::DockerVolume => "docker_volume",
            ResourceType::DockerNetwork => "docker_network",
            ResourceType::DockerComposeProject => "docker_compose_project",
            ResourceType::ChildProcess => "child_process",
            ResourceType::TempDirectory => "temp_directory",
            ResourceType::TempFile => "temp_file",
            ResourceType::PortReservation => "port_reservation",
            ResourceType::BuildCache => "build_cache",
        }
    }
}

impl FromStr for ResourceType {
    type Err = AutospecError;

    /// Parses the canonical string form. An unrecognized value is a hard
    /// `Err` — it is NEVER coerced to any default variant.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "git_worktree" => Ok(ResourceType::GitWorktree),
            "git_branch" => Ok(ResourceType::GitBranch),
            "docker_container" => Ok(ResourceType::DockerContainer),
            "docker_image" => Ok(ResourceType::DockerImage),
            "docker_volume" => Ok(ResourceType::DockerVolume),
            "docker_network" => Ok(ResourceType::DockerNetwork),
            "docker_compose_project" => Ok(ResourceType::DockerComposeProject),
            "child_process" => Ok(ResourceType::ChildProcess),
            "temp_directory" => Ok(ResourceType::TempDirectory),
            "temp_file" => Ok(ResourceType::TempFile),
            "port_reservation" => Ok(ResourceType::PortReservation),
            "build_cache" => Ok(ResourceType::BuildCache),
            other => Err(AutospecError::parse(
                "resource_type",
                format!("unknown resource type: {other}"),
            )),
        }
    }
}

/// The lifecycle state of a `ManagedResource`.
///
/// Spec §10 — exactly these 10 variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    Creating,
    Active,
    Retained,
    CleanupPending,
    Cleaning,
    Released,
    Missing,
    Quarantined,
    CleanupFailed,
    Orphaned,
}

impl ResourceState {
    /// Every variant, in declaration order — see `ResourceType::ALL` for why.
    pub const ALL: &'static [ResourceState] = &[
        ResourceState::Creating,
        ResourceState::Active,
        ResourceState::Retained,
        ResourceState::CleanupPending,
        ResourceState::Cleaning,
        ResourceState::Released,
        ResourceState::Missing,
        ResourceState::Quarantined,
        ResourceState::CleanupFailed,
        ResourceState::Orphaned,
    ];

    /// Canonical lowercase-snake-case string for ledger TEXT columns
    /// (spec §12 `resources.state TEXT NOT NULL`).
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceState::Creating => "creating",
            ResourceState::Active => "active",
            ResourceState::Retained => "retained",
            ResourceState::CleanupPending => "cleanup_pending",
            ResourceState::Cleaning => "cleaning",
            ResourceState::Released => "released",
            ResourceState::Missing => "missing",
            ResourceState::Quarantined => "quarantined",
            ResourceState::CleanupFailed => "cleanup_failed",
            ResourceState::Orphaned => "orphaned",
        }
    }
}

impl FromStr for ResourceState {
    type Err = AutospecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "creating" => Ok(ResourceState::Creating),
            "active" => Ok(ResourceState::Active),
            "retained" => Ok(ResourceState::Retained),
            "cleanup_pending" => Ok(ResourceState::CleanupPending),
            "cleaning" => Ok(ResourceState::Cleaning),
            "released" => Ok(ResourceState::Released),
            "missing" => Ok(ResourceState::Missing),
            "quarantined" => Ok(ResourceState::Quarantined),
            "cleanup_failed" => Ok(ResourceState::CleanupFailed),
            "orphaned" => Ok(ResourceState::Orphaned),
            other => Err(AutospecError::parse(
                "resource_state",
                format!("unknown resource state: {other}"),
            )),
        }
    }
}

/// Who a resource belongs to, and therefore how aggressively it may ever be
/// reclaimed. Spec §11.
///
/// Deliberately does NOT derive `Default` — a default would pick one variant
/// (almost certainly the first-listed, `RunExclusive`) as the fallback for
/// any caller that forgets to set ownership explicitly, which is exactly the
/// silent-coercion failure mode `from_str` is required to refuse (AC #5).
/// There is no safe default ownership class; every caller must state one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipClass {
    /// Owned by exactly one AutoSpec run (a worktree, a task branch, a
    /// temporary container, a test volume).
    RunExclusive,
    /// Shared by multiple AutoSpec runs in one repository (a repo-level
    /// cache, a shared base image).
    RepoShared,
    /// Shared across multiple repositories (a common runtime image, a build
    /// cache).
    GlobalShared,
    /// Referenced but not created by AutoSpec. MUST NOT be deleted.
    External,
}

impl OwnershipClass {
    /// Every variant, in declaration order — see `ResourceType::ALL` for why.
    pub const ALL: &'static [OwnershipClass] = &[
        OwnershipClass::RunExclusive,
        OwnershipClass::RepoShared,
        OwnershipClass::GlobalShared,
        OwnershipClass::External,
    ];

    /// Canonical lowercase-snake-case string for ledger TEXT columns
    /// (spec §12 `resources.ownership TEXT NOT NULL`).
    pub fn as_str(&self) -> &'static str {
        match self {
            OwnershipClass::RunExclusive => "run_exclusive",
            OwnershipClass::RepoShared => "repo_shared",
            OwnershipClass::GlobalShared => "global_shared",
            OwnershipClass::External => "external",
        }
    }

    /// Whether this ownership class is EVER eligible for reclamation under
    /// some policy — NOT "safe to delete right now with no further checks."
    /// Per spec §11: `RunExclusive` "may generally be reclaimed after the
    /// run finishes," `RepoShared` "may only be reclaimed after reference
    /// count / lease checks," and `GlobalShared` "requires explicit
    /// policy" — all three still gate an actual reclaim behind checks this
    /// crate does not implement yet (lease state, reference counting, a
    /// policy engine). Only `External` ("MUST NOT be deleted") is excluded
    /// categorically, at this layer, with no further check possible.
    /// Callers must not treat `true` here as authorization to delete.
    pub fn is_reclaimable(&self) -> bool {
        !matches!(self, OwnershipClass::External)
    }
}

impl FromStr for OwnershipClass {
    type Err = AutospecError;

    /// Parses the canonical string form. An unrecognized value is a hard
    /// `Err` — it is NEVER coerced to `RunExclusive` or any other variant.
    /// A counterfeit ownership string (e.g. from a corrupted ledger row or
    /// an attacker-controlled label) must fail loudly, not silently become
    /// the most-permissive class.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "run_exclusive" => Ok(OwnershipClass::RunExclusive),
            "repo_shared" => Ok(OwnershipClass::RepoShared),
            "global_shared" => Ok(OwnershipClass::GlobalShared),
            "external" => Ok(OwnershipClass::External),
            other => Err(AutospecError::parse(
                "ownership_class",
                format!("unknown ownership class: {other}"),
            )),
        }
    }
}

/// A resource AutoSpec is tracking in the ledger. Spec §10.
///
/// `id` / `run_id` stay `String` rather than dedicated `ResourceId` /
/// `RunId` newtypes, and `cleanup_policy` stays `serde_json::Value` rather
/// than a `CleanupPolicy` struct: none of those types has a home yet (no
/// ledger or handler issue has landed), and inventing one here would be
/// scope creep this issue does not own. `serde_json` is already a pinned
/// dependency, so this costs zero new dependencies (AC #6).
///
/// Timestamps are RFC3339 `String` rather than `chrono::DateTime<Utc>` —
/// `chrono` is not a dependency of this crate and this issue adds none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagedResource {
    pub id: String,
    pub run_id: String,
    pub work_item_id: Option<String>,
    pub repository_id: Option<String>,
    pub worker_id: Option<String>,

    pub resource_type: ResourceType,
    pub external_id: String,

    pub state: ResourceState,
    pub ownership: OwnershipClass,
    /// Placeholder for the not-yet-defined `CleanupPolicy` (spec §12
    /// `cleanup_policy_json TEXT NOT NULL`, §38 `ResourceHandler::cleanup`).
    pub cleanup_policy: serde_json::Value,

    /// RFC3339 timestamp.
    pub created_at: String,
    /// RFC3339 timestamp.
    pub updated_at: String,
    /// RFC3339 timestamp, when present.
    pub lease_expires_at: Option<String>,
    /// RFC3339 timestamp, when present.
    pub last_heartbeat_at: Option<String>,

    pub cleanup_attempts: u32,
    pub last_cleanup_error: Option<String>,

    pub metadata: serde_json::Value,
}

/// The ONE shape every resource observer (git, Docker, process, temp, port)
/// returns, and every consumer (ledger sync, dry-run planner, `autospec
/// resources`/`doctor resources`) reads. Deliberately has no per-domain
/// sibling type (e.g. no `ObservedGitResource`) — a git-worktree observation
/// and a Docker-container observation both become one `ObservedResource`.
///
/// This is a pure observation: it carries no lease, no cleanup policy, and
/// no ledger row id, because observing a resource is not the same act as
/// managing one (`ManagedResource`, above) — this type is produced by
/// *inspecting* the outside world, not by AutoSpec having created something.
///
/// `ownership` is supplied by the observer's judgment (e.g. "this branch is
/// referenced by an open PR, so it's not ours to delete"), and `reasons`
/// MUST carry the evidence behind that judgment — never a bare name-prefix
/// guess. A prefix match on a branch/container name does not, by itself,
/// prove ownership; the counter-team's challenge to this issue is exactly
/// that claim, and `is_reclaimable()` above make no exception for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedResource {
    pub resource_type: ResourceType,
    pub external_id: String,
    pub ownership: OwnershipClass,
    pub reasons: Vec<String>,
    /// `None` means unknown/unmeasured. Never `0` as a stand-in for
    /// "not measured" — `0` means a measured, genuinely empty resource.
    pub size_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ResourceType ─────────────────────────────────────────────────────

    #[test]
    fn resource_type_has_exactly_twelve_variants() {
        assert_eq!(ResourceType::ALL.len(), 12);
    }

    #[test]
    fn resource_type_round_trips_every_variant_through_as_str_and_from_str() {
        for variant in ResourceType::ALL {
            let encoded = variant.as_str();
            let decoded = ResourceType::from_str(encoded)
                .unwrap_or_else(|err| panic!("from_str({encoded:?}) failed: {err}"));
            assert_eq!(decoded, *variant);
        }
    }

    #[test]
    fn resource_type_serde_string_matches_as_str() {
        for variant in ResourceType::ALL {
            let json = serde_json::to_string(variant).expect("serialize ResourceType");
            assert_eq!(json, format!("\"{}\"", variant.as_str()));
        }
    }

    #[test]
    fn resource_type_from_str_rejects_unknown_variant() {
        let result = ResourceType::from_str("not_a_real_resource_type");
        assert!(result.is_err());
    }

    // ── ResourceState ────────────────────────────────────────────────────

    #[test]
    fn resource_state_has_exactly_ten_variants() {
        assert_eq!(ResourceState::ALL.len(), 10);
    }

    #[test]
    fn resource_state_round_trips_every_variant_through_as_str_and_from_str() {
        for variant in ResourceState::ALL {
            let encoded = variant.as_str();
            let decoded = ResourceState::from_str(encoded)
                .unwrap_or_else(|err| panic!("from_str({encoded:?}) failed: {err}"));
            assert_eq!(decoded, *variant);
        }
    }

    #[test]
    fn resource_state_serde_string_matches_as_str() {
        for variant in ResourceState::ALL {
            let json = serde_json::to_string(variant).expect("serialize ResourceState");
            assert_eq!(json, format!("\"{}\"", variant.as_str()));
        }
    }

    #[test]
    fn resource_state_from_str_rejects_unknown_variant() {
        let result = ResourceState::from_str("not_a_real_resource_state");
        assert!(result.is_err());
    }

    // ── OwnershipClass ───────────────────────────────────────────────────

    #[test]
    fn ownership_class_has_exactly_four_variants() {
        assert_eq!(OwnershipClass::ALL.len(), 4);
    }

    #[test]
    fn ownership_class_round_trips_every_variant_through_as_str_and_from_str() {
        for variant in OwnershipClass::ALL {
            let encoded = variant.as_str();
            let decoded = OwnershipClass::from_str(encoded)
                .unwrap_or_else(|err| panic!("from_str({encoded:?}) failed: {err}"));
            assert_eq!(decoded, *variant);
        }
    }

    #[test]
    fn external_is_never_reclaimable() {
        assert!(!OwnershipClass::External.is_reclaimable());
    }

    #[test]
    fn non_external_classes_are_reclaimable() {
        assert!(OwnershipClass::RunExclusive.is_reclaimable());
        assert!(OwnershipClass::RepoShared.is_reclaimable());
        assert!(OwnershipClass::GlobalShared.is_reclaimable());
    }

    #[test]
    fn ownership_class_from_str_rejects_unknown_variant_and_never_coerces_to_run_exclusive() {
        // The counter-team's challenge: does an unrecognized/forged ownership
        // string quietly become the most-permissive class? It must not.
        let result = OwnershipClass::from_str("totally-made-up-ownership");
        assert!(result.is_err());
        // A coercing implementation would return Ok(RunExclusive) here — assert
        // the actual failure mode directly so a regression to that behavior is
        // caught by an assertion, not just an unwrap panic changing shape.
        match result {
            Err(_) => {}
            Ok(class) => panic!("expected Err, got coerced variant {class:?}"),
        }
    }

    #[test]
    fn ownership_class_does_not_infer_ownership_from_name_prefix() {
        // Regression guard for the counter-team's specific challenge: nothing
        // in this module derives an OwnershipClass from a string prefix. This
        // is enforced structurally (from_str only accepts the 4 canonical
        // strings above) rather than by inspecting arbitrary resource names.
        for candidate in ["autospec-", "run-", "tmp-", "worktree-"] {
            assert!(OwnershipClass::from_str(candidate).is_err());
        }
    }

    // ── ManagedResource ──────────────────────────────────────────────────

    #[test]
    fn managed_resource_serializes_and_round_trips() {
        let resource = ManagedResource {
            id: "res-1".to_string(),
            run_id: "run-1".to_string(),
            work_item_id: Some("3186".to_string()),
            repository_id: Some("berlinguyinca/autospec".to_string()),
            worker_id: None,
            resource_type: ResourceType::GitWorktree,
            external_id: "/tmp/wt-feat-3186-resource-model".to_string(),
            state: ResourceState::Active,
            ownership: OwnershipClass::RunExclusive,
            cleanup_policy: serde_json::json!({"ttl_seconds": 10800}),
            created_at: "2026-09-02T00:00:00Z".to_string(),
            updated_at: "2026-09-02T00:00:00Z".to_string(),
            lease_expires_at: Some("2026-09-02T03:00:00Z".to_string()),
            last_heartbeat_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: serde_json::json!({}),
        };

        let json = serde_json::to_string(&resource).expect("serialize ManagedResource");
        let decoded: ManagedResource =
            serde_json::from_str(&json).expect("deserialize ManagedResource");
        assert_eq!(decoded, resource);
    }

    // ── ObservedResource ─────────────────────────────────────────────────

    #[test]
    fn observed_resource_serde_round_trips_with_size_bytes_set() {
        let observed = ObservedResource {
            resource_type: ResourceType::DockerContainer,
            external_id: "container-abc123".to_string(),
            ownership: OwnershipClass::External,
            reasons: vec!["no autospec label present".to_string()],
            size_bytes: Some(4096),
        };

        let json = serde_json::to_string(&observed).expect("serialize ObservedResource");
        assert!(json.contains("4096"));
        let decoded: ObservedResource =
            serde_json::from_str(&json).expect("deserialize ObservedResource");
        assert_eq!(decoded, observed);
        assert_eq!(decoded.size_bytes, Some(4096));
    }

    #[test]
    fn observed_resource_serde_round_trips_with_size_bytes_none() {
        let observed = ObservedResource {
            resource_type: ResourceType::GitBranch,
            external_id: "refs/heads/feat/example".to_string(),
            ownership: OwnershipClass::RunExclusive,
            reasons: vec!["branch created by this run, size not measured".to_string()],
            size_bytes: None,
        };

        let json = serde_json::to_string(&observed).expect("serialize ObservedResource");
        // None must serialize as JSON null, never as 0 — 0 means "measured
        // and genuinely empty," which is a different fact than "unknown."
        assert!(json.contains("\"size_bytes\":null"));
        let decoded: ObservedResource =
            serde_json::from_str(&json).expect("deserialize ObservedResource");
        assert_eq!(decoded, observed);
        assert_eq!(decoded.size_bytes, None);
    }
}
