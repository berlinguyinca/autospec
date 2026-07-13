use std::collections::BTreeMap;

use crate::state::json::JsonValue;

use super::queue::{
    FailureKind, QueueEntry, QueueStatus, QueueValidationResult, QueueValidationStatus,
    LEGACY_QUEUE_SCHEMA,
};

pub(super) fn parse_entry(value: JsonValue, schema: u64) -> Result<QueueEntry, String> {
    let mut object = value.into_object("queue entry")?;
    validate_entry_keys(&object, schema)?;
    let fields = take_entry_fields(&mut object, schema)?;
    Ok(QueueEntry {
        spec_id: fields.spec_id,
        status: fields.status,
        attempts: fields.attempts,
        failure_kind: fields.failure_kind,
        blocker: fields.blocker,
        started_at: fields.started_at,
        updated_at: fields.updated_at,
        validation: fields.validation,
        agent_result_ids: fields.agent_result_ids,
    })
}

fn validate_entry_keys(object: &BTreeMap<String, JsonValue>, schema: u64) -> Result<(), String> {
    let expected = [
        "spec_id",
        "status",
        "attempts",
        "failure_kind",
        "blocker",
        "started_at",
        "updated_at",
        "validation",
    ];
    if schema == LEGACY_QUEUE_SCHEMA {
        require_keys(object, &expected, "queue entry")?;
    } else {
        require_keys(
            object,
            &[
                "spec_id",
                "status",
                "attempts",
                "failure_kind",
                "blocker",
                "started_at",
                "updated_at",
                "validation",
                "agent_result_ids",
            ],
            "queue entry",
        )?;
    }
    Ok(())
}

struct EntryFields {
    spec_id: String,
    status: QueueStatus,
    attempts: u32,
    failure_kind: Option<FailureKind>,
    blocker: Option<String>,
    started_at: Option<u64>,
    updated_at: u64,
    validation: Option<QueueValidationResult>,
    agent_result_ids: Vec<String>,
}

fn take_entry_fields(
    object: &mut BTreeMap<String, JsonValue>,
    schema: u64,
) -> Result<EntryFields, String> {
    let spec_id = take(object, "spec_id", "queue entry")?.into_string("spec_id")?;
    let status =
        QueueStatus::parse(&take(object, "status", "queue entry")?.into_string("status")?)?;
    let attempts = u32::try_from(take(object, "attempts", "queue entry")?.into_number("attempts")?)
        .map_err(|_| "attempt count exceeds u32".to_string())?;
    let failure_kind =
        optional_string(take(object, "failure_kind", "queue entry")?, "failure_kind")?
            .map(|value| FailureKind::parse(&value))
            .transpose()?;
    let blocker = optional_string(take(object, "blocker", "queue entry")?, "blocker")?;
    let started_at = optional_number(take(object, "started_at", "queue entry")?, "started_at")?;
    let updated_at = take(object, "updated_at", "queue entry")?.into_number("updated_at")?;
    let validation = optional_validation(take(object, "validation", "queue entry")?)?;
    let agent_result_ids = if schema == LEGACY_QUEUE_SCHEMA {
        Vec::new()
    } else {
        take(object, "agent_result_ids", "queue entry")?
            .into_array("agent_result_ids")?
            .into_iter()
            .map(|value| value.into_string("agent_result_id"))
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(EntryFields {
        spec_id,
        status,
        attempts,
        failure_kind,
        blocker,
        started_at,
        updated_at,
        validation,
        agent_result_ids,
    })
}

fn optional_validation(value: JsonValue) -> Result<Option<QueueValidationResult>, String> {
    match value {
        JsonValue::Null => Ok(None),
        value => {
            let mut object = value.into_object("validation")?;
            require_keys(&object, &["status", "summary"], "validation")?;
            Ok(Some(QueueValidationResult::new(
                QueueValidationStatus::parse(
                    &take(&mut object, "status", "validation")?.into_string("status")?,
                )?,
                take(&mut object, "summary", "validation")?.into_string("summary")?,
            )))
        }
    }
}

fn optional_string(value: JsonValue, name: &str) -> Result<Option<String>, String> {
    value.into_optional_string(name)
}

fn optional_number(value: JsonValue, name: &str) -> Result<Option<u64>, String> {
    match value {
        JsonValue::Null => Ok(None),
        value => value
            .into_number(name)
            .map(Some)
            .map_err(|_| format!("{name} must be a JSON number or null")),
    }
}

fn take(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {key} in {context}"))
}

fn require_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    for key in object.keys() {
        if !expected.contains(&key.as_str()) {
            return Err(format!("unknown key {key} in {context}"));
        }
    }
    Ok(())
}
