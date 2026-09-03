//! Execution telemetry (AAR spec section 14).
//!
//! One record per execution, versioned, with the token accounting split three
//! ways — total prompt, cached, newly prefilled — because a cache-hit rate that
//! cannot be computed from the record is a cache-hit rate nobody will compute.
//! Free-text fields are redactable before the record leaves the worktree.

use serde::Serialize;

/// Bumped whenever a persisted field changes meaning or disappears.
pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;

/// Placeholder written over redacted free text.
pub const REDACTED: &str = "[redacted]";

/// How an execution ended, when it did not succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    None,
    TestsFailed,
    ReviewRejected,
    GuardViolation,
    Thrashing,
    ContextExhausted,
    ProviderUnavailable,
    QuotaExhausted,
    HarnessError,
    Blocked,
}

impl FailureCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureCategory::None => "none",
            FailureCategory::TestsFailed => "tests_failed",
            FailureCategory::ReviewRejected => "review_rejected",
            FailureCategory::GuardViolation => "guard_violation",
            FailureCategory::Thrashing => "thrashing",
            FailureCategory::ContextExhausted => "context_exhausted",
            FailureCategory::ProviderUnavailable => "provider_unavailable",
            FailureCategory::QuotaExhausted => "quota_exhausted",
            FailureCategory::HarnessError => "harness_error",
            FailureCategory::Blocked => "blocked",
        }
    }
}

/// Outcome of the independent review, if one ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    NotRun,
    Approved,
    Rejected,
}

impl ReviewOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewOutcome::NotRun => "not_run",
            ReviewOutcome::Approved => "approved",
            ReviewOutcome::Rejected => "rejected",
        }
    }
}

/// One execution's telemetry record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionTelemetry {
    pub schema_version: u32,
    pub task_id: String,
    pub spec_id: String,
    pub plan_id: String,
    pub repository: String,
    pub base_revision: String,
    pub role: String,
    pub harness: String,
    pub model_id: String,
    pub model_version: String,
    pub quantization: String,
    pub provider: String,
    pub backend: String,
    pub node_id: String,
    pub gpu_class: String,
    pub sampling_profile: String,
    pub reasoning_budget: String,
    pub policy_version: String,

    pub prompt_tokens: u64,
    pub cached_prompt_tokens: u64,
    pub new_prefill_tokens: u64,
    pub reasoning_tokens: u64,
    pub output_tokens: u64,
    pub prefill_tokens_per_second: f64,
    pub decode_tokens_per_second: f64,

    pub tool_calls: u32,
    pub files_read: u32,
    pub files_edited: u32,
    pub lines_added: u64,
    pub lines_removed: u64,

    pub queue_ms: u64,
    pub wall_ms: u64,

    pub tests_run: u32,
    pub tests_passed: u32,
    pub retries: u32,
    pub review_outcome: ReviewOutcome,
    pub success: bool,
    pub failure_category: FailureCategory,
    /// Free text; redacted before export.
    pub failure_detail: String,

    pub estimated_cost_micros: u64,
    pub actual_cost_micros: u64,
}

impl Default for ExecutionTelemetry {
    fn default() -> Self {
        Self {
            schema_version: TELEMETRY_SCHEMA_VERSION,
            task_id: String::new(),
            spec_id: String::new(),
            plan_id: String::new(),
            repository: String::new(),
            base_revision: String::new(),
            role: String::new(),
            harness: String::new(),
            model_id: String::new(),
            model_version: String::new(),
            quantization: String::new(),
            provider: String::new(),
            backend: String::new(),
            node_id: String::new(),
            gpu_class: String::new(),
            sampling_profile: String::new(),
            reasoning_budget: String::new(),
            policy_version: String::new(),
            prompt_tokens: 0,
            cached_prompt_tokens: 0,
            new_prefill_tokens: 0,
            reasoning_tokens: 0,
            output_tokens: 0,
            prefill_tokens_per_second: 0.0,
            decode_tokens_per_second: 0.0,
            tool_calls: 0,
            files_read: 0,
            files_edited: 0,
            lines_added: 0,
            lines_removed: 0,
            queue_ms: 0,
            wall_ms: 0,
            tests_run: 0,
            tests_passed: 0,
            retries: 0,
            review_outcome: ReviewOutcome::NotRun,
            success: false,
            failure_category: FailureCategory::None,
            failure_detail: String::new(),
            estimated_cost_micros: 0,
            actual_cost_micros: 0,
        }
    }
}

impl ExecutionTelemetry {
    /// Fraction of the prompt served from cache.
    pub fn cache_hit_rate(&self) -> f64 {
        if self.prompt_tokens == 0 {
            return 0.0;
        }
        self.cached_prompt_tokens as f64 / self.prompt_tokens as f64
    }

    /// Reject records whose token accounting cannot be true.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version == 0 {
            return Err("telemetry schema_version must be set".to_string());
        }
        if self.cached_prompt_tokens + self.new_prefill_tokens != self.prompt_tokens {
            return Err(format!(
                "cached_prompt_tokens {} + new_prefill_tokens {} != prompt_tokens {}",
                self.cached_prompt_tokens, self.new_prefill_tokens, self.prompt_tokens
            ));
        }
        if self.tests_passed > self.tests_run {
            return Err(format!(
                "tests_passed {} exceeds tests_run {}",
                self.tests_passed, self.tests_run
            ));
        }
        if self.success && self.failure_category != FailureCategory::None {
            return Err(format!(
                "successful execution carries failure category {}",
                self.failure_category.as_str()
            ));
        }
        if !self.success && self.failure_category == FailureCategory::None {
            return Err("failed execution must carry a failure category".to_string());
        }
        Ok(())
    }

    /// Drop free text that could carry repository or user content.
    pub fn redacted(mut self) -> Self {
        if !self.failure_detail.is_empty() {
            self.failure_detail = REDACTED.to_string();
        }
        self
    }

    /// One JSONL line.
    pub fn to_json_line(&self) -> Result<String, String> {
        let mut line = serde_json::to_string(self).map_err(|error| error.to_string())?;
        line.push('\n');
        Ok(line)
    }
}

/// Relative path of the telemetry log inside a worktree.
pub const TELEMETRY_LOG: &str = ".autospec/telemetry/executions.jsonl";
