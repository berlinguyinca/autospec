//! Provider-neutral executor abstraction — `docs/specs/2026-08-16-multi-model-engineering-team-design.md` §16.
//!
//! Every model is accessed through one common execution contract,
//! `dispatch(request) -> result`. Orchestration code programs against
//! [`Executor`] only: a local Qwen runtime and a cloud API are
//! interchangeable behind the same interface, and nothing in orchestration
//! special-cases a local implementation (spec §16, "Do not special-case local
//! implementations throughout orchestration code").
//!
//! The inherited hard constraints of the multi-model epic bind this module:
//!
//! - **`unknown` over fabricated values** (spec §25): every metric a provider
//!   cannot report is `None` in [`ExecutorResult`] (serialized as `null`).
//!   A result that fabricates a metric — `NaN` rates, a cache ratio above its
//!   input — fails validation instead of entering the ledger.
//! - **Fail closed** (spec §15): an invalid request, an unknown provider, or
//!   an ambiguous provider failure surfaces as [`ExecutorError`]. There is no
//!   silent fallback to another provider, and an error is never reported as a
//!   healthy result.
//! - **Backward compatibility** (spec §52): a registry holding exactly one
//!   provider is a supported configuration, not a special case —
//!   single-provider cloud-only operation keeps working unchanged.
//!
//! Role vocabulary is the 14 snake_case names fixed by
//! `docs/decisions/0001-as-aeo-001-phase-0-integration-strategy.md` D2.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The 14 snake_case engineering roles (ADR 0001 D2).
///
/// The vocabulary is closed: a role outside these 14 names is a validation
/// error, never folded into an `other` bucket that would quietly widen the
/// role set per deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "orchestrator")]
    Orchestrator,
    #[serde(rename = "planner")]
    Planner,
    #[serde(rename = "architect")]
    Architect,
    #[serde(rename = "test_planner")]
    TestPlanner,
    #[serde(rename = "implementer")]
    Implementer,
    #[serde(rename = "code_reviewer")]
    CodeReviewer,
    #[serde(rename = "test_reviewer")]
    TestReviewer,
    #[serde(rename = "qa_verifier")]
    QaVerifier,
    #[serde(rename = "documentation_writer")]
    DocumentationWriter,
    #[serde(rename = "documentation_reviewer")]
    DocumentationReviewer,
    #[serde(rename = "ui_ux_reviewer")]
    UiUxReviewer,
    #[serde(rename = "security_reviewer")]
    SecurityReviewer,
    #[serde(rename = "researcher")]
    Researcher,
    #[serde(rename = "advisor")]
    Advisor,
}

impl Role {
    /// All 14 roles, in the fixed ADR D2 order.
    pub const ALL: [Role; 14] = [
        Role::Orchestrator,
        Role::Planner,
        Role::Architect,
        Role::TestPlanner,
        Role::Implementer,
        Role::CodeReviewer,
        Role::TestReviewer,
        Role::QaVerifier,
        Role::DocumentationWriter,
        Role::DocumentationReviewer,
        Role::UiUxReviewer,
        Role::SecurityReviewer,
        Role::Researcher,
        Role::Advisor,
    ];

    /// The snake_case wire name, e.g. `"ui_ux_reviewer"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Orchestrator => "orchestrator",
            Role::Planner => "planner",
            Role::Architect => "architect",
            Role::TestPlanner => "test_planner",
            Role::Implementer => "implementer",
            Role::CodeReviewer => "code_reviewer",
            Role::TestReviewer => "test_reviewer",
            Role::QaVerifier => "qa_verifier",
            Role::DocumentationWriter => "documentation_writer",
            Role::DocumentationReviewer => "documentation_reviewer",
            Role::UiUxReviewer => "ui_ux_reviewer",
            Role::SecurityReviewer => "security_reviewer",
            Role::Researcher => "researcher",
            Role::Advisor => "advisor",
        }
    }

    /// Parses a snake_case role name. Unknown names are a fail-closed error.
    pub fn parse(name: &str) -> Result<Role, String> {
        Role::ALL
            .iter()
            .copied()
            .find(|role| role.as_str() == name)
            .ok_or_else(|| format!("unknown role: {name}"))
    }
}

