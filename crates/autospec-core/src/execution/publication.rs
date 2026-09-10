//! Verify-and-publish design (#3788).
//!
//! **Design decision: production and publication are the same component.**
//! The agent runs the real (merge) gate — with the fixtures it provisions
//! itself — and, on green, pushes the branch and opens the pull request.
//! There is no queue between producing a patch and opening its PR, and
//! therefore none of the defect classes a queue exists to schedule: no
//! worklist ordering policy, no snapshot staleness, no stored artifact to
//! become stale state, no second verification, and no window in which the
//! base moves underneath finished work.
//!
//! **The constraint that forced the split, and its narrow solution.** The
//! only reason the pipeline ever split production from publication was
//! credential placement: agents run where no push credential to the origin
//! repository may exist, while the converter ran on a host where `gh` was
//! already authenticated. That constraint is met by a *narrower* solution
//! than a queue: a per-run, push-only credential scoped to exactly one
//! repository ([`PushCredential`], [`validate_credential`]) — a push-only
//! deploy key, or a fine-grained token limited to `contents:write` and
//! `pull_requests:write` — delivered per run and never written to shared
//! storage. The broad solution (a long-lived local converter plus a queue
//! between the agent and its PR) is rejected because every measured
//! failure mode of that pipeline was a property of the queue, not of the
//! work.
//!
//! **Verification runs once.** A verdict produced by the agent under the
//! real gate is not re-derived downstream ([`check_verdict`]). The only
//! downstream check is that the base has not moved; when it has, the
//! producer — which still has the context and budget to fix it — re-runs
//! the real gate, and no downstream process re-derives the verdict on a
//! weaker or different gate set ([`under_real_gate`]).
//!
//! **Where the split is retained in transition**, the queue between the
//! stages is a first-class component with a stated scheduling policy, a
//! staleness bound, and a definition of what state is authoritative —
//! [`QueueContract`], [`is_stale`], [`plan_retry`] — not an emergent
//! property of a directory.

use std::collections::BTreeSet;
use std::time::Duration;

// ── 1. The narrow credential solution ─────────────────────────────────────

/// The kind of push-only credential delivered to the agent per run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// A deploy key with push only, scoped to exactly one repository.
    PushOnlyDeployKey,
    /// A fine-grained token limited to `contents:write` and
    /// `pull_requests:write` on exactly one repository.
    FineGrainedToken,
}

/// A push credential for one run of the verify-and-publish agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushCredential {
    /// Which form the per-run credential takes.
    pub kind: CredentialKind,
    /// The single repository this credential is scoped to. Must be
    /// non-empty: an unscoped credential is the broad scope the queue
    /// existed to keep away from the agent, and the design rejects it.
    pub repository: String,
    /// `contents:write` — needed to push the branch.
    pub contents_write: bool,
    /// `pull_requests:write` — needed to open the PR.
    pub pull_requests_write: bool,
    /// Admin on the repository. Must be false: admin is not needed to push
    /// a branch and open a PR, and it is exactly the scope a queue-based
    /// design would otherwise have to protect against.
    pub admin: bool,
    /// True when the credential outlives the run — persisted to shared
    /// storage or reused across runs. Must be false: the credential is
    /// delivered per run and never written to shared storage.
    pub outlives_run: bool,
}

/// Why a credential does not meet the narrow-solution bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialViolation {
    /// The credential carries admin on the repository.
    AdminScope,
    /// The credential outlives its run, i.e. it would be written to or kept
    /// on shared storage.
    OutlivesRun,
    /// The credential is not scoped to exactly one repository.
    Unscoped,
    /// The credential cannot complete both halves of publication (push the
    /// branch *and* open the PR).
    InsufficientForPublication,
}

