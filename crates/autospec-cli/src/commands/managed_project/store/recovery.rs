use super::{ManagedProjectError, JOURNAL_SCHEMA};
use crate::commands::managed_project::{
    empty_journal_digest, extend_journal_digest, io_error, open_private_file,
    open_private_file_read_only,
};
use autospec_core::managed_project::ProductKey;
use serde_json::{json, Value};
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
