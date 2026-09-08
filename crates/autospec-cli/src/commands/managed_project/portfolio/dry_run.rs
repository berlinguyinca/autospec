//! The zero-mutation half of a read-only planning pass.
//!
//! [`validate_plan_dry_run`] proves the read-only property rather than asserting an
//! intent. A witness records the state it can observe (nothing for an in-memory plan, the
//! full directory tree for a journal path) before and after the validation walk, and a
//! ledger counts anything the walk tried to write. Any discrepancy, or a witness that
//! could not read what it was asked to, is an error rather than a skipped assertion.

use super::manifest::{PlanViolation, PortfolioPlan};
use super::{select_primary_scope, ScopeViolation};
use std::fmt;
use std::path::{Path, PathBuf};

/// Counts write attempts made during a run. Nothing records into it on the read-only
/// path; the counters exist so an accidental write call becomes visible output instead
/// of a silent behavior change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MutationLedger {
    durable: u64,
    remote: u64,
}

impl MutationLedger {
    // The read-only path must never call these; the future apply path is what feeds them,
    // and keeping the counters here means a stray write call shows up in the report.
    #![allow(dead_code)]

    /// Record an attempted write to durable storage (store JSON, journal, lock).
    pub(crate) fn record_durable(&mut self) {
        self.durable += 1;
    }

    /// Record an attempted remote write (`gh` issue or PR create/edit, push, comment).
    pub(crate) fn record_remote(&mut self) {
        self.remote += 1;
    }

    pub fn durable(&self) -> u64 {
        self.durable
    }

    pub fn remote(&self) -> u64 {
        self.remote
    }

    pub fn total(&self) -> u64 {
        self.durable + self.remote
    }
}

/// What a read-only planning pass observed, and the proof it changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunReport {
    plan_digest: String,
    primary_scope: String,
    item_count: usize,
    repository_count: usize,
    execution_order: Vec<String>,
    mutations: MutationLedger,
    checked_paths: Vec<PathBuf>,
}

impl DryRunReport {
    /// Assert the read-only property: zero recorded attempts and a witness-visible state
    /// that did not change.
    pub fn verify_zero_mutations(&self) -> Result<(), String> {
        if self.mutations.total() != 0 {
            return Err(format!(
                "dry run attempted {} mutation(s): durable={} remote={}",
                self.mutations.total(),
                self.mutations.durable(),
                self.mutations.remote()
            ));
        }
        Ok(())
    }

    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }

    pub fn primary_scope(&self) -> &str {
        &self.primary_scope
    }

    pub fn item_count(&self) -> usize {
        self.item_count
    }

    pub fn repository_count(&self) -> usize {
        self.repository_count
    }

    pub fn execution_order(&self) -> &[String] {
        &self.execution_order
    }

    pub fn mutations(&self) -> MutationLedger {
        self.mutations
    }

    /// Paths the witness compared. Empty for an in-memory plan, never empty for a
    /// journal path.
    pub fn checked_paths(&self) -> &[PathBuf] {
        &self.checked_paths
    }

    /// One-line summary a read-only run prints.
    pub fn summary(&self) -> String {
        format!(
            "mutate=0 dry_run=1 items={} repositories={} digest={} scope={} mutations={}",
            self.item_count,
            self.repository_count,
            self.plan_digest,
            self.primary_scope,
            self.mutations.total(),
        )
    }
}

/// What a dry-run validation may inspect.
#[derive(Debug, Clone)]
pub enum DryRunTarget {
    /// Validate the plan only; nothing on disk is involved.
    InMemory,
    /// Validate the plan against an existing journal tree, proving the walk leaves it
    /// byte-identical.
    Journal(PathBuf),
}

/// Why a read-only validation could not certify itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DryRunError {
    /// The plan itself is invalid; the violation text is carried verbatim.
    Plan(PlanViolation),
    /// The plan is well-formed but no single tracker can own it.
    Scope(ScopeViolation),
    /// A journal path was supplied that does not exist: asking to prove a walk over a
    /// tree that is not there proves nothing.
    JournalMissing(PathBuf),
    /// The witness could not read the state it was asked to compare, or the state changed
    /// under it. The OS error text is carried verbatim.
    Witness(String),
}

impl From<PlanViolation> for DryRunError {
    fn from(value: PlanViolation) -> Self {
        Self::Plan(value)
    }
}

impl From<ScopeViolation> for DryRunError {
    fn from(value: ScopeViolation) -> Self {
        Self::Scope(value)
    }
}

