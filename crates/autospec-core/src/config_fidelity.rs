//! Config fidelity (#3692): the config file is a description of where
//! configuration lives, not the whole of it.
//!
//! The incident this module exists for (#3692): a service was repointed from
//! one model to another. The configuration file was edited — endpoint URL,
//! model list, title model — every value in the file correct, and the service
//! restarted clean. It then failed at runtime with `model qwen3.5 is not
//! available`, a model that appeared nowhere in the file. The real value lived
//! in a **database record**. The config file pinned a named agent; the agent's
//! own row carried `provider: fry-local, model: qwen3.5:9b`, written when the
//! agent was created and never revisited. Grepping the config directory for
//! the stale model returned nothing, which read as "the migration is complete"
//! — when it only meant the file-shaped half was done, and the file-shaped
//! half is the half a search tool can see.
//!
//! Once a system supports user-created entities — agents, presets, saved
//! sessions, workspaces — those entities carry their own copies of settings,
//! written at creation time and immune to later edits of the file. The failure
//! is quiet in the direction that matters: an empty grep is *evidence of
//! completion* to a reader, when it only proves the searched half is clean.
//!
//! Four invariants, one primitive each. Everything here is pure and
//! testable: no I/O, no clock, no subprocess.
//!
//! 1. **Enumerate the stores before editing, not after failing**
//!    ([`MigrationPlan`], [`StoreKind`], [`MigrationPlan::uncovered`]). A
//!    migration plan names the stores it will touch; against the value's
//!    actual footprint it must name *every* one. A plan that names only the
//!    file is incomplete by construction.
//! 2. **Require zero hits in every store, not just the one you edited**
//!    ([`verify_migration`], [`MigrationAudit`]). The migration is
//!    [`MigrationAudit::is_complete`] only when every store in the footprint
//!    was searched and every searched store reports zero hits for the old
//!    value. The audit carries the clean / dirty / unsearched buckets so no
//!    finding is hidden behind the summary code.
//! 3. **Preserve the prior value when rewriting a record**
//!    ([`record_rewrite`], [`RewrittenRecord`]). A rewrite keeps the old value
//!    and a dated field, so the change is reversible and auditable without a
//!    backup restore.
//! 4. **Treat a clean grep as unverified, not as done**
//!    ([`StoreScan`]). A store that was never inspected is unverified — never
//!    clean — regardless of what its hit count happens to say.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A place a migrated setting can live.
///
/// The set is open by construction (a new store kind is a new variant), but
/// the four named here are the ones the #3692 incident proves easy to forget:
/// the file is the obvious half; the database and the per-entity records are
/// the half a search tool cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    /// A file-shaped store: the configuration file(s) on disk.
    ConfigFile,
    /// Environment variables the service reads at start.
    Environment,
    /// A database collection (a table, a set of rows).
    Database,
    /// A per-user or per-entity record: an agent, preset, saved session, or
    /// workspace — anything that snapshots a setting at creation time.
    PerEntityRecord,
}

impl StoreKind {
    /// Every store kind the vocabulary currently names.
    pub const ALL: [Self; 4] = [
        Self::ConfigFile,
        Self::Environment,
        Self::Database,
        Self::PerEntityRecord,
    ];

    /// The machine name an audit reports for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfigFile => "config_file",
            Self::Environment => "environment",
            Self::Database => "database",
            Self::PerEntityRecord => "per_entity_record",
        }
    }

    /// Parse a machine name back to a kind; `None` when unknown, so a typo
    /// degrades to "uncovered", not a guess.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == name)
    }
}

// ── 1. Enumerate the stores before editing ─────────────────────────────

/// The plan for migrating one setting from one value to another.
///
/// It names the stores the plan *says* it will touch. That list is checked
/// against the value's actual footprint by [`MigrationPlan::uncovered`]; a
/// plan that omits a store the value can live in is the #3692 plan that named
/// only the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationPlan {
    /// The setting being migrated (its name, not its value).
    pub setting: String,
    /// The old value to remove from every store.
    pub from: String,
    /// The new value every store should carry afterwards.
    pub to: String,
    /// The stores the plan names as touched.
    pub stores: BTreeSet<StoreKind>,
}

