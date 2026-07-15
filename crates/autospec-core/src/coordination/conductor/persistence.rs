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
        }
    }

    fn parse(value: JsonValue) -> Result<Self, String> {
        let mut object = value.into_object("conductor outcome")?;
        require_only_keys(&object, &["kind", "reason"], "conductor outcome")?;
        let kind = take_required(&mut object, "kind", "conductor outcome")?
            .into_string("conductor outcome.kind")?;
        let reason = take_required(&mut object, "reason", "conductor outcome")?
            .into_optional_string("conductor outcome.reason")?;
        let outcome = match (kind.as_str(), reason) {
            ("succeeded", None) => Ok(Self::Succeeded),
            ("retryable", Some(reason)) if !reason.is_empty() => Ok(Self::Retryable(reason)),
            ("blocked", Some(reason)) if !reason.is_empty() => Ok(Self::Blocked(reason)),
            _ => Err("invalid conductor outcome".to_string()),
        }?;
        outcome.validate()?;
        Ok(outcome)
    }
}

impl ConductorState {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":{CONDUCTOR_SCHEMA},\"repo\":\"{}\",\"scope\":\"{}\",\"phase\":\"{}\",\"selected_issue\":{},\"serialization_reasons\":[{}],\"retry_count\":{},\"retry_limit\":{},\"last_outcome\":{},\"pause_reason\":{},\"terminal_reason\":{},\"resume_phase\":{}}}",
            escape_json(&self.repo),
            self.scope.as_str(),
            self.phase.as_str(),
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
                "selected_issue",
                "serialization_reasons",
                "retry_count",
                "retry_limit",
                "last_outcome",
                "pause_reason",
                "terminal_reason",
                "resume_phase",
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
        };
        state.validate()?;
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
