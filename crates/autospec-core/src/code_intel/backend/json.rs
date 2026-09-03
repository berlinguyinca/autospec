//! Shared JSON readers for backend adapters.
//!
//! Backends differ in their tool names and payload envelopes but agree on LSP's
//! underlying shapes — a path, a range, a numeric severity, a code that is
//! either a string or an integer. Keeping those readers here means a second
//! adapter (multilspy, lsproxy) reuses them instead of re-deriving them.

use serde_json::Value;

use super::super::error::CodeIntelError;
use super::super::schema::{Location, Position};

pub(super) fn parse_json(payload: &str) -> Result<Value, CodeIntelError> {
    serde_json::from_str(payload)
        .map_err(|error| CodeIntelError::backend(format!("malformed agent-lsp payload: {error}")))
}

pub(super) fn entries(payload: &str) -> Result<Vec<Value>, CodeIntelError> {
    let value = parse_json(payload)?;
    value
        .as_array()
        .cloned()
        .ok_or_else(|| CodeIntelError::backend("agent-lsp payload must be a JSON array"))
}

pub(super) fn group(value: &Value, key: &str) -> Result<Vec<Value>, CodeIntelError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(entries)) => Ok(entries.clone()),
        Some(_) => Err(CodeIntelError::backend(format!(
            "agent-lsp field {key} must be an array"
        ))),
    }
}

pub(super) fn string_group(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn kind_of(value: &Value) -> String {
    value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_lowercase()
}

/// LSP diagnostic codes are either a string or an integer; normalize both.
pub(super) fn code_of(value: &Value) -> Option<String> {
    match value.get("code") {
        Some(Value::String(code)) => Some(code.clone()),
        Some(Value::Number(code)) => Some(code.to_string()),
        _ => None,
    }
}

pub(super) fn location_from(value: &Value) -> Result<Location, CodeIntelError> {
    let path = required_str(value, "path")?;
    let range = value.get("range");
    let start = position_from(range.and_then(|range| range.get("start")))?;
    let end = position_from(range.and_then(|range| range.get("end")))?;
    Ok(Location::new(path, start, end.max(start)))
}

pub(super) fn position_from(value: Option<&Value>) -> Result<Position, CodeIntelError> {
    let Some(value) = value else {
        return Ok(Position::new(0, 0));
    };
    Ok(Position::new(
        coordinate(value, "line")?,
        coordinate(value, "character")?,
    ))
}

pub(super) fn coordinate(value: &Value, key: &str) -> Result<u32, CodeIntelError> {
    let raw = value.get(key).and_then(Value::as_u64).unwrap_or(0);
    u32::try_from(raw)
        .map_err(|_| CodeIntelError::backend(format!("agent-lsp {key} coordinate out of range")))
}

pub(super) fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(super) fn required_str(value: &Value, key: &str) -> Result<String, CodeIntelError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CodeIntelError::backend(format!("agent-lsp entry is missing {key}")))
}
