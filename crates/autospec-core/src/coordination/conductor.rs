mod invariants;
mod normalized;
mod persistence;

const CONDUCTOR_SCHEMA: u64 = 1;
const RETRY_LIMIT_EXHAUSTED: &str = "retry_limit_exhausted";

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
    Reconciled,
    RetryScheduled,
    Pause {
        reason: String,
    },
    Resume,
    AbandonExhausted,
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
            ConductorEvent::AbandonExhausted if self.can_abandon_exhausted() => {
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
