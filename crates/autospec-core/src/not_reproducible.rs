//! Stale bug-report dispatch: the `NOT-REPRODUCIBLE` terminal status (#4072).
//!
//! A bug report was filed, but the bug it described no longer reproduced on
//! unmodified `main` — the fix had already landed. The implementer was
//! dispatched anyway, and a full run produced a test-only "fix" (a test
//! codifying the already-correct behavior) which scored as passing. The
//! report kept re-dispatching, burning runs on a bug that was not there.
//!
//! The invariants this module enforces:
//!
//! 1. **Reproduction is checked before implementation.** [`preflight`]
//!    decides the dispatch from the issue's described reproduction run
//!    against the *unmodified* tree: dispatch, terminate `NOT-REPRODUCIBLE`
//!    (no patch, evidence recorded), or block. A check that cannot conclude
//!    is a block, never a `NOT-REPRODUCIBLE` — "cannot check" is not
//!    "does not reproduce".
//! 2. **`NOT-REPRODUCIBLE` is a first-class terminal status**
//!    ([`TerminalStatus`]), on par with `VERIFIED` and `NEW-TEST-FAILURES`,
//!    and [`route`] sends it to close-out review, not back into the
//!    dispatch pool.
//! 3. **The acceptance gate rejects the stale-report signature.**
//!    [`acceptance_check`] rejects a patch whose only change is a test that
//!    already passes on the pre-change tree: a test codifying behavior the
//!    unmodified code already has is not a fix.
//! 4. **Reconciliation closes on reproduction evidence, not on naming.**
//!    [`reconcile`] flags every issue whose described reproduction no
//!    longer reproduces as a close candidate, with or without a PR
//!    referencing it, and never closes an issue on a PR reference alone.

/// The token the run records when the report is stale.
pub const STATUS_NOT_REPRODUCIBLE: &str = "NOT-REPRODUCIBLE";
/// The token the run records when the patch is verified.
pub const STATUS_VERIFIED: &str = "VERIFIED";
/// The token the run records when the patch introduced new test failures.
pub const STATUS_NEW_TEST_FAILURES: &str = "NEW-TEST-FAILURES";

/// A terminal status for a bug-report run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalStatus {
    /// The patch landed and the suite is green.
    Verified,
    /// The patch introduced new test failures and was rejected.
    NewTestFailures,
    /// The reported bug no longer reproduces on the unmodified tree: the
    /// report is stale and no patch exists.
    NotReproducible,
}

impl TerminalStatus {
    /// The status token as the run records it: `VERIFIED`,
    /// `NEW-TEST-FAILURES`, `NOT-REPRODUCIBLE`.
    pub fn as_str(self) -> &'static str {
        match self {
            TerminalStatus::Verified => STATUS_VERIFIED,
            TerminalStatus::NewTestFailures => STATUS_NEW_TEST_FAILURES,
            TerminalStatus::NotReproducible => STATUS_NOT_REPRODUCIBLE,
        }
    }

    /// Parse a recorded status token. Strict and case-sensitive: these are
    /// machine tokens, not prose.
    pub fn parse(token: &str) -> Result<Self, String> {
        match token {
            STATUS_VERIFIED => Ok(TerminalStatus::Verified),
            STATUS_NEW_TEST_FAILURES => Ok(TerminalStatus::NewTestFailures),
            STATUS_NOT_REPRODUCIBLE => Ok(TerminalStatus::NotReproducible),
            other => Err(format!(
                "unknown terminal status `{other}`: expected one of {STATUS_VERIFIED}, \
                 {STATUS_NEW_TEST_FAILURES}, {STATUS_NOT_REPRODUCIBLE}"
            )),
        }
    }
}

/// Where an issue goes once its run has reached a terminal status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalRoute {
    /// The issue re-enters the dispatch pool for re-dispatch.
    Redispatch,
    /// The issue leaves the dispatch pool and goes to close-out review.
    CloseReview,
}

