//! Finite repair, failure taxonomy and profile escalation (AAR spec section 12).
//!
//! The controller is provider-neutral: it reasons over a fixed taxonomy of
//! failure classes and over *requests* to retry, never over providers, models
//! or harnesses. The driver performs the I/O; this module only decides what
//! the next action is.
//!
//! Three rules make the loop finite instead of a retry until the bill arrives:
//!
//! 1. **Bounded cycles per tier.** Each builder tier (FAST, CODING, DEEP) has
//!    a configured cap on unsuccessful repair cycles. Exhaustion at a tier
//!    that has a successor escalates exactly one tier; exhaustion at DEEP
//!    emits a structured blocked handoff.
//! 2. **No blind retries.** Every retry must record a reason and a changed
//!    input, and its input signature must differ from every signature seen
//!    earlier in the run. Identical requests are rejected with `NO_BLIND_RETRY`.
//! 3. **Node failures change nodes.** A `NODE_FAILURE` retry that keeps the
//!    same `node_id` is rejected with `NODE_NOT_CHANGED`.

use serde::{Deserialize, Serialize};

/// Tiers of the repair ladder, cheapest first. Escalation only ever moves one
/// step up; DEEP is the top of the ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BuilderTier {
    Fast,
    Coding,
    Deep,
}

impl BuilderTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuilderTier::Fast => "FAST",
            BuilderTier::Coding => "CODING",
            BuilderTier::Deep => "DEEP",
        }
    }

    /// The single tier above this one, if any.
    pub fn escalation(&self) -> Option<Self> {
        match self {
            BuilderTier::Fast => Some(BuilderTier::Coding),
            BuilderTier::Coding => Some(BuilderTier::Deep),
            BuilderTier::Deep => None,
        }
    }
}

/// The failure taxonomy. Every unsuccessful repair cycle is classified into
/// exactly one of these before a retry may be requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureClass {
    Inference,
    Tool,
    Edit,
    Compile,
    Test,
    Environment,
    Timeout,
    Context,
    Resource,
    Node,
    Cancellation,
}

impl FailureClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureClass::Inference => "INFERENCE_FAILURE",
            FailureClass::Tool => "TOOL_FAILURE",
            FailureClass::Edit => "EDIT_FAILURE",
            FailureClass::Compile => "COMPILE_FAILURE",
            FailureClass::Test => "TEST_FAILURE",
            FailureClass::Environment => "ENVIRONMENT_FAILURE",
            FailureClass::Timeout => "TIMEOUT_FAILURE",
            FailureClass::Context => "CONTEXT_FAILURE",
            FailureClass::Resource => "RESOURCE_FAILURE",
            FailureClass::Node => "NODE_FAILURE",
            FailureClass::Cancellation => "CANCELLATION_FAILURE",
        }
    }

    /// A provider-neutral suggestion for what to change before the next
    /// attempt; feeds the blocked handoff's `suggested_actions`.
    pub fn suggested_action(&self) -> &'static str {
        match self {
            FailureClass::Inference => "swap the inference backend or model profile",
            FailureClass::Tool => "repair or replace the failing tool before retrying",
            FailureClass::Edit => "re-read the target file and narrow the edit",
            FailureClass::Compile => "fix the compile errors before re-running the build",
            FailureClass::Test => "reproduce the failing test locally before retrying",
            FailureClass::Environment => "restore or rebuild the runtime environment",
            FailureClass::Timeout => "raise the timeout or reduce the work per step",
            FailureClass::Context => "shrink retrieved context or compact the session",
            FailureClass::Resource => "raise the resource budget or move to a larger node",
            FailureClass::Node => "retry on a different node_id",
            FailureClass::Cancellation => "do not auto-retry; hand off to the operator",
        }
    }
}

/// Per-tier caps on unsuccessful repair cycles. Defaults: FAST 1, CODING 2,
/// DEEP 3 — the CODING cap is the one acceptance-locked to the issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepairPolicy {
    pub fast_cycles: u32,
    pub coding_cycles: u32,
    pub deep_cycles: u32,
}

impl Default for RepairPolicy {
    fn default() -> Self {
        Self {
            fast_cycles: 1,
            coding_cycles: 2,
            deep_cycles: 3,
        }
    }
}

impl RepairPolicy {
    pub fn limit_for(&self, tier: BuilderTier) -> u32 {
        match tier {
            BuilderTier::Fast => self.fast_cycles,
            BuilderTier::Coding => self.coding_cycles,
            BuilderTier::Deep => self.deep_cycles,
        }
    }
}

