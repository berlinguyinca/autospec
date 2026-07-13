use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::spec::is_valid_spec_id;
use crate::state::json::{JsonParser, JsonValue};

const QUEUE_SCHEMA: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Blocked,
    Deferred,
    Superseded,
}

impl QueueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Deferred => "deferred",
            Self::Superseded => "superseded",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            "deferred" => Ok(Self::Deferred),
            "superseded" => Ok(Self::Superseded),
            _ => Err(format!("unknown queue status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    Validation,
    Environment,
    Agent,
    Dependency,
    Safety,
}

impl FailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Environment => "environment",
            Self::Agent => "agent",
            Self::Dependency => "dependency",
            Self::Safety => "safety",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "validation" => Ok(Self::Validation),
            "environment" => Ok(Self::Environment),
            "agent" => Ok(Self::Agent),
            "dependency" => Ok(Self::Dependency),
            "safety" => Ok(Self::Safety),
            _ => Err(format!("unknown failure kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueValidationStatus {
    Passed,
    Failed,
}

impl QueueValidationStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("unknown queue validation status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueValidationResult {
    pub status: QueueValidationStatus,
    pub summary: String,
}

impl QueueValidationResult {
    pub fn new(status: QueueValidationStatus, summary: impl Into<String>) -> Self {
        Self {
            status,
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub spec_id: String,
    pub status: QueueStatus,
    pub attempts: u32,
    pub failure_kind: Option<FailureKind>,
    pub blocker: Option<String>,
    pub started_at: Option<u64>,
    pub updated_at: u64,
    pub validation: Option<QueueValidationResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionQueue {
    pub run_id: String,
    pub updated_at: u64,
    entries: Vec<QueueEntry>,
}

impl ExecutionQueue {
    pub fn new(run_id: impl Into<String>, spec_ids: Vec<String>) -> Self {
        Self::new_at(run_id, spec_ids, now())
    }

    pub fn new_at(run_id: impl Into<String>, spec_ids: Vec<String>, timestamp: u64) -> Self {
        Self {
            run_id: run_id.into(),
            updated_at: timestamp,
            entries: spec_ids
                .into_iter()
                .map(|spec_id| QueueEntry {
                    spec_id,
                    status: QueueStatus::Pending,
                    attempts: 0,
                    failure_kind: None,
                    blocker: None,
                    started_at: None,
                    updated_at: timestamp,
                    validation: None,
                })
                .collect(),
        }
    }

    pub fn entry(&self, spec_id: &str) -> Option<&QueueEntry> {
        self.entries.iter().find(|entry| entry.spec_id == spec_id)
    }
    pub fn next_incomplete(&self) -> Option<&QueueEntry> {
        self.entries.iter().find(|entry| {
            matches!(
                entry.status,
                QueueStatus::Pending | QueueStatus::Running | QueueStatus::Failed
            )
        })
    }

    pub fn mark_started_at(&mut self, spec_id: &str, timestamp: u64) -> Result<(), String> {
        self.update(spec_id, timestamp, |entry| {
            if !matches!(
                entry.status,
                QueueStatus::Pending | QueueStatus::Failed | QueueStatus::Running
            ) {
                return Err(format!("cannot restart terminal queue entry: {spec_id}"));
            }
            entry.status = QueueStatus::Running;
            entry.started_at.get_or_insert(timestamp);
            Ok(())
        })
    }
    pub fn mark_passed_at(&mut self, spec_id: &str, timestamp: u64) -> Result<(), String> {
        self.update(spec_id, timestamp, |entry| {
            entry.status = QueueStatus::Passed;
            entry.blocker = None;
            Ok(())
        })
    }
    pub fn record_validation_at(
        &mut self,
        spec_id: &str,
        validation: QueueValidationResult,
        timestamp: u64,
    ) -> Result<(), String> {
        self.update(spec_id, timestamp, |entry| {
            entry.validation = Some(validation);
            Ok(())
        })
    }
    pub fn mark_passed(&mut self, spec_id: &str) -> Result<(), String> {
        self.mark_passed_at(spec_id, now())
    }
    pub fn record_failure(
        &mut self,
        spec_id: &str,
        failure_kind: FailureKind,
        retry_limit: u32,
    ) -> Result<(), String> {
        let timestamp = now();
        let retry_limit_exceeded = {
            let entry = self.entry_mut(spec_id)?;
            entry.attempts += 1;
            entry.failure_kind = Some(failure_kind);
            entry.updated_at = timestamp;
            if entry.attempts > retry_limit {
                entry.status = QueueStatus::Blocked;
                entry.blocker = Some("retry limit exceeded".to_string());
                true
            } else {
                entry.status = QueueStatus::Failed;
                false
            }
        };
        self.updated_at = timestamp;
        if retry_limit_exceeded {
            Err(format!("retry limit exceeded for {spec_id}"))
        } else {
            Ok(())
        }
    }
    pub fn block(&mut self, spec_id: &str, reason: impl Into<String>) -> Result<(), String> {
        let reason = reason.into();
        self.update(spec_id, now(), |entry| {
            entry.status = QueueStatus::Blocked;
            entry.blocker = Some(reason);
            Ok(())
        })
    }

    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), String> {
        validate_queue(self)?;
        let paths = QueuePaths::new(root.as_ref(), &self.run_id)?;
        write_document(&paths, &self.to_json()?)
    }
    pub fn load_named(root: impl AsRef<Path>, run_id: &str) -> Result<Option<Self>, String> {
        let paths = QueuePaths::new(root.as_ref(), run_id)?;
        let queue = load_with_recovery(&paths)?;
        if queue.as_ref().is_some_and(|queue| queue.run_id != run_id) {
            return Err(format!(
                "queue document run id does not match path: {run_id}"
            ));
        }
        Ok(queue)
    }
    pub fn load_latest_incomplete(root: impl AsRef<Path>) -> Result<Option<Self>, String> {
        let runs = root.as_ref().join(".autospec").join("runs");
        let entries = match fs::read_dir(&runs) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "failed to read run directory {}: {error}",
                    runs.display()
                ))
            }
        };
        let mut latest: Option<Self> = None;
        for entry in entries {
            let entry = entry.map_err(|error| format!("failed to read run entry: {error}"))?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                continue;
            }
            let run_id = entry.file_name().to_string_lossy().to_string();
            let queue = match Self::load_named(root.as_ref(), &run_id) {
                Ok(Some(queue)) => queue,
                Ok(None) => continue,
                Err(error)
                    if error.starts_with("invalid queue file")
                        || error.starts_with("invalid queue recovery file") =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            if queue.next_incomplete().is_some()
                && latest.as_ref().is_none_or(|current| {
                    (queue.updated_at, &queue.run_id) > (current.updated_at, &current.run_id)
                })
            {
                latest = Some(queue);
            }
        }
        Ok(latest)
    }

    pub fn handoff_markdown(&self, spec_id: &str) -> Option<String> {
        let entry = self.entry(spec_id)?;
        Some(format!(
            "# Blocked Spec: {}\n\nRun: {}\n\nStatus: {}\n\nReason: {}\n",
            entry.spec_id,
            self.run_id,
            entry.status.as_str(),
            entry.blocker.as_deref().unwrap_or("blocked without reason")
        ))
    }
    pub fn final_report_markdown(&self) -> String {
        format!("# AutoSpec Run Report\n\nRun: {}\n\npassed: {}\nfailed: {}\nblocked: {}\ndeferred: {}\nsuperseded: {}\n", self.run_id, self.count(QueueStatus::Passed), self.count(QueueStatus::Failed), self.count(QueueStatus::Blocked), self.count(QueueStatus::Deferred), self.count(QueueStatus::Superseded))
    }

    fn update(
        &mut self,
        spec_id: &str,
        timestamp: u64,
        change: impl FnOnce(&mut QueueEntry) -> Result<(), String>,
    ) -> Result<(), String> {
        let entry = self.entry_mut(spec_id)?;
        change(entry)?;
        entry.updated_at = timestamp;
        self.updated_at = timestamp;
        Ok(())
    }
    fn count(&self, status: QueueStatus) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.status == status)
            .count()
    }
    fn entry_mut(&mut self, spec_id: &str) -> Result<&mut QueueEntry, String> {
        self.entries
            .iter_mut()
            .find(|entry| entry.spec_id == spec_id)
            .ok_or_else(|| format!("unknown queue spec: {spec_id}"))
    }
    fn to_json(&self) -> Result<String, String> {
        validate_queue(self)?;
        Ok(format!(
            "{{\"schema\":1,\"run_id\":\"{}\",\"updated_at\":{},\"entries\":[{}]}}",
            escape(&self.run_id),
            self.updated_at,
            self.entries
                .iter()
                .map(entry_json)
                .collect::<Vec<_>>()
                .join(",")
        ))
    }
}

