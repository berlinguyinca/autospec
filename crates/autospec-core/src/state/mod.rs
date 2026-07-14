use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::spec::is_valid_spec_id;
use json::{JsonParser, JsonValue};
use storage::{FileState, StatePaths};

pub(crate) mod json;
mod storage;

const STATE_SCHEMA_VERSION: u64 = 1;
const STATE_FILE: &str = "specs.json";
const TEMP_STATE_FILE: &str = "specs.json.tmp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecRunState {
    Planned,
    Ready,
    Running,
    Passed,
    Failed,
    Blocked,
    Deferred,
    Superseded,
}

impl SpecRunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecRunState::Planned => "planned",
            SpecRunState::Ready => "ready",
            SpecRunState::Running => "running",
            SpecRunState::Passed => "passed",
            SpecRunState::Failed => "failed",
            SpecRunState::Blocked => "blocked",
            SpecRunState::Deferred => "deferred",
            SpecRunState::Superseded => "superseded",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "planned" => Ok(Self::Planned),
            "ready" => Ok(Self::Ready),
            "running" => Ok(Self::Running),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            "deferred" => Ok(Self::Deferred),
            "superseded" => Ok(Self::Superseded),
            _ => Err(format!("unknown spec run state: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecLifecycle {
    pub spec_id: String,
    pub state: SpecRunState,
    pub deferred_reason: Option<String>,
    pub superseded_by: Option<String>,
}

impl SpecLifecycle {
    pub fn new(spec_id: impl Into<String>) -> Self {
        Self {
            spec_id: spec_id.into(),
            state: SpecRunState::Planned,
            deferred_reason: None,
            superseded_by: None,
        }
    }

    pub fn transition_to(&mut self, next: SpecRunState) -> Result<(), String> {
        if is_allowed_transition(&self.state, &next) {
            self.state = next;
            Ok(())
        } else {
            Err(format!(
                "invalid transition from {} to {}",
                self.state.as_str(),
                next.as_str()
            ))
        }
    }

    pub fn deferred(mut self, reason: impl Into<String>) -> Result<Self, String> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err("deferred reason is required".to_string());
        }
        self.transition_to(SpecRunState::Deferred)?;
        self.deferred_reason = Some(reason);
        Ok(self)
    }

    pub fn superseded_by(mut self, replacement: impl Into<String>) -> Result<Self, String> {
        let replacement = replacement.into();
        if !is_valid_spec_id(&replacement) {
            return Err(format!("invalid replacement spec id: {replacement}"));
        }
        self.transition_to(SpecRunState::Superseded)?;
        self.superseded_by = Some(replacement);
        Ok(self)
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"spec_id\":\"{}\",\"state\":\"{}\",\"deferred_reason\":{},\"superseded_by\":{}}}",
            escape_json_string(&self.spec_id),
            self.state.as_str(),
            optional_json_string(&self.deferred_reason),
            optional_json_string(&self.superseded_by)
        )
    }
}

/// A validated, deterministic state document for package lifecycle progress.
///
/// The store is intentionally local and non-executing. It owns only
/// `<project-root>/.autospec/state/specs.json`, which later queue and report
/// layers can consume without reinterpreting lifecycle metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecStateStore {
    records: BTreeMap<String, SpecLifecycle>,
}