/// A request to run another repair cycle after an unsuccessful one.
///
/// `reason` and `changed_input` are mandatory: a retry that cannot say what
/// it learned and what it changed is a blind retry. `input_signature` is the
/// canonical signature of the full retry input (prompt, tools, node, tier);
/// it must be unique across the whole run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairRequest {
    pub failed_class: FailureClass,
    pub reason: String,
    pub changed_input: String,
    pub node_id: String,
    pub input_signature: String,
}

/// A retry the controller refused to authorize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairRejection {
    /// Identical input, or a missing reason / changed input.
    NoBlindRetry,
    /// A `NODE_FAILURE` retry that kept the same `node_id`.
    NodeNotChanged,
}

impl RepairRejection {
    pub fn as_str(&self) -> &'static str {
        match self {
            RepairRejection::NoBlindRetry => "NO_BLIND_RETRY",
            RepairRejection::NodeNotChanged => "NODE_NOT_CHANGED",
        }
    }
}

/// One already-run attempt, kept for the blocked handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AttemptRecord {
    pub tier: BuilderTier,
    pub failure_class: FailureClass,
    pub reason: String,
    pub changed_input: String,
    pub node_id: String,
}

/// The next action the driver must take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairAction {
    /// The retry is authorized; run it on `node_id` at `tier`.
    Retry {
        tier: BuilderTier,
        node_id: String,
        cycles_remaining: u32,
    },
    /// The current tier is exhausted; adopt `to` via `RepairController::adopt`.
    Escalate { from: BuilderTier, to: BuilderTier },
    /// The whole ladder is exhausted; `handoff` is the structured handoff.
    Blocked { handoff: BlockedHandoff },
}

/// Structured, schema-validated handoff emitted when the ladder is exhausted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BlockedHandoff {
    pub schema_version: u32,
    /// Always `"blocked"`; kept as a field so consumers can validate it.
    pub status: String,
    pub task_id: String,
    pub tier: BuilderTier,
    pub total_failed_cycles: u32,
    pub last_failure: Option<FailureClass>,
    pub attempts: Vec<AttemptRecord>,
    pub suggested_actions: Vec<String>,
}

impl BlockedHandoff {
    /// Reject handoffs whose accounting cannot be true.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != REPAIR_SCHEMA_VERSION {
            return Err(format!(
                "blocked handoff schema_version {} != expected {}",
                self.schema_version, REPAIR_SCHEMA_VERSION
            ));
        }
        if self.status != "blocked" {
            return Err(format!(
                "blocked handoff status is '{}', expected 'blocked'",
                self.status
            ));
        }
        if self.attempts.is_empty() {
            return Err("blocked handoff must record at least one attempt".to_string());
        }
        if self.total_failed_cycles != self.attempts.len() as u32 {
            return Err(format!(
                "total_failed_cycles {} does not match {} recorded attempts",
                self.total_failed_cycles,
                self.attempts.len()
            ));
        }
        for attempt in &self.attempts {
            if attempt.reason.trim().is_empty() || attempt.changed_input.trim().is_empty() {
                return Err(
                    "blocked handoff attempts must each record a reason and a changed input"
                        .to_string(),
                );
            }
        }
        Ok(())
    }
}

/// Bumped whenever a persisted handoff field changes meaning or disappears.
pub const REPAIR_SCHEMA_VERSION: u32 = 1;

/// Stateful repair controller for one task.
pub struct RepairController {
    task_id: String,
    tier: BuilderTier,
    node_id: String,
    policy: RepairPolicy,
    failed_at_tier: u32,
    failed_total: u32,
    seen_signatures: Vec<String>,
    attempts: Vec<AttemptRecord>,
    last_failure: Option<FailureClass>,
    tiers_used: Vec<BuilderTier>,
}

impl RepairController {
    /// Start a repair loop at `tier` on `node_id` with the default policy.
    pub fn start(
        task_id: impl Into<String>,
        tier: BuilderTier,
        node_id: impl Into<String>,
    ) -> Self {
        Self::start_with_policy(task_id, tier, node_id, RepairPolicy::default())
    }

