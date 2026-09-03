//! Edit guards, thrashing detection and stop conditions (AAR spec section 10).
//!
//! These are the guards AutoSpec can enforce programmatically instead of
//! trusting prompt prose to hold. Everything is pure state-machine logic over
//! observed agent actions; the driver decides what to do with the verdicts.

use std::collections::{BTreeMap, BTreeSet};

/// Starting defaults from spec section 10; all configurable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPolicy {
    pub require_read_before_edit: bool,
    pub require_reread_after_edit: bool,
    pub max_edit_lines: usize,
    pub max_new_file_lines: usize,
    pub one_logical_change_per_step: bool,
    /// Paths explicitly exempted, each with a stated reason.
    exceptions: BTreeMap<String, String>,
}

impl Default for EditPolicy {
    fn default() -> Self {
        Self {
            require_read_before_edit: true,
            require_reread_after_edit: true,
            max_edit_lines: 150,
            max_new_file_lines: 300,
            one_logical_change_per_step: true,
            exceptions: BTreeMap::new(),
        }
    }
}

impl EditPolicy {
    /// Exempt a path. Exceptions must be explicit and carry a reason.
    pub fn with_exception(
        mut self,
        path: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, String> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err("edit policy exceptions require a stated reason".to_string());
        }
        self.exceptions.insert(path.into(), reason);
        Ok(self)
    }

    pub fn exception_reason(&self, path: &str) -> Option<&str> {
        self.exceptions.get(path).map(String::as_str)
    }

    pub fn exceptions(&self) -> &BTreeMap<String, String> {
        &self.exceptions
    }
}

/// One observed agent action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditAction {
    Read { path: String },
    Search { query: String },
    Edit { path: String, lines: usize },
    Create { path: String, lines: usize },
    RunTests { command: String, passed: bool },
    Conclude { claim: String },
}

impl EditAction {
    pub fn kind(&self) -> &'static str {
        match self {
            EditAction::Read { .. } => "read",
            EditAction::Search { .. } => "search",
            EditAction::Edit { .. } => "edit",
            EditAction::Create { .. } => "create",
            EditAction::RunTests { .. } => "run_tests",
            EditAction::Conclude { .. } => "conclude",
        }
    }
}

/// A guard rule an action broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditGuardViolation {
    pub code: &'static str,
    pub detail: String,
}

impl EditGuardViolation {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// Stateful guard over a single agent's action stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditGuard {
    policy: EditPolicy,
    read_paths: BTreeSet<String>,
    /// Paths edited but not yet re-read.
    pending_reread: BTreeSet<String>,
    edits_this_step: usize,
}

impl EditGuard {
    pub fn new(policy: EditPolicy) -> Self {
        Self {
            policy,
            read_paths: BTreeSet::new(),
            pending_reread: BTreeSet::new(),
            edits_this_step: 0,
        }
    }

    pub fn policy(&self) -> &EditPolicy {
        &self.policy
    }

    /// Paths edited whose re-read is still outstanding.
    pub fn pending_reread(&self) -> Vec<&str> {
        self.pending_reread.iter().map(String::as_str).collect()
    }

    /// Close the current step; returns violations that are only visible at a
    /// step boundary, such as an outstanding re-read.
    pub fn end_step(&mut self) -> Vec<EditGuardViolation> {
        let mut violations = Vec::new();
        if self.policy.require_reread_after_edit {
            for path in &self.pending_reread {
                violations.push(EditGuardViolation::new(
                    "reread_after_edit_missing",
                    format!("{path} was edited but not re-read before the step ended"),
                ));
            }
        }
        self.edits_this_step = 0;
        violations
    }

