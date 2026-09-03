use serde::Serialize;

use super::config::WorkflowConfig;
use super::diagnostics::DiagnosticDelta;
use super::error::CodeIntelError;
use super::impact::{ImpactComparison, ImpactSet};

/// The Pi role a gate is evaluated for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Planner,
    Implementer,
    Tester,
    Reviewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Implementer => "implementer",
            Self::Tester => "tester",
            Self::Reviewer => "reviewer",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CodeIntelError> {
        match value {
            "planner" => Ok(Self::Planner),
            "implementer" => Ok(Self::Implementer),
            "tester" => Ok(Self::Tester),
            "reviewer" => Ok(Self::Reviewer),
            other => Err(CodeIntelError::gate(format!("unknown role: {other}"))),
        }
    }
}

/// The outcome of a mandatory semantic gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateOutcome {
    pub role: Role,
    pub gate: String,
    pub passed: bool,
    pub degraded: bool,
    pub findings: Vec<String>,
}

impl GateOutcome {
    fn new(role: Role, gate: &str) -> Self {
        Self {
            role,
            gate: gate.to_string(),
            passed: true,
            degraded: false,
            findings: Vec::new(),
        }
    }

    fn fail(mut self, finding: impl Into<String>) -> Self {
        self.passed = false;
        self.findings.push(finding.into());
        self
    }

    fn note(mut self, finding: impl Into<String>) -> Self {
        self.findings.push(finding.into());
        self
    }

    fn degrade(mut self, finding: impl Into<String>) -> Self {
        self.degraded = true;
        self.findings.push(finding.into());
        self
    }

    /// Turn a failed gate into an error. Callers that must not proceed use this;
    /// callers that record and continue read `passed` directly.
    pub fn into_result(self) -> Result<Self, CodeIntelError> {
        if self.passed {
            return Ok(self);
        }
        Err(CodeIntelError::gate(format!(
            "{} gate {} failed: {}",
            self.role.as_str(),
            self.gate,
            self.findings.join("; ")
        )))
    }

    pub fn to_json_string(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| error.to_string())
    }
}

/// The planner gate: a plan is not approvable without a non-empty Impact Set
/// backed by an actual analysis.
pub fn planner_gate(
    config: &WorkflowConfig,
    impact: Option<&ImpactSet>,
    planned_files: &[String],
) -> GateOutcome {
    let outcome = GateOutcome::new(Role::Planner, "pre-change-impact");
    if !config.require_pre_change_impact {
        return outcome.note("pre-change impact analysis not required by config");
    }
    let Some(impact) = impact else {
        return outcome.fail("plan has no semantic impact analysis");
    };
    if impact.is_empty() {
        return outcome.fail("semantic impact analysis produced no affected files");
    }
    if planned_files.is_empty() {
        return outcome.fail("plan declares no Impact Set");
    }
    degrade_if_not_semantic(outcome, impact)
}

/// The implementer gate: compare a fresh analysis against the approved plan
/// before editing, then check the post-change diagnostic delta.
pub fn implementer_gate(
    config: &WorkflowConfig,
    comparison: &ImpactComparison,
    delta: Option<&DiagnosticDelta>,
) -> GateOutcome {
    let mut outcome = GateOutcome::new(Role::Implementer, "semantic-change");
    if config.require_pre_change_impact && !comparison.within_plan() {
        outcome = outcome.note(format!(
            "impact analysis exceeds the approved plan: {}",
            comparison.unplanned_files.join(", ")
        ));
    }
    if !config.require_post_change_diagnostics {
        return outcome.note("post-change diagnostics not required by config");
    }
    let Some(delta) = delta else {
        return outcome.fail("no post-change diagnostic delta was captured");
    };
    if delta.is_clean(config.block_new_errors) {
        return outcome.note(delta.summary());
    }
    outcome.fail(format!(
        "{} new semantic error(s) after implementation: {}",
        delta.new_errors().len(),
        delta.summary()
    ))
}

