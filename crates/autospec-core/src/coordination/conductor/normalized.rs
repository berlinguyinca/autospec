use crate::autonomous::no_work::{NoWorkDecision, NoWorkState, NoWorkTier};
use crate::autonomous::waterfall::{TierReceipt, TierStatus};

use super::{ConductorOutcome, ConductorPhase, ConductorState, RETRY_LIMIT_EXHAUSTED};

impl ConductorPhase {
    pub(super) fn is_boundary(&self) -> bool {
        matches!(
            self,
            Self::TierDry
                | Self::AllBlocked
                | Self::VerifierUnavailable
                | Self::IdleRescan
                | Self::ResourcePark
                | Self::OperatorStop
        )
    }

    pub(super) fn normalized_state_name(&self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Review => "review",
            Self::Select => "select",
            Self::Claim => "claim",
            Self::Dispatch => "dispatch",
            Self::DispatchRecorded => "dispatch-recorded",
            Self::Retry => "retry",
            Self::Paused => "paused",
            Self::SliceComplete => "slice-complete",
            Self::AllDone => "all-done",
            Self::TierDry => "tier-dry",
            Self::AllBlocked => "all-blocked",
            Self::VerifierUnavailable => "verifier-unavailable",
            Self::IdleRescan => "idle-rescan",
            Self::ResourcePark => "resource-park",
            Self::OperatorStop => "operator-stop",
        }
    }
}

impl ConductorOutcome {
    pub(super) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Succeeded => Ok(()),
            Self::Retryable(reason) if reason.trim().is_empty() => {
                Err("retryable conductor outcome requires a reason".to_string())
            }
            Self::Blocked(reason)
                if reason.trim().is_empty() || reason == RETRY_LIMIT_EXHAUSTED =>
            {
                Err("blocked conductor outcome has an invalid reason".to_string())
            }
            Self::AllBlocked { reason, issues } => {
                validate_named_outcome_reason("all-blocked", reason)?;
                validate_all_blocked_issues(issues)
            }
            Self::VerifierUnavailable { reason }
            | Self::ResourcePark { reason }
            | Self::OperatorStop { reason } => {
                validate_named_outcome_reason(self.normalized_state_name(), reason)
            }
            Self::Retryable(_) | Self::Blocked(_) => Ok(()),
        }
    }

    pub(super) fn normalized_state_name(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Retryable(_) => "retryable",
            Self::Blocked(_) => "blocked",
            Self::AllBlocked { .. } => "all-blocked",
            Self::VerifierUnavailable { .. } => "verifier-unavailable",
            Self::ResourcePark { .. } => "resource-park",
            Self::OperatorStop { .. } => "operator-stop",
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Succeeded => None,
            Self::Retryable(reason)
            | Self::Blocked(reason)
            | Self::AllBlocked { reason, .. }
            | Self::VerifierUnavailable { reason }
            | Self::ResourcePark { reason }
            | Self::OperatorStop { reason } => Some(reason),
        }
    }

    fn affected_issues(&self) -> &[u64] {
        match self {
            Self::AllBlocked { issues, .. } => issues,
            _ => &[],
        }
    }

    fn recommended_next_step(&self) -> &'static str {
        match self {
            Self::Succeeded => "scan for the next issue",
            Self::Retryable(_) => "retry the selected issue",
            Self::Blocked(_) => "resume or unblock the selected issue",
            Self::AllBlocked { .. } => "promote or unblock affected issues",
            Self::VerifierUnavailable { .. } => "retry verifier or park without mutation",
            Self::ResourcePark { .. } => "wait for resource budget or operator resume",
            Self::OperatorStop { .. } => "wait for operator resume",
        }
    }
}

fn validate_all_blocked_issues(issues: &[u64]) -> Result<(), String> {
    if issues.is_empty() || issues.contains(&0) {
        Err("all-blocked conductor outcome requires positive issue ids".to_string())
    } else {
        Ok(())
    }
}

fn validate_named_outcome_reason(kind: &str, reason: &str) -> Result<(), String> {
    if reason.trim().is_empty() || reason == RETRY_LIMIT_EXHAUSTED {
        Err(format!("{kind} conductor outcome has an invalid reason"))
    } else {
        Ok(())
    }
}
impl ConductorState {
    pub fn normalized_state(&self) -> &'static str {
        self.current_normalized_outcome()
            .map(ConductorOutcome::normalized_state_name)
            .unwrap_or_else(|| self.phase.normalized_state_name())
    }

    pub fn normalized_state_for_cycle(&self, cycle: u64) -> String {
        let outcome = self.current_normalized_outcome();
        let reason = if outcome.is_none() && self.phase == ConductorPhase::Scan {
            self.no_progress_reason.as_deref()
        } else {
            None
        }
        .or_else(|| {
            outcome
                .and_then(ConductorOutcome::reason)
                .or(self.pause_reason.as_deref())
                .or(self.terminal_reason.as_deref())
        })
        .unwrap_or("none");
        normalized_state_line(NormalizedStateLine {
            cycle,
            state: self.normalized_state(),
            tier: "none",
            action: self.phase.normalized_state_name(),
            reason,
            affected_issues: &affected_issues_for_state(self, outcome),
            mutation_allowed: mutation_allowed_for_state(self, outcome),
            next: outcome
                .map(ConductorOutcome::recommended_next_step)
                .unwrap_or_else(|| recommended_next_step_for_phase(self.phase)),
        })
    }

    fn current_normalized_outcome(&self) -> Option<&ConductorOutcome> {
        if self.phase == ConductorPhase::Scan && self.no_progress_reason.is_some() {
            None
        } else {
            self.last_outcome.as_ref()
        }
    }

    pub fn normalized_state_for_tier_receipt(&self, cycle: u64, receipt: &TierReceipt) -> String {
        let (state, action, reason, mutation_allowed, next) =
            normalized_tier_receipt_parts(receipt);
        normalized_state_line(NormalizedStateLine {
            cycle,
            state,
            tier: receipt.tier().as_str(),
            action,
            reason: &reason,
            affected_issues: &[],
            mutation_allowed,
            next: &next,
        })
    }

    pub fn normalized_state_for_no_work(&self, cycle: u64, state: &NoWorkState) -> String {
        let next = match state.decision() {
            NoWorkDecision::IdleRescan => "sleep until the next rescan interval",
            NoWorkDecision::RequestIdeation => "request planning-only ideation refresh",
        };
        normalized_state_line(NormalizedStateLine {
            cycle,
            state: ConductorPhase::IdleRescan.normalized_state_name(),
            tier: "all",
            action: "rescan",
            reason: state.decision().as_str(),
            affected_issues: &[],
            mutation_allowed: false,
            next,
        })
    }
}