/// One dispatch request against the common executor contract (spec §16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutorRequest {
    /// The work item identifier this dispatch serves.
    pub work_item: String,
    /// The engineering role the dispatch fills.
    pub role: Role,
    /// The routing-ledger `dispatch_kind` for this dispatch.
    pub dispatch_kind: String,
    /// Concrete model id to dispatch.
    pub model: String,
    /// Provider name; the registry resolves it to exactly one executor.
    pub provider: String,
    /// Token budget for the dispatch context; `None` lets the provider use
    /// its own default.
    #[serde(default)]
    pub context_budget: Option<u64>,
    /// Tool names the dispatch may use.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Workspace the dispatch operates in.
    pub workspace: String,
    /// The acceptance contract the dispatch is judged against.
    pub acceptance_criteria: String,
    /// Wall-clock limit in seconds; `None` means the provider default.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl ExecutorRequest {
    /// Fails closed on any field that would make the dispatch unjudgeable:
    /// a request naming no model, no provider, or no acceptance criteria
    /// cannot produce a result the ledger could score.
    pub fn validate(&self) -> Result<(), String> {
        if self.work_item.trim().is_empty() {
            return Err("work_item must not be empty".into());
        }
        if self.dispatch_kind.trim().is_empty() {
            return Err("dispatch_kind must not be empty".into());
        }
        if self.model.trim().is_empty() {
            return Err("model must not be empty".into());
        }
        if self.provider.trim().is_empty() {
            return Err("provider must not be empty".into());
        }
        if self.workspace.trim().is_empty() {
            return Err("workspace must not be empty".into());
        }
        if self.acceptance_criteria.trim().is_empty() {
            return Err("acceptance_criteria must not be empty".into());
        }
        for tool in &self.tools {
            if tool.trim().is_empty() {
                return Err("tools must not contain empty entries".into());
            }
        }
        Ok(())
    }
}

/// Outcome class of a dispatch (spec §16 `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Completed,
    Failed,
    TimedOut,
}

/// Why a dispatch did not complete.
///
/// `Unknown` is a first-class class, not a hole: when a provider fails
/// opaquely, the honest class is "unknown" rather than a guessed one
/// (spec §25: unknown over fabricated values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// The provider endpoint was unreachable or its state was ambiguous.
    /// Fail closed: never re-routed or reported healthy by the executor.
    ProviderUnavailable,
    /// The provider answered and reported a failure.
    ProviderError,
    /// The dispatch hit its wall-clock limit.
    Timeout,
    /// The provider failed with no classifiable cause.
    Unknown,
}

/// Result of one dispatch (spec §16).
///
/// Every metric a provider cannot report is `None` — serialized as `null`.
/// `None` is the "unknown" the routing ledger requires; a `0` or `false`
/// where the provider has no measurement would read like a measurement and
/// poison every derived weight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutorResult {
    pub status: ExecutionStatus,
    /// The dispatch's textual output (transcript tail, summary, error).
    pub output: String,
    /// The produced patch, when the dispatch produced one.
    #[serde(default)]
    pub patch: Option<String>,
    /// Tokens consumed; `None` when the provider cannot report usage.
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// Prompt-cache hits; a subset of `input_tokens`.
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    /// Prefill throughput in tokens/second; `None` when unmeasurable.
    #[serde(default)]
    pub prompt_tok_s: Option<f64>,
    /// Decode throughput in tokens/second; `None` when unmeasurable.
    #[serde(default)]
    pub decode_tok_s: Option<f64>,
    /// Time to first token in milliseconds; `None` when unmeasurable.
    #[serde(default)]
    pub ttft_ms: Option<u64>,
    /// Total wall clock in milliseconds; `None` when unmeasurable.
    #[serde(default)]
    pub wall_clock_ms: Option<u64>,
    /// Number of tool calls made; `None` when the provider does not count.
    #[serde(default)]
    pub tool_calls: Option<u64>,
    /// Present exactly when `status` is not `Completed`.
    #[serde(default)]
    pub failure_class: Option<FailureClass>,
}

