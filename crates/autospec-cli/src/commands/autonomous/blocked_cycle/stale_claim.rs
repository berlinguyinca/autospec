//! Disposition of stale, un-merged bridge claims (issue #2995).
//!
//! A foreground run that pauses with `executor_receipt_failed` used to release
//! its bridge claim unconditionally. When the pause happened because the
//! executor died *after* pushing a branch and opening a pull request, that
//! release re-labelled the issue `auto-implement` while the pull request was
//! still open, so the next conductor generation claimed the issue again and
//! crashed on the same remote collision — a loop with no exit.
//!
//! The staleness floor is checked first: a claim whose `progress_at` is older
//! than [`STALE_CLAIM_FLOOR_ENV`] (default 24h) with no live descendant is
//! quarantined regardless of pull request state; inside the floor the observed
//! remote state decides. Quarantine releases the claim with
//! [`BridgeClaimDisposition::NeedsHuman`] — removing `in-progress-by-bot` and
//! `auto-implement`, adding `autospec:needs-human` — and never mutates the pull
//! request or the remote branch.

mod observe;

use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::autonomous::waterfall::sha256_hex;
use autospec_core::coordination::{ConductorEvent, ConductorState};

use super::super::executor_bridge::{
    executor_processes_quiescent, invocation_matches_lease, unix_now, validate_private_state_file,
    write_invocation_atomic, BridgePhase, PersistedInvocation,
};
use super::super::{
    clear_claim_acquisition_receipt, load_claim_acquisition_receipt, persist_foreground_state,
    CommandFailure, RunLayout,
};
use crate::commands::claim::{
    transition_bridge_claim, BridgeClaimDisposition, BridgeClaimTransition, ClaimLease,
    ClaimMutationIdentity,
};
use observe::{derive_facts, observe_branch_pull_requests};

/// Env var overriding the staleness floor, in seconds.
pub(crate) const STALE_CLAIM_FLOOR_ENV: &str = "AUTOSPEC_STALE_CLAIM_FLOOR_SECS";
/// One day: older than this, an un-merged claim with no live descendant goes.
pub(crate) const DEFAULT_STALE_CLAIM_FLOOR_SECS: u64 = 86_400;
/// Pause/quarantine reason recorded for a contained stale claim.
pub(crate) const QUARANTINE_REASON: &str = "stale_unmerged_claim";
/// Reason recorded when a stale claim has no remote trace left to collide with.
pub(crate) const TERMINAL_RELEASE_REASON: &str = "stale_claim_terminal_release";

/// Staleness floor in seconds; a zero or unparsable value falls back to 24h.
pub(crate) fn stale_claim_floor_secs() -> u64 {
    std::env::var(STALE_CLAIM_FLOOR_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_STALE_CLAIM_FLOOR_SECS)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StaleClaimDisposition {
    /// Leave the claim alone; the receipt failure is recoverable or live work owns it.
    Resume,
    /// Release to a human: open pull request, broken invariant, or floor exceeded.
    Quarantine,
    /// Release back to the queue: the claim is terminal and nothing remote remains.
    TerminalRelease,
}

impl StaleClaimDisposition {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Resume => "stale_claim_resume",
            Self::Quarantine => QUARANTINE_REASON,
            Self::TerminalRelease => TERMINAL_RELEASE_REASON,
        }
    }

    /// The claim-ref transition for this disposition, `None` for [`Self::Resume`].
    pub(crate) fn claim_disposition(self) -> Option<BridgeClaimDisposition> {
        match self {
            Self::Resume => None,
            Self::Quarantine => Some(BridgeClaimDisposition::NeedsHuman),
            Self::TerminalRelease => Some(BridgeClaimDisposition::Retryable),
        }
    }

    fn terminal_tag(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Quarantine => "needs-human",
            Self::TerminalRelease => "retryable",
        }
    }
}

/// Everything observable about a stalled claim, gathered before deciding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StaleClaimFacts {
    /// Pull request number recorded in the durable invocation, if any.
    pub(crate) recorded_pr: Option<u64>,
    /// The remote pull request state could be read at all.
    pub(crate) pr_state_observed: bool,
    pub(crate) pr_open: bool,
    pub(crate) pr_merged: bool,
    /// Any pull request (any state) still points at the claimed branch.
    pub(crate) remote_branch_present: bool,
    /// Remote head still equals the head the invocation last proved.
    pub(crate) head_oid_matches: bool,
    /// The local worktree identity this claim owns is still usable.
    pub(crate) local_identity_intact: bool,
    /// Live executor descendants: `None` means it could not be proved either way.
    pub(crate) live_descendants: Option<u32>,
    pub(crate) progress_at: u64,
    pub(crate) observed_at: u64,
}

