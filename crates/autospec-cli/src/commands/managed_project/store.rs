use super::{
    append_synced_line, atomic_write, binding_document, empty_journal_digest,
    ensure_private_directory, ensure_private_file, extend_journal_digest, io_error,
    open_private_file, read_persisted_binding, reject_unsafe_file, snapshot_needs_update,
    validate_replay_checkpoint, JournalCheckpoint, ManagedProjectError, ProductLock,
};
use autospec_core::autonomous::waterfall::sha256_hex;
use autospec_core::managed_project::{
    ManagedProjectBinding, ProductKey, RelationshipEdge, RepositoryRecord,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const JOURNAL_SCHEMA: u64 = 1;
const BINDING_FILE: &str = "binding.json";
const EVENTS_FILE: &str = "events.jsonl";
pub(super) const LOCK_FILE: &str = "binding.lock";

pub struct ManagedProjectStore {
    pub(super) root: PathBuf,
    pub(super) product_key: ProductKey,
    pub(super) binding: ManagedProjectBinding,
    event_keys: HashSet<String>,
    known_projections: HashSet<String>,
    next_sequence: u64,
    journal_digest: String,
    append_fault_after: Option<usize>,
}

impl ManagedProjectStore {
    pub fn open(root: &Path, product_key: &ProductKey) -> Result<Self, ManagedProjectError> {
        ensure_private_directory(root)?;
        let projects = root.join("projects");
        ensure_private_directory(&projects)?;
        let root = projects.join(product_key.as_str());
        ensure_private_directory(&root)?;
        let _lock = ProductLock::acquire(&root.join(LOCK_FILE))?;

        let binding_path = root.join(BINDING_FILE);
        let events_path = root.join(EVENTS_FILE);
        reject_unsafe_file(&binding_path)?;
        reject_unsafe_file(&events_path)?;

        let persisted = if binding_path.exists() {
            Some(read_persisted_binding(&binding_path, product_key)?)
        } else {
            None
        };

        let journal_exists = events_path.exists();
        let empty_binding = ManagedProjectBinding::new(product_key.clone());
        if !journal_exists
            && persisted
                .as_ref()
                .is_some_and(|persisted| persisted.binding != empty_binding)
        {
            return Err(ManagedProjectError::new(
                "nonempty managed project binding is missing its durable event journal",
            ));
        }
        ensure_private_file(&events_path)?;
        let recovered = recover_events(&events_path, product_key)?;
        let mut store = Self {
            root,
            product_key: product_key.clone(),
            binding: empty_binding,
            event_keys: HashSet::new(),
            known_projections: HashSet::new(),
            next_sequence: 1,
            journal_digest: empty_journal_digest(),
            append_fault_after: None,
        };
        for event in recovered.events {
            store.apply_event(&event)?;
            store.next_sequence = event
                .sequence
                .checked_add(1)
                .ok_or_else(|| ManagedProjectError::new("managed project sequence overflow"))?;
        }
        store.journal_digest = recovered.final_digest;
        validate_replay_checkpoint(
            persisted.as_ref(),
            &store.binding,
            &recovered.prefix_digests,
        )?;
        if snapshot_needs_update(
            persisted.as_ref(),
            &store.binding,
            store.next_sequence - 1,
            &store.journal_digest,
        ) {
            store.persist_binding()?;
        }
        Ok(store)
    }

    pub fn record_repository(
        &mut self,
        repository: RepositoryRecord,
    ) -> Result<(), ManagedProjectError> {
        let identity = normalize_identity(&repository.repository);
        if identity.is_empty() || repository.entry_kind.trim().is_empty() {
            return Err(ManagedProjectError::new(
                "repository identity and entry kind must not be empty",
            ));
        }
        let key = format!(
            "repository:register:{}:{identity}",
            self.product_key.as_str()
        );
        self.append_event(
            key,
            "repository-recorded",
            serde_json::to_value(repository)?,
        )
    }

    pub fn record_edge(&mut self, edge: RelationshipEdge) -> Result<(), ManagedProjectError> {
        if edge.product_key != self.product_key {
            return Err(ManagedProjectError::new(
                "relationship product key does not match the managed project store",
            ));
        }
        let evidence_digest = sha256_hex(edge.dedupe_key().as_bytes());
        let key = format!(
            "relationship:{}:{}:{}:{}:{evidence_digest}",
            self.product_key.as_str(),
            edge.kind.as_str(),
            normalize_identity(&edge.source),
            normalize_identity(&edge.target),
        );
        self.append_event(key, "relationship-recorded", serde_json::to_value(edge)?)
    }

    pub fn enqueue_projection(
        &mut self,
        projection_key: impl Into<String>,
    ) -> Result<(), ManagedProjectError> {
        let projection_key = projection_key.into();
        if projection_key.trim().is_empty() {
            return Err(ManagedProjectError::new(
                "projection idempotency key must not be empty",
            ));
        }
        self.append_event(
            projection_key.clone(),
            "projection-enqueued",
            Value::String(projection_key),
        )
    }

    pub fn ack_projection(&mut self, projection_key: &str) -> Result<(), ManagedProjectError> {
        let _lock = ProductLock::acquire(&self.root.join(LOCK_FILE))?;
        self.refresh_from_journal()?;
        if !self.known_projections.contains(projection_key) {
            return Err(ManagedProjectError::new(
                "projection acknowledgment has no matching durable enqueue event",
            ));
        }
        if !self
            .binding
            .pending_projections
            .iter()
            .any(|pending| pending == projection_key)
        {
            return Ok(());
        }
        self.append_event_locked(
            format!("projection:ack:{projection_key}"),
            "projection-acknowledged",
            Value::String(projection_key.to_owned()),
        )
    }

    pub fn snapshot(&self) -> &ManagedProjectBinding {
        &self.binding
    }

    #[cfg(test)]
    pub fn fail_next_append_after(&mut self, bytes: usize) {
        self.append_fault_after = Some(bytes);
    }

    fn append_event(
        &mut self,
        key: String,
        kind: &'static str,
        payload: Value,
    ) -> Result<(), ManagedProjectError> {
        let _lock = ProductLock::acquire(&self.root.join(LOCK_FILE))?;
        self.refresh_from_journal()?;
        self.append_event_locked(key, kind, payload)
    }

    pub(super) fn append_event_locked(
        &mut self,
        key: String,
        kind: &'static str,
        payload: Value,
    ) -> Result<(), ManagedProjectError> {
        if self.event_keys.contains(&key) {
            return self.persist_binding();
        }
        let event = JournalEvent {
            sequence: self.next_sequence,
            product_key: self.product_key.clone(),
            key,
            kind: kind.to_owned(),
            payload,
        };
        let mut line = serde_json::to_vec(&event.to_value()).map_err(ManagedProjectError::from)?;
        line.push(b'\n');
        append_synced_line(
            &self.root.join(EVENTS_FILE),
            &line,
            self.append_fault_after.take(),
        )?;
        self.journal_digest = extend_journal_digest(&self.journal_digest, &line);
        self.apply_event(&event)?;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| ManagedProjectError::new("managed project sequence overflow"))?;
        self.persist_binding()
    }

    pub(super) fn refresh_from_journal(&mut self) -> Result<(), ManagedProjectError> {
        let persisted = read_persisted_binding(&self.root.join(BINDING_FILE), &self.product_key)?;
        let recovered = recover_events(&self.root.join(EVENTS_FILE), &self.product_key)?;
        self.binding = ManagedProjectBinding::new(self.product_key.clone());
        self.event_keys.clear();
        self.known_projections.clear();
        self.next_sequence = 1;
        for event in recovered.events {
            self.apply_event(&event)?;
            self.next_sequence = event
                .sequence
                .checked_add(1)
                .ok_or_else(|| ManagedProjectError::new("managed project sequence overflow"))?;
        }
        self.journal_digest = recovered.final_digest;
        validate_replay_checkpoint(Some(&persisted), &self.binding, &recovered.prefix_digests)?;
        Ok(())
    }

    fn apply_event(&mut self, event: &JournalEvent) -> Result<(), ManagedProjectError> {
        if event.product_key != self.product_key {
            return Err(ManagedProjectError::new(
                "journal event product key does not match its state directory",
            ));
        }
        if self.event_keys.contains(&event.key) {
            return Ok(());
        }
        match event.kind.as_str() {
            "project-bound" => {
                super::apply_project_binding(&mut self.binding, &event.payload)?;
            }
            "repository-recorded" => {
                let repository: RepositoryRecord = serde_json::from_value(event.payload.clone())
                    .map_err(|error| {
                        ManagedProjectError::new(format!(
                            "invalid repository journal payload: {error}"
                        ))
                    })?;
                self.binding.repositories.push(repository);
            }
            "relationship-recorded" => {
                let edge: RelationshipEdge = serde_json::from_value(event.payload.clone())
                    .map_err(|error| {
                        ManagedProjectError::new(format!(
                            "invalid relationship journal payload: {error}"
                        ))
                    })?;
                if edge.product_key != self.product_key {
                    return Err(ManagedProjectError::new(
                        "journal relationship product key does not match its state directory",
                    ));
                }
                self.binding.relationships.push(edge);
            }
            "projection-enqueued" => {
                let projection = payload_string(event, "projection enqueue")?;
                if projection != event.key {
                    return Err(ManagedProjectError::new(
                        "projection enqueue key does not match its payload",
                    ));
                }
                self.known_projections.insert(projection.to_owned());
                self.binding.pending_projections.push(projection.to_owned());
            }
            "projection-acknowledged" => {
                let projection = payload_string(event, "projection acknowledgment")?;
                if !self.known_projections.contains(projection) {
                    return Err(ManagedProjectError::new(
                        "projection acknowledgment precedes its durable enqueue event",
                    ));
                }
                let index = self
                    .binding
                    .pending_projections
                    .iter()
                    .position(|pending| pending == projection)
                    .ok_or_else(|| {
                        ManagedProjectError::new(
                            "projection acknowledgment does not match a pending projection",
                        )
                    })?;
                self.binding.pending_projections.remove(index);
            }
            _ => {
                return Err(ManagedProjectError::new(format!(
                    "unknown managed project journal event kind {}",
                    event.kind
                )))
            }
        }
        self.event_keys.insert(event.key.clone());
        Ok(())
    }

    fn persist_binding(&self) -> Result<(), ManagedProjectError> {
        let document = binding_document(
            &self.binding,
            &JournalCheckpoint {
                high_watermark: self.next_sequence - 1,
                digest: self.journal_digest.clone(),
            },
        )?;
        atomic_write(&self.root.join(BINDING_FILE), &document)
    }
}

impl From<serde_json::Error> for ManagedProjectError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}

struct JournalEvent {
    sequence: u64,
    product_key: ProductKey,
    key: String,
    kind: String,
    payload: Value,
}

impl JournalEvent {
    fn to_value(&self) -> Value {
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

fn payload_string<'a>(
    event: &'a JournalEvent,
    description: &str,
) -> Result<&'a str, ManagedProjectError> {
    event
        .payload
        .as_str()
        .ok_or_else(|| ManagedProjectError::new(format!("{description} payload must be a string")))
}

fn recover_events(
    path: &Path,
    product_key: &ProductKey,
) -> Result<RecoveredJournal, ManagedProjectError> {
    let mut file = open_private_file(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        file.set_len(complete as u64).map_err(io_error)?;
        file.seek(SeekFrom::Start(complete as u64))
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
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

struct RecoveredJournal {
    events: Vec<JournalEvent>,
    prefix_digests: Vec<String>,
    final_digest: String,
}

fn normalize_identity(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
}
