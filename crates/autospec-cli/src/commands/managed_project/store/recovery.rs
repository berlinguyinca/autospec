use super::{ManagedProjectError, JOURNAL_SCHEMA};
use crate::commands::managed_project::{
    empty_journal_digest, extend_journal_digest, io_error, open_private_file,
    open_private_file_read_only,
};
use autospec_core::managed_project::{ItemKey, PortfolioId, ProductKey, SourceSpecIdentity};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) struct JournalEvent {
    pub(super) sequence: u64,
    pub(super) product_key: ProductKey,
    pub(super) key: String,
    pub(super) kind: String,
    pub(super) payload: Value,
}

impl JournalEvent {
    pub(super) fn to_value(&self) -> Value {
        json!({
            "schema": JOURNAL_SCHEMA,
            "sequence": self.sequence,
            "product_key": self.product_key,
            "key": self.key,
            "kind": self.kind,
            "payload": self.payload,
        })
    }

    fn from_value(value: Value) -> Result<Self, ManagedProjectError> {
        let object = value
            .as_object()
            .ok_or_else(|| ManagedProjectError::new("journal event must be an object"))?;
        if object.get("schema").and_then(Value::as_u64) != Some(JOURNAL_SCHEMA) {
            return Err(ManagedProjectError::new(
                "unsupported managed project journal schema",
            ));
        }
        let sequence = object
            .get("sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| ManagedProjectError::new("journal event sequence must be unsigned"))?;
        let product_key = serde_json::from_value(
            object
                .get("product_key")
                .cloned()
                .ok_or_else(|| ManagedProjectError::new("journal event has no product key"))?,
        )
        .map_err(|error| {
            ManagedProjectError::new(format!("invalid journal event product key: {error}"))
        })?;
        let string = |field: &str| {
            object
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    ManagedProjectError::new(format!("journal event has invalid {field}"))
                })
        };
        Ok(Self {
            sequence,
            product_key,
            key: string("key")?,
            kind: string("kind")?,
            payload: object
                .get("payload")
                .cloned()
                .ok_or_else(|| ManagedProjectError::new("journal event has no payload"))?,
        })
    }
}

pub(super) fn payload_string<'a>(
    event: &'a JournalEvent,
    description: &str,
) -> Result<&'a str, ManagedProjectError> {
    event
        .payload
        .as_str()
        .ok_or_else(|| ManagedProjectError::new(format!("{description} payload must be a string")))
}

pub(super) fn recover_events(
    path: &Path,
    product_key: &ProductKey,
    repair_truncated_tail: bool,
) -> Result<RecoveredJournal, ManagedProjectError> {
    let mut file = if repair_truncated_tail {
        open_private_file(path)?
    } else {
        open_private_file_read_only(path)?
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        if repair_truncated_tail {
            file.set_len(complete as u64).map_err(io_error)?;
            file.seek(SeekFrom::Start(complete as u64))
                .map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
        }
        bytes.truncate(complete);
    }
    let mut events = Vec::new();
    let mut prefix_digests = vec![empty_journal_digest()];
    if bytes.is_empty() {
        return Ok(RecoveredJournal {
            events,
            final_digest: prefix_digests[0].clone(),
            prefix_digests,
        });
    }
    let lines = bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            if index + 1 == lines.len() {
                continue;
            }
            return Err(ManagedProjectError::new(
                "managed project journal contains an empty completed line",
            ));
        }
        let value = serde_json::from_slice(line).map_err(|error| {
            ManagedProjectError::new(format!("invalid completed journal line: {error}"))
        })?;
        let event = JournalEvent::from_value(value)?;
        let expected = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| ManagedProjectError::new("managed project sequence overflow"))?;
        if event.sequence != expected {
            return Err(ManagedProjectError::new(
                "managed project journal sequence is not contiguous",
            ));
        }
        if &event.product_key != product_key {
            return Err(ManagedProjectError::new(
                "journal event product key does not match its state directory",
            ));
        }
        let mut complete_line = line.to_vec();
        complete_line.push(b'\n');
        let digest = extend_journal_digest(
            prefix_digests.last().expect("initial digest exists"),
            &complete_line,
        );
        prefix_digests.push(digest);
        events.push(event);
    }
    Ok(RecoveredJournal {
        final_digest: prefix_digests
            .last()
            .expect("initial digest exists")
            .clone(),
        events,
        prefix_digests,
    })
}

pub(super) struct RecoveredJournal {
    pub(super) events: Vec<JournalEvent>,
    pub(super) prefix_digests: Vec<String>,
    pub(super) final_digest: String,
}

