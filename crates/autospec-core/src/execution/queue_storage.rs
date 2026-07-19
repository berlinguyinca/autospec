use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::spec::is_valid_spec_id;
use crate::state::json::{JsonParser, JsonValue};

use super::queue::{
    is_valid_run_id, ExecutionQueue, FailureKind, QueueEntry, QueueValidationResult,
    AGENT_RESULTS_QUEUE_SCHEMA, LEGACY_QUEUE_SCHEMA, QUEUE_SCHEMA,
};
use super::queue_parser::parse_entry;

pub(super) struct QueuePaths {
    pub(super) run_id: String,
    pub(super) root: PathBuf,
    pub(super) autospec_directory: PathBuf,
    pub(super) runs_directory: PathBuf,
    pub(super) directory: PathBuf,
    primary: PathBuf,
    temporary: PathBuf,
}

pub(super) struct QueueLock {
    _file: File,
}

impl QueueLock {
    pub(super) fn acquire(paths: &QueuePaths) -> Result<Self, String> {
        fs::create_dir_all(&paths.runs_directory).map_err(|error| {
            format!(
                "failed to create queue run directory {}: {error}",
                paths.runs_directory.display()
            )
        })?;
        sync_queue_lock_directories(paths)?;
        let path = paths
            .runs_directory
            .join(format!(".{}.queue.lock", paths.run_id));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("failed to open queue lock {}: {error}", path.display()))?;
        file.lock()
            .map_err(|error| format!("failed to lock queue {}: {error}", paths.run_id))?;
        Ok(Self { _file: file })
    }
}

impl QueuePaths {
    pub(super) fn new(root: &Path, run_id: &str) -> Result<Self, String> {
        if !is_valid_run_id(run_id) {
            return Err(format!("invalid run id: {run_id}"));
        }
        let root = root.to_path_buf();
        let autospec_directory = root.join(".autospec");
        let runs_directory = autospec_directory.join("runs");
        let directory = runs_directory.join(run_id);
        Ok(Self {
            run_id: run_id.to_string(),
            root,
            autospec_directory,
            runs_directory,
            primary: directory.join("queue.json"),
            temporary: directory.join("queue.json.tmp"),
            directory,
        })
    }
}

