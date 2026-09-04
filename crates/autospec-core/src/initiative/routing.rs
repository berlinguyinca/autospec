//! Role-aware model routing with auditable fallback.
//!
//! Roles declare capabilities, not providers. The orchestrator narrows the
//! catalog to eligible models, checks usable quota and capacity, selects the
//! preferred survivor, and records every rejection so the decision can be
//! audited afterwards. Fallback narrows the model set; it never relaxes the
//! separation of duties rules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ids::{AttemptId, InitiativeId, TaskId};
use super::roles::{AgentRole, SessionIdentity};

/// How capable a model is, coarsely, so policies survive model churn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelClass {
    /// Local or cost-optimized models.
    Small,
    /// General purpose models.
    Standard,
    /// Strong coding and reasoning models.
    High,
    /// The strongest available models.
    Frontier,
}

impl ModelClass {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelClass::Small => "small",
            ModelClass::Standard => "standard",
            ModelClass::High => "high",
            ModelClass::Frontier => "frontier",
        }
    }
}

/// How exposed a model's inference is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyTier {
    /// Runs on hardware the operator controls.
    Local,
    /// A remote provider under a data processing agreement.
    Private,
    /// A remote provider with no such agreement.
    Public,
}

/// One model the orchestrator may route to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// The provider-qualified model identifier.
    pub id: String,
    /// The provider serving it, local or remote.
    pub provider: String,
    /// Coarse capability class.
    pub class: ModelClass,
    /// Whether the model accepts images.
    pub vision: bool,
    /// Whether the model supports tool calls.
    pub tools: bool,
    /// Usable context window in tokens.
    pub context_tokens: u32,
    /// Whether inference runs on operator hardware.
    pub local: bool,
    /// How exposed inference is.
    pub privacy: PrivacyTier,
    /// Cost per 1000 tokens in millicents; integer to keep money off floats.
    pub cost_per_1k_millicents: u32,
}

/// What a role needs from a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRequirements {
    /// The capability class label recorded in the invocation contract.
    pub capability_class: String,
    /// The weakest model class the role tolerates.
    pub minimum_class: ModelClass,
    /// Whether the role needs image input.
    pub requires_vision: bool,
    /// Whether the role needs tool calls.
    pub requires_tools: bool,
    /// The smallest usable context window.
    pub min_context_tokens: u32,
    /// Whether inference must stay on operator hardware.
    pub local_only: bool,
    /// The most exposed privacy tier the role tolerates.
    pub max_privacy: PrivacyTier,
    /// A cost ceiling per 1000 tokens in millicents.
    #[serde(default)]
    pub max_cost_per_1k_millicents: Option<u32>,
    /// Whether the orchestrator may fall back past the preferred model.
    pub fallback_allowed: bool,
}

impl RoleRequirements {
    /// The default requirements for `role`.
    pub fn for_role(role: AgentRole) -> Self {
        let base = Self {
            capability_class: "reasoning-high".to_string(),
            minimum_class: ModelClass::High,
            requires_vision: false,
            requires_tools: true,
            min_context_tokens: 128_000,
            local_only: false,
            max_privacy: PrivacyTier::Private,
            max_cost_per_1k_millicents: None,
            fallback_allowed: true,
        };
        match role {
            AgentRole::Architect | AgentRole::SpecAuthor => base,
            AgentRole::TaskPlanner => RoleRequirements {
                capability_class: "reasoning-standard".to_string(),
                minimum_class: ModelClass::Standard,
                min_context_tokens: 64_000,
                ..base
            },
            AgentRole::Implementer => RoleRequirements {
                capability_class: "coding-high".to_string(),
                ..base
            },
            AgentRole::TestEngineer => RoleRequirements {
                capability_class: "coding-standard".to_string(),
                minimum_class: ModelClass::Standard,
                ..base
            },
            AgentRole::UxReviewer => RoleRequirements {
                capability_class: "vision-review".to_string(),
                requires_vision: true,
                ..base
            },
            AgentRole::Reviewer => RoleRequirements {
                capability_class: "review-high".to_string(),
                ..base
            },
            // Verification reads requirements and evidence; it never falls back
            // to a model too weak to hold both.
            AgentRole::SpecVerifier => RoleRequirements {
                capability_class: "verification-high".to_string(),
                minimum_class: ModelClass::High,
                min_context_tokens: 200_000,
                ..base
            },
        }
    }
}