/// The reviewer gate: the reviewer must run its own analysis. Reusing the
/// implementer's report is the specific failure this gate exists to catch, so a
/// review whose analysis is byte-identical to the implementer's is rejected.
pub fn reviewer_gate(
    config: &WorkflowConfig,
    implementer_impact: Option<&ImpactSet>,
    reviewer_impact: Option<&ImpactSet>,
    delta: Option<&DiagnosticDelta>,
) -> GateOutcome {
    let outcome = GateOutcome::new(Role::Reviewer, "independent-analysis");
    if !config.reviewer_independent_analysis {
        return outcome.note("independent reviewer analysis not required by config");
    }
    let Some(reviewer_impact) = reviewer_impact else {
        return outcome.fail("reviewer ran no independent semantic analysis");
    };
    if reviewer_impact.is_empty() {
        return outcome.fail("reviewer analysis produced no affected files");
    }
    if !review_is_independent(implementer_impact, reviewer_impact) {
        return outcome.fail("reviewer reused the implementer's analysis provenance");
    }
    match delta {
        Some(delta) if !delta.is_clean(config.block_new_errors) => outcome.fail(format!(
            "unresolved new errors at review: {}",
            delta.summary()
        )),
        _ => degrade_if_not_semantic(outcome, reviewer_impact),
    }
}

/// A reviewer analysis is independent when it was produced by its own query,
/// not copied from the implementer's result. Identical provenance (same
/// workspace, revision, backend AND identical affected sets) means the reviewer
/// did not re-run anything.
fn review_is_independent(implementer: Option<&ImpactSet>, reviewer: &ImpactSet) -> bool {
    let Some(implementer) = implementer else {
        return true;
    };
    implementer.provenance != reviewer.provenance
        || implementer.affected_files() != reviewer.affected_files()
        || implementer.affected_symbols() != reviewer.affected_symbols()
}

fn degrade_if_not_semantic(outcome: GateOutcome, impact: &ImpactSet) -> GateOutcome {
    if impact.provenance.is_semantic() {
        return outcome;
    }
    outcome.degrade(format!(
        "analysis degraded to {} ({} confidence)",
        impact.provenance.source, impact.provenance.confidence
    ))
}

#[cfg(test)]
mod tests {
    use super::super::diagnostics::{Diagnostic, DiagnosticSet, Severity};
    use super::super::schema::{Location, Provenance, ResultSource, Symbol};
    use super::*;

    fn impact_with(source: ResultSource, path: &str) -> ImpactSet {
        let provenance = Provenance::new("issue-421", "autospec", "abc123", "agent-lsp", source);
        let mut impact = ImpactSet::new(provenance, "Gateway::resolve");
        impact.definitions.push(Symbol::new(
            "resolve",
            "function",
            Location::point(path, 1, 0),
        ));
        impact
    }

    fn impact() -> ImpactSet {
        impact_with(ResultSource::Lsp, "src/gateway.rs")
    }

    fn clean_delta() -> DiagnosticDelta {
        let empty = DiagnosticSet::new("issue-421", "abc123", Vec::new());
        DiagnosticDelta::between(&empty, &empty)
    }

    fn broken_delta() -> DiagnosticDelta {
        let baseline = DiagnosticSet::new("issue-421", "abc123", Vec::new());
        let current = DiagnosticSet::new(
            "issue-421",
            "abc123",
            vec![Diagnostic::new(
                Location::point("src/gateway.rs", 4, 0),
                Severity::Error,
                "type mismatch",
            )],
        );
        DiagnosticDelta::between(&baseline, &current)
    }

    #[test]
    fn a_plan_without_an_analysis_cannot_be_approved() {
        let outcome = planner_gate(&WorkflowConfig::default(), None, &["src/a.rs".to_string()]);

        assert!(!outcome.passed);
        assert!(outcome.into_result().is_err());
    }

    #[test]
    fn a_plan_without_an_impact_set_cannot_be_approved() {
        let outcome = planner_gate(&WorkflowConfig::default(), Some(&impact()), &[]);

        assert!(!outcome.passed);
        assert!(outcome.findings[0].contains("declares no Impact Set"));
    }

    #[test]
    fn a_plan_with_a_semantic_analysis_and_impact_set_passes() {
        let outcome = planner_gate(
            &WorkflowConfig::default(),
            Some(&impact()),
            &["src/gateway.rs".to_string()],
        );

        assert!(outcome.passed);
        assert!(!outcome.degraded);
    }

    #[test]
    fn a_degraded_analysis_passes_the_planner_gate_but_is_marked_degraded() {
        let outcome = planner_gate(
            &WorkflowConfig::default(),
            Some(&impact_with(ResultSource::Ripgrep, "src/gateway.rs")),
            &["src/gateway.rs".to_string()],
        );

        assert!(outcome.passed);
        assert!(outcome.degraded);
        assert!(outcome.findings[0].contains("textual"));
    }