    /// Feed one action through the guard.
    pub fn observe(&mut self, action: &EditAction) -> Vec<EditGuardViolation> {
        let mut violations = Vec::new();
        match action {
            EditAction::Read { path } => {
                self.read_paths.insert(path.clone());
                self.pending_reread.remove(path);
            }
            EditAction::Search { .. } => {}
            EditAction::Edit { path, lines } => {
                let exempt = self.policy.exception_reason(path).is_some();
                if self.policy.require_read_before_edit
                    && !self.read_paths.contains(path)
                    && !exempt
                {
                    violations.push(EditGuardViolation::new(
                        "read_before_edit",
                        format!("{path} was edited without being read first"),
                    ));
                }
                if *lines > self.policy.max_edit_lines && !exempt {
                    violations.push(EditGuardViolation::new(
                        "max_edit_lines",
                        format!(
                            "{path} edit of {lines} lines exceeds max_edit_lines {}",
                            self.policy.max_edit_lines
                        ),
                    ));
                }
                if self.policy.require_reread_after_edit {
                    self.pending_reread.insert(path.clone());
                }
                self.edits_this_step += 1;
                if self.policy.one_logical_change_per_step && self.edits_this_step > 1 && !exempt {
                    violations.push(EditGuardViolation::new(
                        "one_logical_change_per_step",
                        format!(
                            "{path} is edit {} in a step limited to one logical change",
                            self.edits_this_step
                        ),
                    ));
                }
            }
            EditAction::Create { path, lines } => {
                let exempt = self.policy.exception_reason(path).is_some();
                if *lines > self.policy.max_new_file_lines && !exempt {
                    violations.push(EditGuardViolation::new(
                        "max_new_file_lines",
                        format!(
                            "{path} created with {lines} lines, exceeding max_new_file_lines {}",
                            self.policy.max_new_file_lines
                        ),
                    ));
                }
                self.read_paths.insert(path.clone());
                self.edits_this_step += 1;
                // Creating a file is a change like any other: a create plus an
                // edit in one step is still two logical changes.
                if self.policy.one_logical_change_per_step && self.edits_this_step > 1 && !exempt {
                    violations.push(EditGuardViolation::new(
                        "one_logical_change_per_step",
                        format!(
                            "{path} is change {} in a step limited to one logical change",
                            self.edits_this_step
                        ),
                    ));
                }
            }
            EditAction::RunTests { .. } | EditAction::Conclude { .. } => {}
        }
        violations
    }
}

/// Loop shapes that mean an agent has stopped making progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThrashSignal {
    RepeatedReadsWithoutFindings,
    EquivalentSearches,
    IdenticalTestsWithoutChangedInputs,
    RepeatedConclusions,
    EditRevertCycle,
    TokenGrowthWithoutStateTransition,
}

impl ThrashSignal {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThrashSignal::RepeatedReadsWithoutFindings => "repeated_reads_without_findings",
            ThrashSignal::EquivalentSearches => "equivalent_searches",
            ThrashSignal::IdenticalTestsWithoutChangedInputs => {
                "identical_tests_without_changed_inputs"
            }
            ThrashSignal::RepeatedConclusions => "repeated_conclusions",
            ThrashSignal::EditRevertCycle => "edit_revert_cycle",
            ThrashSignal::TokenGrowthWithoutStateTransition => {
                "token_growth_without_state_transition"
            }
        }
    }
}

/// What AAR may do about a detected loop, cheapest intervention first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThrashResponse {
    SummarizeState,
    ShrinkContext,
    CleanFork,
    EscalateBudget,
    EscalateModel,
    CoordinatorIntervention,
}

impl ThrashResponse {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThrashResponse::SummarizeState => "summarize_state",
            ThrashResponse::ShrinkContext => "shrink_context",
            ThrashResponse::CleanFork => "clean_fork",
            ThrashResponse::EscalateBudget => "escalate_budget",
            ThrashResponse::EscalateModel => "escalate_model",
            ThrashResponse::CoordinatorIntervention => "coordinator_intervention",
        }
    }
}

/// One step of agent activity, as the detector sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEvent {
    pub action: EditAction,
    /// The step recorded a new durable finding.
    pub recorded_finding: bool,
    /// Cumulative tokens spent by this agent.
    pub cumulative_tokens: u64,
    /// Digest of durable state; unchanged means no state transition.
    pub state_digest: String,
}