/// Route an issue by its terminal status.
///
/// `VERIFIED` and `NOT-REPRODUCIBLE` are verdicts about the issue itself —
/// the fix is in, or the bug is not there — so the issue leaves the
/// dispatch pool and close-out review confirms the closure.
/// `NEW-TEST-FAILURES` is a failed attempt, not a verdict about the issue:
/// the bug may still be there, and the issue goes back into the dispatch
/// pool for a retry.
pub fn route(status: TerminalStatus) -> TerminalRoute {
    match status {
        TerminalStatus::Verified | TerminalStatus::NotReproducible => TerminalRoute::CloseReview,
        TerminalStatus::NewTestFailures => TerminalRoute::Redispatch,
    }
}

/// The outcome of running the issue's described reproduction against the
/// unmodified tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReproOutcome {
    /// The bug reproduced: the report is live.
    Reproduced {
        /// Observed evidence: the failing check, command, or diff.
        evidence: String,
    },
    /// The bug did not reproduce on the unmodified tree: the report is
    /// stale.
    NotReproduced {
        /// Observed evidence: the passing check that contradicts the report.
        evidence: String,
    },
    /// The reproduction check could not be run or did not conclude.
    Inconclusive {
        /// Why the check could not conclude.
        reason: String,
    },
}

impl ReproOutcome {
    /// Build a `Reproduced` outcome. Evidence is mandatory and must not be
    /// blank: a reproduction claim without observed evidence is not a claim.
    pub fn reproduced(evidence: impl Into<String>) -> Result<Self, String> {
        require_detail("reproduction evidence", evidence)
            .map(|evidence| ReproOutcome::Reproduced { evidence })
    }

    /// Build a `NotReproduced` outcome. Evidence is mandatory and must not
    /// be blank: the `NOT-REPRODUCIBLE` termination records it.
    pub fn not_reproduced(evidence: impl Into<String>) -> Result<Self, String> {
        require_detail("reproduction evidence", evidence)
            .map(|evidence| ReproOutcome::NotReproduced { evidence })
    }

    /// Build an `Inconclusive` outcome. The reason is mandatory and must
    /// not be blank: it becomes the blocker the run records.
    pub fn inconclusive(reason: impl Into<String>) -> Result<Self, String> {
        require_detail("inconclusive reason", reason)
            .map(|reason| ReproOutcome::Inconclusive { reason })
    }

    /// The evidence or reason this outcome carries.
    pub fn detail(&self) -> &str {
        match self {
            ReproOutcome::Reproduced { evidence } | ReproOutcome::NotReproduced { evidence } => {
                evidence
            }
            ReproOutcome::Inconclusive { reason } => reason,
        }
    }

    /// Whether this outcome observed the bug.
    pub fn observed_bug(&self) -> bool {
        matches!(self, ReproOutcome::Reproduced { .. })
    }
}

fn require_detail(what: &str, detail: impl Into<String>) -> Result<String, String> {
    let detail = detail.into();
    if detail.trim().is_empty() {
        return Err(format!("{what} must be non-blank"));
    }
    Ok(detail)
}

/// The implementer's preflight decision for one bug-report dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightDecision {
    /// The bug reproduces on the unmodified tree: dispatch the implementer.
    Dispatch {
        /// The observed evidence the bug is live.
        evidence: String,
    },
    /// The report is stale: the run terminates with
    /// [`TerminalStatus::NotReproducible`]. No patch is produced and no
    /// implementer run starts.
    NotReproducible {
        /// The terminal status recorded for the run. Always
        /// [`TerminalStatus::NotReproducible`].
        status: TerminalStatus,
        /// The observed evidence the bug does not reproduce.
        evidence: String,
    },
    /// The reproduction check could not conclude. The run is blocked on it
    /// — never terminated `NOT-REPRODUCIBLE` — and the issue may be
    /// retried once the check can run.
    Blocked {
        /// Why the check could not conclude.
        reason: String,
    },
}

impl PreflightDecision {
    /// Whether this decision authorizes producing a patch. Only a live
    /// reproduction does: a `NOT-REPRODUCIBLE` termination and a block never
    /// start an implementer run.
    pub fn produces_patch(&self) -> bool {
        matches!(self, PreflightDecision::Dispatch { .. })
    }
}