impl SpecStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, lifecycle: SpecLifecycle) -> Result<(), String> {
        let mut candidate = self.records.clone();
        if candidate.contains_key(&lifecycle.spec_id) {
            return Err(format!("duplicate spec id: {}", lifecycle.spec_id));
        }
        candidate.insert(lifecycle.spec_id.clone(), lifecycle);
        validate_records(&candidate)?;
        self.records = candidate;
        Ok(())
    }

    pub fn get(&self, spec_id: &str) -> Option<&SpecLifecycle> {
        self.records.get(spec_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SpecLifecycle> {
        self.records.values()
    }

    pub fn to_json(&self) -> Result<String, String> {
        validate_records(&self.records)?;
        let records = self
            .records
            .values()
            .map(SpecLifecycle::to_json)
            .collect::<Vec<_>>()
            .join(",");
        Ok(format!(
            "{{\"schema\":{STATE_SCHEMA_VERSION},\"specs\":[{records}]}}"
        ))
    }

    pub fn load_or_default(root: impl AsRef<Path>) -> Result<Self, String> {
        let paths = StatePaths::new(root.as_ref());
        let primary = storage::load_state_file(&paths.primary);

        match primary {
            FileState::Valid(store) => Ok(store),
            primary_state @ (FileState::Missing | FileState::Invalid(_)) => {
                match storage::load_state_file(&paths.temporary) {
                    FileState::Valid(store) => {
                        storage::promote_temporary(&paths)?;
                        Ok(store)
                    }
                    temporary_state @ (FileState::Missing | FileState::Invalid(_)) => {
                        match (primary_state, temporary_state) {
                            (FileState::Missing, FileState::Missing) => Ok(Self::new()),
                            (FileState::Invalid(error), FileState::Missing) => Err(format!(
                                "invalid spec state file {}: {error}",
                                paths.primary.display()
                            )),
                            (FileState::Missing, FileState::Invalid(error)) => Err(format!(
                                "invalid temporary spec state file {}: {error}",
                                paths.temporary.display()
                            )),
                            (FileState::Invalid(primary_error), FileState::Invalid(temporary_error)) => {
                                Err(format!(
                                    "invalid spec state files: {}: {primary_error}; {}: {temporary_error}",
                                    paths.primary.display(),
                                    paths.temporary.display()
                                ))
                            }
                            (FileState::Valid(_), _) | (_, FileState::Valid(_)) => {
                                unreachable!("valid file states return before this branch")
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn initialize_if_absent(
        root: impl AsRef<Path>,
        records: impl IntoIterator<Item = SpecLifecycle>,
    ) -> Result<Self, String> {
        let mut store = Self::new();
        for lifecycle in records {
            store.insert(lifecycle)?;
        }
        let rendered = store.to_json()?;
        let paths = StatePaths::new(root.as_ref());
        if paths.primary.exists() || paths.temporary.exists() {
            return Err("autospec init refuses to overwrite existing spec state".to_string());
        }

        let autospec_was_missing = !paths.autospec_directory.exists();
        let state_was_missing = !paths.directory.exists();
        fs::create_dir_all(&paths.directory).map_err(|error| {
            format!(
                "failed to create spec state directory {}: {error}",
                paths.directory.display()
            )
        })?;
        storage::sync_created_directories(&paths, autospec_was_missing, state_was_missing)?;

        let mut temporary = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&paths.temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err("autospec init refuses to overwrite existing spec state".to_string())
            }
            Err(error) => {
                return Err(format!(
                    "failed to create initialization state file {}: {error}",
                    paths.temporary.display()
                ))
            }
        };
        temporary.write_all(rendered.as_bytes()).map_err(|error| {
            format!(
                "failed to write initialization state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        temporary.sync_all().map_err(|error| {
            format!(
                "failed to synchronize initialization state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        drop(temporary);

        match fs::hard_link(&paths.temporary, &paths.primary) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&paths.temporary);
                return Err("autospec init refuses to overwrite existing spec state".to_string());
            }
            Err(error) => {
                return Err(format!(
                    "failed to atomically initialize spec state {}: {error}",
                    paths.primary.display()
                ))
            }
        }
        storage::sync_directory(&paths.directory)?;
        fs::remove_file(&paths.temporary).map_err(|error| {
            format!(
                "failed to finalize initialization state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        storage::sync_directory(&paths.directory)?;
        Ok(store)
    }

    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), String> {
        let rendered = self.to_json()?;
        let paths = StatePaths::new(root.as_ref());
        let autospec_was_missing = !paths.autospec_directory.exists();
        let state_was_missing = !paths.directory.exists();
        fs::create_dir_all(&paths.directory).map_err(|error| {
            format!(
                "failed to create spec state directory {}: {error}",
                paths.directory.display()
            )
        })?;
        storage::sync_created_directories(&paths, autospec_was_missing, state_was_missing)?;

        let mut temporary = File::create(&paths.temporary).map_err(|error| {
            format!(
                "failed to create temporary spec state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        temporary.write_all(rendered.as_bytes()).map_err(|error| {
            format!(
                "failed to write temporary spec state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        temporary.sync_all().map_err(|error| {
            format!(
                "failed to synchronize temporary spec state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        storage::sync_directory(&paths.directory)?;
        drop(temporary);

        storage::promote_temporary(&paths)
    }
}

fn parse_store(document: &str) -> Result<SpecStateStore, String> {
    let value = JsonParser::new(document).parse()?;
    let mut root = value.into_object("spec state document")?;
    require_only_keys(&root, &["schema", "specs"], "spec state document")?;

    let schema =
        take_required(&mut root, "schema", "spec state document")?.into_number("schema")?;
    if schema != STATE_SCHEMA_VERSION {
        return Err(format!("unsupported spec state schema: {schema}"));
    }

    let records = take_required(&mut root, "specs", "spec state document")?.into_array("specs")?;
    let mut store = SpecStateStore::new();
    for record in records {
        let lifecycle = parse_lifecycle(record)?;
        if store.records.contains_key(&lifecycle.spec_id) {
            return Err(format!("duplicate spec id: {}", lifecycle.spec_id));
        }
        store.records.insert(lifecycle.spec_id.clone(), lifecycle);
    }
    validate_records(&store.records)?;
    Ok(store)
}

fn parse_lifecycle(value: JsonValue) -> Result<SpecLifecycle, String> {
    let mut record = value.into_object("spec lifecycle record")?;
    require_only_keys(
        &record,
        &["spec_id", "state", "deferred_reason", "superseded_by"],
        "spec lifecycle record",
    )?;
    let spec_id =
        take_required(&mut record, "spec_id", "spec lifecycle record")?.into_string("spec_id")?;
    let state = SpecRunState::parse(
        &take_required(&mut record, "state", "spec lifecycle record")?.into_string("state")?,
    )?;
    let deferred_reason = take_required(&mut record, "deferred_reason", "spec lifecycle record")?
        .into_optional_string("deferred_reason")?;
    let superseded_by = take_required(&mut record, "superseded_by", "spec lifecycle record")?
        .into_optional_string("superseded_by")?;

    Ok(SpecLifecycle {
        spec_id,
        state,
        deferred_reason,
        superseded_by,
    })
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {key} in {context}"))
}

fn require_only_keys(
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

fn validate_records(records: &BTreeMap<String, SpecLifecycle>) -> Result<(), String> {
    for (spec_id, lifecycle) in records {
        if spec_id != &lifecycle.spec_id {
            return Err(format!(
                "state record key does not match spec id: {spec_id}"
            ));
        }
        if !is_valid_spec_id(spec_id) {
            return Err(format!("invalid spec id: {spec_id}"));
        }

        match lifecycle.state {
            SpecRunState::Deferred => {
                if lifecycle
                    .deferred_reason
                    .as_deref()
                    .is_none_or(|reason| reason.trim().is_empty())
                {
                    return Err(format!(
                        "deferred spec {spec_id} requires a deferred reason"
                    ));
                }
                if lifecycle.superseded_by.is_some() {
                    return Err(format!(
                        "deferred spec {spec_id} cannot include a superseded-by reference"
                    ));
                }
            }
            SpecRunState::Superseded => {
                let replacement = lifecycle.superseded_by.as_deref().ok_or_else(|| {
                    format!("superseded spec {spec_id} requires a replacement spec id")
                })?;
                if !is_valid_spec_id(replacement) {
                    return Err(format!(
                        "superseded spec {spec_id} has invalid replacement spec id: {replacement}"
                    ));
                }
                if replacement == spec_id {
                    return Err(format!("superseded spec {spec_id} cannot replace itself"));
                }
                if !records.contains_key(replacement) {
                    return Err(format!(
                        "superseded spec {spec_id} references missing replacement: {replacement}"
                    ));
                }
                if lifecycle.deferred_reason.is_some() {
                    return Err(format!(
                        "superseded spec {spec_id} cannot include a deferred reason"
                    ));
                }
            }
            _ => {
                if lifecycle.deferred_reason.is_some() || lifecycle.superseded_by.is_some() {
                    return Err(format!(
                        "{} spec {spec_id} cannot include deferred or superseded metadata",
                        lifecycle.state.as_str()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_allowed_transition(current: &SpecRunState, next: &SpecRunState) -> bool {
    matches!(
        (current, next),
        (SpecRunState::Planned, SpecRunState::Ready)
            | (SpecRunState::Planned, SpecRunState::Deferred)
            | (SpecRunState::Planned, SpecRunState::Superseded)
            | (SpecRunState::Ready, SpecRunState::Running)
            | (SpecRunState::Ready, SpecRunState::Deferred)
            | (SpecRunState::Ready, SpecRunState::Superseded)
            | (SpecRunState::Running, SpecRunState::Passed)
            | (SpecRunState::Running, SpecRunState::Failed)
            | (SpecRunState::Running, SpecRunState::Blocked)
            | (SpecRunState::Failed, SpecRunState::Running)
            | (SpecRunState::Blocked, SpecRunState::Ready)
    )
}

fn optional_json_string(value: &Option<String>) -> String {
    value
        .as_ref()
        .map(|value| format!("\"{}\"", escape_json_string(value)))
        .unwrap_or_else(|| "null".to_string())
}

fn escape_json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}
