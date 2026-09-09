//! Issue ownership, concurrency metadata, and dependency-reason types.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §11 (ownership partitioning), §12 (candidate dependency graph reason
//! codes), §14 (concurrency metadata contract), §15 (machine-readable issue
//! metadata), §28 (proposed Rust data structures), §29.5 (ownership overlap).
//!
//! This module owns the *shape* of the metadata, not how issue bodies parse
//! it: graph traversal, lint rules, CLI, and skeleton rendering are separate
//! concerns (out of scope per issue #3816). The companion schema
//! `schemas/autospec-issue-concurrency.schema.json` mirrors these types
//! field for field.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Write/read ownership of repo paths and declared symbols (§11, §28).
///
/// `exclusive` is the issue's primary write domain. `shared_read` lists
/// contracts read but not written. `shared_write` declares shared write
/// surfaces (§11.1); shared writes MUST be declared and contribute to
/// conflict-risk scoring, but MUST NOT automatically become dependency
/// edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ownership {
    pub exclusive: Vec<OwnedSurface>,
    pub shared_read: Vec<OwnedSurface>,
    pub shared_write: Vec<OwnedSurface>,
}

/// One owned path with the symbols the issue declares on it (§28).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedSurface {
    pub path: String,
    pub symbols: Vec<String>,
}

/// Per-issue concurrency metadata (§14, §28).
///
/// `parallel_safe` records whether the issue can run alongside siblings.
/// `conflict_domains` names probable merge-conflict areas; conflict risk is
/// NOT a dependency (§18) and must stay represented separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrencyMetadata {
    pub parallel_safe: bool,
    pub conflict_domains: Vec<String>,
}

/// Machine-readable reasons a hard dependency exists (§12, §28).
///
/// Exactly these nine codes are supported; unsupported reason codes MUST
/// fail lint, which is why [`DependencyReason::from_str`] returns `Err`
/// instead of guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyReason {
    ConsumesNewInterface,
    ConsumesNewType,
    ConsumesNewSchema,
    ConsumesMigration,
    ConsumesGeneratedArtifact,
    RequiresStructuralMigration,
    RequiresNewProtocol,
    VerificationRequiresPredecessor,
    ExternalPrerequisite,
}

impl DependencyReason {
    /// Every variant, in declaration order. Tests drive round-trip coverage
    /// off this slice instead of a hand-written list, so the exact count
    /// (§12: nine reason codes) cannot silently drift from the enum.
    pub const ALL: &'static [DependencyReason] = &[
        DependencyReason::ConsumesNewInterface,
        DependencyReason::ConsumesNewType,
        DependencyReason::ConsumesNewSchema,
        DependencyReason::ConsumesMigration,
        DependencyReason::ConsumesGeneratedArtifact,
        DependencyReason::RequiresStructuralMigration,
        DependencyReason::RequiresNewProtocol,
        DependencyReason::VerificationRequiresPredecessor,
        DependencyReason::ExternalPrerequisite,
    ];

    /// Canonical kebab-case reason code from §12. Matches the derived serde
    /// representation (`#[serde(rename_all = "kebab-case")]`) so the two
    /// encodings never drift apart.
    ///
    /// An exhaustive match with no wildcard arm: adding a variant without
    /// adding its string here fails the build.
    pub fn as_str(&self) -> &'static str {
        match self {
            DependencyReason::ConsumesNewInterface => "consumes-new-interface",
            DependencyReason::ConsumesNewType => "consumes-new-type",
            DependencyReason::ConsumesNewSchema => "consumes-new-schema",
            DependencyReason::ConsumesMigration => "consumes-migration",
            DependencyReason::ConsumesGeneratedArtifact => "consumes-generated-artifact",
            DependencyReason::RequiresStructuralMigration => "requires-structural-migration",
            DependencyReason::RequiresNewProtocol => "requires-new-protocol",
            DependencyReason::VerificationRequiresPredecessor => {
                "verification-requires-predecessor"
            }
            DependencyReason::ExternalPrerequisite => "external-prerequisite",
        }
    }
}

impl FromStr for DependencyReason {
    type Err = String;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        match code {
            "consumes-new-interface" => Ok(DependencyReason::ConsumesNewInterface),
            "consumes-new-type" => Ok(DependencyReason::ConsumesNewType),
            "consumes-new-schema" => Ok(DependencyReason::ConsumesNewSchema),
            "consumes-migration" => Ok(DependencyReason::ConsumesMigration),
            "consumes-generated-artifact" => Ok(DependencyReason::ConsumesGeneratedArtifact),
            "requires-structural-migration" => Ok(DependencyReason::RequiresStructuralMigration),
            "requires-new-protocol" => Ok(DependencyReason::RequiresNewProtocol),
            "verification-requires-predecessor" => {
                Ok(DependencyReason::VerificationRequiresPredecessor)
            }
            "external-prerequisite" => Ok(DependencyReason::ExternalPrerequisite),
            other => Err(format!(
                "unsupported dependency reason code: {other} (see spec §12)"
            )),
        }
    }
}

impl fmt::Display for DependencyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Normalize a declared ownership path for glob and child-file comparison
/// (§29.5).
///
/// Collapses `.` segments and duplicate slashes, strips a leading `/`, and
/// removes `..` segments fail-closed: a `..` that would escape the repo root
/// is dropped rather than preserved, so a declared ownership glob can never
/// name paths outside the repository (path-traversal guard for the security
/// counter-team).
pub fn normalize_path(raw: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in raw.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                // Fail closed: cannot go above the repository root.
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

/// True if any owned surface of `a` overlaps any owned surface of `b`
/// (§29.5): exact same file, parent-directory glob vs child file, or the
/// same declared symbol.
///
/// v1 does NOT infer symbol overlap from text heuristics — only symbols
/// explicitly declared on both sides count.
pub fn overlaps(a: &Ownership, b: &Ownership) -> bool {
    for surface_a in all_surfaces(a) {
        for surface_b in all_surfaces(b) {
            if surfaces_overlap(surface_a, surface_b) {
                return true;
            }
        }
    }
    false
}

fn all_surfaces(ownership: &Ownership) -> Vec<&OwnedSurface> {
    ownership
        .exclusive
        .iter()
        .chain(ownership.shared_read.iter())
        .chain(ownership.shared_write.iter())
        .collect()
}

fn surfaces_overlap(a: &OwnedSurface, b: &OwnedSurface) -> bool {
    let (path_a, path_b) = (normalize_path(&a.path), normalize_path(&b.path));
    if paths_overlap(&path_a, &path_b) {
        return true;
    }
    a.symbols
        .iter()
        .any(|symbol_a| b.symbols.iter().any(|symbol_b| symbol_a == symbol_b))
}

fn paths_overlap(a: &str, b: &str) -> bool {
    a == b || glob_covers(a, b) || glob_covers(b, a)
}

/// True if `glob` (normalized) names the parent directory of `path`
/// (normalized) via a trailing `/**` — e.g. `a/b/**` covers `a/b/c.rs`.
/// The glob names files *inside* the base directory: it does not cover the
/// base directory itself or sibling paths (e.g. `a/bc.rs`).
fn glob_covers(glob: &str, path: &str) -> bool {
    let Some(base) = glob.strip_suffix("/**") else {
        return false;
    };
    !base.is_empty() && path.starts_with(&format!("{base}/"))
}