    pub fn start_with_policy(
        task_id: impl Into<String>,
        tier: BuilderTier,
        node_id: impl Into<String>,
        policy: RepairPolicy,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            tier,
            node_id: node_id.into(),
            policy,
            failed_at_tier: 0,
            failed_total: 0,
            seen_signatures: Vec::new(),
            attempts: Vec::new(),
            last_failure: None,
            tiers_used: vec![tier],
        }
    }

    pub fn tier(&self) -> BuilderTier {
        self.tier
    }

    pub fn policy(&self) -> RepairPolicy {
        self.policy
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Unsuccessful cycles remaining at the current tier.
    pub fn cycles_remaining(&self) -> u32 {
        self.policy
            .limit_for(self.tier)
            .saturating_sub(self.failed_at_tier)
    }

    pub fn failed_cycles(&self) -> u32 {
        self.failed_total
    }

    pub fn history(&self) -> &[AttemptRecord] {
        &self.attempts
    }

    /// Adopt the tier above the current one. Only legal once the current tier
    /// is exhausted (i.e. right after an `Escalate` verdict) and only for the
    /// single tier one rung up, so the ladder can only be climbed one rung at
    /// a time and never before its cycles are spent.
    pub fn adopt(&mut self, tier: BuilderTier) -> Result<(), String> {
        if self.failed_at_tier < self.policy.limit_for(self.tier) {
            return Err(format!(
                "cannot adopt tier {}: tier {} still has {} unsuccessful cycle(s) left",
                tier.as_str(),
                self.tier.as_str(),
                self.cycles_remaining()
            ));
        }
        if self.tier.escalation() != Some(tier) {
            return Err(format!(
                "cannot adopt tier {}: the ladder only escalates from {} one rung at a time",
                tier.as_str(),
                self.tier.as_str()
            ));
        }
        self.tier = tier;
        self.failed_at_tier = 0;
        if !self.tiers_used.contains(&tier) {
            self.tiers_used.push(tier);
        }
        Ok(())
    }

    /// Decide the next action after an unsuccessful repair cycle.
    ///
    /// If the current tier's cap on unsuccessful cycles is reached, the
    /// verdict is `Escalate` (one rung up) or, at the top of the ladder,
    /// `Blocked` with a structured handoff — the request is not consulted.
    /// Otherwise the request is validated: a retry that is identical to an
    /// earlier input, or that omits its reason or changed input, is rejected
    /// with `NO_BLIND_RETRY`; a `NODE_FAILURE` retry that keeps `node_id` is
    /// rejected with `NODE_NOT_CHANGED`.
    pub fn request_retry(
        &mut self,
        request: RepairRequest,
    ) -> Result<RepairAction, RepairRejection> {
        if self.failed_at_tier >= self.policy.limit_for(self.tier) {
            if let Some(next) = self.tier.escalation() {
                return Ok(RepairAction::Escalate {
                    from: self.tier,
                    to: next,
                });
            }
            return Ok(RepairAction::Blocked {
                handoff: self.blocked_handoff(),
            });
        }

        if request.reason.trim().is_empty() || request.changed_input.trim().is_empty() {
            return Err(RepairRejection::NoBlindRetry);
        }
        if self.seen_signatures.contains(&request.input_signature) {
            return Err(RepairRejection::NoBlindRetry);
        }
        if request.failed_class == FailureClass::Node && request.node_id == self.node_id {
            return Err(RepairRejection::NodeNotChanged);
        }

        self.seen_signatures.push(request.input_signature.clone());
        self.failed_at_tier += 1;
        self.failed_total += 1;
        self.last_failure = Some(request.failed_class);
        self.node_id = request.node_id.clone();
        self.attempts.push(AttemptRecord {
            tier: self.tier,
            failure_class: request.failed_class,
            reason: request.reason,
            changed_input: request.changed_input,
            node_id: request.node_id,
        });

        Ok(RepairAction::Retry {
            tier: self.tier,
            node_id: self.node_id.clone(),
            cycles_remaining: self.cycles_remaining(),
        })
    }

    /// Build the structured blocked handoff from the current state.
    pub fn blocked_handoff(&self) -> BlockedHandoff {
        let mut actions: Vec<String> = Vec::new();
        for attempt in self.attempts.iter().rev() {
            let action = attempt.failure_class.suggested_action().to_string();
            if !actions.contains(&action) {
                actions.push(action);
            }
        }
        BlockedHandoff {
            schema_version: REPAIR_SCHEMA_VERSION,
            status: "blocked".to_string(),
            task_id: self.task_id.clone(),
            tier: self.tier,
            total_failed_cycles: self.failed_total,
            last_failure: self.last_failure,
            attempts: self.attempts.clone(),
            suggested_actions: actions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(class: FailureClass, sig: &str, node: &str) -> RepairRequest {
        RepairRequest {
            failed_class: class,
            reason: format!("learned something about the {sig} failure"),
            changed_input: format!("patched input for {sig}"),
            node_id: node.to_string(),
            input_signature: sig.to_string(),
        }
    }

    #[test]
    fn failure_taxonomy_has_all_eleven_classes() {
        let codes = [
            "INFERENCE_FAILURE",
            "TOOL_FAILURE",
            "EDIT_FAILURE",
            "COMPILE_FAILURE",
            "TEST_FAILURE",
            "ENVIRONMENT_FAILURE",
            "TIMEOUT_FAILURE",
            "CONTEXT_FAILURE",
            "RESOURCE_FAILURE",
            "NODE_FAILURE",
            "CANCELLATION_FAILURE",
        ];
        let classes = [
            FailureClass::Inference,
            FailureClass::Tool,
            FailureClass::Edit,
            FailureClass::Compile,
            FailureClass::Test,
            FailureClass::Environment,
            FailureClass::Timeout,
            FailureClass::Context,
            FailureClass::Resource,
            FailureClass::Node,
            FailureClass::Cancellation,
        ];
        assert_eq!(classes.len(), 11);
        for (class, code) in classes.iter().zip(codes.iter()) {
            assert_eq!(class.as_str(), *code);
            assert!(!class.suggested_action().is_empty());
        }
    }

    #[test]
    fn coding_builder_stops_after_two_failed_cycles() {
        let mut c = RepairController::start("task-coding", BuilderTier::Coding, "node-a");
        assert_eq!(c.policy().coding_cycles, 2);

        // First failed cycle, retry authorized.
        match c.request_retry(request(FailureClass::Compile, "sig-1", "node-a")) {
            Ok(RepairAction::Retry {
                tier,
                cycles_remaining,
                ..
            }) => {
                assert_eq!(tier, BuilderTier::Coding);
                assert_eq!(cycles_remaining, 1);
            }
            other => panic!("expected first retry, got {other:?}"),
        }
        // Second failed cycle, retry authorized.
        match c.request_retry(request(FailureClass::Test, "sig-2", "node-a")) {
            Ok(RepairAction::Retry {
                tier,
                cycles_remaining,
                ..
            }) => {
                assert_eq!(tier, BuilderTier::Coding);
                assert_eq!(cycles_remaining, 0);
            }
            other => panic!("expected second retry, got {other:?}"),
        }
        // Third failure: the CODING tier stops — no third retry, it escalates.
        match c.request_retry(request(FailureClass::Compile, "sig-3", "node-a")) {
            Ok(RepairAction::Escalate { from, to }) => {
                assert_eq!(from, BuilderTier::Coding);
                assert_eq!(to, BuilderTier::Deep);
            }
            other => panic!("expected escalation after two cycles, got {other:?}"),
        }
    }

    #[test]
    fn identical_retry_is_rejected_with_no_blind_retry() {
        let mut c = RepairController::start("task-blind", BuilderTier::Coding, "node-a");
        assert!(matches!(
            c.request_retry(request(FailureClass::Test, "sig-1", "node-a")),
            Ok(RepairAction::Retry { .. })
        ));
        // Same signature as the previous attempt, even with fresh wording.
        let blind = RepairRequest {
            failed_class: FailureClass::Test,
            reason: "surely this will work now".to_string(),
            changed_input: "nothing".to_string(),
            node_id: "node-a".to_string(),
            input_signature: "sig-1".to_string(),
        };
        let err = c
            .request_retry(blind)
            .expect_err("identical retry must be rejected");
        assert_eq!(err, RepairRejection::NoBlindRetry);
        assert_eq!(err.as_str(), "NO_BLIND_RETRY");
        // The failed attempt was not recorded.
        assert_eq!(c.failed_cycles(), 1);
        assert_eq!(c.history().len(), 1);
    }

    #[test]
    fn retry_without_reason_or_changed_input_is_rejected() {
        let mut c = RepairController::start("task-justify", BuilderTier::Fast, "node-a");
        let no_reason = RepairRequest {
            reason: String::new(),
            ..request(FailureClass::Edit, "sig-a", "node-a")
        };
        assert_eq!(
            c.request_retry(no_reason).err(),
            Some(RepairRejection::NoBlindRetry)
        );
        let no_change = RepairRequest {
            changed_input: String::new(),
            ..request(FailureClass::Edit, "sig-b", "node-a")
        };
        assert_eq!(
            c.request_retry(no_change).err(),
            Some(RepairRejection::NoBlindRetry)
        );
    }

    #[test]
    fn node_failure_changes_node_id_before_retry() {
        let mut c = RepairController::start("task-node", BuilderTier::Coding, "node-a");
        // Keeping the same node after a NODE_FAILURE is rejected.
        let same_node = request(FailureClass::Node, "sig-1", "node-a");
        assert_eq!(
            c.request_retry(same_node).err(),
            Some(RepairRejection::NodeNotChanged)
        );
        // A different node is accepted.
        match c.request_retry(request(FailureClass::Node, "sig-2", "node-b")) {
            Ok(RepairAction::Retry { node_id, .. }) => assert_eq!(node_id, "node-b"),
            other => panic!("expected retry on node-b, got {other:?}"),
        }
        assert_eq!(c.node_id(), "node-b");
    }

    #[test]
    fn impossible_fixture_ends_blocked_with_valid_handoff() {
        let mut c = RepairController::start("task-impossible", BuilderTier::Fast, "node-a");

        // FAST: 1 cycle, then escalation to CODING.
        assert!(matches!(
            c.request_retry(request(FailureClass::Inference, "s-fast-1", "node-a")),
            Ok(RepairAction::Retry { .. })
        ));
        match c.request_retry(request(FailureClass::Inference, "s-fast-2", "node-a")) {
            Ok(RepairAction::Escalate { to, .. }) => assert_eq!(to, BuilderTier::Coding),
            other => panic!("expected escalate, got {other:?}"),
        }
        c.adopt(BuilderTier::Coding).unwrap();

        // CODING: 2 cycles, then escalation to DEEP.
        for sig in ["s-code-1", "s-code-2"] {
            assert!(matches!(
                c.request_retry(request(FailureClass::Compile, sig, "node-a")),
                Ok(RepairAction::Retry { .. })
            ));
        }
        match c.request_retry(request(FailureClass::Compile, "s-code-3", "node-a")) {
            Ok(RepairAction::Escalate { to, .. }) => assert_eq!(to, BuilderTier::Deep),
            other => panic!("expected escalate, got {other:?}"),
        }
        c.adopt(BuilderTier::Deep).unwrap();

        // DEEP: 3 cycles, then the ladder is exhausted.
        for sig in ["s-deep-1", "s-deep-2", "s-deep-3"] {
            assert!(matches!(
                c.request_retry(request(FailureClass::Test, sig, "node-a")),
                Ok(RepairAction::Retry { .. })
            ));
        }
        let handoff = match c.request_retry(request(FailureClass::Test, "s-deep-4", "node-a")) {
            Ok(RepairAction::Blocked { handoff }) => handoff,
            other => panic!("expected blocked, got {other:?}"),
        };

        assert_eq!(handoff.status, "blocked");
        assert_eq!(handoff.total_failed_cycles, 6);
        assert_eq!(handoff.attempts.len(), 6);
        assert_eq!(handoff.last_failure, Some(FailureClass::Test));
        assert_eq!(handoff.tier, BuilderTier::Deep);
        assert!(handoff.suggested_actions.iter().any(|a| a.contains("test")));
        handoff.validate().expect("handoff schema must validate");

        // The handoff is structured: it round-trips through JSON with status intact.
        let json = serde_json::to_value(&handoff).expect("handoff must serialize");
        assert_eq!(json["status"], "blocked");
        let back: BlockedHandoff = serde_json::from_value(json).expect("handoff must deserialize");
        assert_eq!(back, handoff);
    }

    #[test]
    fn escalation_is_one_rung_at_a_time() {
        let mut c = RepairController::start("task-ladder", BuilderTier::Fast, "node-a");
        // Adopting before the current tier is exhausted is rejected.
        assert!(c.adopt(BuilderTier::Coding).is_err());
        // FAST's single cycle, then the escalation verdict.
        assert!(c
            .request_retry(request(FailureClass::Tool, "s-1", "node-a"))
            .is_ok());
        match c.request_retry(request(FailureClass::Tool, "s-2", "node-a")) {
            Ok(RepairAction::Escalate { .. }) => {}
            other => panic!("expected escalate, got {other:?}"),
        }
        c.adopt(BuilderTier::Coding).unwrap();
        // Skipping CODING's cap to reach DEEP is not allowed.
        assert!(c.adopt(BuilderTier::Deep).is_err());
    }

    #[test]
    fn configured_limit_is_honored() {
        let policy = RepairPolicy {
            fast_cycles: 2,
            coding_cycles: 1,
            deep_cycles: 1,
        };
        let mut c = RepairController::start_with_policy(
            "task-custom",
            BuilderTier::Coding,
            "node-a",
            policy,
        );
        assert!(matches!(
            c.request_retry(request(FailureClass::Resource, "s-1", "node-a")),
            Ok(RepairAction::Retry { .. })
        ));
        // One configured cycle for CODING here: escalate instead of a second retry.
        assert!(matches!(
            c.request_retry(request(FailureClass::Resource, "s-2", "node-a")),
            Ok(RepairAction::Escalate { .. })
        ));
    }
}