impl MigrationPlan {
    pub fn new(
        setting: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        stores: impl IntoIterator<Item = StoreKind>,
    ) -> Self {
        Self {
            setting: setting.into(),
            from: from.into(),
            to: to.into(),
            stores: stores.into_iter().collect(),
        }
    }

    /// The stores in `footprint` the plan forgot to name, in deterministic
    /// order. Non-empty means the plan is incomplete by construction — it
    /// cannot migrate a value it never told the operator to look for.
    pub fn uncovered(&self, footprint: &BTreeSet<StoreKind>) -> Vec<StoreKind> {
        footprint.difference(&self.stores).copied().collect()
    }

    /// Whether the plan covers the value's full footprint.
    pub fn is_complete(&self, footprint: &BTreeSet<StoreKind>) -> bool {
        self.uncovered(footprint).is_empty()
    }
}

// ── 4. A clean grep is unverified, not done ────────────────────────────

/// The result of scanning one store for the old value.
///
/// The distinction that matters is not the hit count but whether the store was
/// *inspected at all*. A store that was never searched is
/// [`StoreScan::is_unverified`], never clean — an empty grep of the file is
/// evidence of nothing about a database nobody opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreScan {
    pub kind: StoreKind,
    /// A human name for the concrete store, for the audit trail
    /// (`"agents"`, `"conversations"`, `"config.yaml"`).
    pub name: String,
    /// Whether this store was actually inspected for the old value.
    pub searched: bool,
    /// Occurrences of the old value found. Meaningful only when `searched` is
    /// true; a never-searched store reports zero and must not be read as
    /// clean.
    pub hits: u64,
}

impl StoreScan {
    /// A store that was searched and found to hold no occurrence of the old
    /// value.
    pub fn clean(kind: StoreKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            searched: true,
            hits: 0,
        }
    }

    /// A store that was searched and still holds the old value.
    pub fn dirty(kind: StoreKind, name: impl Into<String>, hits: u64) -> Self {
        Self {
            kind,
            name: name.into(),
            searched: true,
            hits,
        }
    }

    /// A store that was never inspected. Unverified, not clean, no matter what
    /// `hits` would otherwise say (and `hits` is forced to zero so it cannot
    /// masquerade as a searched-but-clean store).
    pub fn unverified(kind: StoreKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            searched: false,
            hits: 0,
        }
    }

    /// Whether this scan reports the old value as gone: the store was
    /// searched *and* found zero occurrences. A never-searched store is never
    /// clean — that is invariant 4.
    pub fn is_clean(&self) -> bool {
        self.searched && self.hits == 0
    }

    /// Whether this store was never inspected: the verification has a hole.
    pub fn is_unverified(&self) -> bool {
        !self.searched
    }

    /// Whether this store still holds the old value.
    pub fn is_dirty(&self) -> bool {
        self.searched && self.hits > 0
    }
}

// ── 2. Require zero hits in every store ────────────────────────────────

/// The summary of a migration check, in precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationVerdict {
    /// Every store in the footprint was searched and every one is clean.
    Complete,
    /// At least one searched store still holds the old value.
    Incomplete,
    /// At least one store was never searched: nothing can be claimed, because
    /// the verification has a hole. A clean grep of the file is this verdict,
    /// not [`MigrationVerdict::Complete`].
    Unverified,
}

/// The full picture of a migration check across stores.
///
/// Three buckets, so no finding is hidden behind the summary code: *clean*
/// (searched, zero hits), *dirty* (searched, still holds the old value), and
/// *unsearched* (never inspected — the #3692 blind spot).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MigrationAudit {
    pub clean: Vec<String>,
    pub dirty: Vec<String>,
    pub unsearched: Vec<String>,
}

impl MigrationAudit {
    /// Whether the migration is verifiably complete: no store still holds the
    /// old value *and* no store was left unsearched.
    pub fn is_complete(&self) -> bool {
        self.dirty.is_empty() && self.unsearched.is_empty()
    }