impl StaleClaimFacts {
    pub(crate) fn age_secs(&self) -> u64 {
        self.observed_at.saturating_sub(self.progress_at)
    }

    pub(crate) fn floor_reached(&self, floor_secs: u64) -> bool {
        self.age_secs() >= floor_secs
    }
}

/// Pure disposition decision. The floor is checked before pull request state so
/// an abandoned claim cannot stay pinned by a pull request nobody is driving.
pub(crate) fn classify_stale_claim(
    facts: &StaleClaimFacts,
    floor_secs: u64,
) -> StaleClaimDisposition {
    if facts.pr_merged {
        // The merged reconciliation path owns this claim, not us.
        return StaleClaimDisposition::Resume;
    }
    if facts.floor_reached(floor_secs) && facts.live_descendants.unwrap_or(0) == 0 {
        return StaleClaimDisposition::Quarantine;
    }
    if facts.live_descendants.unwrap_or(0) > 0 {
        // Never steal work a live executor is still driving.
        return StaleClaimDisposition::Resume;
    }
    if !facts.pr_state_observed {
        // Unreadable remote state is not evidence; wait for a readable one.
        return StaleClaimDisposition::Resume;
    }
    if !facts.pr_open {
        return if facts.remote_branch_present {
            StaleClaimDisposition::Quarantine
        } else {
            StaleClaimDisposition::TerminalRelease
        };
    }
    if !facts.head_oid_matches || !facts.local_identity_intact {
        // Open pull request but the branch/local identity diverged from the claim.
        return StaleClaimDisposition::Quarantine;
    }
    StaleClaimDisposition::Resume
}

/// Does the durable disposition logic apply to this invocation at all?
fn disposition_applies(invocation: &PersistedInvocation) -> bool {
    if invocation.phase == BridgePhase::Complete || invocation.terminal_result.is_some() {
        return false;
    }
    // Only a claim that already recorded a pull request can collide with one.
    // Pre-PR phases keep the legacy recovery route, and a pushed branch with no
    // pull request is a legitimate adoption path (#917), not a stale claim.
    invocation.pr.is_some()
        && matches!(
            invocation.phase,
            BridgePhase::BranchPushing
                | BridgePhase::BranchPushed
                | BridgePhase::DraftCreating
                | BridgePhase::DraftCleanupPending
                | BridgePhase::DraftCreated
                | BridgePhase::Ready
                | BridgePhase::CiPassed
                | BridgePhase::ReviewPassed
                | BridgePhase::ResultAccepted
                | BridgePhase::MergeRequested
                | BridgePhase::CleanupPending
        )
}

fn local_identity_intact(invocation: &PersistedInvocation) -> bool {
    if matches!(
        invocation.phase,
        BridgePhase::CleanupPending | BridgePhase::DraftCleanupPending
    ) {
        // The worktree was removed on purpose at this phase.
        return true;
    }
    fs::symlink_metadata(&invocation.identity.worktree)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
}

fn invocation_state_path(state_dir: &Path, lease: &ClaimLease) -> PathBuf {
    let generation = &sha256_hex(lease.claim_id.as_bytes())[..16];
    state_dir.join(format!("issue-{}-{generation}.json", lease.issue))
}

fn load_lease_invocation(
    state_dir: &Path,
    lease: &ClaimLease,
) -> Result<Option<PersistedInvocation>, String> {
    let state_path = invocation_state_path(state_dir, lease);
    if !state_path.is_file() {
        return Ok(None);
    }
    validate_private_state_file(&state_path)?;
    let invocation = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path)
            .map_err(|error| format!("read stale claim invocation: {error}"))?,
    )?;
    if !invocation_matches_lease(&invocation, lease) {
        return Err(
            "stale claim invocation does not match the durable local acquisition".to_string(),
        );
    }
    Ok(Some(invocation))
}

/// Observe the stalled claim. `Ok(None)` means there is nothing to dispose and
/// the caller must fall back to the legacy route.
pub(crate) fn observe_stale_claim_facts(
    state_dir: &Path,
    lease: &ClaimLease,
) -> Result<Option<StaleClaimFacts>, String> {
    let Some(invocation) = load_lease_invocation(state_dir, lease)? else {
        return Ok(None);
    };
    if !disposition_applies(&invocation) {
        return Ok(None);
    }
    let live_descendants = match executor_processes_quiescent(&invocation) {
        Ok(true) => Some(0),
        Ok(false) => Some(1),
        // Ambiguous liveness: unknown, never "proved dead" outside the floor.
        Err(_) => None,
    };
    let remote =
        observe_branch_pull_requests(&invocation.identity.repository, &invocation.identity.branch);
    Ok(Some(derive_facts(
        invocation.pr,
        invocation.head_oid.as_deref(),
        live_descendants,
        local_identity_intact(&invocation),
        invocation.progress_at,
        unix_now()?,
        remote,
    )))
}

