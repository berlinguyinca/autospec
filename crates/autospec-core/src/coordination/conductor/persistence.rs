use std::collections::BTreeMap;

use crate::state::json::{JsonParser, JsonValue};

use super::{ConductorOutcome, ConductorPhase, ConductorScope, ConductorState, CONDUCTOR_SCHEMA};

impl ConductorScope {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Slice => "slice",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "repository" => Ok(Self::Repository),
            "slice" => Ok(Self::Slice),
            _ => Err(format!("unknown conductor scope: {value}")),
        }
    }
}

impl ConductorPhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Review => "review",
            Self::Select => "select",
            Self::Claim => "claim",
            Self::Dispatch => "dispatch",
            Self::DispatchRecorded => "dispatch_recorded",
            Self::Retry => "retry",
            Self::Paused => "paused",
            Self::SliceComplete => "slice_complete",
            Self::AllDone => "all_done",
            Self::TierDry => "tier_dry",
            Self::AllBlocked => "all_blocked",
            Self::VerifierUnavailable => "verifier_unavailable",
            Self::IdleRescan => "idle_rescan",
            Self::ResourcePark => "resource_park",
            Self::OperatorStop => "operator_stop",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "scan" => Ok(Self::Scan),
            "review" => Ok(Self::Review),
            "select" => Ok(Self::Select),
            "claim" => Ok(Self::Claim),
            "dispatch" => Ok(Self::Dispatch),
            "dispatch_recorded" => Ok(Self::DispatchRecorded),
            "retry" => Ok(Self::Retry),
            "paused" => Ok(Self::Paused),
            "slice_complete" => Ok(Self::SliceComplete),
            "all_done" => Ok(Self::AllDone),
            "tier_dry" => Ok(Self::TierDry),
            "all_blocked" => Ok(Self::AllBlocked),
            "verifier_unavailable" => Ok(Self::VerifierUnavailable),
            "idle_rescan" => Ok(Self::IdleRescan),
            "resource_park" => Ok(Self::ResourcePark),
            "operator_stop" => Ok(Self::OperatorStop),
            _ => Err(format!("unknown conductor phase: {value}")),
        }
    }
}