impl ExecutorResult {
    /// Rejects results that could not be true:
    ///
    /// - a failure with no class (an unclassified failure is not actionable
    ///   and must not be recorded as if it were a clean outcome);
    /// - a completion carrying a failure class;
    /// - a cache count above the input count (the caller double-counted);
    /// - a non-finite or negative rate (a fabricated throughput).
    pub fn validate(&self) -> Result<(), String> {
        match self.status {
            ExecutionStatus::Completed => {
                if self.failure_class.is_some() {
                    return Err("completed result must not carry a failure_class".into());
                }
            }
            ExecutionStatus::Failed | ExecutionStatus::TimedOut => {
                if self.failure_class.is_none() {
                    return Err("failed or timed-out result must carry a failure_class".into());
                }
            }
        }
        if let (Some(cached), Some(input)) = (self.cached_tokens, self.input_tokens) {
            if cached > input {
                return Err("cached_tokens may not exceed input_tokens".into());
            }
        }
        for (name, rate) in [
            ("prompt_tok_s", self.prompt_tok_s),
            ("decode_tok_s", self.decode_tok_s),
        ] {
            if let Some(rate) = rate {
                if !rate.is_finite() || rate < 0.0 {
                    return Err(format!("{name} must be a finite non-negative rate"));
                }
            }
        }
        Ok(())
    }
}

/// Error class of the common executor contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorError {
    /// The request failed validation before any provider was touched.
    InvalidRequest(String),
    /// The provider returned a result that failed validation.
    InvalidResult(String),
    /// No executor is registered under that provider name. There is no
    /// fallback: dispatching to a provider nobody wired is a hard stop.
    UnknownProvider(String),
    /// The provider itself failed; the cause is the provider's own.
    Backend { provider: String, message: String },
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutorError::InvalidRequest(message) => write!(f, "invalid request: {message}"),
            ExecutorError::InvalidResult(message) => write!(f, "invalid result: {message}"),
            ExecutorError::UnknownProvider(provider) => write!(f, "unknown provider: {provider}"),
            ExecutorError::Backend { provider, message } => {
                write!(f, "provider {provider} failed: {message}")
            }
        }
    }
}

impl std::error::Error for ExecutorError {}

/// The common execution contract (spec §16).
///
/// Implementations are the only place a provider's concrete API may appear.
/// Orchestration never names a provider type; it holds a registry and calls
/// [`ExecutorRegistry::dispatch`].
pub trait Executor {
    /// The provider name this executor answers for; must be non-empty and
    /// unique within a registry.
    fn provider(&self) -> &str;

    /// Runs one dispatch. Errors are fail-closed: an ambiguous provider
    /// state is an `Err`, never an `Ok` with a guessed status.
    fn dispatch(&self, request: &ExecutorRequest) -> Result<ExecutorResult, ExecutorError>;
}

/// Resolves provider names to executors.
///
/// Provider identity is exact and unambiguous: a registry never holds two
/// executors under one name, and a dispatch never resolves to more than one.
pub struct ExecutorRegistry {
    executors: BTreeMap<String, Box<dyn Executor>>,
}

impl Default for ExecutorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutorRegistry {
    pub fn new() -> Self {
        Self {
            executors: BTreeMap::new(),
        }
    }

    /// Registers an executor under its own provider name.
    ///
    /// Rejects an empty name and a duplicate name: two executors claiming
    /// one provider would make routing to that provider ambiguous, and
    /// ambiguity is a fail-closed condition, not a coin flip.
    pub fn register(&mut self, executor: Box<dyn Executor>) -> Result<(), ExecutorError> {
        let name = executor.provider();
        if name.trim().is_empty() {
            return Err(ExecutorError::InvalidRequest(
                "provider name must not be empty".into(),
            ));
        }
        if self.executors.contains_key(name) {
            return Err(ExecutorError::InvalidRequest(format!(
                "provider already registered: {name}"
            )));
        }
        self.executors.insert(name.to_string(), executor);
        Ok(())
    }

    /// Registered provider names, sorted for deterministic output.
    pub fn providers(&self) -> Vec<&str> {
        self.executors.keys().map(String::as_str).collect()
    }

    /// The executor registered under `provider`, if any.
    pub fn get(&self, provider: &str) -> Option<&dyn Executor> {
        self.executors.get(provider).map(|boxed| boxed.as_ref())
    }

