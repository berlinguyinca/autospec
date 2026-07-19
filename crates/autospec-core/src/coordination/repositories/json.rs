use std::collections::BTreeMap;

use crate::state::json::{JsonParser, JsonValue};

use super::{RepositoryEvidence, RepositoryFinding, RepositoryRoutingInput};

pub fn parse_repository_routing_input_json(input: &str) -> Result<RepositoryRoutingInput, String> {
    let mut root = JsonParser::new(input)
        .parse()?
        .into_object("repository routing input")?;
    reject_unknown_keys(
        &root,
        &["repositories", "findings"],
        "repository routing input",
    )?;
    let repositories = take_required(&mut root, "repositories", "repository routing input")?
        .into_array("repository routing input.repositories")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| parse_repository(value, &format!("repositories[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    let findings = take_optional(&mut root, "findings")
        .unwrap_or(JsonValue::Array(Vec::new()))
        .into_array("repository routing input.findings")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| parse_finding(value, &format!("findings[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RepositoryRoutingInput {
        repositories,
        findings,
    })
}

fn parse_repository(value: JsonValue, context: &str) -> Result<RepositoryEvidence, String> {
    let mut object = value.into_object(context)?;
    reject_unknown_keys(
        &object,
        &[
            "name",
            "archived",
            "pushed_at",
            "readme",
            "module_paths",
            "packages",
            "dependency_references",
            "revival_requested",
        ],
        context,
    )?;
    Ok(RepositoryEvidence {
        name: take_required(&mut object, "name", context)?
            .into_string(&format!("{context}.name"))?,
        archived: take_optional(&mut object, "archived")
            .unwrap_or(JsonValue::Bool(false))
            .into_bool(&format!("{context}.archived"))?,
        pushed_at: take_optional(&mut object, "pushed_at")
            .unwrap_or(JsonValue::Null)
            .into_optional_string(&format!("{context}.pushed_at"))?,
        readme: take_optional(&mut object, "readme")
            .unwrap_or(JsonValue::String(String::new()))
            .into_string(&format!("{context}.readme"))?,
        module_paths: string_array(
            take_optional(&mut object, "module_paths"),
            &format!("{context}.module_paths"),
        )?,
        packages: string_array(
            take_optional(&mut object, "packages"),
            &format!("{context}.packages"),
        )?,
        dependency_references: string_array(
            take_optional(&mut object, "dependency_references"),
            &format!("{context}.dependency_references"),
        )?,
        revival_requested: take_optional(&mut object, "revival_requested")
            .unwrap_or(JsonValue::Bool(false))
            .into_bool(&format!("{context}.revival_requested"))?,
    })
}

fn parse_finding(value: JsonValue, context: &str) -> Result<RepositoryFinding, String> {
    let mut object = value.into_object(context)?;
    reject_unknown_keys(
        &object,
        &["repository", "fingerprint", "title", "evidence"],
        context,
    )?;
    Ok(RepositoryFinding {
        repository: take_required(&mut object, "repository", context)?
            .into_string(&format!("{context}.repository"))?,
        fingerprint: take_required(&mut object, "fingerprint", context)?
            .into_string(&format!("{context}.fingerprint"))?,
        title: take_optional(&mut object, "title")
            .unwrap_or(JsonValue::String(String::new()))
            .into_string(&format!("{context}.title"))?,
        evidence: take_optional(&mut object, "evidence")
            .unwrap_or(JsonValue::String(String::new()))
            .into_string(&format!("{context}.evidence"))?,
    })
}

fn string_array(value: Option<JsonValue>, context: &str) -> Result<Vec<String>, String> {
    value
        .unwrap_or(JsonValue::Array(Vec::new()))
        .into_array(context)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| value.into_string(&format!("{context}[{index}]")))
        .collect()
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("{context}.{key} is required"))
}

fn take_optional(object: &mut BTreeMap<String, JsonValue>, key: &str) -> Option<JsonValue> {
    object.remove(key)
}

fn reject_unknown_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    for key in object.keys() {
        if !expected.contains(&key.as_str()) {
            return Err(format!("unknown {context} key: {key}"));
        }
    }
    Ok(())
}
