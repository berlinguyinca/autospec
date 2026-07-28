use std::collections::{BTreeMap, BTreeSet};

use crate::state::json::{JsonParser, JsonValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthBranchInput {
    pub explicit_branch: Option<String>,
    pub configured_branch: Option<String>,
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthBranchSource {
    Explicit,
    Configured,
    Default,
}

impl HealthBranchSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Configured => "configured",
            Self::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHealthBranch {
    pub branch: String,
    pub source: HealthBranchSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainlineHealthOutcome {
    Continue,
    Wait,
    Halt,
}

impl MainlineHealthOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Wait => "wait",
            Self::Halt => "halt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainlineHealthDiagnostic {
    ChecksPassed,
    NoRequiredChecks,
    BranchNotFound,
    DefaultBranchMissing,
    GhApiFailed,
    CheckRunsApiFailed,
    RequiredCheckPending,
    RequiredCheckFailed,
    BaselineStale,
    BaselineReadFailed,
    NewCheckFailed,
}

impl MainlineHealthDiagnostic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChecksPassed => "checks-passed",
            Self::NoRequiredChecks => "no-required-checks",
            Self::BranchNotFound => "branch-not-found",
            Self::DefaultBranchMissing => "default-branch-missing",
            Self::GhApiFailed => "gh-api-failed",
            Self::CheckRunsApiFailed => "check-runs-api-failed",
            Self::RequiredCheckPending => "required-check-pending",
            Self::RequiredCheckFailed => "required-check-failed",
            Self::BaselineStale => "baseline-stale",
            Self::BaselineReadFailed => "baseline-read-failed",
            Self::NewCheckFailed => "new-check-failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckEvidence {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckVerdict {
    Pending,
    Passed,
    Failed,
}

impl CheckEvidence {
    pub fn required(
        name: impl Into<String>,
        status: impl Into<String>,
        conclusion: Option<&str>,
    ) -> Self {
        Self {
            name: name.into(),
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
            required: true,
        }
    }

    pub fn advisory(
        name: impl Into<String>,
        status: impl Into<String>,
        conclusion: Option<&str>,
    ) -> Self {
        Self {
            name: name.into(),
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
            required: false,
        }
    }

    fn verdict(&self) -> CheckVerdict {
        if !self.status.eq_ignore_ascii_case("completed") {
            return CheckVerdict::Pending;
        }
        let Some(conclusion) = self.conclusion.as_deref().map(str::trim) else {
            return CheckVerdict::Pending;
        };
        if ["success", "neutral", "skipped"]
            .iter()
            .any(|terminal| conclusion.eq_ignore_ascii_case(terminal))
        {
            CheckVerdict::Passed
        } else if [
            "failure",
            "error",
            "cancelled",
            "timed_out",
            "action_required",
            "startup_failure",
        ]
        .iter()
        .any(|terminal| conclusion.eq_ignore_ascii_case(terminal))
        {
            CheckVerdict::Failed
        } else {
            CheckVerdict::Pending
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainlineHealth {
    pub branch: String,
    pub evidence: Vec<CheckEvidence>,
    pub outcome: MainlineHealthOutcome,
    pub diagnostic: MainlineHealthDiagnostic,
    pub baseline_checks: Vec<String>,
    pub newly_red_checks: Vec<String>,
}

impl MainlineHealth {
    pub fn diagnostic(
        branch: impl Into<String>,
        outcome: MainlineHealthOutcome,
        diagnostic: MainlineHealthDiagnostic,
    ) -> Self {
        Self {
            branch: branch.into(),
            evidence: Vec::new(),
            outcome,
            diagnostic,
            baseline_checks: Vec::new(),
            newly_red_checks: Vec::new(),
        }
    }

    pub fn to_json(&self, repo: &str) -> String {
        self.render_json(repo, None)
    }

    pub fn to_json_with_policy_digest(&self, repo: &str, policy_digest: &str) -> String {
        self.render_json(repo, Some(policy_digest))
    }

    fn render_json(&self, repo: &str, policy_digest: Option<&str>) -> String {
        let policy_digest = policy_digest
            .map(|digest| format!(",\"effective_policy_digest\":\"{}\"", escape_json(digest)))
            .unwrap_or_default();
        format!(
            "{{\"repo\":\"{}\",\"branch\":\"{}\",\"outcome\":\"{}\",\"diagnostic\":\"{}\"{},\"baseline_checks\":[{}],\"newly_red_checks\":[{}],\"evidence\":[{}]}}",
            escape_json(repo),
            escape_json(&self.branch),
            self.outcome.as_str(),
            self.diagnostic.as_str(),
            policy_digest,
            self.baseline_checks.iter().map(|name| format!("\"{}\"", escape_json(name))).collect::<Vec<_>>().join(","),
            self.newly_red_checks.iter().map(|name| format!("\"{}\"", escape_json(name))).collect::<Vec<_>>().join(","),
            self.evidence
                .iter()
                .map(check_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthBaseline {
    Ready(BTreeSet<String>),
    Stale,
    Failed,
}

pub fn evaluate_health_with_baseline(
    branch: &str,
    branch_exists: bool,
    evidence: Vec<CheckEvidence>,
    baseline: HealthBaseline,
) -> MainlineHealth {
    let baseline_checks = match &baseline {
        HealthBaseline::Ready(names) => names.iter().cloned().collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    if !matches!(baseline, HealthBaseline::Ready(_)) {
        let diagnostic = if matches!(baseline, HealthBaseline::Stale) {
            MainlineHealthDiagnostic::BaselineStale
        } else {
            MainlineHealthDiagnostic::BaselineReadFailed
        };
        let mut health =
            MainlineHealth::diagnostic(branch, MainlineHealthOutcome::Wait, diagnostic);
        health.baseline_checks = baseline_checks;
        return health;
    }
    let names = match baseline {
        HealthBaseline::Ready(names) => names,
        _ => unreachable!(),
    };
    let mut health = evaluate_health(branch, branch_exists, evidence.clone());
    let newly_red_checks = evidence
        .into_iter()
        .filter(|check| {
            check.required
                && check.verdict() == CheckVerdict::Failed
                && !names.contains(&check.name)
        })
        .map(|check| check.name)
        .collect::<Vec<_>>();
    health.baseline_checks = baseline_checks;
    health.newly_red_checks = newly_red_checks.clone();
    if !newly_red_checks.is_empty() {
        health.outcome = MainlineHealthOutcome::Halt;
        health.diagnostic = MainlineHealthDiagnostic::NewCheckFailed;
    } else if health.diagnostic == MainlineHealthDiagnostic::RequiredCheckFailed {
        health.outcome = MainlineHealthOutcome::Continue;
        health.diagnostic = MainlineHealthDiagnostic::ChecksPassed;
    }
    health
}

pub fn resolve_health_branch(
    input: &HealthBranchInput,
) -> Result<ResolvedHealthBranch, MainlineHealthDiagnostic> {
    if let Some(branch) = non_empty(input.explicit_branch.as_deref()) {
        return Ok(ResolvedHealthBranch {
            branch: branch.to_string(),
            source: HealthBranchSource::Explicit,
        });
    }
    if let Some(branch) = non_empty(input.configured_branch.as_deref()) {
        return Ok(ResolvedHealthBranch {
            branch: branch.to_string(),
            source: HealthBranchSource::Configured,
        });
    }
    if let Some(branch) = non_empty(input.default_branch.as_deref()) {
        return Ok(ResolvedHealthBranch {
            branch: branch.to_string(),
            source: HealthBranchSource::Default,
        });
    }
    Err(MainlineHealthDiagnostic::DefaultBranchMissing)
}

pub fn apply_ignored_checks(
    evidence: Vec<CheckEvidence>,
    ignored_check_names: &BTreeSet<String>,
) -> Vec<CheckEvidence> {
    evidence
        .into_iter()
        .map(|check| {
            if ignored_check_names.contains(&check.name) {
                CheckEvidence {
                    required: false,
                    ..check
                }
            } else {
                check
            }
        })
        .collect()
}

pub fn evaluate_health(
    branch: &str,
    branch_exists: bool,
    evidence: Vec<CheckEvidence>,
) -> MainlineHealth {
    if !branch_exists {
        return MainlineHealth::diagnostic(
            branch,
            MainlineHealthOutcome::Halt,
            MainlineHealthDiagnostic::BranchNotFound,
        );
    }

    let required = evidence
        .iter()
        .filter(|check| check.required)
        .collect::<Vec<_>>();
    if required.is_empty() {
        return MainlineHealth {
            branch: branch.to_string(),
            evidence,
            outcome: MainlineHealthOutcome::Continue,
            diagnostic: MainlineHealthDiagnostic::NoRequiredChecks,
            baseline_checks: Vec::new(),
            newly_red_checks: Vec::new(),
        };
    }

    if required
        .iter()
        .any(|check| !check.status.eq_ignore_ascii_case("completed"))
    {
        return MainlineHealth {
            branch: branch.to_string(),
            evidence,
            outcome: MainlineHealthOutcome::Wait,
            diagnostic: MainlineHealthDiagnostic::RequiredCheckPending,
            baseline_checks: Vec::new(),
            newly_red_checks: Vec::new(),
        };
    }

    if required
        .iter()
        .any(|check| check.verdict() == CheckVerdict::Failed)
    {
        return MainlineHealth {
            branch: branch.to_string(),
            evidence,
            outcome: MainlineHealthOutcome::Halt,
            diagnostic: MainlineHealthDiagnostic::RequiredCheckFailed,
            baseline_checks: Vec::new(),
            newly_red_checks: Vec::new(),
        };
    }

    if required
        .iter()
        .any(|check| check.verdict() == CheckVerdict::Pending)
    {
        return MainlineHealth {
            branch: branch.to_string(),
            evidence,
            outcome: MainlineHealthOutcome::Wait,
            diagnostic: MainlineHealthDiagnostic::RequiredCheckPending,
            baseline_checks: Vec::new(),
            newly_red_checks: Vec::new(),
        };
    }

    MainlineHealth {
        branch: branch.to_string(),
        evidence,
        outcome: MainlineHealthOutcome::Continue,
        diagnostic: MainlineHealthDiagnostic::ChecksPassed,
        baseline_checks: Vec::new(),
        newly_red_checks: Vec::new(),
    }
}

pub fn legacy_status_evidence(raw: &str) -> Result<(String, bool, Vec<CheckEvidence>), String> {
    let mut root = object(raw, "legacy status")?;
    let state = take_optional_string(&mut root, "state", "legacy status")?.unwrap_or_default();
    let has_total_count = root.contains_key("total_count");
    let statuses = take_optional_array(&mut root, "statuses", "legacy status")?;
    let mut evidence = Vec::new();
    for value in statuses {
        let mut status = value.into_object("legacy status entry")?;
        let name = take_optional_string(&mut status, "context", "legacy status entry")?
            .unwrap_or_else(|| "legacy-status".to_string());
        let conclusion = take_optional_string(&mut status, "state", "legacy status entry")?
            .unwrap_or_else(|| "pending".to_string());
        let check_status = if conclusion == "pending" {
            "pending"
        } else {
            "completed"
        };
        evidence.push(CheckEvidence::required(
            name,
            check_status,
            Some(&conclusion),
        ));
    }
    Ok((state, has_total_count, evidence))
}

pub fn check_run_evidence(raw: &str) -> Result<Vec<CheckEvidence>, String> {
    let mut root = object(raw, "check runs")?;
    let runs = take_optional_array(&mut root, "check_runs", "check runs")?;
    let mut evidence = Vec::new();
    for value in runs {
        let mut run = value.into_object("check run")?;
        let name = take_optional_string(&mut run, "name", "check run")?
            .unwrap_or_else(|| "check-run".to_string());
        let status = take_optional_string(&mut run, "status", "check run")?
            .unwrap_or_else(|| "queued".to_string());
        let conclusion = take_optional_string(&mut run, "conclusion", "check run")?;
        evidence.push(CheckEvidence::required(name, status, conclusion.as_deref()));
    }
    Ok(evidence)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn object(raw: &str, context: &str) -> Result<BTreeMap<String, JsonValue>, String> {
    JsonParser::new(raw).parse()?.into_object(context)
}

fn take_optional_string(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<String>, String> {
    match object.remove(key) {
        Some(value) => value.into_optional_string(&format!("{context}.{key}")),
        None => Ok(None),
    }
}

fn take_optional_array(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Vec<JsonValue>, String> {
    match object.remove(key) {
        Some(value) => value.into_array(&format!("{context}.{key}")),
        None => Ok(Vec::new()),
    }
}

fn check_json(check: &CheckEvidence) -> String {
    format!(
        "{{\"name\":\"{}\",\"status\":\"{}\",\"conclusion\":{},\"required\":{}}}",
        escape_json(&check.name),
        escape_json(&check.status),
        check
            .conclusion
            .as_ref()
            .map(|value| format!("\"{}\"", escape_json(value)))
            .unwrap_or_else(|| "null".to_string()),
        check.required
    )
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}
