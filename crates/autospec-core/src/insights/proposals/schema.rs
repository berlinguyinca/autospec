//! §21 improvement-proposal schema and the §22 validation gate.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §20-§22,
//! §35.
//!
//! A [`Proposal`] is the engine's output record: one of the thirteen §20
//! improvement types, citing the finding it was derived from, backed by at
//! least one traceable evidence row (§35: "every finding must be traceable
//! back to evidence"), committing to at least one measurable expected effect
//! (§22: "every proposal MUST define measurable expected effects"), and
//! carrying its change as *patch text only* — §4.5 "No Silent
//! Self-Modification": nothing in this module applies a patch to the tree.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::AutospecError;

/// §20 improvement proposal types — the thirteen wire values, in the §20
/// listing order. The names are the contract: they are what the §34
/// `improvement_proposals.type` column stores, so an unrecognised name is a
/// parse error rather than a silent variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalType {
    #[default]
    AgentInstruction,
    Skill,
    Prompt,
    ToolWrapper,
    ToolRemoval,
    QualityGate,
    RoutingPolicy,
    ContextPolicy,
    ArchitectureRule,
    DocumentationUpdate,
    TestPolicy,
    WorkflowChange,
    CodeChange,
}

impl ProposalType {
    /// The §20 snake_case wire name for this type.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalType::AgentInstruction => "agent_instruction",
            ProposalType::Skill => "skill",
            ProposalType::Prompt => "prompt",
            ProposalType::ToolWrapper => "tool_wrapper",
            ProposalType::ToolRemoval => "tool_removal",
            ProposalType::QualityGate => "quality_gate",
            ProposalType::RoutingPolicy => "routing_policy",
            ProposalType::ContextPolicy => "context_policy",
            ProposalType::ArchitectureRule => "architecture_rule",
            ProposalType::DocumentationUpdate => "documentation_update",
            ProposalType::TestPolicy => "test_policy",
            ProposalType::WorkflowChange => "workflow_change",
            ProposalType::CodeChange => "code_change",
        }
    }
}

impl FromStr for ProposalType {
    type Err = AutospecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let err = || {
            AutospecError::parse(
                "improvement_proposals.type",
                format!("unknown §20 proposal type {value:?}"),
            )
        };
        Ok(match value {
            "agent_instruction" => ProposalType::AgentInstruction,
            "skill" => ProposalType::Skill,
            "prompt" => ProposalType::Prompt,
            "tool_wrapper" => ProposalType::ToolWrapper,
            "tool_removal" => ProposalType::ToolRemoval,
            "quality_gate" => ProposalType::QualityGate,
            "routing_policy" => ProposalType::RoutingPolicy,
            "context_policy" => ProposalType::ContextPolicy,
            "architecture_rule" => ProposalType::ArchitectureRule,
            "documentation_update" => ProposalType::DocumentationUpdate,
            "test_policy" => ProposalType::TestPolicy,
            "workflow_change" => ProposalType::WorkflowChange,
            "code_change" => ProposalType::CodeChange,
            _ => return Err(err()),
        })
    }
}

/// §21/§24 proposal lifecycle. A fresh proposal is always `draft` (§4.5: a
/// draft patch stays a draft); later transitions happen at the §23
/// evaluation gate and §24 PR workflow, never inside this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    #[default]
    Draft,
    Evaluated,
    Approved,
    Rejected,
    Retired,
}

impl ProposalStatus {
    /// The §21 snake_case wire name for this status.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalStatus::Draft => "draft",
            ProposalStatus::Evaluated => "evaluated",
            ProposalStatus::Approved => "approved",
            ProposalStatus::Rejected => "rejected",
            ProposalStatus::Retired => "retired",
        }
    }
}

