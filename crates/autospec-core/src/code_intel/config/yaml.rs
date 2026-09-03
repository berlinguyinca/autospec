use std::collections::HashSet;

use yaml_edit::Mapping;

use super::super::error::CodeIntelError;

pub(super) fn section(mapping: &Mapping, key: &str) -> Result<Option<Mapping>, CodeIntelError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_mapping()
        .cloned()
        .map(Some)
        .ok_or_else(|| CodeIntelError::config(format!("{key} must be a mapping")))
}

pub(super) fn entry_key(
    entry: &yaml_edit::MappingEntry,
    label: &str,
) -> Result<String, CodeIntelError> {
    entry
        .key_node()
        .and_then(|node| node.as_scalar().map(|scalar| scalar.as_string()))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CodeIntelError::config(format!("{label} key must be a non-empty string")))
}

pub(super) fn optional_scalar(
    mapping: &Mapping,
    key: &str,
    label: &str,
) -> Result<Option<String>, CodeIntelError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_scalar()
        .map(|scalar| scalar.as_string())
        .map(Some)
        .ok_or_else(|| CodeIntelError::config(format!("{label} must be a scalar")))
}

pub(super) fn required_scalar(
    mapping: &Mapping,
    key: &str,
    label: &str,
) -> Result<String, CodeIntelError> {
    optional_scalar(mapping, key, label)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CodeIntelError::config(format!("{label} is required")))
}

pub(super) fn flag(mapping: &Mapping, key: &str, default: bool) -> Result<bool, CodeIntelError> {
    let Some(value) = optional_scalar(mapping, key, key)? else {
        return Ok(default);
    };
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(CodeIntelError::config(format!(
            "{key} must be true or false, got: {other}"
        ))),
    }
}

pub(super) fn unsigned(
    mapping: &Mapping,
    key: &str,
    label: &str,
    default: u64,
) -> Result<u64, CodeIntelError> {
    let Some(value) = optional_scalar(mapping, key, label)? else {
        return Ok(default);
    };
    value
        .parse::<u64>()
        .map_err(|_| CodeIntelError::config(format!("{label} must be a non-negative integer")))
}

pub(super) fn validate_keys(
    mapping: &Mapping,
    allowed: &[&str],
    label: &str,
) -> Result<(), CodeIntelError> {
    let mut seen = HashSet::new();
    for key in mapping.keys() {
        let key = key
            .as_scalar()
            .map(|scalar| scalar.as_string())
            .ok_or_else(|| CodeIntelError::config(format!("{label} key must be a string")))?;
        if !seen.insert(key.clone()) {
            return Err(CodeIntelError::config(format!(
                "duplicate {label} key: {key}"
            )));
        }
        if !allowed.contains(&key.as_str()) {
            return Err(CodeIntelError::config(format!(
                "unknown key in {label}: {key}"
            )));
        }
    }
    Ok(())
}