/// Usable quota and capacity for one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    /// Tokens the operator may still spend on this model.
    pub remaining_tokens: u64,
    /// Whether the provider is currently serving requests.
    pub capacity_available: bool,
}

impl Quota {
    /// Quota that permits `remaining_tokens` of work.
    pub fn available(remaining_tokens: u64) -> Self {
        Self {
            remaining_tokens,
            capacity_available: true,
        }
    }

    /// Quota that is exhausted.
    pub fn exhausted() -> Self {
        Self {
            remaining_tokens: 0,
            capacity_available: true,
        }
    }

    /// Quota that exists but has no serving capacity.
    pub fn at_capacity() -> Self {
        Self {
            remaining_tokens: u64::MAX,
            capacity_available: false,
        }
    }
}

/// Quota state for the whole catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaState {
    /// Quota by model id; an absent model is treated as unavailable.
    #[serde(default)]
    pub models: BTreeMap<String, Quota>,
}

impl QuotaState {
    /// Quota state where every listed model is available.
    pub fn all_available(models: &[&str], remaining_tokens: u64) -> Self {
        Self {
            models: models
                .iter()
                .map(|id| ((*id).to_string(), Quota::available(remaining_tokens)))
                .collect(),
        }
    }

    /// Record quota for one model.
    pub fn set(&mut self, model: impl Into<String>, quota: Quota) -> &mut Self {
        self.models.insert(model.into(), quota);
        self
    }
}

/// Why a model was not selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    /// Weaker than the role's minimum class.
    BelowMinimumClass,
    /// The role needs image input.
    NoVision,
    /// The role needs tool calls.
    NoTools,
    /// The context window is too small for the role.
    ContextTooSmall,
    /// The role requires local inference.
    NotLocal,
    /// The model is more exposed than the role tolerates.
    PrivacyTooOpen,
    /// The model costs more than the role's ceiling.
    TooExpensive,
    /// No usable quota remains.
    QuotaExhausted,
    /// The provider has no serving capacity.
    NoCapacity,
    /// The orchestrator holds no quota record for the model.
    QuotaUnknown,
}

impl RejectionReason {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            RejectionReason::BelowMinimumClass => "below_minimum_class",
            RejectionReason::NoVision => "no_vision",
            RejectionReason::NoTools => "no_tools",
            RejectionReason::ContextTooSmall => "context_too_small",
            RejectionReason::NotLocal => "not_local",
            RejectionReason::PrivacyTooOpen => "privacy_too_open",
            RejectionReason::TooExpensive => "too_expensive",
            RejectionReason::QuotaExhausted => "quota_exhausted",
            RejectionReason::NoCapacity => "no_capacity",
            RejectionReason::QuotaUnknown => "quota_unknown",
        }
    }
}

/// One model the router considered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// The model identifier.
    pub model: String,
    /// Whether it survived every check.
    pub eligible: bool,
    /// Why it did not, when it did not.
    #[serde(default)]
    pub rejection: Option<RejectionReason>,
}

/// The auditable record of one routing decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// The role the model was selected for.
    pub role: AgentRole,
    /// The task, when the dispatch is task-scoped.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// The capability class the role asked for.
    pub capability_class: String,
    /// The model selected.
    pub selected: String,
    /// How many preferred models were skipped before this one.
    pub fallback_depth: usize,
    /// Every model considered, in preference order.
    pub considered: Vec<Candidate>,
}

impl RoutingDecision {
    /// Whether the selection fell back past the preferred model.
    pub fn used_fallback(&self) -> bool {
        self.fallback_depth > 0
    }