pub(super) fn validate_portfolio_snapshot(snapshot: &Value) -> Result<(), ManagedProjectError> {
    let object = snapshot
        .as_object()
        .ok_or_else(|| ManagedProjectError::new("portfolio snapshot must be an object"))?;
    if object.get("schema").and_then(Value::as_str) != Some("autospec.portfolio-snapshot.v1") {
        return Err(ManagedProjectError::new(
            "unsupported portfolio snapshot schema",
        ));
    }
    let portfolio_id = required_portfolio_id(object, "portfolio_id", "portfolio snapshot")?;
    let source = required_source_spec(object, "source_spec", "portfolio snapshot")?;
    if source.portfolio_id() != portfolio_id {
        return Err(ManagedProjectError::new(
            "portfolio snapshot identity does not match its source spec",
        ));
    }
    required_nonempty_string(object, "owner", "portfolio snapshot")?;
    required_nonempty_string(object, "project_node_id", "portfolio snapshot")?;
    required_nonempty_string(object, "state", "portfolio snapshot")?;
    object
        .get("project_number")
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .ok_or_else(|| ManagedProjectError::new("portfolio project number must be positive"))?;
    let project_url = required_nonempty_string(object, "project_url", "portfolio snapshot")?;
    if !project_url.starts_with("https://github.com/") {
        return Err(ManagedProjectError::new(
            "portfolio project URL must be canonical HTTPS",
        ));
    }
    let plan_digest = required_digest(object, "plan_digest", 64, "portfolio snapshot")?;
    object
        .get("lease_generation")
        .and_then(Value::as_u64)
        .ok_or_else(|| ManagedProjectError::new("portfolio lease generation must be unsigned"))?;
    object
        .get("projection_high_watermark")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ManagedProjectError::new("portfolio projection high-water mark must be unsigned")
        })?;
    let capsule = object
        .get("recovery_capsule")
        .ok_or_else(|| ManagedProjectError::new("portfolio snapshot has no recovery capsule"))?;
    validate_recovery_capsule(capsule, &portfolio_id, plan_digest)
}

pub(super) fn validate_portfolio_item_binding(
    snapshot: &Value,
    existing: &[Value],
    binding: &Value,
) -> Result<(), ManagedProjectError> {
    validate_portfolio_snapshot(snapshot)?;
    let capsule_items = snapshot["recovery_capsule"]["items"]
        .as_array()
        .expect("validated recovery capsule has items");
    let expected = capsule_items.get(existing.len()).ok_or_else(|| {
        ManagedProjectError::new("portfolio item binding exceeds the frozen recovery capsule")
    })?;
    let object = binding
        .as_object()
        .ok_or_else(|| ManagedProjectError::new("portfolio item binding must be an object"))?;
    let item_key = required_item_key(object, "item_key", "portfolio item binding")?;
    if expected["item_key"].as_str() != Some(item_key.as_str()) {
        return Err(ManagedProjectError::new(
            "portfolio item bindings must follow frozen capsule order",
        ));
    }
    let repository = required_nonempty_string(object, "repository", "portfolio item binding")?;
    if expected["repository"].as_str() != Some(repository) {
        return Err(ManagedProjectError::new(
            "portfolio item binding repository does not match recovery capsule",
        ));
    }
    let issue_url = required_nonempty_string(object, "issue_url", "portfolio item binding")?;
    let expected_prefix = format!("https://github.com/{repository}/issues/");
    if !issue_url
        .strip_prefix(&expected_prefix)
        .is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(ManagedProjectError::new(
            "portfolio item binding issue URL is not canonical",
        ));
    }
    required_nonempty_string(object, "role", "portfolio item binding")?;
    let dependencies = required_string_array(object, "dependencies", "portfolio item binding")?;
    if expected["dependencies"].as_array() != Some(&dependencies) {
        return Err(ManagedProjectError::new(
            "portfolio item binding dependencies do not match recovery capsule",
        ));
    }
    if !matches!(
        object.get("terminal_state"),
        Some(Value::Null | Value::String(_))
    ) {
        return Err(ManagedProjectError::new(
            "portfolio item binding terminal state must be null or a string",
        ));
    }
    Ok(())
}

pub(super) fn validate_operation_transition(
    operations: &[Value],
    operation_id: &str,
    state: &str,
    payload: &Value,
) -> Result<(), ManagedProjectError> {
    ItemKey::new(operation_id).map_err(ManagedProjectError::new)?;
    if !payload.is_object() {
        return Err(ManagedProjectError::new(
            "portfolio operation payload must be an object",
        ));
    }
    let previous = operations.iter().rev().find_map(|operation| {
        (operation["operation_id"].as_str() == Some(operation_id))
            .then(|| operation["state"].as_str())
            .flatten()
    });
    let expected = match previous {
        None => "intent",
        Some("intent") => "sent",
        Some("sent") => "acknowledged",
        Some("acknowledged") => {
            return Err(ManagedProjectError::new(
                "acknowledged portfolio operation cannot transition again",
            ))
        }
        Some(_) => {
            return Err(ManagedProjectError::new(
                "invalid stored portfolio operation state",
            ))
        }
    };
    if state != expected {
        return Err(ManagedProjectError::new(format!(
            "portfolio operation must transition to {expected}"
        )));
    }
    Ok(())
}