impl FromStr for ProposalStatus {
    type Err = AutospecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let err = || {
            AutospecError::parse(
                "improvement_proposals.status",
                format!("unknown proposal status {value:?}"),
            )
        };
        Ok(match value {
            "draft" => ProposalStatus::Draft,
            "evaluated" => ProposalStatus::Evaluated,
            "approved" => ProposalStatus::Approved,
            "rejected" => ProposalStatus::Rejected,
            "retired" => ProposalStatus::Retired,
            _ => return Err(err()),
        })
    }
}

/// §22 measurable expected effects. Each field is a committed percentage
/// delta on one of the five §22 metrics (tool errors, tokens/task, user
/// corrections, review rework, success rate); a `None` field means the
/// proposal makes no commitment on that metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExpectedEffect {
    /// §22 "tool_errors" — committed % reduction in tool errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_error_reduction_pct: Option<i32>,
    /// §22 "tokens/task" — committed % reduction in tokens per task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_reduction_pct: Option<i32>,
    /// §22 "user corrections" — committed % reduction in user corrections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_correction_reduction_pct: Option<i32>,
    /// §22 "review rework" — committed % reduction in review rework.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rework_reduction_pct: Option<i32>,
    /// §22 "success rate" — committed percentage-point increase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_rate_increase_pct: Option<i32>,
}

impl ExpectedEffect {
    /// §22: a proposal is measurable only when it commits to at least one
    /// concrete numeric effect. An all-`None` effect is exactly what the
    /// §22 gate ("every proposal MUST define measurable expected effects")
    /// exists to reject.
    pub fn measurable(&self) -> bool {
        self.tool_error_reduction_pct.is_some()
            || self.token_reduction_pct.is_some()
            || self.user_correction_reduction_pct.is_some()
            || self.rework_reduction_pct.is_some()
            || self.success_rate_increase_pct.is_some()
    }
}

/// §35 evidence row: a traceable reference back to the telemetry the finding
/// (and therefore the proposal) was derived from. The field set follows the
/// §21 example (`session_id`/`event_id`); `detail` carries any other §35
/// catalogue anchor (git commit, PR, CI run, review, quality gate) or a
/// `finding:<finding_id>` citation for a proposal that anchors on its
/// finding record.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Evidence {
    /// §35 session the evidence row comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// §35 event within that session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Any other §35 anchor, e.g. `finding:<finding_id>`, `pr:123`,
    /// `ci_run:456`; opaque to this module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Evidence {
    /// Cite a finding record as the evidence anchor — the shape a
    /// deterministic draft uses before strong-model enrichment fills in
    /// session/event rows (§4.1).
    pub fn finding_citation(finding_id: &str) -> Self {
        Self {
            session_id: None,
            event_id: None,
            detail: Some(format!("finding:{finding_id}")),
        }
    }

    /// An evidence row only counts toward the §35 contract if it identifies
    /// at least one source; an all-`None` row is an empty citation.
    pub fn identified(&self) -> bool {
        self.session_id.is_some() || self.event_id.is_some() || self.detail.is_some()
    }
}

/// §23 evaluation plan: how the proposal will be scored before merge.
/// `methods` draws from the §23 catalogue (historical replay, synthetic
/// benchmarks, A/B agent evaluation, shadow mode, static validation, prompt
/// regression tests, skill invocation tests, routing simulation);
/// `task_suite` names the representative task suite the baseline-vs-candidate
/// comparison runs against.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EvaluationPlan {
    /// §23 evaluation methods.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    /// Representative task suite for the baseline-vs-candidate run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_suite: Option<String>,
}