impl ConductorOutcome {
    fn to_json(&self) -> String {
        match self {
            Self::Succeeded => "{\"kind\":\"succeeded\",\"reason\":null}".to_string(),
            Self::Retryable(reason) => format!(
                "{{\"kind\":\"retryable\",\"reason\":\"{}\"}}",
                escape_json(reason)
            ),
            Self::Blocked(reason) => format!(
                "{{\"kind\":\"blocked\",\"reason\":\"{}\"}}",
                escape_json(reason)
            ),
            Self::AllBlocked { reason, issues } => format!(
                "{{\"kind\":\"all_blocked\",\"reason\":\"{}\",\"issues\":[{}]}}",
                escape_json(reason),
                issues
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::VerifierUnavailable { reason } => format!(
                "{{\"kind\":\"verifier_unavailable\",\"reason\":\"{}\"}}",
                escape_json(reason)
            ),
            Self::ResourcePark { reason } => format!(
                "{{\"kind\":\"resource_park\",\"reason\":\"{}\"}}",
                escape_json(reason)
            ),
            Self::OperatorStop { reason } => format!(
                "{{\"kind\":\"operator_stop\",\"reason\":\"{}\"}}",
                escape_json(reason)
            ),
        }
    }

    fn parse(value: JsonValue) -> Result<Self, String> {
        let mut object = value.into_object("conductor outcome")?;
        require_only_keys(&object, &["kind", "reason", "issues"], "conductor outcome")?;
        let kind = take_required(&mut object, "kind", "conductor outcome")?
            .into_string("conductor outcome.kind")?;
        let reason = take_required(&mut object, "reason", "conductor outcome")?
            .into_optional_string("conductor outcome.reason")?;
        let issues = optional_issues(object.remove("issues"), "conductor outcome.issues")?;
        let outcome = match (kind.as_str(), reason) {
            ("succeeded", None) => Ok(Self::Succeeded),
            ("retryable", Some(reason)) if !reason.is_empty() => Ok(Self::Retryable(reason)),
            ("blocked", Some(reason)) if !reason.is_empty() => Ok(Self::Blocked(reason)),
            ("all_blocked", Some(reason)) if !reason.is_empty() => Ok(Self::AllBlocked {
                reason,
                issues: issues.into_boxed_slice(),
            }),
            ("verifier_unavailable", Some(reason)) if !reason.is_empty() => {
                Ok(Self::VerifierUnavailable { reason })
            }
            ("resource_park", Some(reason)) if !reason.is_empty() => {
                Ok(Self::ResourcePark { reason })
            }
            ("operator_stop", Some(reason)) if !reason.is_empty() => {
                Ok(Self::OperatorStop { reason })
            }
            _ => Err("invalid conductor outcome".to_string()),
        }?;
        outcome.validate()?;
        Ok(outcome)
    }
}

impl ConductorState {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":{CONDUCTOR_SCHEMA},\"repo\":\"{}\",\"scope\":\"{}\",\"phase\":\"{}\",\"state\":\"{}\",\"selected_issue\":{},\"serialization_reasons\":[{}],\"retry_count\":{},\"retry_limit\":{},\"last_outcome\":{},\"pause_reason\":{},\"terminal_reason\":{},\"resume_phase\":{},\"no_progress_cycles\":{},\"no_progress_reason\":{},\"blocked_backlog_cycles\":{},\"blocked_backlog_reason\":{},\"blocked_backlog_issues\":[{}]}}",
            escape_json(&self.repo),
            self.scope.as_str(),
            self.phase.as_str(),
            self.normalized_state(),
            optional_number_json(self.selected_issue),
            self.serialization_reasons
                .iter()
                .map(|reason| format!("\"{}\"", escape_json(reason)))
                .collect::<Vec<_>>()
                .join(","),
            self.retry_count,
            self.retry_limit,
            self.last_outcome
                .as_ref()
                .map_or_else(|| "null".to_string(), ConductorOutcome::to_json),
            optional_string_json(self.pause_reason.as_deref()),
            optional_string_json(self.terminal_reason.as_deref()),
            self.resume_phase
                .as_ref()
                .map_or_else(|| "null".to_string(), |phase| format!("\"{}\"", phase.as_str())),
            self.no_progress_cycles,
            optional_string_json(self.no_progress_reason.as_deref()),
            self.blocked_backlog_cycles,
            optional_string_json(self.blocked_backlog_reason.as_deref()),
            self.blocked_backlog_issues
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(","),
        )
    }

    pub fn parse_json(input: &str) -> Result<Self, String> {
        let mut object = JsonParser::new(input)
            .parse()?
            .into_object("conductor state")?;
        require_only_keys(
            &object,
            &[
                "schema",
                "repo",
                "scope",
                "phase",
                "state",
                "selected_issue",
                "serialization_reasons",
                "retry_count",
                "retry_limit",
                "last_outcome",
                "pause_reason",
                "terminal_reason",
                "resume_phase",
                "no_progress_cycles",
                "no_progress_reason",
                "blocked_backlog_cycles",
                "blocked_backlog_reason",
                "blocked_backlog_issues",
            ],
            "conductor state",
        )?;
        let schema = take_required(&mut object, "schema", "conductor state")?
            .into_number("conductor state.schema")?;
        if schema != CONDUCTOR_SCHEMA {
            return Err(format!("unsupported conductor state schema: {schema}"));
        }
        let repo = take_required(&mut object, "repo", "conductor state")?
            .into_string("conductor state.repo")?;
        if repo.trim().is_empty() {
            return Err("conductor state.repo must not be empty".to_string());
        }
        let scope = ConductorScope::parse(
            &take_required(&mut object, "scope", "conductor state")?
                .into_string("conductor state.scope")?,
        )?;
        let phase = ConductorPhase::parse(
            &take_required(&mut object, "phase", "conductor state")?
                .into_string("conductor state.phase")?,
        )?;
        let normalized_state = optional_string(object.remove("state"), "conductor state.state")?;
        let selected_issue = optional_number(
            take_required(&mut object, "selected_issue", "conductor state")?,
            "conductor state.selected_issue",
        )?;
        if selected_issue == Some(0) {
            return Err("conductor state.selected_issue must be positive".to_string());
        }
        let serialization_reasons = parse_serialization_reasons(take_required(
            &mut object,
            "serialization_reasons",
            "conductor state",
        )?)?;
        let retry_count = take_required(&mut object, "retry_count", "conductor state")?
            .into_number("conductor state.retry_count")?
            .try_into()
            .map_err(|_| "conductor state.retry_count exceeds u32".to_string())?;
        let retry_limit = take_required(&mut object, "retry_limit", "conductor state")?
            .into_number("conductor state.retry_limit")?
            .try_into()
            .map_err(|_| "conductor state.retry_limit exceeds u32".to_string())?;
        let last_outcome = optional_outcome(take_required(
            &mut object,
            "last_outcome",
            "conductor state",
        )?)?;
        let pause_reason = take_required(&mut object, "pause_reason", "conductor state")?
            .into_optional_string("conductor state.pause_reason")?;
        let terminal_reason = take_required(&mut object, "terminal_reason", "conductor state")?
            .into_optional_string("conductor state.terminal_reason")?;
        let resume_phase = optional_phase(
            take_required(&mut object, "resume_phase", "conductor state")?,
            "conductor state.resume_phase",
        )?;
        let no_progress_cycles = object
            .remove("no_progress_cycles")
            .map(|value| {
                value
                    .into_number("conductor state.no_progress_cycles")?
                    .try_into()
                    .map_err(|_| "conductor state.no_progress_cycles exceeds u32".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        let no_progress_reason = optional_string(
            object.remove("no_progress_reason"),
            "conductor state.no_progress_reason",
        )?;
        let blocked_backlog_cycles = object
            .remove("blocked_backlog_cycles")
            .map(|value| {
                value
                    .into_number("conductor state.blocked_backlog_cycles")?
                    .try_into()
                    .map_err(|_| "conductor state.blocked_backlog_cycles exceeds u32".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        let blocked_backlog_reason = optional_string(
            object.remove("blocked_backlog_reason"),
            "conductor state.blocked_backlog_reason",
        )?;
        let blocked_backlog_issues = optional_issues(
            object.remove("blocked_backlog_issues"),
            "conductor state.blocked_backlog_issues",
        )?;
        let state = Self {
            repo,
            scope,
            phase,
            selected_issue,
            serialization_reasons,
            retry_count,
            retry_limit,
            last_outcome,
            pause_reason,
            terminal_reason,
            resume_phase,
            no_progress_cycles,
            no_progress_reason,
            blocked_backlog_cycles,
            blocked_backlog_reason,
            blocked_backlog_issues,
        };
        state.validate()?;
        if let Some(normalized_state) = normalized_state {
            let legacy_blocked_scan = normalized_state == "blocked"
                && state.phase == ConductorPhase::Scan
                && state.selected_issue.is_none()
                && state.no_progress_reason.is_some()
                && matches!(state.last_outcome, Some(ConductorOutcome::Blocked(_)));
            if normalized_state != state.normalized_state() && !legacy_blocked_scan {
                return Err(
                    "conductor state normalized state does not match phase/outcome".to_string(),
                );
            }
        }
        Ok(state)
    }
}

fn require_only_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        return Err(format!("unexpected {context} field: {key}"));
    }
    Ok(())
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {context} field: {key}"))
}

fn optional_number(value: JsonValue, context: &str) -> Result<Option<u64>, String> {
    match value {
        JsonValue::Null => Ok(None),
        value => value.into_number(context).map(Some),
    }
}

fn optional_string(value: Option<JsonValue>, context: &str) -> Result<Option<String>, String> {
    value.map_or(Ok(None), |value| {
        value
            .into_optional_string(context)
            .map(|value| value.filter(|value| !value.is_empty()))
    })
}

fn parse_serialization_reasons(value: JsonValue) -> Result<Vec<String>, String> {
    let values = value.into_array("conductor state.serialization_reasons")?;
    let mut reasons = Vec::with_capacity(values.len());
    for (index, value) in values.into_iter().enumerate() {
        let context = format!("conductor state.serialization_reasons[{index}]");
        reasons.push(value.into_string(&context)?);
    }
    Ok(reasons)
}

fn optional_outcome(value: JsonValue) -> Result<Option<ConductorOutcome>, String> {
    match value {
        JsonValue::Null => Ok(None),
        value => ConductorOutcome::parse(value).map(Some),
    }
}

fn optional_issues(value: Option<JsonValue>, context: &str) -> Result<Vec<u64>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value.into_array(context)?;
    let mut issues = Vec::with_capacity(values.len());
    for (index, value) in values.into_iter().enumerate() {
        let issue = value.into_number(&format!("{context}[{index}]"))?;
        if issue == 0 {
            return Err(format!("{context}[{index}] must be positive"));
        }
        issues.push(issue);
    }
    Ok(issues)
}

fn optional_phase(value: JsonValue, context: &str) -> Result<Option<ConductorPhase>, String> {
    match value {
        JsonValue::Null => Ok(None),
        value => ConductorPhase::parse(&value.into_string(context)?).map(Some),
    }
}

fn optional_number_json(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_string(), |number| number.to_string())
}

fn optional_string_json(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".to_string(),
        |value| format!("\"{}\"", escape_json(value)),
    )
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character if character.is_control() => format!("\\u{:04x}", character as u32)
                .chars()
                .collect::<Vec<_>>(),
            character => vec![character],
        })
        .collect()
}
