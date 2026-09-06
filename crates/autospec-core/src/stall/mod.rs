//! Stall handling: lease, liveness, partial work, rotation, escalation.
//!
//! The pipeline this serves reaped implementer subprocesses that produced no
//! output and re-queued the issue indefinitely. Four separate mistakes were in
//! that, and each has a place in this module:
//!
//! * **A claim was a label.** Nothing recorded who held an issue or for how
//!   long, so a dead run left it claimed, or a live one was treated as dead.
//!   A claim is now a [`lease::IssueLease`] with an expiry, renewed only by
//!   progress the supervisor observes from outside the child.
//! * **Output was the liveness signal.** A reading agent produces no diff for
//!   minutes at a time, which is correct behaviour on a design-heavy issue.
//!   [`liveness`] watches transcript growth alongside output growth.
//! * **Partial work was never captured, and commits were invisible.** The
//!   scratch directory went at job end, and the check that should have saved
//!   the work looked only at the working tree, so it reported a *committing*
//!   agent as having produced nothing. [`partial_work`] captures commits, the
//!   working tree, and the transcript tail before anything is torn down.
//! * **Retries were uncounted and identical.** Retrying a stall on the model
//!   that just stalled re-learns nothing. [`attempts`] keeps the history and
//!   rotates architecture families; [`release`] stops at the attempt limit and
//!   hands the case to spec repair, because a stall that survives rotation is
//!   evidence about the spec.
//!
//! Everything here is a pure decision over data the caller supplies, plus a
//! narrow [`tracker::IssueTracker`] interface, so the reasoning is testable
//! without a GitHub, a worktree, or an agent.

pub mod attempts;
pub mod lease;
pub mod liveness;
pub mod note;
pub mod partial_work;
pub mod release;
pub mod tracker;

use std::path::PathBuf;

pub use attempts::{
    AttemptHistory, AttemptOutcome, AttemptRecord, ModelChoice, ModelRoster, Rotation,
};
pub use lease::{progress_revision, IssueLease, LeasePolicy, Renewal};
pub use liveness::{Liveness, LivenessMonitor, LivenessSample};
pub use note::{SpecRepairReport, StallNote};
pub use partial_work::{
    capture_partial_work, classify_work, read_tail, Artifact, ArtifactStore, CommitRecord,
    GitWorktreeEvidence, PartialWork, WorkProduced, WorktreeEvidence,
};
pub use release::{
    plan_release, EscalationReason, ReleaseDecision, ReleasePlan, StallPolicy, StallReason,
    LABEL_ATTEMPTS_EXHAUSTED, LABEL_SPEC_REPAIR,
};
pub use tracker::{IssueRef, IssueTracker, TrackerError};

/// One ended attempt, handed to the release path while its worktree still exists.
pub struct FinishedAttempt<'a> {
    /// The attempt as it ran: model, configuration, worker, duration, last activity.
    pub record: AttemptRecord,
    /// Why it ended.
    pub reason: StallReason,
    /// What the liveness signals said at the end.
    pub liveness: Liveness,
    /// Base revision the worktree diff is measured against.
    pub base: String,
    /// Where to read commits, working tree, and transcript from.
    pub evidence: &'a dyn WorktreeEvidence,
}

/// What one finished attempt produced, and what the tracker was told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseResult {
    /// The decision taken, with the note and artifacts recorded with it.
    pub plan: ReleasePlan,
    /// The attempt counter the tracker returned (0 when nothing was re-queued).
    pub attempt_counter: u32,
    /// Where captured work landed on disk, for the next attempt to read.
    pub stored_artifacts: Vec<PathBuf>,
    /// Attachments the tracker refused. Local copies still exist; this never
    /// silently drops evidence.
    pub attachment_errors: Vec<String>,
}

/// The release path, wired to one tracker, one artifact store, and one roster.
pub struct StallRelease<'a> {
    tracker: &'a mut dyn IssueTracker,
    store: ArtifactStore,
    policy: StallPolicy,
    roster: ModelRoster,
}

impl<'a> StallRelease<'a> {
    pub fn new(
        tracker: &'a mut dyn IssueTracker,
        store: ArtifactStore,
        policy: StallPolicy,
        roster: ModelRoster,
    ) -> Self {
        Self {
            tracker,
            store,
            policy,
            roster,
        }
    }

