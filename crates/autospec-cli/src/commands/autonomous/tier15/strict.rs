use std::collections::BTreeMap;

use autospec_core::coordination::{parse_remote_issue_page_json, RemoteIssuePage};
use autospec_core::state::json::{JsonParser, JsonValue};

pub(super) fn parse_projected_page_bytes(input: &[u8]) -> Result<RemoteIssuePage, String> {
    let input = std::str::from_utf8(input).map_err(|_| {
        "could not parse projected GitHub issue page: output is not valid UTF-8".to_string()
    })?;
    validate_projected_shape(input)
        .map_err(|error| format!("could not parse projected GitHub issue page: {error}"))?;
    parse_remote_issue_page_json(input)
        .map_err(|error| format!("could not parse projected GitHub issue page: {error}"))
}

fn validate_projected_shape(input: &str) -> Result<(), String> {
    let value = JsonParser::new(input)
        .parse()
        .map_err(|error| format!("could not parse projected GitHub issue page: {error}"))?;
    let page = require_object(&value, "projected GitHub issue page")?;
    require_exact_keys(page, &["raw_count", "items"], "projected GitHub issue page")?;
    require_number(
        required(page, "raw_count", "projected GitHub issue page")?,
        "raw_count",
    )?;
    let items = require_array(
        required(page, "items", "projected GitHub issue page")?,
        "projected GitHub issue page.items",
    )?;
    for (index, item) in items.iter().enumerate() {
        validate_item(item, index)?;
    }
    Ok(())
}

fn validate_item(item: &JsonValue, index: usize) -> Result<(), String> {
    let context = format!("projected GitHub issue page.items[{index}]");
    let item = require_object(item, &context)?;
    require_exact_keys(
        item,
        &["number", "title", "body", "labels", "author", "state"],
        &context,
    )?;
    require_number(
        required(item, "number", &context)?,
        &format!("{context}.number"),
    )?;
    require_string(
        required(item, "title", &context)?,
        &format!("{context}.title"),
    )?;
    require_string(
        required(item, "body", &context)?,
        &format!("{context}.body"),
    )?;
    let labels = require_array(
        required(item, "labels", &context)?,
        &format!("{context}.labels"),
    )?;
    for (label_index, label) in labels.iter().enumerate() {
        require_string(label, &format!("{context}.labels[{label_index}]"))?;
    }
    let author = require_object(
        required(item, "author", &context)?,
        &format!("{context}.author"),
    )?;
    require_exact_keys(author, &["login"], &format!("{context}.author"))?;
    require_string(
        required(author, "login", &format!("{context}.author"))?,
        &format!("{context}.author.login"),
    )?;
    let state = require_string(
        required(item, "state", &context)?,
        &format!("{context}.state"),
    )?;
    if !state.eq_ignore_ascii_case("OPEN") && !state.eq_ignore_ascii_case("CLOSED") {
        return Err(format!("{context}.state must be OPEN or CLOSED"));
    }
    Ok(())
}

fn require_exact_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    for key in expected {
        if !object.contains_key(*key) {
            return Err(format!("{context}.{key} is required"));
        }
    }
    if let Some(key) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        return Err(format!("{context} contains unexpected key: {key}"));
    }
    Ok(())
}

fn required<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<&'a JsonValue, String> {
    object
        .get(key)
        .ok_or_else(|| format!("{context}.{key} is required"))
}

fn require_object<'a>(
    value: &'a JsonValue,
    context: &str,
) -> Result<&'a BTreeMap<String, JsonValue>, String> {
    match value {
        JsonValue::Object(object) => Ok(object),
        _ => Err(format!("{context} must be a JSON object")),
    }
}

fn require_array<'a>(value: &'a JsonValue, context: &str) -> Result<&'a [JsonValue], String> {
    match value {
        JsonValue::Array(array) => Ok(array),
        _ => Err(format!("{context} must be a JSON array")),
    }
}

fn require_number(value: &JsonValue, context: &str) -> Result<(), String> {
    if matches!(value, JsonValue::Number(_)) {
        Ok(())
    } else {
        Err(format!("{context} must be a JSON number"))
    }
}

fn require_string<'a>(value: &'a JsonValue, context: &str) -> Result<&'a str, String> {
    match value {
        JsonValue::String(value) => Ok(value),
        _ => Err(format!("{context} must be a JSON string")),
    }
}