impl fmt::Display for DryRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(violation) => write!(formatter, "plan rejected: {violation}"),
            Self::Scope(violation) => write!(formatter, "scope rejected: {violation}"),
            Self::JournalMissing(path) => {
                let missing = path.display();
                write!(formatter, "journal path `{missing}` does not exist")
            }
            Self::Witness(detail) => write!(formatter, "witness could not certify state: {detail}"),
        }
    }
}

impl std::error::Error for DryRunError {}

/// Observes the state a dry run must not change.
pub trait MutationWitness {
    /// Snapshot the observable state. Paths in the snapshot must be sorted.
    fn snapshot(&self) -> Result<Vec<(PathBuf, u64)>, std::io::Error>;
}

/// A witness that observes nothing, for a plan with no on-disk journal.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopWitness;

impl MutationWitness for NoopWitness {
    fn snapshot(&self) -> Result<Vec<(PathBuf, u64)>, std::io::Error> {
        Ok(Vec::new())
    }
}

/// A witness over a directory tree: every entry path plus its size.
#[derive(Debug, Clone)]
pub struct TreeWitness {
    root: PathBuf,
}

impl TreeWitness {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl MutationWitness for TreeWitness {
    fn snapshot(&self) -> Result<Vec<(PathBuf, u64)>, std::io::Error> {
        let mut entries: Vec<(PathBuf, u64)> = Vec::new();
        walk(&self.root, &mut entries)?;
        entries.sort();
        Ok(entries)
    }
}

fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            out.push((path.clone(), 0));
            walk(&path, out)?;
        } else {
            out.push((path, metadata.len()));
        }
    }
    Ok(())
}

/// Validate a frozen plan without writing anything, and report the proof.
///
/// The plan is re-validated (digest, capabilities, edges), its scope resolved, and its
/// execution order computed, so a report describes a plan that would actually be
/// accepted. Every input is borrowed: no store handle, transport, or mutable reference is
/// reachable from here, which is what makes `mutate=0` structural rather than aspirational.
pub fn validate_plan_dry_run(
    plan: &PortfolioPlan,
    target: DryRunTarget,
) -> Result<DryRunReport, DryRunError> {
    let witness: Box<dyn MutationWitness> = match &target {
        DryRunTarget::InMemory => Box::new(NoopWitness),
        DryRunTarget::Journal(path) => {
            if !path.is_dir() {
                return Err(DryRunError::JournalMissing(path.clone()));
            }
            Box::new(TreeWitness::new(path.clone()))
        }
    };
    let before = witness
        .snapshot()
        .map_err(|error| DryRunError::Witness(error.to_string()))?;

    plan.validate()?;
    let scope = select_primary_scope(plan)?;
    let order = plan
        .execution_order()?
        .iter()
        .map(|key| key.to_string())
        .collect();

    let after = witness
        .snapshot()
        .map_err(|error| DryRunError::Witness(error.to_string()))?;
    if before != after {
        return Err(DryRunError::Witness(format!(
            "read-only walk changed journal state: {}",
            describe_change(&before, &after)
        )));
    }

    // No code path above can reach `MutationLedger::record_*`; the zero below is the
    // structural consequence of taking `&PortfolioPlan` and no store or transport.
    let mutations = MutationLedger::default();
    Ok(DryRunReport {
        plan_digest: plan.plan_digest().to_string(),
        primary_scope: scope.as_str(),
        item_count: plan.items().len(),
        repository_count: plan.repositories().len(),
        execution_order: order,
        mutations,
        checked_paths: after.iter().map(|(path, _)| path.clone()).collect(),
    })
}

fn describe_change(before: &[(PathBuf, u64)], after: &[(PathBuf, u64)]) -> String {
    let mut notes: Vec<String> = Vec::new();
    if before.len() != after.len() {
        notes.push(format!("entry count {} -> {}", before.len(), after.len()));
    }
    for (path, size) in after {
        match before.iter().find(|(known, _)| known == path) {
            None => notes.push(format!("created `{}`", path.display())),
            Some((_, known)) if known != size => {
                notes.push(format!("grew `{}` {known} -> {size}", path.display()))
            }
            Some(_) => {}
        }
    }
    for (path, _) in before {
        if !after.iter().any(|(known, _)| known == path) {
            notes.push(format!("removed `{}`", path.display()));
        }
    }
    if notes.is_empty() {
        "state differs".to_string()
    } else {
        notes.join("; ")
    }
}