    /// The session identity this decision dispatches into.
    ///
    /// The session name is derived from role and attempt, so a fallback model
    /// still gets its own session and the separation rules keep holding.
    pub fn session(&self, initiative: InitiativeId, attempt: AttemptId) -> SessionIdentity {
        match &self.task {
            Some(task) => SessionIdentity::for_task(
                initiative,
                task.clone(),
                attempt,
                self.role,
                self.selected.clone(),
            ),
            None => SessionIdentity::for_initiative(
                initiative,
                attempt,
                self.role,
                self.selected.clone(),
            ),
        }
    }
}

/// Why routing produced no model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoutingError {
    /// No model in the catalog satisfies the role.
    NoEligibleModel {
        /// The role that could not be served.
        role: AgentRole,
        /// Every model considered, with its rejection reason.
        considered: Vec<Candidate>,
    },
    /// The preferred model is unavailable and the role forbids fallback.
    FallbackForbidden {
        /// The role that could not be served.
        role: AgentRole,
        /// The preferred model.
        preferred: String,
        /// Why the preferred model was rejected.
        rejection: RejectionReason,
    },
}

impl RoutingError {
    /// A human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            RoutingError::NoEligibleModel { role, considered } => format!(
                "no model satisfies {role}; considered {}",
                considered
                    .iter()
                    .map(|candidate| candidate.model.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            RoutingError::FallbackForbidden {
                role,
                preferred,
                rejection,
            } => format!(
                "{role} forbids fallback and preferred model {preferred} is unavailable ({})",
                rejection.as_str()
            ),
        }
    }
}

/// The models the orchestrator may route to, in preference order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalog {
    /// Models, most preferred first.
    #[serde(default)]
    pub models: Vec<ModelDescriptor>,
}

impl ModelCatalog {
    /// A catalog in preference order.
    pub fn new(models: Vec<ModelDescriptor>) -> Self {
        Self { models }
    }

    /// Select a model for `role`, recording every rejection.
    pub fn select(
        &self,
        role: AgentRole,
        task: Option<TaskId>,
        requirements: &RoleRequirements,
        quota: &QuotaState,
    ) -> Result<RoutingDecision, RoutingError> {
        let mut considered = Vec::new();
        let mut selected: Option<(String, usize)> = None;

        for model in &self.models {
            let rejection = reject(model, requirements, quota);
            considered.push(Candidate {
                model: model.id.clone(),
                eligible: rejection.is_none(),
                rejection,
            });
            if rejection.is_none() && selected.is_none() {
                selected = Some((model.id.clone(), considered.len() - 1));
            }
        }

        let Some((model, index)) = selected else {
            return Err(RoutingError::NoEligibleModel { role, considered });
        };

        if index > 0 && !requirements.fallback_allowed {
            let preferred = &considered[0];
            return Err(RoutingError::FallbackForbidden {
                role,
                preferred: preferred.model.clone(),
                rejection: preferred.rejection.unwrap_or(RejectionReason::QuotaUnknown),
            });
        }

        Ok(RoutingDecision {
            role,
            task,
            capability_class: requirements.capability_class.clone(),
            selected: model,
            fallback_depth: index,
            considered,
        })
    }
}