    #[test]
    fn disabling_the_impact_requirement_records_that_it_was_skipped() {
        let config = WorkflowConfig {
            require_pre_change_impact: false,
            ..WorkflowConfig::default()
        };

        let outcome = planner_gate(&config, None, &[]);

        assert!(outcome.passed);
        assert!(outcome.findings[0].contains("not required by config"));
    }

    #[test]
    fn an_implementer_without_a_diagnostic_delta_is_blocked() {
        let comparison = ImpactComparison::between(&[], &[]);

        let outcome = implementer_gate(&WorkflowConfig::default(), &comparison, None);

        assert!(!outcome.passed);
        assert!(outcome.findings[0].contains("no post-change diagnostic delta"));
    }

    #[test]
    fn new_errors_block_the_implementer_gate() {
        let comparison = ImpactComparison::between(&[], &[]);

        let outcome = implementer_gate(
            &WorkflowConfig::default(),
            &comparison,
            Some(&broken_delta()),
        );

        assert!(!outcome.passed);
        assert!(outcome.findings[0].contains("1 new semantic error"));
    }

    #[test]
    fn unplanned_files_are_recorded_without_blocking_a_clean_change() {
        let comparison = ImpactComparison::between(
            &["src/gateway.rs".to_string()],
            &["src/gateway.rs".to_string(), "src/router.rs".to_string()],
        );

        let outcome = implementer_gate(
            &WorkflowConfig::default(),
            &comparison,
            Some(&clean_delta()),
        );

        assert!(outcome.passed);
        assert!(outcome.findings[0].contains("exceeds the approved plan"));
        assert!(outcome.findings[0].contains("src/router.rs"));
    }

    #[test]
    fn a_clean_implementation_passes() {
        let comparison = ImpactComparison::between(&[], &[]);

        let outcome = implementer_gate(
            &WorkflowConfig::default(),
            &comparison,
            Some(&clean_delta()),
        );

        assert!(outcome.passed);
    }

    #[test]
    fn a_reviewer_that_ran_no_analysis_is_blocked() {
        let outcome = reviewer_gate(
            &WorkflowConfig::default(),
            Some(&impact()),
            None,
            Some(&clean_delta()),
        );

        assert!(!outcome.passed);
        assert!(outcome.findings[0].contains("no independent semantic analysis"));
    }

    #[test]
    fn a_reviewer_that_reused_the_implementers_analysis_is_blocked() {
        let shared = impact();

        let outcome = reviewer_gate(
            &WorkflowConfig::default(),
            Some(&shared),
            Some(&shared),
            Some(&clean_delta()),
        );

        assert!(!outcome.passed);
        assert!(outcome.findings[0].contains("reused the implementer's analysis"));
    }

    #[test]
    fn a_reviewer_with_its_own_analysis_passes() {
        let reviewer = impact_with(ResultSource::Lsp, "src/router.rs");

        let outcome = reviewer_gate(
            &WorkflowConfig::default(),
            Some(&impact()),
            Some(&reviewer),
            Some(&clean_delta()),
        );

        assert!(outcome.passed);
    }

    #[test]
    fn unresolved_new_errors_block_the_reviewer_gate() {
        let reviewer = impact_with(ResultSource::Lsp, "src/router.rs");

        let outcome = reviewer_gate(
            &WorkflowConfig::default(),
            Some(&impact()),
            Some(&reviewer),
            Some(&broken_delta()),
        );

        assert!(!outcome.passed);
        assert!(outcome.findings[0].contains("unresolved new errors"));
    }

    #[test]
    fn roles_round_trip_through_their_names() {
        for role in [
            Role::Planner,
            Role::Implementer,
            Role::Tester,
            Role::Reviewer,
        ] {
            assert_eq!(Role::parse(role.as_str()).unwrap(), role);
        }
        assert!(Role::parse("architect").is_err());
    }

    #[test]
    fn a_failed_gate_names_the_role_and_gate_in_its_error() {
        let error = planner_gate(&WorkflowConfig::default(), None, &[])
            .into_result()
            .unwrap_err();

        assert!(error.message().contains("planner gate pre-change-impact"));
    }
}
