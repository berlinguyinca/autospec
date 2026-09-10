//! Semantic integration conflicts (issue #3667).
//!
//! Two patches, each correct against its own base, that do not compose.
//! One adds fields to `DeploymentManifest` and `ResourceEnvelope`; the other
//! adds a file that constructs both. Each agent worked against a `main` that
//! did not contain the other's change, and each produced a patch that built
//! and passed its tests. The first merged; the second then fails to compile
//! at the first's construction sites (`E0063`: missing fields), even though
//! git merges it *cleanly* — no two edits touch the same lines.
//!
//! This is the third and hardest form of integration conflict: **the merge
//! succeeds and the code is wrong.** A three-way merge has nothing to
//! complain about, because the incompatibility is between a *type* and its
//! *constructors*, which live in different files that only one of the two
//! patches touched.
//!
//! Four rules this module encodes:
//!
//! 1. **Verify a patch against the main it will land on, not the one it was
//!    written against.** A verdict is evidence about the revision it was
//!    recorded on. Every merge that lands in between invalidates it, so a
//!    patch that has not landed is re-verified after *each* such merge — not
//!    once, at open time.
//! 2. **Serialise on types, not files.** Two ready issues that touch the
//!    same public type are serialised — or both specs declare the type's
//!    interface contract, so the second agent knows the shape it must
//!    satisfy. A file-level write-surface check is not enough: the two
//!    patches in the incident touched disjoint files.
//! 3. **Shape changes name their call sites.** A patch that adds a field to
//!    a public struct declares the constructors it must update, and a spec
//!    that changes a type's shape names its call sites as acceptance
//!    criteria. A constructor on the landing main that the spec does not
//!    name is an `E0063` waiting to happen.
//! 4. **Re-dispatch over hand-repair.** When a semantic conflict is
//!    detected, the patch is re-dispatched against the post-landing main:
//!    the fix is cheap for the agent with the full context and
//!    expensive-and-risky for anyone reconstructing intent from a compiler
//!    error. Hand-supplying the missing *values* of a capacity model is a
//!    guess, and a plausible wrong default is worse than a build error.

use std::collections::BTreeSet;