/// §21 improvement proposal — the fourteen wire fields, in §21 order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// §21 `proposal_id` — unique within the `improvement_proposals` table.
    pub proposal_id: String,
    /// §21 `finding_id` — the finding this proposal was derived from.
    pub finding_id: String,
    /// §21 `type` — one of the thirteen §20 types.
    #[serde(rename = "type")]
    pub kind: ProposalType,
    /// §21 `title`.
    pub title: String,
    /// §21 `status` — a fresh proposal is always `draft`.
    pub status: ProposalStatus,
    /// §21 `rationale` — why this improvement should exist (§4.2 evidence
    /// before policy).
    pub rationale: String,
    /// §21 `evidence` — at least one row for a valid proposal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// §21 `expected_effect` — must be measurable to pass validation.
    #[serde(default)]
    pub expected_effect: ExpectedEffect,
    /// §21 `risks`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<String>,
    /// §21 `affected_components` — which instructions/skills/tools/gates
    /// would change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_components: Vec<String>,
    /// §21 `patch` — the generated patch **as unified-diff text**. Text only:
    /// applying it is a separate, human-approved stage (§4.5, §24).
    #[serde(default)]
    pub patch: String,
    /// §21 `evaluation_plan`.
    #[serde(default)]
    pub evaluation_plan: EvaluationPlan,
    /// §21 `created_by_model` — which model drafted this proposal.
    pub created_by_model: String,
    /// §21 `review_model` — §25 separation of duties: the reviewer SHOULD be
    /// a different model than `created_by_model`; `None` until assigned.
    #[serde(default)]
    pub review_model: Option<String>,
}