/// The first requirement `model` fails, if any.
fn reject(
    model: &ModelDescriptor,
    requirements: &RoleRequirements,
    quota: &QuotaState,
) -> Option<RejectionReason> {
    if model.class < requirements.minimum_class {
        return Some(RejectionReason::BelowMinimumClass);
    }
    if requirements.requires_vision && !model.vision {
        return Some(RejectionReason::NoVision);
    }
    if requirements.requires_tools && !model.tools {
        return Some(RejectionReason::NoTools);
    }
    if model.context_tokens < requirements.min_context_tokens {
        return Some(RejectionReason::ContextTooSmall);
    }
    if requirements.local_only && !model.local {
        return Some(RejectionReason::NotLocal);
    }
    if model.privacy > requirements.max_privacy {
        return Some(RejectionReason::PrivacyTooOpen);
    }
    if let Some(ceiling) = requirements.max_cost_per_1k_millicents {
        if model.cost_per_1k_millicents > ceiling {
            return Some(RejectionReason::TooExpensive);
        }
    }
    match quota.models.get(&model.id) {
        None => Some(RejectionReason::QuotaUnknown),
        Some(quota) if !quota.capacity_available => Some(RejectionReason::NoCapacity),
        Some(quota) if quota.remaining_tokens == 0 => Some(RejectionReason::QuotaExhausted),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initiative::roles::SeparationPolicy;

    fn model(id: &str, class: ModelClass) -> ModelDescriptor {
        ModelDescriptor {
            id: id.to_string(),
            provider: "remote".to_string(),
            class,
            vision: false,
            tools: true,
            context_tokens: 200_000,
            local: false,
            privacy: PrivacyTier::Private,
            cost_per_1k_millicents: 1_500,
        }
    }

    fn local_model(id: &str) -> ModelDescriptor {
        ModelDescriptor {
            id: id.to_string(),
            provider: "inferweave".to_string(),
            class: ModelClass::High,
            vision: false,
            tools: true,
            context_tokens: 200_000,
            local: true,
            privacy: PrivacyTier::Local,
            cost_per_1k_millicents: 0,
        }
    }

    fn catalog() -> ModelCatalog {
        ModelCatalog::new(vec![
            model("remote/frontier", ModelClass::Frontier),
            model("remote/high", ModelClass::High),
            local_model("inferweave/local-high"),
            model("remote/small", ModelClass::Small),
        ])
    }

    fn quota() -> QuotaState {
        QuotaState::all_available(
            &[
                "remote/frontier",
                "remote/high",
                "inferweave/local-high",
                "remote/small",
            ],
            1_000_000,
        )
    }

    fn task() -> TaskId {
        TaskId::parse("TASK-0017").expect("valid task id")
    }

    #[test]
    fn the_preferred_model_is_selected_when_it_is_available() {
        let decision = catalog()
            .select(
                AgentRole::Implementer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Implementer),
                &quota(),
            )
            .expect("a model is available");

        assert_eq!(decision.selected, "remote/frontier");
        assert!(!decision.used_fallback());
        assert_eq!(decision.capability_class, "coding-high");
    }

    #[test]
    fn an_exhausted_quota_falls_back_and_records_why() {
        let mut quota = quota();
        quota.set("remote/frontier", Quota::exhausted());

        let decision = catalog()
            .select(
                AgentRole::Implementer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Implementer),
                &quota,
            )
            .expect("fallback is allowed");

        assert_eq!(decision.selected, "remote/high");
        assert_eq!(decision.fallback_depth, 1);
        assert_eq!(
            decision.considered[0].rejection,
            Some(RejectionReason::QuotaExhausted)
        );
    }

    #[test]
    fn a_provider_without_capacity_is_skipped() {
        let mut quota = quota();
        quota.set("remote/frontier", Quota::at_capacity());
        quota.set("remote/high", Quota::at_capacity());

        let decision = catalog()
            .select(
                AgentRole::Implementer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Implementer),
                &quota,
            )
            .expect("a local model remains");

        assert_eq!(decision.selected, "inferweave/local-high");
        assert_eq!(decision.fallback_depth, 2);
    }

    #[test]
    fn a_local_only_role_never_routes_to_a_remote_provider() {
        let mut requirements = RoleRequirements::for_role(AgentRole::Implementer);
        requirements.local_only = true;

        let decision = catalog()
            .select(
                AgentRole::Implementer,
                Some(task()),
                &requirements,
                &quota(),
            )
            .expect("a local model exists");

        assert_eq!(decision.selected, "inferweave/local-high");
        assert_eq!(
            decision.considered[0].rejection,
            Some(RejectionReason::NotLocal)
        );
    }

    #[test]
    fn a_vision_role_rejects_every_text_only_model() {
        let error = catalog()
            .select(
                AgentRole::UxReviewer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::UxReviewer),
                &quota(),
            )
            .expect_err("no vision model is catalogued");

        assert!(matches!(error, RoutingError::NoEligibleModel { .. }));
        assert!(error.message().contains("ux-reviewer"));
    }

    #[test]
    fn a_role_that_forbids_fallback_fails_loudly_instead_of_downgrading() {
        let mut requirements = RoleRequirements::for_role(AgentRole::SpecVerifier);
        requirements.fallback_allowed = false;
        let mut quota = quota();
        quota.set("remote/frontier", Quota::exhausted());

        let error = catalog()
            .select(AgentRole::SpecVerifier, None, &requirements, &quota)
            .expect_err("fallback is forbidden");

        assert!(matches!(
            error,
            RoutingError::FallbackForbidden {
                rejection: RejectionReason::QuotaExhausted,
                ..
            }
        ));
    }

    #[test]
    fn a_model_below_the_minimum_class_is_never_selected() {
        let catalog = ModelCatalog::new(vec![model("remote/small", ModelClass::Small)]);

        let error = catalog
            .select(
                AgentRole::Implementer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Implementer),
                &QuotaState::all_available(&["remote/small"], 1_000),
            )
            .expect_err("too weak");

        let RoutingError::NoEligibleModel { considered, .. } = error else {
            panic!("expected no eligible model");
        };
        assert_eq!(
            considered[0].rejection,
            Some(RejectionReason::BelowMinimumClass)
        );
    }

    #[test]
    fn a_model_with_no_quota_record_is_treated_as_unavailable() {
        let decision = catalog()
            .select(
                AgentRole::Implementer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Implementer),
                &QuotaState::all_available(&["remote/high"], 1_000),
            )
            .expect("the model with a quota record wins");

        assert_eq!(decision.selected, "remote/high");
        assert_eq!(
            decision.considered[0].rejection,
            Some(RejectionReason::QuotaUnknown)
        );
    }

    #[test]
    fn a_cost_ceiling_rejects_models_above_it() {
        let mut requirements = RoleRequirements::for_role(AgentRole::TestEngineer);
        requirements.max_cost_per_1k_millicents = Some(100);

        let decision = catalog()
            .select(
                AgentRole::TestEngineer,
                Some(task()),
                &requirements,
                &quota(),
            )
            .expect("the free local model is under the ceiling");

        assert_eq!(decision.selected, "inferweave/local-high");
        assert_eq!(
            decision.considered[0].rejection,
            Some(RejectionReason::TooExpensive)
        );
    }

    #[test]
    fn fallback_still_dispatches_into_a_session_separate_from_the_implementer() {
        let initiative = InitiativeId::parse("INIT-2026-0042").expect("valid initiative id");
        let mut quota = quota();
        quota.set("remote/frontier", Quota::exhausted());
        let catalog = catalog();

        let implementation = catalog
            .select(
                AgentRole::Implementer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Implementer),
                &quota,
            )
            .expect("implementer routed")
            .session(initiative.clone(), AttemptId::from_sequence(3, 3));
        let review = catalog
            .select(
                AgentRole::Reviewer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Reviewer),
                &quota,
            )
            .expect("reviewer routed")
            .session(initiative, AttemptId::from_sequence(4, 3));

        assert_eq!(implementation.model, review.model);
        assert_ne!(implementation.session_name(), review.session_name());
        assert!(SeparationPolicy::default()
            .check(&[implementation, review])
            .is_permitted());
    }

    #[test]
    fn a_routing_decision_serializes_every_rejection_for_audit() {
        let mut quota = quota();
        quota.set("remote/frontier", Quota::exhausted());
        let decision = catalog()
            .select(
                AgentRole::Implementer,
                Some(task()),
                &RoleRequirements::for_role(AgentRole::Implementer),
                &quota,
            )
            .expect("routed");

        let rendered = serde_json::to_string(&decision).expect("serializable");

        assert!(rendered.contains("\"quota_exhausted\""), "{rendered}");
        assert!(rendered.contains("\"fallback_depth\":1"), "{rendered}");
    }
}