    /// Validates the request, resolves the provider, dispatches, and
    /// validates the result. Every failure mode is an error: this method
    /// never retries another provider and never papers over a provider
    /// error as a completed dispatch.
    pub fn dispatch(
        &self,
        provider: &str,
        request: &ExecutorRequest,
    ) -> Result<ExecutorResult, ExecutorError> {
        if let Err(message) = request.validate() {
            return Err(ExecutorError::InvalidRequest(message));
        }
        let executor = self
            .get(provider)
            .ok_or_else(|| ExecutorError::UnknownProvider(provider.to_string()))?;
        let result = executor.dispatch(request)?;
        if let Err(message) = result.validate() {
            return Err(ExecutorError::InvalidResult(message));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ExecutorRequest {
        ExecutorRequest {
            work_item: "ISSUE-1".into(),
            role: Role::Implementer,
            dispatch_kind: "implementer".into(),
            model: "qwen3-32b".into(),
            provider: "local".into(),
            context_budget: Some(32_000),
            tools: vec!["edit".into()],
            workspace: ".autospec/wt-issue-1".into(),
            acceptance_criteria: "cargo test -p autospec-core passes".into(),
            timeout_secs: Some(600),
        }
    }

    fn completed() -> ExecutorResult {
        ExecutorResult {
            status: ExecutionStatus::Completed,
            output: "done".into(),
            patch: None,
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
            prompt_tok_s: None,
            decode_tok_s: None,
            ttft_ms: None,
            wall_clock_ms: None,
            tool_calls: None,
            failure_class: None,
        }
    }

    /// A provider that reports nothing it cannot measure.
    struct Stub {
        name: &'static str,
        outcome: Result<ExecutorResult, ExecutorError>,
        /// Set when this executor is actually invoked.
        invoked: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }

    impl Executor for Stub {
        fn provider(&self) -> &str {
            self.name
        }
        fn dispatch(&self, _request: &ExecutorRequest) -> Result<ExecutorResult, ExecutorError> {
            if let Some(flag) = &self.invoked {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            self.outcome.clone()
        }
    }

    fn stub(name: &'static str, outcome: Result<ExecutorResult, ExecutorError>) -> Stub {
        Stub {
            name,
            outcome,
            invoked: None,
        }
    }

    #[test]
    fn role_vocabulary_is_exactly_the_14_adr_d2_names() {
        let names: Vec<&str> = Role::ALL.iter().map(|role| role.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "orchestrator",
                "planner",
                "architect",
                "test_planner",
                "implementer",
                "code_reviewer",
                "test_reviewer",
                "qa_verifier",
                "documentation_writer",
                "documentation_reviewer",
                "ui_ux_reviewer",
                "security_reviewer",
                "researcher",
                "advisor"
            ]
        );
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Ok(role));
            let wire = serde_json::to_string(&role).unwrap();
            assert_eq!(wire, format!("\"{}\"", role.as_str()));
        }
        assert!(Role::parse("chief_scientist").is_err());
        assert!(Role::parse("Implementer").is_err());
    }

    #[test]
    fn request_rejects_fields_that_make_dispatch_unjudgeable() {
        let mut r = request();
        assert!(r.validate().is_ok());

        r.model = " ".into();
        assert!(r.validate().is_err());

        let mut r = request();
        r.acceptance_criteria = String::new();
        assert!(r.validate().is_err());

        let mut r = request();
        r.tools.push(String::new());
        assert!(r.validate().is_err());
    }

    #[test]
    fn unknown_metrics_serialize_as_null_not_fabricated_values() {
        let value = serde_json::to_value(completed()).unwrap();
        // Unknown is recorded, not omitted: the key is present as an
        // explicit null so a ledger reader can tell "not reported" from
        // "field predates this schema version".
        assert_eq!(value.get("input_tokens"), Some(&serde_json::Value::Null));
        assert_eq!(value.get("decode_tok_s"), Some(&serde_json::Value::Null));
        assert_eq!(value.get("failure_class"), Some(&serde_json::Value::Null));
        assert_eq!(value["status"], "completed");
        // And a pre-extension record without the keys deserializes as None,
        // keeping old ledger rows readable (spec §52).
        let legacy: ExecutorResult = serde_json::from_value(serde_json::json!({
            "status": "completed",
            "output": "done"
        }))
        .unwrap();
        assert_eq!(legacy.input_tokens, None);
        assert_eq!(legacy.failure_class, None);
    }