/// Trim, drop empties, de-duplicate and sort a list of names.
fn normalise_names(items: impl IntoIterator<Item = impl AsRef<str>>) -> BTreeSet<String> {
    items
        .into_iter()
        .map(|item| item.as_ref().trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Rule 1 — the landing base, not the written base
// ---------------------------------------------------------------------------

/// Whether a patch's verification verdict still holds for the main it will
/// land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseCurrency {
    /// The verdict was recorded on the revision the patch will land on.
    Current,
    /// A merge landed after the verdict was recorded: it is about a tree the
    /// patch will not land on. Re-verify (or re-dispatch) against the
    /// landing base before conversion.
    Stale {
        /// The issues whose merges landed between the verified revision and
        /// the landing base, in landing order.
        landed: Vec<u64>,
    },
}

/// A verdict is evidence about the revision it was recorded on — and only
/// that revision. If the landing main has moved since, the verdict is stale
/// regardless of how recently it was recorded: re-testing once at open time
/// is the hole that let the incident through.
pub fn base_currency(
    verified_at: &str,
    landing: &str,
    landed_since_verified: &[u64],
) -> BaseCurrency {
    if verified_at == landing {
        BaseCurrency::Current
    } else {
        BaseCurrency::Stale {
            landed: landed_since_verified.to_vec(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rule 2 — type surfaces, not file surfaces
// ---------------------------------------------------------------------------

/// A ready issue's predicted *type* surface: the public types it will change
/// or construct. File-level write surfaces miss the incident shape, where
/// two patches touch disjoint files but one changes a type the other
/// constructs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSurface {
    /// The issue that declares this surface.
    pub issue: u64,
    /// The public types the patch will change or construct (trimmed,
    /// de-duplicated, sorted).
    pub types: BTreeSet<String>,
    /// The types whose *interface contract* the spec declares — the field
    /// set and semantics other agents must satisfy, or that this patch must
    /// satisfy. Only a contract declared in **both** specs makes concurrent
    /// work on the shared type safe.
    pub interface_contracts: BTreeSet<String>,
}

impl TypeSurface {
    pub fn new(
        issue: u64,
        types: impl IntoIterator<Item = impl AsRef<str>>,
        interface_contracts: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        Self {
            issue,
            types: normalise_names(types),
            interface_contracts: normalise_names(interface_contracts),
        }
    }
}

/// Whether two ready issues may work concurrently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeCoexistence {
    /// Disjoint type surfaces: the issues may run in parallel.
    Concurrent,
    /// Every shared type carries an interface contract in both specs: the
    /// contract is the shared interface, and the issues may run in
    /// parallel. `shared` lists the contracted types, for the record.
    Contracted { shared: Vec<String> },
    /// At least one shared type has no contract in both specs: serialise
    /// the issues. `shared` lists the uncontracted types, in sorted order.
    Serialise { shared: Vec<String> },
}

/// Decide coexistence from two type surfaces. A contract counts only when
/// *both* specs declare it: one side declaring the shape it expects is an
/// assumption, not an interface.
pub fn type_coexistence(a: &TypeSurface, b: &TypeSurface) -> TypeCoexistence {
    let shared: BTreeSet<String> = a.types.intersection(&b.types).cloned().collect();
    if shared.is_empty() {
        return TypeCoexistence::Concurrent;
    }
    let in_a: BTreeSet<String> = shared
        .intersection(&a.interface_contracts)
        .cloned()
        .collect();
    let contracted: BTreeSet<String> = in_a.intersection(&b.interface_contracts).cloned().collect();
    let uncontracted: Vec<String> = shared.difference(&contracted).cloned().collect();
    if uncontracted.is_empty() {
        TypeCoexistence::Contracted {
            shared: shared.into_iter().collect(),
        }
    } else {
        TypeCoexistence::Serialise {
            shared: uncontracted,
        }
    }
}

// ---------------------------------------------------------------------------
// Rule 3 — shape changes name their call sites
// ---------------------------------------------------------------------------

/// One construction site of a public type in the tree a patch will land on:
/// the place that will fail with `E0063` when a field is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructorSite {
    /// The repo-relative path of the file that constructs the type.
    pub file: String,
    /// The constructor — function, `impl`, or struct initializer — that must
    /// gain the new fields.
    pub site: String,
}

impl ConstructorSite {
    /// The canonical key a spec uses to name this call site: `file::site`.
    pub fn key(&self) -> String {
        format!("{}::{}", self.file, self.site)
    }
}

/// A shape change: the fields a patch adds to a public struct, and the
/// constructors the spec declares it must update (as acceptance criteria).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeChange {
    /// The public struct the patch changes.
    pub type_name: String,
    /// The fields the patch adds (trimmed, de-duplicated, sorted).
    pub added_fields: BTreeSet<String>,
    /// The call sites the spec declares must be updated, in
    /// [`ConstructorSite::key`] form (trimmed, de-duplicated, sorted).
    pub declared_sites: BTreeSet<String>,
}

impl ShapeChange {
    pub fn new(
        type_name: impl AsRef<str>,
        added_fields: impl IntoIterator<Item = impl AsRef<str>>,
        declared_sites: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        Self {
            type_name: type_name.as_ref().trim().to_string(),
            added_fields: normalise_names(added_fields),
            declared_sites: normalise_names(declared_sites),
        }
    }
}

/// The call sites on the landing main that the shape change will break and
/// the spec does not name — the `E0063` set. Sorted by key.
pub fn unaddressed_constructors(
    change: &ShapeChange,
    landing_sites: &[ConstructorSite],
) -> Vec<ConstructorSite> {
    let declared: BTreeSet<String> = change.declared_sites.iter().cloned().collect();
    let mut missing: Vec<&ConstructorSite> = landing_sites
        .iter()
        .filter(|site| !declared.contains(&site.key()))
        .collect();
    missing.sort_by_key(|site| site.key());
    missing.into_iter().cloned().collect()
}

/// Whether a spec for a shape change names *any* call sites. A spec that
/// changes a type's shape and names none of its constructors cannot be
/// satisfied: the agent does not know the constructors exist.
pub fn spec_names_call_sites(change: &ShapeChange) -> bool {
    !change.declared_sites.is_empty()
}

// ---------------------------------------------------------------------------
// Rule 4 — re-dispatch over hand-repair
// ---------------------------------------------------------------------------

/// The verdict of composing a patch with the main it will land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationVerdict {
    /// The patch composes with the landing main.
    Clean,
    /// The merge succeeds and the code is wrong: the type and its
    /// constructors are out of sync, and the construction sites the spec
    /// did not name fail with `E0063`.
    SemanticConflict {
        /// The unaddressed construction sites, sorted by key.
        missing: Vec<ConstructorSite>,
    },
}

