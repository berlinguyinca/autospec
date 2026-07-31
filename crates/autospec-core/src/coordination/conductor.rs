mod invariants;
mod normalized;
mod persistence;

const CONDUCTOR_SCHEMA: u64 = 1;
const RETRY_LIMIT_EXHAUSTED: &str = "retry_limit_exhausted";
const TERMINAL_RETIREMENT: &str = "executor_terminal_retirement";
const OWNERSHIP_RETIREMENT: &str = "executor_ownership_retirement";
pub const BLOCKED_BACKLOG_THRESHOLD: u32 = 5;
const MAX_NO_PROGRESS_REASON_LENGTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConductorScope {
    Repository,
    Slice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConductorPhase {
    Scan,
    Review,
    Select,
    Claim,
    Dispatch,
    DispatchRecorded,
    Retry,
    Paused,
    SliceComplete,
    AllDone,
    TierDry,
    AllBlocked,
    VerifierUnavailable,
    IdleRescan,
    ResourcePark,
    OperatorStop,
}

impl ConductorPhase {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::SliceComplete | Self::AllDone)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConductorOutcome {
    Succeeded,
    Retryable(String),
    Blocked(String),
    AllBlocked { reason: String, issues: Box<[u64]> },
    VerifierUnavailable { reason: String },
    ResourcePark { reason: String },
    OperatorStop { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConductorEvent {
    ScanFoundWork,
    ScanEmpty,
    SafetyReviewed,
    SafetySkipped,
    Selected {
        issue: u64,
        serialization_reasons: Vec<String>,
    },
    Claimed,
    DispatchRecorded {
        outcome: ConductorOutcome,
    },
    BeginTerminalRetirement {
        outcome: ConductorOutcome,
    },
    BeginOwnershipRetirement,
    Reconciled,
    RetryScheduled,
    Pause {
        reason: String,
    },
    Resume,
    RetireObsoleteSelection,
    AbandonExhausted,
    AbandonTerminal,
    AbandonOwnership,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConductorState {
    repo: String,
    scope: ConductorScope,
    phase: ConductorPhase,
    selected_issue: Option<u64>,
    serialization_reasons: Vec<String>,
    retry_count: u32,
    retry_limit: u32,
    last_outcome: Option<ConductorOutcome>,
    pause_reason: Option<String>,
    terminal_reason: Option<String>,
    resume_phase: Option<ConductorPhase>,
    blocked_backlog_cycles: u32,
    blocked_backlog_reason: Option<String>,
    blocked_backlog_issues: Vec<u64>,
    no_progress_cycles: u32,
    no_progress_reason: Option<String>,
}

impl ConductorState {
    pub fn new(
        repo: impl Into<String>,
        scope: ConductorScope,
        retry_limit: u32,
    ) -> Result<Self, String> {
        let repo = repo.into();
        if repo.trim().is_empty() {
            return Err("conductor state.repo must not be empty".to_string());
        }
        Ok(Self {
            repo,
            scope,
            phase: ConductorPhase::Scan,
            selected_issue: None,
            serialization_reasons: Vec::new(),
            retry_count: 0,
            retry_limit,
            last_outcome: None,
            pause_reason: None,
            terminal_reason: None,
            resume_phase: None,
            blocked_backlog_cycles: 0,
            blocked_backlog_reason: None,
            blocked_backlog_issues: Vec::new(),
            no_progress_cycles: 0,
            no_progress_reason: None,
        })
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn scope(&self) -> ConductorScope {
        self.scope
    }

    pub fn phase(&self) -> ConductorPhase {
        self.phase
    }

    pub fn selected_issue(&self) -> Option<u64> {
        self.selected_issue
    }

    pub fn serialization_reasons(&self) -> &[String] {
        &self.serialization_reasons
    }

    pub fn retry_count(&self) -> u32 {
        self.retry_count
    }

    pub fn retry_limit(&self) -> u32 {
        self.retry_limit
    }

    pub fn last_outcome(&self) -> Option<&ConductorOutcome> {
        self.last_outcome.as_ref()
    }

    pub fn pause_reason(&self) -> Option<&str> {
        self.pause_reason.as_deref()
    }

    pub fn terminal_reason(&self) -> Option<&str> {
        self.terminal_reason.as_deref()
    }

    pub fn blocked_backlog_cycles(&self) -> u32 {
        self.blocked_backlog_cycles
    }

    pub fn blocked_backlog_reason(&self) -> Option<&str> {
        self.blocked_backlog_reason.as_deref()
    }

    pub fn blocked_backlog_issues(&self) -> &[u64] {
        &self.blocked_backlog_issues
    }

    pub fn no_progress_cycles(&self) -> u32 {
        self.no_progress_cycles
    }

    pub fn no_progress_reason(&self) -> Option<&str> {
        self.no_progress_reason.as_deref()
    }

    pub fn record_no_progress_cycle(mut self, reason: impl Into<String>) -> Result<Self, String> {
        let reason = reason.into();
        if reason.trim().is_empty() || reason.chars().count() > MAX_NO_PROGRESS_REASON_LENGTH {
            return Err(format!(
                "no-progress reason must contain 1..={MAX_NO_PROGRESS_REASON_LENGTH} characters"
            ));
        }
        self.no_progress_cycles = if self.no_progress_reason.as_deref() == Some(reason.as_str()) {
            self.no_progress_cycles.saturating_add(1)
        } else {
            1
        };
        self.no_progress_reason = Some(reason);
        self.validate()?;
        Ok(self)
    }

    pub fn clear_no_progress_diagnostic(mut self) -> Result<Self, String> {
        self.no_progress_cycles = 0;
        self.no_progress_reason = None;
        self.validate()?;
        Ok(self)
    }

    /// Record one complete Tier 1 blocked cycle. Repeated cycles with a new
    /// issue set or reason reset the governor; five identical cycles seal the
    /// state as `AllBlocked` and prevent descent into discovery tiers.
    pub fn record_blocked_backlog_cycle(
        mut self,
        reason: impl Into<String>,
        mut issues: Vec<u64>,
    ) -> Result<Self, String> {
        let reason = reason.into();
        if reason.trim().is_empty() || issues.contains(&0) {
            return Err(
                "blocked backlog requires a non-empty reason and positive issue ids".to_string(),
            );
        }
        issues.sort_unstable();
        issues.dedup();
        let same = self.blocked_backlog_reason.as_deref() == Some(reason.as_str())
            && self.blocked_backlog_issues == issues;
        if !same && self.phase == ConductorPhase::AllBlocked {
            self.phase = ConductorPhase::Scan;
            self.last_outcome = None;
        }
        self.blocked_backlog_cycles = if same {
            self.blocked_backlog_cycles.saturating_add(1)
        } else {
            1
        };
        self.blocked_backlog_reason = Some(reason.clone());
        self.blocked_backlog_issues = issues.clone();
        if self.blocked_backlog_cycles >= BLOCKED_BACKLOG_THRESHOLD {
            self.clear_selection();
            self.phase = ConductorPhase::AllBlocked;
            self.last_outcome = Some(ConductorOutcome::AllBlocked {
                reason,
                issues: issues.into_boxed_slice(),
            });
        }
        self.validate()?;
        Ok(self)
    }

    pub fn clear_blocked_backlog_governor(mut self) -> Result<Self, String> {
        self.blocked_backlog_cycles = 0;
        self.blocked_backlog_reason = None;
        self.blocked_backlog_issues.clear();
        if self.phase == ConductorPhase::AllBlocked {
            self.phase = ConductorPhase::Scan;
            self.last_outcome = None;
        }
        self.validate()?;
        Ok(self)
    }

    pub fn transition(mut self, event: ConductorEvent) -> Result<Self, String> {
        match event {
            ConductorEvent::ScanFoundWork if self.phase == ConductorPhase::Scan => {
                self.phase = ConductorPhase::Review
            }
            ConductorEvent::ScanEmpty if self.phase == ConductorPhase::Scan => {
                self.record_empty_scan()
            }
            ConductorEvent::SafetyReviewed if self.phase == ConductorPhase::Review => {
                self.phase = ConductorPhase::Select
            }
            ConductorEvent::SafetySkipped if self.phase == ConductorPhase::Review => {
                self.phase = ConductorPhase::Scan
            }
            ConductorEvent::Selected {
                issue,
                serialization_reasons,
            } if self.phase == ConductorPhase::Select => {
                self.select_issue(issue, serialization_reasons)?
            }
            ConductorEvent::Claimed if self.phase == ConductorPhase::Claim => {
                self.phase = ConductorPhase::Dispatch
            }
            ConductorEvent::DispatchRecorded { outcome }
                if self.phase == ConductorPhase::Dispatch =>
            {
                self.record_dispatch(outcome)?
            }
            ConductorEvent::BeginTerminalRetirement { outcome }
                if matches!(
                    self.phase,
                    ConductorPhase::Dispatch | ConductorPhase::DispatchRecorded
                ) =>
            {
                self.begin_terminal_retirement(outcome)?
            }
            ConductorEvent::BeginOwnershipRetirement
                if matches!(
                    self.phase,
                    ConductorPhase::Dispatch | ConductorPhase::DispatchRecorded
                ) =>
            {
                self.begin_ownership_retirement()
            }
            ConductorEvent::Reconciled if self.phase == ConductorPhase::DispatchRecorded => {
                self.reconcile_success()?
            }
            ConductorEvent::RetryScheduled if self.phase == ConductorPhase::Retry => {
                self.phase = ConductorPhase::Claim
            }
            ConductorEvent::Pause { reason }
                if !self.phase.is_terminal() && self.phase != ConductorPhase::Paused =>
            {
                self.pause(reason)?
            }
            ConductorEvent::Resume if self.phase == ConductorPhase::Paused => self.resume()?,
            ConductorEvent::RetireObsoleteSelection
                if self.phase == ConductorPhase::Paused && self.selected_issue.is_some() =>
            {
                self.abandon_exhausted()
            }
            ConductorEvent::AbandonExhausted if self.can_abandon_exhausted() => {
                self.abandon_exhausted()
            }
            ConductorEvent::AbandonTerminal if self.can_abandon_terminal() => {
                self.abandon_exhausted()
            }
            ConductorEvent::AbandonOwnership
                if self.phase == ConductorPhase::Dispatch
                    || (self.phase == ConductorPhase::Paused
                        && self.pause_reason.as_deref() == Some(OWNERSHIP_RETIREMENT)) =>
            {
                self.abandon_exhausted()
            }
            event => return Err(format!("invalid conductor event: {event:?}")),
        }
        self.validate()?;
        Ok(self)
    }

    fn record_empty_scan(&mut self) {
        self.clear_for_terminal();
        let (phase, reason) = match self.scope {
            ConductorScope::Repository => (ConductorPhase::AllDone, "repository_empty"),
            ConductorScope::Slice => (ConductorPhase::SliceComplete, "slice_empty"),
        };
        self.phase = phase;
        self.terminal_reason = Some(reason.to_string());
    }

    fn select_issue(
        &mut self,
        issue: u64,
        serialization_reasons: Vec<String>,
    ) -> Result<(), String> {
        if issue == 0 {
            return Err("selected issue must be positive".to_string());
        }
        self.phase = ConductorPhase::Claim;
        self.selected_issue = Some(issue);
        self.serialization_reasons = serialization_reasons;
        self.retry_count = 0;
        self.last_outcome = None;
        Ok(())
    }

    fn record_dispatch(&mut self, outcome: ConductorOutcome) -> Result<(), String> {
        outcome.validate()?;
        self.last_outcome = Some(outcome.clone());
        match outcome {
            ConductorOutcome::Succeeded => self.phase = ConductorPhase::DispatchRecorded,
            ConductorOutcome::Retryable(_) => self.record_retryable_dispatch()?,
            ConductorOutcome::Blocked(reason) => self.record_blocked_dispatch(reason),
            ConductorOutcome::AllBlocked { reason, .. }
            | ConductorOutcome::VerifierUnavailable { reason }
            | ConductorOutcome::ResourcePark { reason }
            | ConductorOutcome::OperatorStop { reason } => self.record_blocked_dispatch(reason),
        }
        Ok(())
    }

    fn record_retryable_dispatch(&mut self) -> Result<(), String> {
        self.retry_count = self
            .retry_count
            .checked_add(1)
            .ok_or_else(|| "conductor retry count overflowed".to_string())?;
        if self.retry_count > self.retry_limit {
            self.pause_reason = Some(RETRY_LIMIT_EXHAUSTED.to_string());
            self.resume_phase = None;
            self.phase = ConductorPhase::Paused;
        } else {
            self.phase = ConductorPhase::Retry;
        }
        Ok(())
    }

    fn begin_terminal_retirement(&mut self, outcome: ConductorOutcome) -> Result<(), String> {
        if self.phase == ConductorPhase::Dispatch {
            self.record_dispatch(outcome)?;
        } else if self.phase != ConductorPhase::DispatchRecorded
            || self.last_outcome.as_ref() != Some(&outcome)
        {
            return Err(
                "terminal retirement does not match the recorded dispatch outcome".to_string(),
            );
        }
        if self.phase == ConductorPhase::Retry {
            return Ok(());
        }
        if self.phase != ConductorPhase::DispatchRecorded && !self.can_abandon_terminal() {
            return Err("finalized dispatch did not reach a terminal conductor state".to_string());
        }
        self.phase = ConductorPhase::Paused;
        self.pause_reason = Some(TERMINAL_RETIREMENT.to_string());
        self.resume_phase = None;
        Ok(())
    }

    fn begin_ownership_retirement(&mut self) {
        self.phase = ConductorPhase::Paused;
        self.pause_reason = Some(OWNERSHIP_RETIREMENT.to_string());
        self.resume_phase = None;
    }

    fn record_blocked_dispatch(&mut self, reason: String) {
        self.pause_reason = Some(reason);
        self.resume_phase = Some(ConductorPhase::Claim);
        self.phase = ConductorPhase::Paused;
    }

    fn reconcile_success(&mut self) -> Result<(), String> {
        if self.last_outcome != Some(ConductorOutcome::Succeeded) {
            return Err("only a recorded successful outcome can reconcile".to_string());
        }
        self.clear_selection();
        self.phase = ConductorPhase::Scan;
        Ok(())
    }

    fn pause(&mut self, reason: String) -> Result<(), String> {
        if reason.trim().is_empty() {
            return Err("pause reason must not be empty".to_string());
        }
        if reason == RETRY_LIMIT_EXHAUSTED {
            return Err("retry exhaustion is recorded only from a dispatch outcome".to_string());
        }
        self.resume_phase = Some(self.phase);
        self.pause_reason = Some(reason);
        self.phase = ConductorPhase::Paused;
        Ok(())
    }

    fn resume(&mut self) -> Result<(), String> {
        self.phase = self
            .resume_phase
            .take()
            .ok_or_else(|| "paused conductor state requires explicit recovery".to_string())?;
        self.pause_reason = None;
        Ok(())
    }

    fn abandon_exhausted(&mut self) {
        self.clear_selection();
        self.phase = ConductorPhase::Scan;
    }

    fn can_abandon_exhausted(&self) -> bool {
        self.phase == ConductorPhase::Paused
            && self.pause_reason.as_deref() == Some(RETRY_LIMIT_EXHAUSTED)
    }

    fn can_abandon_terminal(&self) -> bool {
        self.phase == ConductorPhase::Paused
            && (self.pause_reason.as_deref() == Some(TERMINAL_RETIREMENT)
                || self.pause_reason.as_deref() == Some(RETRY_LIMIT_EXHAUSTED)
                || matches!(self.last_outcome, Some(ConductorOutcome::Blocked(_))))
    }

    fn clear_selection(&mut self) {
        self.selected_issue = None;
        self.serialization_reasons.clear();
        self.pause_reason = None;
        self.resume_phase = None;
    }

    fn clear_for_terminal(&mut self) {
        self.clear_selection();
        self.last_outcome = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{ConductorEvent, ConductorPhase, ConductorScope, ConductorState};

    #[test]
    fn obsolete_paused_selection_retires_to_scan() {
        let retired = ConductorState::new("owner/repo", ConductorScope::Repository, 3)
            .unwrap()
            .transition(ConductorEvent::ScanFoundWork)
            .unwrap()
            .transition(ConductorEvent::SafetyReviewed)
            .unwrap()
            .transition(ConductorEvent::Selected {
                issue: 1600,
                serialization_reasons: Vec::new(),
            })
            .unwrap()
            .transition(ConductorEvent::Pause {
                reason: "operator_wait".to_string(),
            })
            .unwrap()
            .transition(ConductorEvent::RetireObsoleteSelection)
            .unwrap();

        assert_eq!(retired.phase(), ConductorPhase::Scan);
        assert_eq!(retired.selected_issue(), None);
    }
}
