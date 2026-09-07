//! The integration phase (issue #3565).
//!
//! Autospec dispatches parallel implementers and, without this module, stops
//! caring at the PR: rebasing, resolving conflicts, and verifying the
//! resolution were left to a human or an ad-hoc agent. This module owns
//! integration as a phase:
//!
//! 1. **Own the batch.** After a batch of parallel implementer branches,
//!    rebase and land the set in dependency order
//!    ([`IntegrationPhase::run`]).
//! 2. **Gate on preservation, not on a green build.** After resolving, the
//!    union of the symbols added by either parent must still be present
//!    ([`symbols::check_preservation`]); a resolution that drops one is
//!    rejected even if the build is green.
//! 3. **Never skip.** [`Vcs`] exposes no skip operation; a branch that
//!    cannot be integrated is left untouched and reported.
//! 4. **Stop, do not force.** A conflict where the two sides genuinely
//!    disagree (not merely coexist) halts that branch and reports which
//!    hunk and why; the rest of the batch stays landable.
//! 5. **Re-verify after each rebase.** Each rebase changes the base for the
//!    next, so the verifier runs after every rebase and a failure names the
//!    branch that introduced it.

pub mod git;
pub mod resolution;
pub mod symbols;
pub mod vcs;

pub use git::GitVcs;
pub use resolution::{Resolution, ResolveError, Resolver, SemanticConflict, UnionResolver};
pub use symbols::{check_preservation, MissingSymbol, PreservationFailure};
pub use vcs::{ConflictHunk, ConflictedFile, RebaseOutcome, ResolvedFile, Vcs, VcsError};

/// Result of running the verifier for one branch after its rebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    Passed,
    Failed { reason: String },
}

/// Verifies the integration of one branch after its rebase (build, tests,
/// or any pipeline check). Runs after *every* rebase, not once at the end.
pub trait Verifier {
    fn verify(&self, branch: &str) -> VerifyOutcome;
}

/// Verifier that records no failure. Used when the pipeline has no local
/// check configured; the symbol-preservation gate still applies.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopVerifier;

impl Verifier for NoopVerifier {
    fn verify(&self, _branch: &str) -> VerifyOutcome {
        VerifyOutcome::Passed
    }
}

impl Verifier for Box<dyn Verifier> {
    fn verify(&self, branch: &str) -> VerifyOutcome {
        (**self).verify(branch)
    }
}

/// What happened to one branch of the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchOutcome {
    /// Rebased, gated, verified, and landed into the trunk.
    Landed,
    /// The resolution dropped a symbol added by a parent. The symbol(s) are
    /// named; the branch was not landed.
    RejectedSymbolLoss { missing: Vec<MissingSymbol> },
    /// A genuine semantic disagreement. The offending hunks and reasons are
    /// reported for a human; the branch was not landed and the rest of the
    /// batch stays landable.
    HaltedSemantic { conflicts: Vec<SemanticConflict> },
    /// Post-rebase verification failed. The branch is named here; the
    /// branch was not landed.
    VerificationFailed { reason: String },
    /// The resolver failed.
    ResolveError { message: String },
    /// A VCS operation failed; the message is the backend's.
    VcsError { message: String },
}

impl BranchOutcome {
    pub fn is_landed(&self) -> bool {
        matches!(self, Self::Landed)
    }

    /// One-line human description, hunk- and symbol-named.
    pub fn describe(&self) -> String {
        match self {
            Self::Landed => "landed".to_string(),
            Self::RejectedSymbolLoss { missing } => {
                format!(
                    "rejected: resolution dropped symbol(s) added by a parent: {}",
                    missing
                        .iter()
                        .map(|m| format!("{}: {}", m.file, m.symbol))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Self::HaltedSemantic { conflicts } => {
                let first = &conflicts[0];
                let mut text = format!(
                    "halted: semantic conflict at {}: line {} ({}); a human must adjudicate",
                    first.file,
                    first.start + 1,
                    first.reason
                );
                if conflicts.len() > 1 {
                    for extra in &conflicts[1..] {
                        text.push_str(&format!(
                            "; also {}: line {} ({})",
                            extra.file,
                            extra.start + 1,
                            extra.reason
                        ));
                    }
                }
                text
            }
            Self::VerificationFailed { reason } => {
                format!("verification failed after rebase: {reason}")
            }
            Self::ResolveError { message } => format!("resolver failed: {message}"),
            Self::VcsError { message } => format!("vcs error: {message}"),
        }
    }
}

/// One branch and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchResult {
    pub branch: String,
    pub outcome: BranchOutcome,
}

/// The result of integrating one batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchReport {
    /// Set when the batch itself could not even be listed.
    pub error: Option<String>,
    /// Outcomes in dependency (processing) order.
    pub results: Vec<BranchResult>,
}

impl BatchReport {
    /// Branches that were landed, in order.
    pub fn landed(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| r.outcome.is_landed())
            .map(|r| r.branch.as_str())
            .collect()
    }

    /// Branches that did not land, in order.
    pub fn halted(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| !r.outcome.is_landed())
            .map(|r| r.branch.as_str())
            .collect()
    }

    /// True only when every branch landed and no error occurred.
    pub fn ok(&self) -> bool {
        self.error.is_none() && self.results.iter().all(|r| r.outcome.is_landed())
    }
}

