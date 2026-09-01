use super::{
    append_synced_line, atomic_write, binding_document, empty_journal_digest,
    ensure_private_directory, ensure_private_file, extend_journal_digest, read_persisted_binding,
    reject_unsafe_file, snapshot_needs_update, validate_replay_checkpoint, JournalCheckpoint,
    ManagedProjectError, ProductLock, ProjectIdentity,
};
use autospec_core::autonomous::waterfall::sha256_hex;
use autospec_core::managed_project::{
    ManagedProjectBinding, ProductKey, RelationshipEdge, RepositoryRecord,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[path = "store/recovery.rs"]
mod recovery;

use recovery::{
    payload_string, recover_events, validate_operation_transition, validate_portfolio_item_binding,
    validate_portfolio_snapshot, JournalEvent,
};

const JOURNAL_SCHEMA: u64 = 1;
const BINDING_FILE: &str = "binding.json";
const EVENTS_FILE: &str = "events.jsonl";
const PORTFOLIO_FILE: &str = "portfolio.json";
pub(super) const LOCK_FILE: &str = "binding.lock";

pub struct ManagedProjectStore {
    pub(super) root: PathBuf,
    pub(super) product_key: ProductKey,
    pub(super) binding: ManagedProjectBinding,
    event_keys: HashSet<String>,
    known_projections: HashSet<String>,
    provisional_project: Option<ProjectIdentity>,
    portfolio_snapshot: Option<Value>,
    portfolio_item_bindings: Vec<Value>,
    portfolio_operations: Vec<Value>,
    next_sequence: u64,
    journal_digest: String,
    append_fault_after: Option<usize>,
}

impl ManagedProjectStore {
    pub fn open_global(
        root: &Path,
        legacy_root: Option<&Path>,
        product_key: &ProductKey,
    ) -> Result<Self, ManagedProjectError> {
        import_legacy_state(root, legacy_root, product_key)?;
        Self::open(root, product_key)
    }

    pub fn open_read_only(
        root: &Path,
        product_key: &ProductKey,
    ) -> Result<Self, ManagedProjectError> {
        validate_read_only_ancestors(root, product_key)?;
        let project_root = root.join("projects").join(product_key.as_str());
        let binding_path = project_root.join(BINDING_FILE);
        let events_path = project_root.join(EVENTS_FILE);
        let portfolio_path = project_root.join(PORTFOLIO_FILE);
        reject_unsafe_file(&portfolio_path)?;
        let persisted = if binding_path.exists() {
            Some(read_persisted_binding(&binding_path, product_key)?)
        } else {
            None
        };
        let empty_binding = ManagedProjectBinding::new(product_key.clone());
        if !events_path.exists() {
            if portfolio_path.exists() {
                return Err(ManagedProjectError::new(
                    "portfolio snapshot is missing its durable event journal",
                ));
            }
            if persisted
                .as_ref()
                .is_some_and(|persisted| persisted.binding != empty_binding)
            {
                return Err(ManagedProjectError::new(
                    "nonempty managed project binding is missing its durable event journal",
                ));
            }
            return Ok(Self {
                root: project_root,
                product_key: product_key.clone(),
                binding: empty_binding,
                event_keys: HashSet::new(),
                known_projections: HashSet::new(),
                provisional_project: None,
                portfolio_snapshot: None,
                portfolio_item_bindings: Vec::new(),
                portfolio_operations: Vec::new(),
                next_sequence: 1,
                journal_digest: empty_journal_digest(),
                append_fault_after: None,
            });
        }
        let recovered = recover_events(&events_path, product_key, false)?;
        let mut store = Self {
            root: project_root,
            product_key: product_key.clone(),
            binding: empty_binding,
            event_keys: HashSet::new(),
            known_projections: HashSet::new(),
            provisional_project: None,
            portfolio_snapshot: None,
            portfolio_item_bindings: Vec::new(),
            portfolio_operations: Vec::new(),
            next_sequence: 1,
            journal_digest: empty_journal_digest(),
            append_fault_after: None,
        };
        for event in &recovered.events {
            store.apply_event(event)?;
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
        store.validate_portfolio_document(false)?;
        Ok(store)
    }

    pub fn open(root: &Path, product_key: &ProductKey) -> Result<Self, ManagedProjectError> {
        ensure_private_directory(root)?;
        let projects = root.join("projects");
        ensure_private_directory(&projects)?;
        let root = projects.join(product_key.as_str());
        ensure_private_directory(&root)?;
        let _lock = ProductLock::acquire(&root.join(LOCK_FILE))?;

        let binding_path = root.join(BINDING_FILE);
        let events_path = root.join(EVENTS_FILE);
        let portfolio_path = root.join(PORTFOLIO_FILE);
        reject_unsafe_file(&binding_path)?;
        reject_unsafe_file(&events_path)?;
        reject_unsafe_file(&portfolio_path)?;

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
        let recovered = recover_events(&events_path, product_key, true)?;
        let mut store = Self {
            root,
            product_key: product_key.clone(),
            binding: empty_binding,
            event_keys: HashSet::new(),
            known_projections: HashSet::new(),
            provisional_project: None,
            portfolio_snapshot: None,
            portfolio_item_bindings: Vec::new(),
            portfolio_operations: Vec::new(),
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
        store.validate_or_persist_portfolio()?;
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
        if self
            .binding
            .repositories
            .iter()
            .any(|record| normalize_identity(&record.repository) == identity)
        {
            return Ok(());
        }
        let key = format!("repository:record:{}:{identity}", self.product_key.as_str());
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
            format!(
                "projection:ack:{}:{}",
                sha256_hex(projection_key.as_bytes()),
                self.next_sequence
            ),
            "projection-acknowledged",
            Value::String(projection_key.to_owned()),
        )
    }

    pub fn ensure_projection_pending(
        &mut self,
        projection_key: &str,
    ) -> Result<(), ManagedProjectError> {
        let _lock = ProductLock::acquire(&self.root.join(LOCK_FILE))?;
        self.refresh_from_journal()?;
        if self
            .binding
            .pending_projections
            .iter()
            .any(|pending| pending == projection_key)
        {
            return Ok(());
        }
        if !self.known_projections.contains(projection_key) {
            return self.append_event_locked(
                projection_key.to_owned(),
                "projection-enqueued",
                Value::String(projection_key.to_owned()),
            );
        }
        self.append_event_locked(
            format!(
                "projection:restore:{}:{}",
                sha256_hex(projection_key.as_bytes()),
                self.next_sequence
            ),
            "projection-restored",
            Value::String(projection_key.to_owned()),
        )
    }

    pub(crate) fn record_portfolio_snapshot(
        &mut self,
        snapshot: Value,
    ) -> Result<(), ManagedProjectError> {
        let _lock = ProductLock::acquire(&self.root.join(LOCK_FILE))?;
        self.refresh_from_journal()?;
        validate_portfolio_snapshot(&snapshot)?;
        if self
            .portfolio_snapshot
            .as_ref()
            .is_some_and(|existing| !portfolio_snapshots_match(existing, &snapshot))
        {
            return Err(ManagedProjectError::new(
                "portfolio snapshot conflicts with the frozen recovery capsule",
            ));
        }
        let portfolio_id = snapshot["portfolio_id"]
            .as_str()
            .expect("validated snapshot has portfolio id");
        let plan_digest = snapshot["plan_digest"]
            .as_str()
            .expect("validated snapshot has plan digest");
        self.append_event_locked(
            format!("portfolio:snapshot:{portfolio_id}:{plan_digest}"),
            "portfolio-snapshot-recorded",
            snapshot,
        )
    }

    pub(crate) fn record_portfolio_item_binding(
        &mut self,
        binding: Value,
    ) -> Result<(), ManagedProjectError> {
        let _lock = ProductLock::acquire(&self.root.join(LOCK_FILE))?;
        self.refresh_from_journal()?;
        let snapshot = self.portfolio_snapshot.as_ref().ok_or_else(|| {
            ManagedProjectError::new("portfolio item binding requires a recovery capsule")
        })?;
        validate_portfolio_item_binding(snapshot, &self.portfolio_item_bindings, &binding)?;
        let item_key = binding["item_key"]
            .as_str()
            .expect("validated binding has item key");
        self.append_event_locked(
            format!("portfolio:item-binding:{item_key}"),
            "portfolio-item-bound",
            binding,
        )
    }

    pub(crate) fn transition_portfolio_operation(
        &mut self,
        operation_id: &str,
        state: &str,
        payload: Value,
    ) -> Result<(), ManagedProjectError> {
        let _lock = ProductLock::acquire(&self.root.join(LOCK_FILE))?;
        self.refresh_from_journal()?;
        if self.portfolio_snapshot.is_none() {
            return Err(ManagedProjectError::new(
                "portfolio operation requires a recovery capsule",
            ));
        }
        validate_operation_transition(&self.portfolio_operations, operation_id, state, &payload)?;
        self.append_event_locked(
            format!("portfolio:operation:{operation_id}:{state}"),
            "portfolio-operation-transitioned",
            serde_json::json!({
                "operation_id": operation_id,
                "state": state,
                "payload": payload,
            }),
        )
    }

    pub(crate) fn portfolio_snapshot(&self) -> Option<&Value> {
        self.portfolio_snapshot.as_ref()
    }

    pub(crate) fn portfolio_item_bindings(&self) -> &[Value] {
        &self.portfolio_item_bindings
    }

    pub(crate) fn portfolio_operation_states(&self) -> Vec<(String, String)> {
        self.portfolio_operations
            .iter()
            .filter_map(|operation| {
                Some((
                    operation.get("operation_id")?.as_str()?.to_owned(),
                    operation.get("state")?.as_str()?.to_owned(),
                ))
            })
            .collect()
    }

    pub fn snapshot(&self) -> &ManagedProjectBinding {
        &self.binding
    }

    pub(super) fn provisional_project(&self) -> Option<&ProjectIdentity> {
        self.provisional_project.as_ref()
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
            return self.persist_state();
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
        self.persist_state()
    }

    pub(super) fn refresh_from_journal(&mut self) -> Result<(), ManagedProjectError> {
        let persisted = read_persisted_binding(&self.root.join(BINDING_FILE), &self.product_key)?;
        let recovered = recover_events(&self.root.join(EVENTS_FILE), &self.product_key, true)?;
        self.binding = ManagedProjectBinding::new(self.product_key.clone());
        self.event_keys.clear();
        self.known_projections.clear();
        self.provisional_project = None;
        self.portfolio_snapshot = None;
        self.portfolio_item_bindings.clear();
        self.portfolio_operations.clear();
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
            "project-created" => self.apply_project_created(event)?,
            "project-bound" => self.apply_project_bound(event)?,
            "repository-recorded" => self.apply_repository_recorded(event)?,
            "relationship-recorded" => self.apply_relationship_recorded(event)?,
            "projection-enqueued" => self.apply_projection_enqueued(event)?,
            "projection-restored" => self.apply_projection_restored(event)?,
            "projection-acknowledged" => self.apply_projection_acknowledged(event)?,
            "portfolio-snapshot-recorded" => self.apply_portfolio_snapshot(event)?,
            "portfolio-item-bound" => self.apply_portfolio_item_binding(event)?,
            "portfolio-operation-transitioned" => self.apply_portfolio_operation(event)?,
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

    fn apply_project_created(&mut self, event: &JournalEvent) -> Result<(), ManagedProjectError> {
        let projection = format!("project:create:{}", self.product_key.as_str());
        if self.binding.project_node_id.is_some() {
            return Err(ManagedProjectError::new(
                "provisional project identity follows a final binding",
            ));
        }
        if !self.known_projections.contains(&projection)
            || !self
                .binding
                .pending_projections
                .iter()
                .any(|pending| pending == &projection)
        {
            return Err(ManagedProjectError::new(
                "provisional project identity has no pending create projection",
            ));
        }
        let identity = super::parse_project_identity(&event.payload)?;
        if self
            .provisional_project
            .as_ref()
            .is_some_and(|existing| !existing.same_immutable_identity(&identity))
        {
            return Err(ManagedProjectError::new(
                "journal contains conflicting provisional project identities",
            ));
        }
        self.provisional_project = Some(identity);
        Ok(())
    }

    fn apply_project_bound(&mut self, event: &JournalEvent) -> Result<(), ManagedProjectError> {
        let identity = super::parse_project_identity(&event.payload)?;
        if let Some(existing) = super::project_binding_payload(&self.binding) {
            let existing = super::parse_project_identity(&existing)?;
            if !existing.same_immutable_identity(&identity) {
                return Err(ManagedProjectError::new(
                    "journal contains conflicting final project bindings",
                ));
            }
        }
        if self
            .provisional_project
            .as_ref()
            .is_some_and(|project| !project.same_immutable_identity(&identity))
        {
            return Err(ManagedProjectError::new(
                "final project binding conflicts with provisional identity",
            ));
        }
        super::apply_project_binding(&mut self.binding, &event.payload)?;
        self.provisional_project = None;
        Ok(())
    }

    fn apply_repository_recorded(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
        let repository = serde_json::from_value(event.payload.clone()).map_err(|error| {
            ManagedProjectError::new(format!("invalid repository journal payload: {error}"))
        })?;
        self.binding.repositories.push(repository);
        Ok(())
    }

    fn apply_relationship_recorded(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
        let edge: RelationshipEdge =
            serde_json::from_value(event.payload.clone()).map_err(|error| {
                ManagedProjectError::new(format!("invalid relationship journal payload: {error}"))
            })?;
        if edge.product_key != self.product_key {
            return Err(ManagedProjectError::new(
                "journal relationship product key does not match its state directory",
            ));
        }
        self.binding.relationships.push(edge);
        Ok(())
    }

    fn apply_projection_enqueued(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
        let projection = payload_string(event, "projection enqueue")?;
        if projection != event.key {
            return Err(ManagedProjectError::new(
                "projection enqueue key does not match its payload",
            ));
        }
        self.known_projections.insert(projection.to_owned());
        self.binding.pending_projections.push(projection.to_owned());
        Ok(())
    }

    fn apply_projection_restored(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
        let projection = payload_string(event, "projection restore")?;
        if !self.known_projections.contains(projection) {
            return Err(ManagedProjectError::new(
                "projection restore precedes its durable enqueue event",
            ));
        }
        if !self
            .binding
            .pending_projections
            .iter()
            .any(|pending| pending == projection)
        {
            self.binding.pending_projections.push(projection.to_owned());
        }
        Ok(())
    }

    fn apply_projection_acknowledged(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
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
        Ok(())
    }

    fn apply_portfolio_snapshot(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
        validate_portfolio_snapshot(&event.payload)?;
        if self
            .portfolio_snapshot
            .as_ref()
            .is_some_and(|existing| existing != &event.payload)
        {
            return Err(ManagedProjectError::new(
                "journal contains conflicting portfolio recovery capsules",
            ));
        }
        self.portfolio_snapshot = Some(event.payload.clone());
        Ok(())
    }

    fn apply_portfolio_item_binding(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
        let snapshot = self.portfolio_snapshot.as_ref().ok_or_else(|| {
            ManagedProjectError::new("portfolio item binding precedes its recovery capsule")
        })?;
        validate_portfolio_item_binding(snapshot, &self.portfolio_item_bindings, &event.payload)?;
        self.portfolio_item_bindings.push(event.payload.clone());
        Ok(())
    }

    fn apply_portfolio_operation(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ManagedProjectError> {
        if self.portfolio_snapshot.is_none() {
            return Err(ManagedProjectError::new(
                "portfolio operation precedes its recovery capsule",
            ));
        }
        let operation_id = event.payload["operation_id"]
            .as_str()
            .ok_or_else(|| ManagedProjectError::new("portfolio operation has no operation id"))?;
        let state = event.payload["state"]
            .as_str()
            .ok_or_else(|| ManagedProjectError::new("portfolio operation has no state"))?;
        let payload = event
            .payload
            .get("payload")
            .ok_or_else(|| ManagedProjectError::new("portfolio operation has no payload"))?;
        validate_operation_transition(&self.portfolio_operations, operation_id, state, payload)?;
        let mut operation = event.payload.clone();
        operation["sequence"] = Value::from(event.sequence);
        self.portfolio_operations.push(operation);
        if state == "acknowledged" {
            self.portfolio_snapshot
                .as_mut()
                .expect("portfolio snapshot checked above")["projection_high_watermark"] =
                Value::from(event.sequence);
        }
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

    fn persist_state(&self) -> Result<(), ManagedProjectError> {
        self.persist_binding()?;
        self.persist_portfolio()
    }

    fn portfolio_document(&self) -> Value {
        serde_json::json!({
            "schema": "autospec.portfolio-store.v1",
            "checkpoint": {
                "high_watermark": self.next_sequence - 1,
                "digest": self.journal_digest,
            },
            "snapshot": self.portfolio_snapshot,
            "item_bindings": self.portfolio_item_bindings,
            "operations": self.portfolio_operations,
        })
    }

    fn persist_portfolio(&self) -> Result<(), ManagedProjectError> {
        if self.portfolio_snapshot.is_none() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(&self.portfolio_document())?;
        atomic_write(&self.root.join(PORTFOLIO_FILE), &bytes)
    }

    fn validate_or_persist_portfolio(&self) -> Result<(), ManagedProjectError> {
        self.validate_portfolio_document(true)
    }

    fn validate_portfolio_document(&self, repair_stale: bool) -> Result<(), ManagedProjectError> {
        let path = self.root.join(PORTFOLIO_FILE);
        if !path.exists() {
            if self.portfolio_snapshot.is_some() {
                if repair_stale {
                    return self.persist_portfolio();
                }
                return Err(ManagedProjectError::new(
                    "portfolio recovery state is missing its atomic snapshot",
                ));
            }
            return Ok(());
        }
        let bytes = super::read_private_file(&path)?;
        let persisted: Value = serde_json::from_str(&bytes).map_err(|error| {
            ManagedProjectError::new(format!("invalid portfolio snapshot document: {error}"))
        })?;
        if persisted["schema"].as_str() != Some("autospec.portfolio-store.v1") {
            return Err(ManagedProjectError::new(
                "unsupported portfolio store schema",
            ));
        }
        let high_watermark = persisted["checkpoint"]["high_watermark"]
            .as_u64()
            .ok_or_else(|| {
                ManagedProjectError::new("portfolio checkpoint has no high-water mark")
            })?;
        let digest = persisted["checkpoint"]["digest"]
            .as_str()
            .ok_or_else(|| ManagedProjectError::new("portfolio checkpoint has no digest"))?;
        if high_watermark > self.next_sequence - 1
            || digest != self.journal_digest_at(high_watermark)?
        {
            return Err(ManagedProjectError::new(
                "portfolio snapshot checkpoint does not match the durable journal",
            ));
        }
        let expected = self.portfolio_document();
        if persisted != expected {
            if repair_stale {
                return self.persist_portfolio();
            }
            return Err(ManagedProjectError::new(
                "portfolio snapshot is stale relative to the durable journal",
            ));
        }
        Ok(())
    }

    fn journal_digest_at(&self, high_watermark: u64) -> Result<String, ManagedProjectError> {
        let recovered = recover_events(&self.root.join(EVENTS_FILE), &self.product_key, false)?;
        let index = usize::try_from(high_watermark)
            .map_err(|_| ManagedProjectError::new("portfolio checkpoint overflow"))?;
        recovered
            .prefix_digests
            .get(index)
            .cloned()
            .ok_or_else(|| ManagedProjectError::new("portfolio checkpoint exceeds journal"))
    }
}

fn portfolio_snapshots_match(left: &Value, right: &Value) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left["projection_high_watermark"] = Value::from(0);
    right["projection_high_watermark"] = Value::from(0);
    left == right
}

fn import_legacy_state(
    root: &Path,
    legacy_root: Option<&Path>,
    product_key: &ProductKey,
) -> Result<(), ManagedProjectError> {
    let Some(legacy_root) = legacy_root.filter(|legacy| *legacy != root) else {
        return Ok(());
    };
    ensure_private_directory(root)?;
    let projects = root.join("projects");
    ensure_private_directory(&projects)?;
    let project_root = projects.join(product_key.as_str());
    ensure_private_directory(&project_root)?;
    let _lock = ProductLock::acquire(&project_root.join(LOCK_FILE))?;
    let binding_path = project_root.join(BINDING_FILE);
    let events_path = project_root.join(EVENTS_FILE);
    if binding_path.exists() || events_path.exists() {
        return Ok(());
    }
    let legacy_project = legacy_root.join("projects").join(product_key.as_str());
    if !legacy_project.exists() {
        return Ok(());
    }
    validate_read_only_ancestors(legacy_root, product_key)?;
    let _legacy_lock = ProductLock::acquire(&legacy_project.join(LOCK_FILE))?;
    let legacy = ManagedProjectStore::open_read_only(legacy_root, product_key)?;
    let legacy_binding = legacy_project.join(BINDING_FILE);
    let legacy_events = legacy_project.join(EVENTS_FILE);
    let binding = super::read_private_file(&legacy_binding)?;
    let events = super::read_private_file(&legacy_events)?;
    super::atomic_write(&events_path, events.as_bytes())?;
    super::atomic_write(&binding_path, binding.as_bytes())?;
    drop(legacy);
    Ok(())
}

fn validate_read_only_ancestors(
    root: &Path,
    product_key: &ProductKey,
) -> Result<(), ManagedProjectError> {
    for directory in [
        root.to_path_buf(),
        root.join("projects"),
        root.join("projects").join(product_key.as_str()),
    ] {
        let metadata = match std::fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(super::io_error(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ManagedProjectError::new(
                "managed project read-only ancestor is not a safe directory",
            ));
        }
        super::validate_owner(&metadata)?;
        #[cfg(unix)]
        if std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o077 != 0 {
            return Err(ManagedProjectError::new(
                "managed project read-only ancestor permissions must be private",
            ));
        }
    }
    Ok(())
}

impl From<serde_json::Error> for ManagedProjectError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}

fn normalize_identity(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
}