/// Check that a credential is the narrow solution: push-only, per-run,
/// scoped to one repository, and sufficient to publish. Checks run in
/// severity order: the violations that a queue-based design existed to
/// prevent come first.
pub fn validate_credential(credential: &PushCredential) -> Result<(), CredentialViolation> {
    if credential.admin {
        return Err(CredentialViolation::AdminScope);
    }
    if credential.outlives_run {
        return Err(CredentialViolation::OutlivesRun);
    }
    if credential.repository.trim().is_empty() {
        return Err(CredentialViolation::Unscoped);
    }
    if !credential.contents_write || !credential.pull_requests_write {
        return Err(CredentialViolation::InsufficientForPublication);
    }
    Ok(())
}

// ── 2. Verification runs once ─────────────────────────────────────────────

/// The agent's verdict: green under a named gate set, against a named base
/// sha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// The gate set the agent ran. Must be non-empty and, for the verdict
    /// to stand, equal to the real (merge) gate set — see
    /// [`under_real_gate`].
    pub gate_set: Vec<String>,
    /// The base sha the verdict was verified against. Must be non-empty; it
    /// is the only fact the downstream check consults.
    pub base_sha: String,
}

impl Verdict {
    /// Construct a verdict, rejecting an empty gate set or base sha: a
    /// verdict that names no gates proves nothing, and one that names no
    /// base cannot be confirmed against anything.
    pub fn new(gate_set: Vec<String>, base_sha: impl Into<String>) -> Result<Self, String> {
        let base_sha = base_sha.into();
        if gate_set.is_empty() {
            return Err("a verdict must name the gate set it ran".to_string());
        }
        if base_sha.trim().is_empty() {
            return Err("verdict base sha must not be empty".to_string());
        }
        Ok(Self { gate_set, base_sha })
    }
}

/// A verdict produced under a gate set that is not the real (merge) gate
/// set is not a verdict: it re-creates the two-verifiers, two-gate-sets
/// defect, where the agent proves less than the merge gate demands and the
/// gap is found — if at all — hours later by a human.
///
/// Comparison is set equality: order and duplicates do not matter, but a
/// weaker (subset) or wider (superset) gate set does not stand in for the
/// real gate.
pub fn under_real_gate(verdict: &Verdict, merge_gate_set: &[String]) -> bool {
    let verdict_gates: BTreeSet<&str> = verdict.gate_set.iter().map(String::as_str).collect();
    let merge_gates: BTreeSet<&str> = merge_gate_set.iter().map(String::as_str).collect();
    verdict_gates == merge_gates
}

/// The only check downstream of a green verdict: has the base moved?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictCheck {
    /// The base has not moved. The verdict stands and the agent publishes.
    /// No gate is re-run: re-deriving what the agent already proved is the
    /// double-verification the design removes.
    BaseUnchanged,
    /// The base moved. The verdict no longer stands; the producer re-runs
    /// the real gate against the current base, while it still has the
    /// context and budget to fix it. Nothing is re-derived downstream.
    BaseMoved,
}

/// Confirm the base has not moved. This is the sole downstream use of a
/// verdict: the verdict is never re-derived, only confirmed.
pub fn check_verdict(verdict: &Verdict, current_base_sha: &str) -> VerdictCheck {
    if verdict.base_sha == current_base_sha {
        VerdictCheck::BaseUnchanged
    } else {
        VerdictCheck::BaseMoved
    }
}

// ── 3. The queue, where the split is retained in transition ───────────────

/// The queue between production and publication, as a first-class
/// component with a stated contract — not an emergent property of a
/// directory.
///
/// Scheduling policy: cost order — terminal, cheap cases first, compute
/// only on genuine candidates, as encoded by
/// [`crate::execution::patch_pipeline::order_by_cost`], with the pass's
/// stable walk order breaking ties inside a cost class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueContract {
    /// Staleness bound: the maximum age a patch's verdict may reach in the
    /// queue before it must be re-verified by the producer. Beyond the
    /// bound the base has presumptively moved, and only the producer can
    /// re-establish the verdict.
    pub staleness_bound: Duration,
}