/// Decide the dispatch from the pre-implementation reproduction outcome.
///
/// Only an *observed* non-reproduction terminates the run
/// `NOT-REPRODUCIBLE`. An inconclusive check fails closed to a block:
/// terminating a run on a check that could not run would close live bugs.
pub fn preflight(outcome: &ReproOutcome) -> PreflightDecision {
    match outcome {
        ReproOutcome::Reproduced { evidence } => PreflightDecision::Dispatch {
            evidence: evidence.clone(),
        },
        ReproOutcome::NotReproduced { evidence } => PreflightDecision::NotReproducible {
            status: TerminalStatus::NotReproducible,
            evidence: evidence.clone(),
        },
        ReproOutcome::Inconclusive { reason } => PreflightDecision::Blocked {
            reason: reason.clone(),
        },
    }
}

/// The changed tests' results on the pre-change (unmodified) tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineRun {
    /// The baseline ran the changed tests: how many passed and failed.
    Ran { passed: usize, failed: usize },
    /// The changed tests could not be run on the pre-change tree.
    NotRun,
}

/// The shape of a patch as the acceptance gate sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PatchShape {
    /// Production (non-test) files the patch changes.
    pub production_files: Vec<String>,
    /// Test files the patch changes.
    pub test_files: Vec<String>,
}

impl PatchShape {
    /// A patch that changes nothing.
    pub fn is_empty(&self) -> bool {
        self.production_files.is_empty() && self.test_files.is_empty()
    }

    /// A patch whose only change is tests.
    pub fn is_test_only(&self) -> bool {
        !self.test_files.is_empty() && self.production_files.is_empty()
    }
}

/// The acceptance gate's decision on one patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceptance {
    /// The gate accepts: the patch changes production code (other gates
    /// judge it), or it is test-only and its changed tests fail on the
    /// pre-change tree — they reproduce the bug, and since the patch
    /// changes no production code they fail on the patched tree too, where
    /// the full-suite gate holds them.
    Accept,
    /// Rejected: the patch's only change is a test, and every changed test
    /// already passes on the pre-change tree. The patch codifies behavior
    /// the unmodified code already has — the signature of a stale bug
    /// report being "fixed" by a test.
    RejectTestOnlyPassingPreChange,
    /// Rejected: the patch is test-only and its changed tests could not be
    /// run or were not found on the pre-change tree. The gate cannot rule
    /// out the stale-report signature and fails closed.
    RejectTestOnlyBaselineUnknown,
}

impl Acceptance {
    /// `true` when the gate accepted the patch.
    pub fn is_accepted(self) -> bool {
        self == Acceptance::Accept
    }
}

/// The acceptance gate's check for the stale-report signature (#4072):
/// reject a patch whose only change is a test that passes on the
/// pre-change tree.
pub fn acceptance_check(shape: &PatchShape, baseline: &BaselineRun) -> Acceptance {
    if !shape.is_test_only() {
        return Acceptance::Accept;
    }
    match baseline {
        BaselineRun::Ran { passed, failed } if *failed == 0 && *passed > 0 => {
            Acceptance::RejectTestOnlyPassingPreChange
        }
        BaselineRun::Ran { passed: _, failed } if *failed > 0 => Acceptance::Accept,
        // A test-only patch whose changed tests neither ran nor were found
        // on the baseline (including zero tests found): the gate cannot
        // rule out the stale-report signature and fails closed.
        _ => Acceptance::RejectTestOnlyBaselineUnknown,
    }
}

/// An open issue as reconciliation sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenIssue {
    /// The issue number.
    pub issue: u64,
    /// Whether delivered (merged) work references this issue — a PR body,
    /// a closing keyword, or a branch name. This is the signal the old
    /// reconciler trusted on its own.
    pub referenced_by_delivered_work: bool,
    /// The result of running the issue's described reproduction on the
    /// current unmodified tree, when one has been run.
    pub repro: Option<ReproOutcome>,
}

/// Why an issue could not be reconciled to a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// No reproduction check has been run for this issue.
    NoReproCheck,
    /// The reproduction check ran but could not conclude.
    InconclusiveRepro {
        /// Why the check could not conclude.
        reason: String,
    },
}