/// A detected loop, its evidence and the suggested response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThrashFinding {
    pub signal: ThrashSignal,
    pub evidence: String,
    pub response: ThrashResponse,
}

/// Counts at which a repetition becomes a loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrashThresholds {
    pub repeated_reads: usize,
    pub equivalent_searches: usize,
    pub identical_tests: usize,
    pub repeated_conclusions: usize,
    pub edit_revert_cycles: usize,
    /// Tokens that may accrue without any state transition.
    pub tokens_without_transition: u64,
}

impl Default for ThrashThresholds {
    fn default() -> Self {
        Self {
            repeated_reads: 3,
            equivalent_searches: 3,
            identical_tests: 2,
            repeated_conclusions: 2,
            edit_revert_cycles: 2,
            tokens_without_transition: 20_000,
        }
    }
}

/// Detects the loop shapes listed in spec section 10.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThrashDetector {
    thresholds: ThrashThresholds,
    reads_without_findings: BTreeMap<String, usize>,
    searches: BTreeMap<String, usize>,
    tests: BTreeMap<String, usize>,
    conclusions: BTreeMap<String, usize>,
    edit_counts: BTreeMap<String, usize>,
    last_state_digest: Option<String>,
    tokens_at_last_transition: u64,
}

impl ThrashDetector {
    pub fn new(thresholds: ThrashThresholds) -> Self {
        Self {
            thresholds,
            reads_without_findings: BTreeMap::new(),
            searches: BTreeMap::new(),
            tests: BTreeMap::new(),
            conclusions: BTreeMap::new(),
            edit_counts: BTreeMap::new(),
            last_state_digest: None,
            tokens_at_last_transition: 0,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(ThrashThresholds::default())
    }

    /// Feed one step; returns every loop signal that fired on it.
    pub fn observe(&mut self, event: &StepEvent) -> Vec<ThrashFinding> {
        let mut findings = Vec::new();

        match &event.action {
            EditAction::Read { path } => {
                if event.recorded_finding {
                    self.reads_without_findings.remove(path);
                } else {
                    let count = self.reads_without_findings.entry(path.clone()).or_insert(0);
                    *count += 1;
                    if *count >= self.thresholds.repeated_reads {
                        findings.push(ThrashFinding {
                            signal: ThrashSignal::RepeatedReadsWithoutFindings,
                            evidence: format!("{path} read {count} times with no finding recorded"),
                            response: ThrashResponse::SummarizeState,
                        });
                    }
                }
            }
            EditAction::Search { query } => {
                let normalized = normalize_query(query);
                let count = self.searches.entry(normalized.clone()).or_insert(0);
                *count += 1;
                if *count >= self.thresholds.equivalent_searches {
                    findings.push(ThrashFinding {
                        signal: ThrashSignal::EquivalentSearches,
                        evidence: format!("equivalent search '{normalized}' issued {count} times"),
                        response: ThrashResponse::ShrinkContext,
                    });
                }
            }
            EditAction::RunTests { command, .. } => {
                let key = format!("{command}|{}", event.state_digest);
                let count = self.tests.entry(key).or_insert(0);
                *count += 1;
                if *count > self.thresholds.identical_tests {
                    findings.push(ThrashFinding {
                        signal: ThrashSignal::IdenticalTestsWithoutChangedInputs,
                        evidence: format!(
                            "'{command}' run {count} times with unchanged inputs"
                        ),
                        response: ThrashResponse::SummarizeState,
                    });
                }
            }
            EditAction::Conclude { claim } => {
                let normalized = normalize_query(claim);
                let count = self.conclusions.entry(normalized.clone()).or_insert(0);
                *count += 1;
                if *count > self.thresholds.repeated_conclusions {
                    findings.push(ThrashFinding {
                        signal: ThrashSignal::RepeatedConclusions,
                        evidence: format!("conclusion '{normalized}' re-proved {count} times"),
                        response: ThrashResponse::CleanFork,
                    });
                }
            }
            EditAction::Edit { path, .. } => {
                let count = self.edit_counts.entry(path.clone()).or_insert(0);
                *count += 1;
                if *count > self.thresholds.edit_revert_cycles * 2 {
                    findings.push(ThrashFinding {
                        signal: ThrashSignal::EditRevertCycle,
                        evidence: format!("{path} edited {count} times in one execution"),
                        response: ThrashResponse::EscalateBudget,
                    });
                }
            }
            EditAction::Create { .. } => {}
        }

        let transitioned = self
            .last_state_digest
            .as_deref()
            .is_none_or(|digest| digest != event.state_digest);
        if transitioned {
            self.last_state_digest = Some(event.state_digest.clone());
            self.tokens_at_last_transition = event.cumulative_tokens;
        } else {
            let spent = event
                .cumulative_tokens
                .saturating_sub(self.tokens_at_last_transition);
            if spent >= self.thresholds.tokens_without_transition {
                findings.push(ThrashFinding {
                    signal: ThrashSignal::TokenGrowthWithoutStateTransition,
                    evidence: format!("{spent} tokens spent without a state transition"),
                    response: ThrashResponse::CoordinatorIntervention,
                });
            }
        }

        findings
    }
}

fn normalize_query(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// When an execution must stop, successfully or otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPolicy {
    pub max_steps: u32,
    pub max_retries: u32,
    pub max_tokens: u64,
    /// Stop as soon as every acceptance criterion is satisfied.
    pub stop_on_acceptance_met: bool,
    pub stop_on_thrashing: bool,
}

impl Default for StopPolicy {
    fn default() -> Self {
        Self {
            max_steps: 60,
            max_retries: 3,
            max_tokens: 400_000,
            stop_on_acceptance_met: true,
            stop_on_thrashing: true,
        }
    }
}

/// Why an execution stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    AcceptanceMet,
    MaxSteps,
    MaxRetries,
    MaxTokens,
    Thrashing,
    Blocked,
}