impl QueueContract {
    /// Create a queue contract. A zero bound is a configuration error, not
    /// a policy: it would make every verdict stale the instant it arrives
    /// and turn the queue into a re-verification farm, which is the
    /// double-verification the design removes.
    pub fn new(staleness_bound: Duration) -> Result<Self, String> {
        if staleness_bound.is_zero() {
            return Err(
                "queue staleness bound must not be zero: every verdict would be stale on arrival"
                    .to_string(),
            );
        }
        Ok(Self { staleness_bound })
    }
}

/// Whether a verdict has reached the queue's staleness bound. A verdict is
/// stale at the bound, not after it: at age == bound the base has
/// presumptively moved.
pub fn is_stale(contract: &QueueContract, verdict_age: Duration) -> bool {
    verdict_age >= contract.staleness_bound
}

/// The latest record for a patch in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunRecord {
    /// A finished or failed run left an artifact behind. The artifact is
    /// evidence, not state: it never gates a retry. Blocking a retry on
    /// artifact existence is what turned one failed run into a permanent
    /// hold.
    pub artifact_present: bool,
    /// How long the latest verdict has sat in the queue.
    pub verdict_age: Duration,
}

/// What the queue does with a patch whose latest run failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPlan {
    /// Re-run the work under the real gate right away. The failed run's
    /// artifact does not block this: the verdict record is the only state
    /// consulted.
    Retry,
    /// The latest verdict is stale beyond the contract's bound: the
    /// producer re-verifies against the current base before anything is
    /// converted. The old verdict is discarded, never reused.
    RetryAfterReverify,
}