struct QueuePaths {
    root: PathBuf,
    autospec_directory: PathBuf,
    runs_directory: PathBuf,
    directory: PathBuf,
    primary: PathBuf,
    temporary: PathBuf,
}
impl QueuePaths {
    fn new(root: &Path, run_id: &str) -> Result<Self, String> {
        if !valid_run_id(run_id) {
            return Err(format!("invalid run id: {run_id}"));
        }
        let root = root.to_path_buf();
        let autospec_directory = root.join(".autospec");
        let runs_directory = autospec_directory.join("runs");
        let directory = runs_directory.join(run_id);
        Ok(Self {
            root,
            autospec_directory,
            runs_directory,
            primary: directory.join("queue.json"),
            temporary: directory.join("queue.json.tmp"),
            directory,
        })
    }
}

fn load_with_recovery(paths: &QueuePaths) -> Result<Option<ExecutionQueue>, String> {
    match load_queue(&paths.primary) {
        FileQueue::Valid(queue) => Ok(Some(queue)),
        primary @ (FileQueue::Missing | FileQueue::Invalid(_)) => {
            match load_queue(&paths.temporary) {
                FileQueue::Valid(queue) => {
                    promote(paths)?;
                    Ok(Some(queue))
                }
                FileQueue::Missing => match primary {
                    FileQueue::Missing => Ok(None),
                    FileQueue::Invalid(error) => Err(format!(
                        "invalid queue file {}: {error}",
                        paths.primary.display()
                    )),
                    FileQueue::Valid(_) | FileQueue::Operational(_) => unreachable!(),
                },
                FileQueue::Invalid(error) => Err(format!(
                    "invalid queue recovery file {}: {error}",
                    paths.temporary.display()
                )),
                FileQueue::Operational(error) => Err(error),
            }
        }
        FileQueue::Operational(error) => Err(error),
    }
}
enum FileQueue {
    Missing,
    Valid(ExecutionQueue),
    Invalid(String),
    Operational(String),
}
fn load_queue(path: &Path) -> FileQueue {
    match fs::read_to_string(path) {
        Ok(value) => match parse_queue(&value) {
            Ok(queue) => FileQueue::Valid(queue),
            Err(error) => FileQueue::Invalid(error),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileQueue::Missing,
        Err(error) => FileQueue::Operational(format!(
            "failed to read queue file {}: {error}",
            path.display()
        )),
    }
}
fn write_document(paths: &QueuePaths, content: &str) -> Result<(), String> {
    fs::create_dir_all(&paths.directory).map_err(|error| error.to_string())?;
    sync_directory_chain(paths)?;
    let mut file = File::create(&paths.temporary).map_err(|error| error.to_string())?;
    file.write_all(content.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    sync_directory_chain(paths)?;
    drop(file);
    promote(paths)
}
fn promote(paths: &QueuePaths) -> Result<(), String> {
    fs::rename(&paths.temporary, &paths.primary)
        .or_else(|first| {
            if paths.primary.exists() {
                fs::remove_file(&paths.primary)?;
                fs::rename(&paths.temporary, &paths.primary)
            } else {
                Err(first)
            }
        })
        .map_err(|error| error.to_string())?;
    sync_directory_chain(paths)
}

#[cfg(unix)]
fn sync_directory_chain(paths: &QueuePaths) -> Result<(), String> {
    for directory in [
        &paths.directory,
        &paths.runs_directory,
        &paths.autospec_directory,
        &paths.root,
    ] {
        sync_directory(directory)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory_chain(_paths: &QueuePaths) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn parse_queue(value: &str) -> Result<ExecutionQueue, String> {
    let mut object = JsonParser::new(value).parse()?.into_object("queue")?;
    require_keys(
        &object,
        &["schema", "run_id", "updated_at", "entries"],
        "queue",
    )?;
    let schema = take(&mut object, "schema", "queue")?.into_number("schema")?;
    if schema != QUEUE_SCHEMA {
        return Err(format!("unsupported queue schema: {schema}"));
    }
    let run_id = take(&mut object, "run_id", "queue")?.into_string("run_id")?;
    let updated_at = take(&mut object, "updated_at", "queue")?.into_number("updated_at")?;
    let entries = take(&mut object, "entries", "queue")?
        .into_array("entries")?
        .into_iter()
        .map(parse_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let queue = ExecutionQueue {
        run_id,
        updated_at,
        entries,
    };
    validate_queue(&queue)?;
    Ok(queue)
}
fn parse_entry(value: JsonValue) -> Result<QueueEntry, String> {
    let mut object = value.into_object("queue entry")?;
    require_keys(
        &object,
        &[
            "spec_id",
            "status",
            "attempts",
            "failure_kind",
            "blocker",
            "started_at",
            "updated_at",
            "validation",
        ],
        "queue entry",
    )?;
    let spec_id = take(&mut object, "spec_id", "queue entry")?.into_string("spec_id")?;
    let status =
        QueueStatus::parse(&take(&mut object, "status", "queue entry")?.into_string("status")?)?;
    let attempts =
        u32::try_from(take(&mut object, "attempts", "queue entry")?.into_number("attempts")?)
            .map_err(|_| "attempt count exceeds u32".to_string())?;
    let failure_kind = optional_string(
        take(&mut object, "failure_kind", "queue entry")?,
        "failure_kind",
    )?
    .map(|value| FailureKind::parse(&value))
    .transpose()?;
    let blocker = optional_string(take(&mut object, "blocker", "queue entry")?, "blocker")?;
    let started_at = optional_number(
        take(&mut object, "started_at", "queue entry")?,
        "started_at",
    )?;
    let updated_at = take(&mut object, "updated_at", "queue entry")?.into_number("updated_at")?;
    let validation = optional_validation(take(&mut object, "validation", "queue entry")?)?;
    Ok(QueueEntry {
        spec_id,
        status,
        attempts,
        failure_kind,
        blocker,
        started_at,
        updated_at,
        validation,
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
fn validate_queue(queue: &ExecutionQueue) -> Result<(), String> {
    if !valid_run_id(&queue.run_id) {
        return Err(format!("invalid run id: {}", queue.run_id));
    }
    let mut ids = BTreeMap::new();
    for entry in &queue.entries {
        if !is_valid_spec_id(&entry.spec_id) {
            return Err(format!("invalid queue spec id: {}", entry.spec_id));
        }
        if ids.insert(&entry.spec_id, ()).is_some() {
            return Err(format!("duplicate queue spec id: {}", entry.spec_id));
        }
        if entry
            .started_at
            .is_some_and(|timestamp| timestamp > entry.updated_at)
        {
            return Err(format!(
                "queue entry {} starts after its update",
                entry.spec_id
            ));
        }
    }
    Ok(())
}
fn valid_run_id(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
fn entry_json(entry: &QueueEntry) -> String {
    format!("{{\"spec_id\":\"{}\",\"status\":\"{}\",\"attempts\":{},\"failure_kind\":{},\"blocker\":{},\"started_at\":{},\"updated_at\":{},\"validation\":{}}}", escape(&entry.spec_id), entry.status.as_str(), entry.attempts, optional_failure(&entry.failure_kind), optional_text(&entry.blocker), optional_number_json(entry.started_at), entry.updated_at, optional_validation_json(&entry.validation))
}
fn optional_failure(value: &Option<FailureKind>) -> String {
    value
        .as_ref()
        .map(|value| format!("\"{}\"", value.as_str()))
        .unwrap_or_else(|| "null".to_string())
}
fn optional_text(value: &Option<String>) -> String {
    value
        .as_ref()
        .map(|value| format!("\"{}\"", escape(value)))
        .unwrap_or_else(|| "null".to_string())
}
fn optional_number_json(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}
fn optional_validation_json(value: &Option<QueueValidationResult>) -> String {
    value
        .as_ref()
        .map(|value| {
            format!(
                "{{\"status\":\"{}\",\"summary\":\"{}\"}}",
                value.status.as_str(),
                escape(&value.summary)
            )
        })
        .unwrap_or_else(|| "null".to_string())
}
fn escape(value: &str) -> String {
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
                escaped.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
