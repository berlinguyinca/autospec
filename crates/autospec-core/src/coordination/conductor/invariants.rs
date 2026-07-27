use super::{
    ConductorOutcome, ConductorPhase, ConductorScope, ConductorState,
    MAX_NO_PROGRESS_REASON_LENGTH, OWNERSHIP_RETIREMENT, RETRY_LIMIT_EXHAUSTED,
    TERMINAL_RETIREMENT,
};

impl ConductorState {
    pub(super) fn validate(&self) -> Result<(), String> {
        self.validate_identity_and_outcome()?;
        if self.phase.is_terminal() {
            return self.validate_terminal_state();
        }
        if self.terminal_reason.is_some() {
            return Err("nonterminal conductor state cannot carry terminal reason".to_string());
        }
        self.validate_nonterminal_phase()?;
        self.validate_nonterminal_pause_metadata()
    }

    fn validate_identity_and_outcome(&self) -> Result<(), String> {
        if self.repo.trim().is_empty() {
            return Err("conductor state.repo must not be empty".to_string());
        }
        if self.selected_issue == Some(0) {
            return Err("selected issue must be positive".to_string());
        }
        if let Some(outcome) = &self.last_outcome {
            outcome.validate()?;
        }
        if self.blocked_backlog_cycles == 0
            && (self.blocked_backlog_reason.is_some() || !self.blocked_backlog_issues.is_empty())
        {
            return Err("blocked backlog metadata requires a positive cycle count".to_string());
        }
        if self.blocked_backlog_issues.contains(&0)
            || self
                .blocked_backlog_issues
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err("blocked backlog issue ids must be positive and sorted".to_string());
        }
        if self.blocked_backlog_cycles >= super::BLOCKED_BACKLOG_THRESHOLD
            && self.phase != ConductorPhase::AllBlocked
        {
            return Err("threshold blocked backlog must be sealed as all-blocked".to_string());
        }
        match (self.no_progress_cycles, self.no_progress_reason.as_deref()) {
            (0, None) => {}
            (0, Some(_)) | (_, None) => {
                return Err(
                    "no-progress reason and cycle count must be recorded together".to_string(),
                )
            }
            (_, Some(reason))
                if reason.trim().is_empty()
                    || reason.chars().count() > MAX_NO_PROGRESS_REASON_LENGTH =>
            {
                return Err(format!(
                    "no-progress reason must contain 1..={MAX_NO_PROGRESS_REASON_LENGTH} characters"
                ))
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_terminal_state(&self) -> Result<(), String> {
        let valid_terminal = matches!(
            (&self.scope, &self.phase, self.terminal_reason.as_deref()),
            (
                ConductorScope::Repository,
                ConductorPhase::AllDone,
                Some("repository_empty")
            ) | (
                ConductorScope::Slice,
                ConductorPhase::SliceComplete,
                Some("slice_empty")
            )
        );
        if !valid_terminal {
            return Err("terminal conductor state has incompatible scope or reason".to_string());
        }
        if self.selected_issue.is_some()
            || !self.serialization_reasons.is_empty()
            || self.last_outcome.is_some()
            || self.pause_reason.is_some()
            || self.resume_phase.is_some()
        {
            return Err("terminal conductor state carries active issue metadata".to_string());
        }
        Ok(())
    }

    fn validate_nonterminal_phase(&self) -> Result<(), String> {
        match self.phase {
            ConductorPhase::Scan | ConductorPhase::Review | ConductorPhase::Select => {
                self.validate_unselected_state()
            }
            ConductorPhase::Claim | ConductorPhase::Dispatch => self.validate_selected_state(),
            ConductorPhase::DispatchRecorded => {
                self.validate_selected_state()?;
                self.validate_successful_dispatch()
            }
            ConductorPhase::Retry => {
                self.validate_selected_state()?;
                self.validate_retry_state()
            }
            ConductorPhase::Paused => self.validate_paused_state(),
            ConductorPhase::TierDry
            | ConductorPhase::AllBlocked
            | ConductorPhase::VerifierUnavailable
            | ConductorPhase::IdleRescan
            | ConductorPhase::ResourcePark
            | ConductorPhase::OperatorStop => self.validate_boundary_state(),
            ConductorPhase::SliceComplete | ConductorPhase::AllDone => {
                Err("terminal conductor phase reached nonterminal validation".to_string())
            }
        }
    }

    fn validate_unselected_state(&self) -> Result<(), String> {
        if self.selected_issue.is_some() || !self.serialization_reasons.is_empty() {
            return Err("unselected conductor phase carries selected issue metadata".to_string());
        }
        Ok(())
    }

    fn validate_selected_state(&self) -> Result<(), String> {
        if self.selected_issue.is_none() {
            return Err("selected conductor phase requires selected issue".to_string());
        }
        Ok(())
    }

    fn validate_successful_dispatch(&self) -> Result<(), String> {
        if self.last_outcome != Some(ConductorOutcome::Succeeded) {
            return Err("recorded dispatch phase requires a successful outcome".to_string());
        }
        Ok(())
    }

    fn validate_retry_state(&self) -> Result<(), String> {
        if !matches!(self.last_outcome, Some(ConductorOutcome::Retryable(_))) {
            return Err("retry conductor phase requires a retryable outcome".to_string());
        }
        if self.retry_count > self.retry_limit {
            return Err("retry conductor phase exceeds retry limit".to_string());
        }
        Ok(())
    }

    fn validate_nonterminal_pause_metadata(&self) -> Result<(), String> {
        if self.phase != ConductorPhase::Paused
            && (self.pause_reason.is_some() || self.resume_phase.is_some())
        {
            return Err("unpaused conductor state cannot carry pause metadata".to_string());
        }
        Ok(())
    }

    fn validate_paused_state(&self) -> Result<(), String> {
        let reason = self
            .pause_reason
            .as_deref()
            .ok_or_else(|| "paused conductor state requires pause reason".to_string())?;
        if reason == TERMINAL_RETIREMENT {
            if self.resume_phase.is_some()
                || self.selected_issue.is_none()
                || !matches!(
                    self.last_outcome,
                    Some(
                        ConductorOutcome::Succeeded
                            | ConductorOutcome::Retryable(_)
                            | ConductorOutcome::Blocked(_)
                    )
                )
            {
                return Err(
                    "terminal retirement pause has incompatible recovery metadata".to_string(),
                );
            }
            return Ok(());
        }
        if reason == OWNERSHIP_RETIREMENT {
            if self.resume_phase.is_some() || self.selected_issue.is_none() {
                return Err(
                    "ownership retirement pause has incompatible recovery metadata".to_string(),
                );
            }
            return Ok(());
        }
        if reason == RETRY_LIMIT_EXHAUSTED {
            if self.resume_phase.is_some()
                || self.selected_issue.is_none()
                || !matches!(self.last_outcome, Some(ConductorOutcome::Retryable(_)))
                || self.retry_count <= self.retry_limit
            {
                return Err("exhausted retry pause has incompatible recovery metadata".to_string());
            }
            return Ok(());
        }
        let resume_phase = self
            .resume_phase
            .ok_or_else(|| "paused conductor state requires resume phase".to_string())?;
        if resume_phase.is_terminal()
            || resume_phase.is_boundary()
            || resume_phase == ConductorPhase::Paused
        {
            return Err("paused conductor state has invalid resume phase".to_string());
        }
        let mut resumed = self.clone();
        resumed.phase = resume_phase;
        resumed.pause_reason = None;
        resumed.resume_phase = None;
        resumed.validate()
    }

    fn validate_boundary_state(&self) -> Result<(), String> {
        if self.selected_issue.is_some()
            || !self.serialization_reasons.is_empty()
            || self.pause_reason.is_some()
            || self.resume_phase.is_some()
        {
            return Err("conductor boundary state carries active issue metadata".to_string());
        }
        self.validate_boundary_outcome()
    }

    fn validate_boundary_outcome(&self) -> Result<(), String> {
        let valid = matches!(
            (&self.phase, &self.last_outcome),
            (ConductorPhase::TierDry | ConductorPhase::IdleRescan, None)
                | (
                    ConductorPhase::AllBlocked,
                    Some(ConductorOutcome::AllBlocked { .. }),
                )
                | (
                    ConductorPhase::VerifierUnavailable,
                    Some(ConductorOutcome::VerifierUnavailable { .. }),
                )
                | (
                    ConductorPhase::ResourcePark,
                    Some(ConductorOutcome::ResourcePark { .. }),
                )
                | (
                    ConductorPhase::OperatorStop,
                    Some(ConductorOutcome::OperatorStop { .. }),
                )
        );
        if valid {
            Ok(())
        } else {
            Err("conductor boundary state has incompatible phase/outcome".to_string())
        }
    }
}