/// Stamp the invocation terminal so a later generation cannot adopt it again.
pub(crate) fn stamp_terminal(
    state_dir: &Path,
    lease: &ClaimLease,
    disposition: StaleClaimDisposition,
) -> Result<(), String> {
    if disposition == StaleClaimDisposition::Resume {
        return Ok(());
    }
    let state_path = invocation_state_path(state_dir, lease);
    let Some(mut invocation) = load_lease_invocation(state_dir, lease)? else {
        return Ok(());
    };
    invocation.phase = BridgePhase::Complete;
    invocation.terminal_result = Some(format!(
        "{}:{}",
        disposition.terminal_tag(),
        disposition.reason()
    ));
    invocation.progress_at = unix_now()?;
    write_invocation_atomic(&state_path, &invocation)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StaleClaimRelease {
    /// Our claim ref moved to the requested disposition.
    Transitioned,
    /// Someone else owns the claim now; nothing was mutated.
    OwnershipLost,
}

/// Compare-and-swap release of the claim. The transition validates
/// worker/claim/branch identity itself, so an unrelated live claim is never
/// overwritten, and neither disposition closes a pull request or touches a ref.
pub(crate) fn release_stale_claim(
    lease: &ClaimLease,
    disposition: StaleClaimDisposition,
    pull_request: Option<u64>,
) -> Result<StaleClaimRelease, String> {
    let Some(claim) = disposition.claim_disposition() else {
        return Ok(StaleClaimRelease::OwnershipLost);
    };
    match transition_bridge_claim(
        ClaimMutationIdentity {
            repo: &lease.repo,
            issue: lease.issue,
            worker_id: &lease.worker_id,
            branch: &lease.branch,
            claim_id: &lease.claim_id,
        },
        pull_request,
        claim,
    ) {
        Ok(BridgeClaimTransition::Transitioned) => Ok(StaleClaimRelease::Transitioned),
        Ok(BridgeClaimTransition::OwnershipLost) => Ok(StaleClaimRelease::OwnershipLost),
        Err(failure) => Err(failure.message),
    }
}

/// Injected side effects so the disposition path is testable without GitHub.
pub(crate) struct StaleClaimEffects<'a> {
    pub(crate) persist_terminal: &'a mut dyn FnMut(StaleClaimDisposition) -> Result<(), String>,
    pub(crate) release_claim:
        &'a mut dyn FnMut(StaleClaimDisposition) -> Result<StaleClaimRelease, String>,
    pub(crate) clear_receipt: &'a mut dyn FnMut() -> Result<(), String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StaleClaimOutcome {
    pub(crate) disposition: StaleClaimDisposition,
    pub(crate) release: StaleClaimRelease,
}

/// Apply a disposition: durable stamp first, then the compare-and-swap release,
/// then the local acquisition receipt. A failed stamp aborts before any claim
/// mutation so a crash never leaves a released claim with an adoptable
/// invocation.
pub(crate) fn dispose_stale_claim(
    disposition: StaleClaimDisposition,
    effects: &mut StaleClaimEffects<'_>,
) -> Result<Option<StaleClaimOutcome>, String> {
    if disposition == StaleClaimDisposition::Resume {
        return Ok(None);
    }
    (effects.persist_terminal)(disposition)?;
    let release = (effects.release_claim)(disposition)?;
    (effects.clear_receipt)()?;
    Ok(Some(StaleClaimOutcome {
        disposition,
        release,
    }))
}

/// Where the foreground loop should go after a receipt failure pause.
#[derive(Debug)]
pub(crate) enum ReceiptFailureRoute {
    /// Re-enter the executor: the receipt is recoverable or live work owns it.
    Resumed(ConductorState),
    /// Drop this selection and continue with the next issue.
    Retired(ConductorState),
    /// Nothing changed; keep the pause as-is (non-continuous runs).
    Unchanged(ConductorState),
    /// Surface the failure to the caller.
    Failed(CommandFailure),
}

fn transition_and_persist(
    state_path: &Path,
    state: ConductorState,
    event: ConductorEvent,
) -> Result<ConductorState, CommandFailure> {
    let next = state
        .transition(event)
        .map_err(CommandFailure::diagnostic)?;
    persist_foreground_state(state_path, &next).map_err(CommandFailure::diagnostic)?;
    Ok(next)
}