/// The integration phase: rebase and land a batch in dependency order,
/// gating each conflict resolution on symbol preservation and re-verifying
/// after every rebase.
pub struct IntegrationPhase<V: Vcs, R: Resolver, T: Verifier> {
    vcs: V,
    resolver: R,
    verifier: T,
}

impl<V: Vcs, R: Resolver, T: Verifier> IntegrationPhase<V, R, T> {
    pub fn new(vcs: V, resolver: R, verifier: T) -> Self {
        Self {
            vcs,
            resolver,
            verifier,
        }
    }

    /// Integrate the whole batch. Never skips a branch: every branch ends
    /// in exactly one outcome, and a halt on one branch leaves the rest of
    /// the batch landable.
    pub fn run(mut self) -> BatchReport {
        let branches = match self.vcs.batch() {
            Ok(branches) => branches,
            Err(error) => {
                return BatchReport {
                    error: Some(error.0),
                    results: Vec::new(),
                }
            }
        };
        let mut report = BatchReport {
            error: None,
            results: Vec::new(),
        };
        for branch in branches {
            let outcome = match self.vcs.rebase(&branch) {
                Ok(RebaseOutcome::Clean) => self.finish(&branch),
                Ok(RebaseOutcome::Conflict { files }) => self.handle_conflict(&branch, files),
                Err(error) => BranchOutcome::VcsError { message: error.0 },
            };
            report.results.push(BranchResult { branch, outcome });
        }
        // Hand the repository back in a settled state: no rebase in
        // progress, trunk checked out — even when the last branch halted.
        if let Err(error) = self.vcs.settle() {
            report.error = Some(error.0);
        }
        report
    }

    /// Classify and resolve one conflicted rebase.
    ///
    /// Stop, do not force: a genuine semantic disagreement (a side modified
    /// or deleted an ancestor line) halts this branch and reports the hunk
    /// for a human. Otherwise the resolution must pass the symbol-
    /// preservation gate — what was preserved, not what compiles.
    fn handle_conflict(&mut self, branch: &str, files: Vec<ConflictedFile>) -> BranchOutcome {
        let conflicts = SemanticConflict::find(&files);
        if !conflicts.is_empty() {
            return BranchOutcome::HaltedSemantic { conflicts };
        }
        let resolution = match self.resolver.resolve(&files) {
            Ok(resolution) => resolution,
            Err(error) => return BranchOutcome::ResolveError { message: error.0 },
        };
        match check_preservation(&files, &resolution) {
            Some(PreservationFailure::SymbolLoss { missing }) => {
                BranchOutcome::RejectedSymbolLoss { missing }
            }
            Some(PreservationFailure::FileUnresolved { path }) => BranchOutcome::ResolveError {
                message: format!("conflicted file has no resolved version: {path}"),
            },
            None => match self.vcs.apply_resolution(branch, &resolution.files) {
                Err(error) => BranchOutcome::VcsError { message: error.0 },
                Ok(()) => self.finish(branch),
            },
        }
    }

    /// Re-verify after this branch's rebase, then land when verification
    /// passes. A failure names this branch.
    fn finish(&mut self, branch: &str) -> BranchOutcome {
        match self.verifier.verify(branch) {
            VerifyOutcome::Passed => match self.vcs.land(branch) {
                Ok(()) => BranchOutcome::Landed,
                Err(error) => BranchOutcome::VcsError { message: error.0 },
            },
            VerifyOutcome::Failed { reason } => BranchOutcome::VerificationFailed { reason },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_landed_halted_and_ok() {
        let report = BatchReport {
            error: None,
            results: vec![
                BranchResult {
                    branch: "a".to_string(),
                    outcome: BranchOutcome::Landed,
                },
                BranchResult {
                    branch: "b".to_string(),
                    outcome: BranchOutcome::VerificationFailed {
                        reason: "tests failed".to_string(),
                    },
                },
            ],
        };
        assert_eq!(report.landed(), vec!["a"]);
        assert_eq!(report.halted(), vec!["b"]);
        assert!(!report.ok());
    }

    #[test]
    fn describe_names_symbol_and_hunk() {
        let loss = BranchOutcome::RejectedSymbolLoss {
            missing: vec![MissingSymbol {
                file: "store.go".to_string(),
                symbol: "regToken".to_string(),
            }],
        };
        assert!(loss.describe().contains("store.go: regToken"));

        let semantic = BranchOutcome::HaltedSemantic {
            conflicts: vec![SemanticConflict {
                file: "store.go".to_string(),
                start: 4,
                reason: "trunk side modified or deleted 1 ancestor line(s)".to_string(),
                ancestor: vec![],
                ours: vec![],
                theirs: vec![],
            }],
        };
        assert!(semantic.describe().contains("store.go"));
        assert!(semantic.describe().contains("line 5"));
        assert!(semantic.describe().contains("trunk side"));
    }
}