    #[test]
    fn result_validation_is_fail_closed() {
        let mut r = completed();
        r.status = ExecutionStatus::Failed;
        assert_eq!(
            r.validate(),
            Err("failed or timed-out result must carry a failure_class".into())
        );

        r.failure_class = Some(FailureClass::Unknown);
        assert!(r.validate().is_ok());

        r.status = ExecutionStatus::Completed;
        assert_eq!(
            r.validate(),
            Err("completed result must not carry a failure_class".into())
        );

        let mut r = completed();
        r.input_tokens = Some(100);
        r.cached_tokens = Some(101);
        assert!(r.validate().is_err());

        let mut r = completed();
        r.decode_tok_s = Some(f64::NAN);
        assert!(r.validate().is_err());
    }

    #[test]
    fn registry_dispatch_routes_and_validates() {
        let mut registry = ExecutorRegistry::new();
        let done = completed();
        registry
            .register(Box::new(stub("local", Ok(done.clone()))))
            .unwrap();

        let mut r = request();
        r.provider = "local".into();
        assert_eq!(registry.dispatch("local", &r), Ok(done));
    }

    #[test]
    fn registry_fails_closed_on_unknown_provider_and_duplicates() {
        let mut registry = ExecutorRegistry::new();
        registry
            .register(Box::new(stub("local", Ok(completed()))))
            .unwrap();

        let duplicate = registry.register(Box::new(stub("local", Ok(completed()))));
        assert!(matches!(duplicate, Err(ExecutorError::InvalidRequest(_))));

        let empty = registry.register(Box::new(stub(" ", Ok(completed()))));
        assert!(matches!(empty, Err(ExecutorError::InvalidRequest(_))));

        let r = request();
        let missing = registry.dispatch("cloud", &r);
        assert!(matches!(missing, Err(ExecutorError::UnknownProvider(_))));
        assert_eq!(missing.unwrap_err().to_string(), "unknown provider: cloud");
    }

    #[test]
    fn registry_rejects_bad_requests_and_fabricated_results() {
        let mut registry = ExecutorRegistry::new();
        registry
            .register(Box::new(stub("local", Ok(completed()))))
            .unwrap();

        let mut r = request();
        r.model = String::new();
        assert!(matches!(
            registry.dispatch("local", &r),
            Err(ExecutorError::InvalidRequest(_))
        ));

        let mut unclassified = completed();
        unclassified.status = ExecutionStatus::TimedOut;
        let mut registry = ExecutorRegistry::new();
        registry
            .register(Box::new(stub("local", Ok(unclassified))))
            .unwrap();
        let r = request();
        assert!(matches!(
            registry.dispatch("local", &r),
            Err(ExecutorError::InvalidResult(_))
        ));
    }

    #[test]
    fn provider_failure_propagates_with_no_silent_fallback() {
        let cloud_invoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut registry = ExecutorRegistry::new();
        registry
            .register(Box::new(Stub {
                name: "local",
                outcome: Err(ExecutorError::Backend {
                    provider: "local".into(),
                    message: "GPU state ambiguous".into(),
                }),
                invoked: None,
            }))
            .unwrap();
        registry
            .register(Box::new(Stub {
                name: "cloud",
                outcome: Ok(completed()),
                invoked: Some(std::sync::Arc::clone(&cloud_invoked)),
            }))
            .unwrap();

        let r = request();
        let err = registry.dispatch("local", &r).unwrap_err();
        assert!(matches!(err, ExecutorError::Backend { .. }));
        assert_eq!(
            err.to_string(),
            "provider local failed: GPU state ambiguous"
        );
        // The cloud executor exists but must not be invoked on the local
        // failure: a silent GPU->cloud fallback is exactly what spec §15
        // forbids.
        assert!(!cloud_invoked.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn single_provider_registry_is_the_back_compatible_configuration() {
        let mut registry = ExecutorRegistry::new();
        let mut done = completed();
        done.input_tokens = Some(1_000);
        registry
            .register(Box::new(stub("cloud", Ok(done.clone()))))
            .unwrap();
        assert_eq!(registry.providers(), vec!["cloud"]);

        let mut r = request();
        r.provider = "cloud".into();
        r.context_budget = None;
        r.timeout_secs = None;
        assert_eq!(registry.dispatch("cloud", &r), Ok(done));
    }
}