/// The legacy route: release the claim as retryable and retire the selection.
fn legacy_receipt_failure_route(
    state_path: &Path,
    lease: Option<&ClaimLease>,
    recovery: Result<bool, CommandFailure>,
    continuous: bool,
    state: ConductorState,
) -> Result<ReceiptFailureRoute, CommandFailure> {
    if !continuous {
        return Ok(match recovery {
            Err(failure) => ReceiptFailureRoute::Failed(failure),
            Ok(_) => ReceiptFailureRoute::Unchanged(state),
        });
    }
    if let Some(lease) = lease {
        let _ = release_stale_claim(lease, StaleClaimDisposition::TerminalRelease, None);
    }
    clear_claim_acquisition_receipt(state_path).map_err(CommandFailure::diagnostic)?;
    Ok(ReceiptFailureRoute::Retired(transition_and_persist(
        state_path,
        state,
        ConductorEvent::RetireObsoleteSelection,
    )?))
}

/// Single entry point wired from the `executor_receipt_failed` pause branch.
pub(crate) fn dispose_stale_receipt_failure(
    layout: &RunLayout,
    state_path: &Path,
    issue: u64,
    recovery: Result<bool, CommandFailure>,
    continuous: bool,
    state: ConductorState,
) -> Result<ReceiptFailureRoute, CommandFailure> {
    if matches!(recovery, Ok(true)) {
        return Ok(ReceiptFailureRoute::Resumed(transition_and_persist(
            state_path,
            state,
            ConductorEvent::Resume,
        )?));
    }
    let executor_dir = layout.state_dir.join("executor");
    let recovery_note = match recovery.as_ref().err() {
        Some(failure) => format!(" (recovery probe failed: {})", failure.message),
        None => String::new(),
    };
    let lease =
        load_claim_acquisition_receipt(state_path, &layout.repo, issue).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "cannot load the receipt for issue #{issue}{recovery_note}: {error}"
            ))
        })?;
    let observation = lease
        .as_ref()
        .map(|lease| observe_stale_claim_facts(&executor_dir, lease));
    // A missing invocation or a failed read leaves the legacy route in charge.
    let observed: Option<StaleClaimFacts> = match observation {
        Some(Ok(facts)) => facts,
        Some(Err(_)) | None => None,
    };
    let disposition = observed
        .as_ref()
        .map(|facts| classify_stale_claim(facts, stale_claim_floor_secs()));
    if !matches!(disposition, Some(found) if found != StaleClaimDisposition::Resume) {
        return legacy_receipt_failure_route(
            state_path,
            lease.as_ref(),
            recovery,
            continuous,
            state,
        );
    }
    // A non-continuous run must not mutate labels it was never asked to move.
    if !continuous {
        return Ok(match recovery {
            Err(failure) => ReceiptFailureRoute::Failed(failure),
            Ok(_) => ReceiptFailureRoute::Unchanged(state),
        });
    }
    let disposition = disposition.expect("checked non-resume disposition");
    let lease = lease.expect("checked lease presence");
    let pull_request = match disposition {
        // Keep the pull request linkage on the quarantined record for the human.
        StaleClaimDisposition::Quarantine => observed.expect("checked facts").recorded_pr,
        _ => None,
    };
    let mut effects = StaleClaimEffects {
        persist_terminal: &mut |disposition| stamp_terminal(&executor_dir, &lease, disposition),
        release_claim: &mut |disposition| release_stale_claim(&lease, disposition, pull_request),
        clear_receipt: &mut || clear_claim_acquisition_receipt(state_path),
    };
    match dispose_stale_claim(disposition, &mut effects) {
        Ok(outcome) => {
            if let Some(outcome) = outcome {
                eprintln!(
                    "stale claim: issue #{issue} {} ({})",
                    outcome.disposition.reason(),
                    match outcome.release {
                        StaleClaimRelease::Transitioned => "claim released",
                        StaleClaimRelease::OwnershipLost => "claim owned elsewhere",
                    }
                );
            }
            Ok(ReceiptFailureRoute::Retired(transition_and_persist(
                state_path,
                state,
                ConductorEvent::RetireObsoleteSelection,
            )?))
        }
        Err(error) => {
            // Containment failed; fall back to the legacy route and log it.
            eprintln!("stale claim disposition deferred: {error}");
            legacy_receipt_failure_route(state_path, Some(&lease), recovery, continuous, state)
        }
    }
}

#[cfg(test)]
mod tests;