/// What to do about a semantic conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairStrategy {
    /// Nothing to repair.
    NoAction,
    /// Re-run the authoring agent against the post-landing main. The agent
    /// that wrote the type knows what the missing fields should be; the fix
    /// is cheap for it and expensive-and-risky for anyone reconstructing
    /// intent from a compiler error.
    ReDispatch {
        /// The landing revision the re-dispatch must build and test against.
        against: String,
    },
    /// Hand-repair: the last resort, only when re-dispatch is impossible.
    /// Someone supplying the missing *values* is guessing — a plausible
    /// wrong default in a capacity model is worse than a build error.
    ManualRepair,
}

/// The repair rule: a semantic conflict is re-dispatched against the
/// landing main, not hand-patched. Hand-repair is only chosen when the
/// authoring context is unavailable.
pub fn repair_strategy(
    verdict: &IntegrationVerdict,
    redispatch_available: bool,
    landing: &str,
) -> RepairStrategy {
    match verdict {
        IntegrationVerdict::Clean => RepairStrategy::NoAction,
        IntegrationVerdict::SemanticConflict { .. } => {
            if redispatch_available {
                RepairStrategy::ReDispatch {
                    against: landing.to_string(),
                }
            } else {
                RepairStrategy::ManualRepair
            }
        }
    }
}

/// The full assessment of a patch against the main it will land on: whether
/// its verdict is still current, whether it composes, and what to do if it
/// does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationAssessment {
    /// Rule 1: is the recorded verdict still about the landing revision?
    pub base: BaseCurrency,
    /// Rules 3/4: does the patch compose with the landing main?
    pub verdict: IntegrationVerdict,
    /// Rule 4: what to do about the verdict.
    pub repair: RepairStrategy,
}

/// Assess a shape-change patch against the main it will land on.
///
/// `landing_sites` is the set of constructors of the changed type in the
/// *landing* main — the tree neither the patch nor its verdict may have
/// seen. `landed_since_verified` are the merges that landed after the
/// verified revision, in landing order.
pub fn assess(
    verified_at: &str,
    landing: &str,
    landed_since_verified: &[u64],
    change: &ShapeChange,
    landing_sites: &[ConstructorSite],
    redispatch_available: bool,
) -> IntegrationAssessment {
    let base = base_currency(verified_at, landing, landed_since_verified);
    let missing = unaddressed_constructors(change, landing_sites);
    let verdict = if missing.is_empty() {
        IntegrationVerdict::Clean
    } else {
        IntegrationVerdict::SemanticConflict { missing }
    };
    let repair = repair_strategy(&verdict, redispatch_available, landing);
    IntegrationAssessment {
        base,
        verdict,
        repair,
    }
}