impl StopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            StopReason::AcceptanceMet => "acceptance_met",
            StopReason::MaxSteps => "max_steps",
            StopReason::MaxRetries => "max_retries",
            StopReason::MaxTokens => "max_tokens",
            StopReason::Thrashing => "thrashing",
            StopReason::Blocked => "blocked",
        }
    }

    /// True when stopping here means the work succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self, StopReason::AcceptanceMet)
    }
}

/// Execution progress as the stop evaluator sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionProgress {
    pub steps: u32,
    pub retries: u32,
    pub tokens: u64,
    pub acceptance_criteria_total: u32,
    pub acceptance_criteria_met: u32,
    pub unresolved_thrash_signals: u32,
    pub blocked_reason: Option<String>,
}

impl ExecutionProgress {
    pub fn acceptance_met(&self) -> bool {
        self.acceptance_criteria_total > 0
            && self.acceptance_criteria_met >= self.acceptance_criteria_total
    }
}

/// Decide whether execution must stop now.
///
/// Acceptance is checked first: an agent that has met every criterion must stop
/// even if it has budget left, which is the rule agents most often ignore.
pub fn evaluate_stop(policy: &StopPolicy, progress: &ExecutionProgress) -> Option<StopReason> {
    if policy.stop_on_acceptance_met && progress.acceptance_met() {
        return Some(StopReason::AcceptanceMet);
    }
    if progress.blocked_reason.is_some() {
        return Some(StopReason::Blocked);
    }
    if progress.retries > policy.max_retries {
        return Some(StopReason::MaxRetries);
    }
    if progress.steps >= policy.max_steps {
        return Some(StopReason::MaxSteps);
    }
    if progress.tokens >= policy.max_tokens {
        return Some(StopReason::MaxTokens);
    }
    if policy.stop_on_thrashing && progress.unresolved_thrash_signals > 0 {
        return Some(StopReason::Thrashing);
    }
    None
}