/// The reconciler's verdict for one open issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciledIssue {
    /// The issue's described reproduction no longer reproduces on the
    /// current unmodified tree: a close candidate, routed to close-out
    /// review — whether or not any delivered work references the issue.
    Stale {
        /// The issue number.
        issue: u64,
        /// The observed evidence the reproduction no longer reproduces.
        evidence: String,
        /// Whether delivered work also references the issue. Reported,
        /// never required: the close candidate stands on the reproduction
        /// evidence alone.
        referenced_by_delivered_work: bool,
    },
    /// Delivered work references the issue but the described reproduction
    /// still reproduces: the claimed fix is not in effect. A verification
    /// gap, not a close candidate.
    VerificationGap {
        /// The issue number.
        issue: u64,
        /// The observed evidence the bug still reproduces.
        evidence: String,
    },
    /// No evidence to close or to confirm: the reproduction was not run or
    /// could not conclude. A reference from delivered work is not closure
    /// evidence on its own.
    Unresolved {
        /// The issue number.
        issue: u64,
        /// Why the issue could not be reconciled.
        reason: UnresolvedReason,
    },
    /// The bug still reproduces and no delivered work claims the issue: it
    /// stays open and dispatchable.
    Open {
        /// The issue number.
        issue: u64,
    },
}

impl ReconciledIssue {
    /// The issue number this verdict is about.
    pub fn issue(&self) -> u64 {
        match self {
            ReconciledIssue::Stale { issue, .. }
            | ReconciledIssue::VerificationGap { issue, .. }
            | ReconciledIssue::Unresolved { issue, .. }
            | ReconciledIssue::Open { issue } => *issue,
        }
    }

    /// Whether this verdict makes the issue a close candidate.
    pub fn is_close_candidate(&self) -> bool {
        matches!(self, ReconciledIssue::Stale { .. })
    }
}

/// Reconcile open issues with delivered work.
///
/// A close candidate is decided by reproduction evidence, never by naming:
/// an issue whose described reproduction no longer reproduces is flagged
/// with or without a PR referencing it, and an issue that a PR references
/// but whose reproduction has not been checked is [`ReconciledIssue::Unresolved`],
/// not closed. Issues are returned in ascending issue-number order.
pub fn reconcile(issues: &[OpenIssue]) -> Vec<ReconciledIssue> {
    let mut verdicts: Vec<ReconciledIssue> = issues.iter().map(reconcile_one).collect();
    verdicts.sort_by_key(ReconciledIssue::issue);
    verdicts
}