impl Proposal {
    /// The §22 validation gate plus the data-integrity contract every stored
    /// proposal satisfies:
    ///
    /// - cites a non-empty `finding_id` (§35: proposals are derived from
    ///   findings),
    /// - carries at least one evidence row (§35: "every finding must be
    ///   traceable back to evidence"),
    /// - defines a measurable expected effect (§22: "every proposal MUST
    ///   define measurable expected effects"),
    /// - has a non-empty title (§21).
    ///
    /// Fails closed: any missing commitment is an
    /// [`AutospecError::Validation`], never a silent default.
    pub fn validate(&self) -> Result<(), AutospecError> {
        if self.finding_id.trim().is_empty() {
            return Err(AutospecError::validation(
                "proposal has no finding_id; every proposal must cite its finding (§35)",
            ));
        }
        if self.evidence.iter().all(|row| !row.identified()) {
            return Err(AutospecError::validation(
                "proposal has no identifiable evidence rows; every finding must be traceable back to evidence (§35)",
            ));
        }
        if !self.expected_effect.measurable() {
            return Err(AutospecError::validation(
                "proposal defines no measurable expected effect (§22)",
            ));
        }
        if self.title.trim().is_empty() {
            return Err(AutospecError::validation("proposal has no title (§21)"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_proposal() -> Proposal {
        Proposal {
            proposal_id: "proposal_test".to_string(),
            finding_id: "finding_001".to_string(),
            kind: ProposalType::AgentInstruction,
            title: "Draft improvement".to_string(),
            status: ProposalStatus::default(),
            rationale: "recurring pattern".to_string(),
            evidence: vec![Evidence::finding_citation("finding_001")],
            expected_effect: ExpectedEffect {
                rework_reduction_pct: Some(20),
                ..ExpectedEffect::default()
            },
            risks: vec!["none known".to_string()],
            affected_components: vec!["AGENTS.md".to_string()],
            patch: String::new(),
            evaluation_plan: EvaluationPlan::default(),
            created_by_model: "qwen3-32b".to_string(),
            review_model: None,
        }
    }

    #[test]
    fn the_thirteen_section_20_types_roundtrip_through_their_wire_names() {
        let wire_names = [
            "agent_instruction",
            "skill",
            "prompt",
            "tool_wrapper",
            "tool_removal",
            "quality_gate",
            "routing_policy",
            "context_policy",
            "architecture_rule",
            "documentation_update",
            "test_policy",
            "workflow_change",
            "code_change",
        ];
        assert_eq!(wire_names.len(), 13);
        for name in wire_names {
            let parsed: ProposalType = name.parse().unwrap();
            assert_eq!(parsed.as_str(), name);
            let json = serde_json::to_string(&parsed).unwrap();
            assert_eq!(json, format!("\"{name}\""));
        }
    }

    #[test]
    fn unknown_type_names_are_parse_errors_not_silent_variants() {
        let err = "vibes".parse::<ProposalType>().unwrap_err();
        assert!(
            matches!(err, AutospecError::Parse { .. }),
            "expected Parse, got {err:?}"
        );
        let err = "AGENT_INSTRUCTION".parse::<ProposalType>().unwrap_err();
        assert!(matches!(err, AutospecError::Parse { .. }));
    }

    #[test]
    fn a_valid_proposal_passes_validation() {
        base_proposal().validate().unwrap();
    }

    #[test]
    fn a_proposal_missing_expected_effect_returns_a_validation_error() {
        let mut proposal = base_proposal();
        proposal.expected_effect = ExpectedEffect::default();
        let err = proposal.validate().unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "expected Validation, got {err:?}"
        );
    }

    #[test]
    fn a_proposal_without_a_finding_returns_a_validation_error() {
        let mut proposal = base_proposal();
        proposal.finding_id = "   ".to_string();
        assert!(matches!(
            proposal.validate(),
            Err(AutospecError::Validation { .. })
        ));
    }

    #[test]
    fn a_proposal_without_evidence_returns_a_validation_error() {
        let mut proposal = base_proposal();
        proposal.evidence.clear();
        assert!(matches!(
            proposal.validate(),
            Err(AutospecError::Validation { .. })
        ));
    }

    #[test]
    fn a_proposal_with_only_unidentified_evidence_rows_is_rejected() {
        let mut proposal = base_proposal();
        proposal.evidence = vec![Evidence::default()];
        assert!(matches!(
            proposal.validate(),
            Err(AutospecError::Validation { .. })
        ));
    }

    #[test]
    fn the_fourteen_wire_fields_match_the_section_21_schema() {
        let json: serde_json::Value = serde_json::to_value(&base_proposal()).unwrap();
        let object = json.as_object().unwrap();
        let expected = [
            "proposal_id",
            "finding_id",
            "type",
            "title",
            "status",
            "rationale",
            "evidence",
            "expected_effect",
            "risks",
            "affected_components",
            "patch",
            "evaluation_plan",
            "created_by_model",
            "review_model",
        ];
        assert_eq!(object.len(), 14, "got: {json}");
        for field in expected {
            assert!(object.contains_key(field), "missing §21 field {field}");
        }
        assert_eq!(object["type"], "agent_instruction");
        assert_eq!(object["status"], "draft");
    }

    #[test]
    fn the_section_21_example_shape_deserializes() {
        let json = r#"{
            "proposal_id": "proposal_001",
            "finding_id": "finding_001",
            "type": "agent_instruction",
            "title": "Ban unused abstractions",
            "status": "draft",
            "rationale": "Repeated pattern across sessions",
            "evidence": [
                { "session_id": "session_001", "event_id": "event_014" }
            ],
            "expected_effect": { "rework_reduction_pct": 20, "token_reduction_pct": 10 },
            "risks": ["May be too strict for exploratory work"],
            "affected_components": ["agents_md"],
            "patch": "--- a/AGENTS.md\n+++ b/AGENTS.md",
            "evaluation_plan": {},
            "created_by_model": "qwen3-32b",
            "review_model": "claude-4.6"
        }"#;
        let proposal: Proposal = serde_json::from_str(json).unwrap();
        assert_eq!(proposal.kind, ProposalType::AgentInstruction);
        assert_eq!(proposal.status, ProposalStatus::Draft);
        assert_eq!(proposal.expected_effect.rework_reduction_pct, Some(20));
        assert_eq!(proposal.expected_effect.token_reduction_pct, Some(10));
        assert_eq!(proposal.review_model.as_deref(), Some("claude-4.6"));
        // The §21 example carries a measurable effect and an evidence row, so
        // it clears the §22 gate.
        proposal.validate().unwrap();
    }
}