struct NormalizedStateLine<'a> {
    cycle: u64,
    state: &'a str,
    tier: &'a str,
    action: &'a str,
    reason: &'a str,
    affected_issues: &'a [u64],
    mutation_allowed: bool,
    next: &'a str,
}

fn normalized_state_line(parts: NormalizedStateLine<'_>) -> String {
    format!(
        "cycle {}: {}; tier={}; action={}; reason={}; affected issues={}; mutation_allowed={}; next={}",
        parts.cycle,
        parts.state,
        parts.tier,
        parts.action,
        parts.reason,
        issue_list(parts.affected_issues),
        parts.mutation_allowed,
        parts.next
    )
}

fn affected_issues_for_state(
    state: &ConductorState,
    outcome: Option<&ConductorOutcome>,
) -> Vec<u64> {
    let outcome_issues = outcome
        .map(ConductorOutcome::affected_issues)
        .unwrap_or_default();
    if !outcome_issues.is_empty() {
        outcome_issues.to_vec()
    } else {
        state.selected_issue.into_iter().collect()
    }
}

fn mutation_allowed_for_state(state: &ConductorState, outcome: Option<&ConductorOutcome>) -> bool {
    if matches!(
        outcome,
        Some(
            ConductorOutcome::AllBlocked { .. }
                | ConductorOutcome::VerifierUnavailable { .. }
                | ConductorOutcome::ResourcePark { .. }
                | ConductorOutcome::OperatorStop { .. }
        )
    ) {
        return false;
    }
    matches!(
        state.phase,
        ConductorPhase::Claim | ConductorPhase::Dispatch | ConductorPhase::DispatchRecorded
    )
}

fn recommended_next_step_for_phase(phase: ConductorPhase) -> &'static str {
    match phase {
        ConductorPhase::Scan => "scan for ready issues",
        ConductorPhase::Review => "review safety for the ready queue",
        ConductorPhase::Select => "select the next ready issue",
        ConductorPhase::Claim => "claim the selected issue",
        ConductorPhase::Dispatch => "dispatch the selected issue",
        ConductorPhase::DispatchRecorded => "reconcile the executor result",
        ConductorPhase::Retry => "schedule a retry",
        ConductorPhase::Paused => "wait for resume",
        ConductorPhase::SliceComplete | ConductorPhase::AllDone => "no next step",
        ConductorPhase::TierDry => "advance waterfall to the next tier",
        ConductorPhase::AllBlocked => "promote or unblock affected issues",
        ConductorPhase::VerifierUnavailable => "retry verifier or park without mutation",
        ConductorPhase::IdleRescan => "sleep until the next rescan interval",
        ConductorPhase::ResourcePark => "wait for resource budget or operator resume",
        ConductorPhase::OperatorStop => "wait for operator resume",
    }
}

fn normalized_tier_receipt_parts(
    receipt: &TierReceipt,
) -> (&'static str, &'static str, String, bool, String) {
    match receipt.status() {
        TierStatus::Exhausted { reason } => (
            ConductorPhase::TierDry.normalized_state_name(),
            "advance-waterfall",
            reason.as_str().to_string(),
            false,
            next_tier_step(receipt.tier()),
        ),
        TierStatus::Produced { count } => (
            "tier-produced",
            "file",
            format!("produced_{count}"),
            true,
            "dispatch produced candidates".to_string(),
        ),
        TierStatus::Failed { reason } => (
            "tier-failed",
            "halt-tier",
            reason.clone(),
            false,
            "inspect sealed tier failure evidence".to_string(),
        ),
        TierStatus::Blocked { reason } => (
            "tier-blocked",
            "park-tier",
            reason.clone(),
            false,
            "unblock the tier and retry".to_string(),
        ),
        TierStatus::NotRun { reason } => (
            "tier-not-run",
            "skip-tier",
            reason.clone(),
            false,
            "run prerequisite tier first".to_string(),
        ),
    }
}

fn next_tier_step(tier: NoWorkTier) -> String {
    match tier {
        NoWorkTier::Tier1 => "advance waterfall to tier1_5",
        NoWorkTier::Tier1_5 => "advance waterfall to tier2",
        NoWorkTier::Tier2 => "advance waterfall to tier3",
        NoWorkTier::Tier3 => "advance waterfall to tier4",
        NoWorkTier::Tier4 => "enter idle-rescan if value floor remains unmet",
    }
    .to_string()
}

fn issue_list(issues: &[u64]) -> String {
    if issues.is_empty() {
        "none".to_string()
    } else {
        issues
            .iter()
            .map(|issue| format!("#{issue}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}