/// Plan the retry for a failed run. The decision consults the verdict
/// record (its age), never the artifact: `artifact_present` deliberately
/// does not change the outcome, because a failed run's artifact is not
/// authoritative state.
pub fn plan_retry(contract: &QueueContract, record: &RunRecord) -> RetryPlan {
    if is_stale(contract, record.verdict_age) {
        RetryPlan::RetryAfterReverify
    } else {
        RetryPlan::Retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(kind: CredentialKind, contents: bool, pull_requests: bool) -> PushCredential {
        PushCredential {
            kind,
            repository: "org/repo".to_string(),
            contents_write: contents,
            pull_requests_write: pull_requests,
            admin: false,
            outlives_run: false,
        }
    }

    #[test]
    fn accepts_a_per_run_push_only_credential_scoped_to_one_repo() {
        assert!(
            validate_credential(&credential(CredentialKind::PushOnlyDeployKey, true, true)).is_ok()
        );
        assert!(
            validate_credential(&credential(CredentialKind::FineGrainedToken, true, true)).is_ok()
        );
    }

    #[test]
    fn rejects_admin_scope() {
        let mut c = credential(CredentialKind::FineGrainedToken, true, true);
        c.admin = true;
        assert_eq!(
            validate_credential(&c),
            Err(CredentialViolation::AdminScope)
        );
    }

    #[test]
    fn rejects_a_credential_that_outlives_the_run() {
        let mut c = credential(CredentialKind::PushOnlyDeployKey, true, true);
        c.outlives_run = true;
        assert_eq!(
            validate_credential(&c),
            Err(CredentialViolation::OutlivesRun)
        );
    }

    #[test]
    fn rejects_an_unscoped_credential() {
        let mut c = credential(CredentialKind::FineGrainedToken, true, true);
        c.repository = "  ".to_string();
        assert_eq!(validate_credential(&c), Err(CredentialViolation::Unscoped));
    }

    #[test]
    fn rejects_a_credential_missing_either_half_of_publication() {
        // Can push but cannot open the PR.
        assert_eq!(
            validate_credential(&credential(CredentialKind::PushOnlyDeployKey, true, false)),
            Err(CredentialViolation::InsufficientForPublication)
        );
        // Can open the PR but cannot push the branch.
        assert_eq!(
            validate_credential(&credential(CredentialKind::FineGrainedToken, false, true)),
            Err(CredentialViolation::InsufficientForPublication)
        );
    }

    #[test]
    fn verdict_requires_a_gate_set_and_a_base_sha() {
        assert!(Verdict::new(vec![], "sha").is_err());
        assert!(Verdict::new(vec!["gate".to_string()], "  ").is_err());
        assert!(Verdict::new(vec!["gate".to_string()], "sha").is_ok());
    }

    #[test]
    fn verdict_must_be_under_the_real_gate() {
        let merge = vec![
            "compile".to_string(),
            "test".to_string(),
            "lint".to_string(),
        ];
        // Same set, different order: the real gate.
        let verdict = Verdict::new(
            vec![
                "lint".to_string(),
                "test".to_string(),
                "compile".to_string(),
            ],
            "sha",
        )
        .unwrap();
        assert!(under_real_gate(&verdict, &merge));
        // A weaker gate set is the #3786 defect: it is not a verdict.
        let weaker = Verdict::new(vec!["compile".to_string(), "test".to_string()], "sha").unwrap();
        assert!(!under_real_gate(&weaker, &merge));
        // A wider gate set does not stand in for the real gate either.
        let wider = Verdict::new(
            vec![
                "compile".to_string(),
                "test".to_string(),
                "lint".to_string(),
                "extra".to_string(),
            ],
            "sha",
        )
        .unwrap();
        assert!(!under_real_gate(&wider, &merge));
    }

    #[test]
    fn verdict_stands_while_the_base_has_not_moved() {
        let verdict = Verdict::new(vec!["gate".to_string()], "sha-1").unwrap();
        assert_eq!(
            check_verdict(&verdict, "sha-1"),
            VerdictCheck::BaseUnchanged
        );
    }

    #[test]
    fn verdict_falls_when_the_base_moves() {
        let verdict = Verdict::new(vec!["gate".to_string()], "sha-1").unwrap();
        assert_eq!(check_verdict(&verdict, "sha-2"), VerdictCheck::BaseMoved);
    }

    #[test]
    fn queue_contract_rejects_a_zero_staleness_bound() {
        assert!(QueueContract::new(Duration::from_secs(0)).is_err());
        assert!(QueueContract::new(Duration::from_secs(3600)).is_ok());
    }

    #[test]
    fn verdict_is_stale_at_and_beyond_the_bound() {
        let contract = QueueContract::new(Duration::from_secs(3600)).unwrap();
        assert!(!is_stale(&contract, Duration::from_secs(3599)));
        assert!(is_stale(&contract, Duration::from_secs(3600)));
        assert!(is_stale(&contract, Duration::from_secs(7200)));
    }

    #[test]
    fn a_failed_runs_artifact_never_blocks_its_own_retry() {
        let contract = QueueContract::new(Duration::from_secs(3600)).unwrap();
        // The failed run left its artifact behind, but the verdict record
        // is the only state consulted and it is fresh: retry right away.
        let record = RunRecord {
            artifact_present: true,
            verdict_age: Duration::from_secs(60),
        };
        assert_eq!(plan_retry(&contract, &record), RetryPlan::Retry);
        // The same decision with no artifact: the artifact changes nothing.
        let record = RunRecord {
            artifact_present: false,
            verdict_age: Duration::from_secs(60),
        };
        assert_eq!(plan_retry(&contract, &record), RetryPlan::Retry);
    }

    #[test]
    fn a_stale_verdict_is_reverified_before_conversion() {
        let contract = QueueContract::new(Duration::from_secs(3600)).unwrap();
        let record = RunRecord {
            artifact_present: true,
            verdict_age: Duration::from_secs(7200),
        };
        assert_eq!(
            plan_retry(&contract, &record),
            RetryPlan::RetryAfterReverify
        );
    }
}