fn reconcile_one(issue: &OpenIssue) -> ReconciledIssue {
    match &issue.repro {
        Some(ReproOutcome::NotReproduced { evidence }) => ReconciledIssue::Stale {
            issue: issue.issue,
            evidence: evidence.clone(),
            referenced_by_delivered_work: issue.referenced_by_delivered_work,
        },
        Some(ReproOutcome::Reproduced { evidence }) => {
            if issue.referenced_by_delivered_work {
                ReconciledIssue::VerificationGap {
                    issue: issue.issue,
                    evidence: evidence.clone(),
                }
            } else {
                ReconciledIssue::Open { issue: issue.issue }
            }
        }
        Some(ReproOutcome::Inconclusive { reason }) => ReconciledIssue::Unresolved {
            issue: issue.issue,
            reason: UnresolvedReason::InconclusiveRepro {
                reason: reason.clone(),
            },
        },
        None => ReconciledIssue::Unresolved {
            issue: issue.issue,
            reason: UnresolvedReason::NoReproCheck,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AC1: pre-implementation reproduction check ──────────────────────

    #[test]
    fn reproducible_bug_dispatches_with_evidence() {
        let outcome =
            ReproOutcome::reproduced("test parser::roundtrip failed on unmodified main").unwrap();
        let decision = preflight(&outcome);
        assert_eq!(
            decision,
            PreflightDecision::Dispatch {
                evidence: "test parser::roundtrip failed on unmodified main".to_string()
            }
        );
        assert!(decision.produces_patch());
    }

    #[test]
    fn non_reproducible_report_terminates_not_reproducible_without_patch() {
        let outcome =
            ReproOutcome::not_reproduced("repro command exits 0 on unmodified main").unwrap();
        let decision = preflight(&outcome);
        assert_eq!(
            decision,
            PreflightDecision::NotReproducible {
                status: TerminalStatus::NotReproducible,
                evidence: "repro command exits 0 on unmodified main".to_string(),
            }
        );
        assert!(!decision.produces_patch());
    }

    #[test]
    fn inconclusive_check_blocks_instead_of_terminating() {
        let outcome =
            ReproOutcome::inconclusive("repro input file missing from the issue").unwrap();
        let decision = preflight(&outcome);
        assert_eq!(
            decision,
            PreflightDecision::Blocked {
                reason: "repro input file missing from the issue".to_string(),
            }
        );
        assert!(!decision.produces_patch());
        // "Cannot check" is never "does not reproduce".
        assert!(!matches!(
            decision,
            PreflightDecision::NotReproducible { .. }
        ));
    }

    #[test]
    fn blank_evidence_is_rejected() {
        assert!(ReproOutcome::reproduced("   ").is_err());
        assert!(ReproOutcome::not_reproduced("").is_err());
        assert!(ReproOutcome::inconclusive(" ").is_err());
    }

    // ── AC2: first-class terminal status, routed to close-out review ────

    #[test]
    fn terminal_status_tokens_round_trip() {
        for status in [
            TerminalStatus::Verified,
            TerminalStatus::NewTestFailures,
            TerminalStatus::NotReproducible,
        ] {
            let token = status.as_str();
            assert_eq!(TerminalStatus::parse(token).unwrap(), status);
        }
        assert_eq!(TerminalStatus::NotReproducible.as_str(), "NOT-REPRODUCIBLE");
    }

    #[test]
    fn unknown_terminal_status_token_is_an_error() {
        let err = TerminalStatus::parse("MAYBE").unwrap_err();
        assert!(err.contains("`MAYBE`"), "{err}");
    }

    #[test]
    fn not_reproducible_routes_to_close_review_not_the_dispatch_pool() {
        assert_eq!(
            route(TerminalStatus::NotReproducible),
            TerminalRoute::CloseReview
        );
    }

    #[test]
    fn verified_routes_to_close_review_and_failed_run_redispatches() {
        assert_eq!(route(TerminalStatus::Verified), TerminalRoute::CloseReview);
        assert_eq!(
            route(TerminalStatus::NewTestFailures),
            TerminalRoute::Redispatch
        );
    }

    // ── AC3: acceptance gate rejects test-only patches passing pre-change ─

    fn test_only_patch() -> PatchShape {
        PatchShape {
            production_files: vec![],
            test_files: vec!["tests/parser_roundtrip.rs".to_string()],
        }
    }

    #[test]
    fn test_only_patch_passing_pre_change_is_rejected() {
        let shape = test_only_patch();
        assert!(shape.is_test_only());
        let acceptance = acceptance_check(
            &shape,
            &BaselineRun::Ran {
                passed: 36,
                failed: 0,
            },
        );
        assert_eq!(acceptance, Acceptance::RejectTestOnlyPassingPreChange);
        assert!(!acceptance.is_accepted());
    }

    #[test]
    fn test_only_patch_failing_pre_change_is_left_to_the_suite_gate() {
        let shape = test_only_patch();
        let acceptance = acceptance_check(
            &shape,
            &BaselineRun::Ran {
                passed: 1,
                failed: 2,
            },
        );
        assert_eq!(acceptance, Acceptance::Accept);
    }

    #[test]
    fn test_only_patch_with_unrunnable_baseline_fails_closed() {
        let shape = test_only_patch();
        assert_eq!(
            acceptance_check(&shape, &BaselineRun::NotRun),
            Acceptance::RejectTestOnlyBaselineUnknown
        );
        assert_eq!(
            acceptance_check(
                &shape,
                &BaselineRun::Ran {
                    passed: 0,
                    failed: 0
                }
            ),
            Acceptance::RejectTestOnlyBaselineUnknown
        );
    }

    #[test]
    fn patch_with_production_changes_is_out_of_this_gates_scope() {
        let shape = PatchShape {
            production_files: vec!["crates/autospec-core/src/lib.rs".to_string()],
            test_files: vec!["tests/parser_roundtrip.rs".to_string()],
        };
        assert!(!shape.is_test_only());
        let acceptance = acceptance_check(
            &shape,
            &BaselineRun::Ran {
                passed: 36,
                failed: 0,
            },
        );
        assert_eq!(acceptance, Acceptance::Accept);
    }

    // ── AC4: reconciliation on reproduction evidence, not on naming ─────

    #[test]
    fn stale_report_without_any_pr_reference_is_a_close_candidate() {
        let issue = OpenIssue {
            issue: 24,
            referenced_by_delivered_work: false,
            repro: Some(
                ReproOutcome::not_reproduced("acceptance suite 36/0 on unmodified main").unwrap(),
            ),
        };
        let verdicts = reconcile(&[issue]);
        assert_eq!(
            verdicts,
            vec![ReconciledIssue::Stale {
                issue: 24,
                evidence: "acceptance suite 36/0 on unmodified main".to_string(),
                referenced_by_delivered_work: false,
            }]
        );
        assert!(verdicts[0].is_close_candidate());
    }

    #[test]
    fn stale_report_with_pr_reference_is_still_a_close_candidate() {
        let issue = OpenIssue {
            issue: 31,
            referenced_by_delivered_work: true,
            repro: Some(
                ReproOutcome::not_reproduced("repro command exits 0 on unmodified main").unwrap(),
            ),
        };
        let verdicts = reconcile(&[issue]);
        assert_eq!(
            verdicts,
            vec![ReconciledIssue::Stale {
                issue: 31,
                evidence: "repro command exits 0 on unmodified main".to_string(),
                referenced_by_delivered_work: true,
            }]
        );
    }

    #[test]
    fn pr_reference_alone_never_closes_an_issue() {
        let issue = OpenIssue {
            issue: 42,
            referenced_by_delivered_work: true,
            repro: None,
        };
        let verdicts = reconcile(&[issue]);
        assert_eq!(
            verdicts,
            vec![ReconciledIssue::Unresolved {
                issue: 42,
                reason: UnresolvedReason::NoReproCheck,
            }]
        );
        assert!(!verdicts[0].is_close_candidate());
    }

    #[test]
    fn referenced_issue_that_still_reproduces_is_a_verification_gap() {
        let issue = OpenIssue {
            issue: 55,
            referenced_by_delivered_work: true,
            repro: Some(
                ReproOutcome::reproduced("repro command still fails on unmodified main").unwrap(),
            ),
        };
        let verdicts = reconcile(&[issue]);
        assert_eq!(
            verdicts,
            vec![ReconciledIssue::VerificationGap {
                issue: 55,
                evidence: "repro command still fails on unmodified main".to_string(),
            }]
        );
        assert!(!verdicts[0].is_close_candidate());
    }

    #[test]
    fn unreferenced_issue_that_still_reproduces_stays_open() {
        let issue = OpenIssue {
            issue: 66,
            referenced_by_delivered_work: false,
            repro: Some(ReproOutcome::reproduced("repro fails").unwrap()),
        };
        let verdicts = reconcile(&[issue]);
        assert_eq!(verdicts, vec![ReconciledIssue::Open { issue: 66 }]);
    }

    #[test]
    fn inconclusive_repro_is_unresolved() {
        let issue = OpenIssue {
            issue: 77,
            referenced_by_delivered_work: false,
            repro: Some(ReproOutcome::inconclusive("environment unavailable").unwrap()),
        };
        let verdicts = reconcile(&[issue]);
        assert_eq!(
            verdicts,
            vec![ReconciledIssue::Unresolved {
                issue: 77,
                reason: UnresolvedReason::InconclusiveRepro {
                    reason: "environment unavailable".to_string(),
                },
            }]
        );
    }

    #[test]
    fn reconcile_output_is_in_ascending_issue_order() {
        let issues = vec![
            OpenIssue {
                issue: 9,
                referenced_by_delivered_work: false,
                repro: Some(ReproOutcome::reproduced("fails").unwrap()),
            },
            OpenIssue {
                issue: 3,
                referenced_by_delivered_work: false,
                repro: None,
            },
            OpenIssue {
                issue: 5,
                referenced_by_delivered_work: false,
                repro: Some(ReproOutcome::not_reproduced("passes").unwrap()),
            },
        ];
        let verdicts = reconcile(&issues);
        assert_eq!(
            verdicts
                .iter()
                .map(ReconciledIssue::issue)
                .collect::<Vec<_>>(),
            vec![3, 5, 9]
        );
    }
}