pub(super) fn load_with_recovery(
    paths: &QueuePaths,
    expected_run_id: &str,
) -> Result<Option<ExecutionQueue>, String> {
    let primary = match load_queue(&paths.primary) {
        FileQueue::Valid(queue) => match bind_queue(queue, expected_run_id) {
            Ok(queue) => return Ok(Some(queue)),
            Err(error) => FileQueue::Invalid(error),
        },
        file => file,
    };
    match primary {
        primary @ (FileQueue::Missing | FileQueue::Invalid(_)) => {
            match load_queue(&paths.temporary) {
                FileQueue::Valid(queue) => {
                    let queue = bind_queue(queue, expected_run_id).map_err(|error| {
                        format!(
                            "invalid queue recovery file {}: {error}",
                            paths.temporary.display()
                        )
                    })?;
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
        FileQueue::Valid(_) => unreachable!("valid queues return before recovery"),
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

pub(super) fn write_document(paths: &QueuePaths, content: &str) -> Result<(), String> {
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

pub(super) fn save_if_current(
    queue: &mut ExecutionQueue,
    paths: &QueuePaths,
) -> Result<(), String> {
    let current = load_with_recovery(paths, &queue.run_id)?;
    if current.as_ref().map(|current| current.revision) != Some(queue.revision)
        && !(current.is_none() && queue.revision == 0)
    {
        return Err(format!(
            "queue revision conflict for run {}; reload before saving",
            queue.run_id
        ));
    }
    let next_revision = queue
        .revision
        .checked_add(1)
        .ok_or_else(|| format!("queue revision overflow for run {}", queue.run_id))?;
    let mut candidate = queue.clone();
    candidate.revision = next_revision;
    write_document(paths, &queue_json(&candidate)?)?;
    queue.revision = next_revision;
    Ok(())
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

fn bind_queue(queue: ExecutionQueue, expected_run_id: &str) -> Result<ExecutionQueue, String> {
    if queue.run_id != expected_run_id {
        return Err(format!(
            "queue document run id does not match path: {expected_run_id}"
        ));
    }
    Ok(queue)
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

#[cfg(unix)]
fn sync_queue_lock_directories(paths: &QueuePaths) -> Result<(), String> {
    for directory in [
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

#[cfg(not(unix))]
fn sync_queue_lock_directories(_paths: &QueuePaths) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

fn parse_queue(value: &str) -> Result<ExecutionQueue, String> {
    let mut object = JsonParser::new(value).parse()?.into_object("queue")?;
    let schema = take(&mut object, "schema", "queue")?.into_number("schema")?;
    if !matches!(
        schema,
        LEGACY_QUEUE_SCHEMA | AGENT_RESULTS_QUEUE_SCHEMA | QUEUE_SCHEMA
    ) {
        return Err(format!("unsupported queue schema: {schema}"));
    }
    if schema == QUEUE_SCHEMA {
        require_keys(
            &object,
            &["run_id", "updated_at", "revision", "entries"],
            "queue",
        )?;
    } else {
        require_keys(&object, &["run_id", "updated_at", "entries"], "queue")?;
    }
    let run_id = take(&mut object, "run_id", "queue")?.into_string("run_id")?;
    let updated_at = take(&mut object, "updated_at", "queue")?.into_number("updated_at")?;
    let revision = if schema == QUEUE_SCHEMA {
        take(&mut object, "revision", "queue")?.into_number("revision")?
    } else {
        0
    };
    let entries = take(&mut object, "entries", "queue")?
        .into_array("entries")?
        .into_iter()
        .map(|value| parse_entry(value, schema))
        .collect::<Result<Vec<_>, _>>()?;
    let queue = ExecutionQueue {
        run_id,
        updated_at,
        revision,
        entries,
    };
    validate_queue(&queue)?;
    Ok(queue)
}

pub(super) fn validate_queue(queue: &ExecutionQueue) -> Result<(), String> {
    if !is_valid_run_id(&queue.run_id) {
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
        let mut result_ids = BTreeMap::new();
        for result_id in &entry.agent_result_ids {
            if !is_valid_run_id(result_id) || result_ids.insert(result_id, ()).is_some() {
                return Err(format!(
                    "invalid or duplicate queue agent result id: {result_id}"
                ));
            }
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

pub(super) fn queue_json(queue: &ExecutionQueue) -> Result<String, String> {
    validate_queue(queue)?;
    Ok(format!(
        "{{\"schema\":{QUEUE_SCHEMA},\"run_id\":\"{}\",\"updated_at\":{},\"revision\":{},\"entries\":[{}]}}",
        escape(&queue.run_id),
        queue.updated_at,
        queue.revision,
        queue
            .entries
            .iter()
            .map(entry_json)
            .collect::<Vec<_>>()
            .join(",")
    ))
}

fn entry_json(entry: &QueueEntry) -> String {
    format!(
        "{{\"spec_id\":\"{}\",\"status\":\"{}\",\"attempts\":{},\"failure_kind\":{},\"blocker\":{},\"started_at\":{},\"updated_at\":{},\"validation\":{},\"agent_result_ids\":{}}}",
        escape(&entry.spec_id),
        entry.status.as_str(),
        entry.attempts,
        optional_failure(&entry.failure_kind),
        optional_text(&entry.blocker),
        optional_number_json(entry.started_at),
        entry.updated_at,
        optional_validation_json(&entry.validation),
        string_array_json(&entry.agent_result_ids)
    )
}

fn string_array_json(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!("\"{}\"", escape(value)))
            .collect::<Vec<_>>()
            .join(",")
    )
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

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