fn validate_recovery_capsule(
    capsule: &Value,
    portfolio_id: &PortfolioId,
    plan_digest: &str,
) -> Result<(), ManagedProjectError> {
    let object = capsule
        .as_object()
        .ok_or_else(|| ManagedProjectError::new("portfolio recovery capsule must be an object"))?;
    if object.get("schema").and_then(Value::as_str) != Some("autospec.portfolio-recovery.v1") {
        return Err(ManagedProjectError::new(
            "unsupported portfolio recovery capsule schema",
        ));
    }
    let capsule_id = required_portfolio_id(object, "portfolio_id", "portfolio recovery capsule")?;
    if &capsule_id != portfolio_id {
        return Err(ManagedProjectError::new(
            "portfolio recovery capsule identity does not match snapshot",
        ));
    }
    if required_digest(object, "plan_digest", 64, "portfolio recovery capsule")? != plan_digest {
        return Err(ManagedProjectError::new(
            "portfolio recovery capsule plan digest does not match snapshot",
        ));
    }
    required_digest(object, "create_nonce", 32, "portfolio recovery capsule")?;
    let items = object
        .get("items")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| ManagedProjectError::new("portfolio recovery capsule has no items"))?;
    let mut keys = HashSet::new();
    let mut repositories = HashMap::new();
    for item in items {
        let item = item.as_object().ok_or_else(|| {
            ManagedProjectError::new("portfolio recovery capsule item must be an object")
        })?;
        let key = required_item_key(item, "item_key", "portfolio recovery capsule item")?;
        if !keys.insert(key.to_string()) {
            return Err(ManagedProjectError::new(
                "portfolio recovery capsule contains duplicate item keys",
            ));
        }
        let repository =
            required_nonempty_string(item, "repository", "portfolio recovery capsule item")?;
        validate_repository(repository)?;
        repositories.insert(key.to_string(), repository.to_owned());
        required_string_array(item, "local_parents", "portfolio recovery capsule item")?;
        required_string_array(item, "dependencies", "portfolio recovery capsule item")?;
    }
    for item in items {
        let item = item
            .as_object()
            .expect("validated capsule item is an object");
        let key = item["item_key"].as_str().expect("validated item key");
        for dependency in array_strings(&item["dependencies"]) {
            if dependency == key || !keys.contains(dependency) {
                return Err(ManagedProjectError::new(
                    "portfolio recovery capsule contains an invalid dependency",
                ));
            }
        }
        for parent in array_strings(&item["local_parents"]) {
            if parent == key
                || !keys.contains(parent)
                || repositories.get(parent) != repositories.get(key)
            {
                return Err(ManagedProjectError::new(
                    "portfolio recovery capsule contains an invalid local parent",
                ));
            }
        }
    }
    Ok(())
}

fn required_portfolio_id(
    object: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<PortfolioId, ManagedProjectError> {
    PortfolioId::new(required_nonempty_string(object, field, context)?)
        .map_err(|error| ManagedProjectError::new(format!("invalid {context} {field}: {error}")))
}

fn required_source_spec(
    object: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<SourceSpecIdentity, ManagedProjectError> {
    required_nonempty_string(object, field, context)?
        .parse()
        .map_err(|error| ManagedProjectError::new(format!("invalid {context} {field}: {error}")))
}

fn required_item_key(
    object: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<ItemKey, ManagedProjectError> {
    ItemKey::new(required_nonempty_string(object, field, context)?)
        .map_err(|error| ManagedProjectError::new(format!("invalid {context} {field}: {error}")))
}

fn required_nonempty_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, ManagedProjectError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ManagedProjectError::new(format!("{context} has invalid {field}")))
}

fn required_digest<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    length: usize,
    context: &str,
) -> Result<&'a str, ManagedProjectError> {
    let value = required_nonempty_string(object, field, context)?;
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ManagedProjectError::new(format!(
            "{context} {field} must be lowercase hexadecimal"
        )));
    }
    Ok(value)
}

fn required_string_array(
    object: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<Vec<Value>, ManagedProjectError> {
    let values = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| ManagedProjectError::new(format!("{context} {field} must be an array")))?;
    if values.iter().any(|value| value.as_str().is_none()) {
        return Err(ManagedProjectError::new(format!(
            "{context} {field} must contain only strings"
        )));
    }
    Ok(values.clone())
}

fn array_strings(values: &Value) -> impl Iterator<Item = &str> {
    values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn validate_repository(repository: &str) -> Result<(), ManagedProjectError> {
    let mut parts = repository.split('/');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            })
    };
    if !parts.next().is_some_and(valid_part)
        || !parts.next().is_some_and(valid_part)
        || parts.next().is_some()
    {
        return Err(ManagedProjectError::new(
            "portfolio repository must be canonical owner/name",
        ));
    }
    Ok(())
}