    pub fn policy(&self) -> &StallPolicy {
        &self.policy
    }

    pub fn roster(&self) -> &ModelRoster {
        &self.roster
    }

    pub fn store(&self) -> &ArtifactStore {
        &self.store
    }

    /// Take a lease for a new attempt.
    pub fn lease(
        &self,
        issue: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        model: impl Into<String>,
        at: u64,
    ) -> Result<IssueLease, String> {
        IssueLease::take(issue, worker_id, branch, model, at, self.policy.lease)
    }

    /// Finish one attempt: capture first, record, then decide and act.
    ///
    /// Capture happens before any tracker call so that teardown of the worktree
    /// can never race the evidence out of existence, and so a tracker that is
    /// unreachable still leaves the work preserved in the artifact store.
    pub fn finish(
        &mut self,
        issue: &IssueRef,
        history: &mut AttemptHistory,
        attempt: FinishedAttempt<'_>,
    ) -> Result<ReleaseResult, TrackerError> {
        let FinishedAttempt {
            mut record,
            reason,
            liveness,
            base,
            evidence,
        } = attempt;

        // 1. Capture the partial work while the worktree is still there.
        let work: PartialWork =
            partial_work::capture_partial_work(evidence, &base, self.policy.transcript_tail_bytes);
        record.produced = work.work_produced();

        // 2. Number the attempt against the issue's history.
        let record = history.push(record);
        let attempt_number = record.attempt;

        // 3. Decide.
        let mut plan = plan_release(
            issue.number,
            &self.policy,
            &self.roster,
            history,
            &work,
            reason,
            liveness,
        );

        if let ReleaseDecision::Completed { .. } = plan.decision {
            return Ok(ReleaseResult {
                plan,
                attempt_counter: 0,
                stored_artifacts: Vec::new(),
                attachment_errors: Vec::new(),
            });
        }

        // The stall note travels with the artifacts so the reason survives
        // whatever happens to the comment thread.
        if let Some(note) = plan.note.clone() {
            plan.artifacts.push(Artifact {
                name: format!("attempt-{attempt_number}-stall-note.md"),
                body: note.markdown(),
            });
        }

        // 4. Store locally before the tracker sees anything: the next attempt
        //    reads from here even if the tracker call below fails.
        let mut stored = Vec::new();
        for artifact in &plan.artifacts {
            let path = self
                .store
                .write(issue.number, attempt_number, artifact)
                .map_err(|error| {
                    TrackerError::new(format!(
                        "could not store {} for issue #{}: {error}",
                        artifact.name, issue.number
                    ))
                })?;
            stored.push(path);
        }
        // Report paths from the store, not from the artifact names.
        if let ReleaseDecision::Escalate { report, .. } = &mut plan.decision {
            report.artifact_paths = stored.clone();
        }

        // 5. Attach, count, and hand off.
        let mut attachment_errors = Vec::new();
        for artifact in &plan.artifacts {
            if let Some(body) = self.attach_best_effort(issue, artifact) {
                attachment_errors.push(body);
            }
        }

        let attempt_counter = self.tracker.bump_attempt_counter(issue)?;

        match &plan.decision {
            ReleaseDecision::Requeue { rotation_note, .. } => {
                // The note already carries the next step; the rotation note is
                // the fallback when a caller built a plan without one.
                let body = plan
                    .note_markdown()
                    .unwrap_or_else(|| rotation_note.clone());
                self.tracker.release_to_queue(issue, &body)?;
            }
            ReleaseDecision::Escalate { report, .. } => {
                self.tracker.add_label(issue, LABEL_ATTEMPTS_EXHAUSTED)?;
                self.tracker.add_label(issue, LABEL_SPEC_REPAIR)?;
                self.tracker.escalate_to_spec_repair(issue, report)?;
            }
            ReleaseDecision::Completed { .. } => {}
        }

        Ok(ReleaseResult {
            plan,
            attempt_counter,
            stored_artifacts: stored,
            attachment_errors,
        })
    }

    /// Attach one artifact, reporting a refusal instead of losing the evidence.
    fn attach_best_effort(&mut self, issue: &IssueRef, artifact: &Artifact) -> Option<String> {
        self.tracker
            .attach(issue, artifact)
            .err()
            .map(|error| format!("{}: {error}", artifact.name))
    }
}