    /// The single summary verdict, in precedence order: an unsearched store
    /// means the verification itself has a hole (invariant 4), which outranks
    /// a known-dirty store, which outranks complete.
    pub fn verdict(&self) -> MigrationVerdict {
        if !self.unsearched.is_empty() {
            MigrationVerdict::Unverified
        } else if !self.dirty.is_empty() {
            MigrationVerdict::Incomplete
        } else {
            MigrationVerdict::Complete
        }
    }

    /// The machine code a report emits for [`Self::verdict`].
    pub fn code(&self) -> &'static str {
        match self.verdict() {
            MigrationVerdict::Complete => "COMPLETE",
            MigrationVerdict::Incomplete => "INCOMPLETE",
            MigrationVerdict::Unverified => "UNVERIFIED",
        }
    }
}

/// Bucket the scans and fold in the footprint, so a store the plan never even
/// scanned is a blind spot the code can see.
///
/// `footprint` is the value's full set of stores (from invariant 1); any
/// footprint kind with no scan at all is added to *unsearched*. The audit
/// never reports [`MigrationVerdict::Complete`] unless every footprint store
/// was searched and clean — the #3692 failure (file clean, database never
/// opened) comes back `Unverified`, never `Complete`.
pub fn verify_migration(footprint: &BTreeSet<StoreKind>, scans: &[StoreScan]) -> MigrationAudit {
    let mut audit = MigrationAudit::default();
    let mut covered: BTreeSet<StoreKind> = BTreeSet::new();
    for scan in scans {
        covered.insert(scan.kind);
        if !scan.searched {
            audit.unsearched.push(scan.name.clone());
        } else if scan.hits > 0 {
            audit.dirty.push(scan.name.clone());
        } else {
            audit.clean.push(scan.name.clone());
        }
    }
    // A footprint store with no scan at all: the #3692 database nobody opened.
    for kind in footprint.difference(&covered) {
        audit.unsearched.push(kind.as_str().to_string());
    }
    // Deterministic, human-readable ordering.
    audit.clean.sort();
    audit.dirty.sort();
    audit.unsearched.sort();
    audit
}

// ── 3. Preserve the prior value when rewriting ─────────────────────────

/// A record rewrite that keeps the prior value.
///
/// Invariant 3: when a stored record is rewritten to carry the new value, the
/// old value is written to a dated field, so the change is reversible and
/// auditable without a backup restore. The type makes the prior value a
/// required field — a rewrite that drops it cannot be expressed here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewrittenRecord {
    pub store: StoreKind,
    /// The record's stable id (`"agent-42"`, `"conv-7"`).
    pub record_id: String,
    /// The setting's field within the record (`"model"`, `"provider"`).
    pub field: String,
    /// The value before the rewrite — preserved for reversibility and the
    /// audit trail.
    pub previous_value: String,
    /// The value the record carries after the rewrite.
    pub new_value: String,
    /// When the rewrite happened. The caller supplies this; the module keeps
    /// no clock.
    pub rewritten_at: String,
}

/// Build a rewritten record, carrying the prior value so the change is
/// reversible.
///
/// `previous_value` is mandatory — a rewrite that forgets the old value is the
/// thing that made a later rollback impossible in #3692.
pub fn record_rewrite(
    store: StoreKind,
    record_id: impl Into<String>,
    field: impl Into<String>,
    previous_value: impl Into<String>,
    new_value: impl Into<String>,
    rewritten_at: impl Into<String>,
) -> RewrittenRecord {
    RewrittenRecord {
        store,
        record_id: record_id.into(),
        field: field.into(),
        previous_value: previous_value.into(),
        new_value: new_value.into(),
        rewritten_at: rewritten_at.into(),
    }
}

impl RewrittenRecord {
    /// Whether the rewrite is reversible: the prior value was preserved and a
    /// dated marker exists so the change can be audited and undone.
    pub fn is_reversible(&self) -> bool {
        !self.previous_value.is_empty() && !self.rewritten_at.is_empty()
    }
}
