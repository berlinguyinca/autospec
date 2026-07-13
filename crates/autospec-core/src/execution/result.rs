use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::agent::AgentResult;
use crate::spec::is_valid_spec_id;
use crate::state::json::{JsonParser, JsonValue};

use super::queue::{is_valid_run_id, FailureKind};

const AGENT_RESULT_SCHEMA: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentOutcome {
    Passed,
    Failed { failure_kind: FailureKind },
    Blocked,
}

impl AgentOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed { .. } => "failed",
            Self::Blocked => "blocked",
        }
    }

    fn to_json(&self) -> String {
        let failure_kind = match self {
            Self::Failed { failure_kind } => format!("\"{}\"", failure_kind.as_str()),
            Self::Passed | Self::Blocked => "null".to_string(),
        };
        format!(
            "{{\"status\":\"{}\",\"failure_kind\":{failure_kind}}}",
            self.as_str()
        )
    }

    fn from_json(value: JsonValue) -> Result<Self, String> {
        let mut object = value.into_object("agent outcome")?;
        require_keys(&object, &["status", "failure_kind"], "agent outcome")?;
        let status = take(&mut object, "status", "agent outcome")?.into_string("status")?;
        let failure_kind = take(&mut object, "failure_kind", "agent outcome")?;
        match status.as_str() {
            "passed" => {
                require_null(failure_kind, "failure_kind")?;
                Ok(Self::Passed)
            }
            "failed" => Ok(Self::Failed {
                failure_kind: FailureKind::parse(&failure_kind.into_string("failure_kind")?)?,
            }),
            "blocked" => {
                require_null(failure_kind, "failure_kind")?;
                Ok(Self::Blocked)
            }
            _ => Err(format!("unknown agent outcome: {status}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestedAgentResult {
    pub run_id: String,
    pub spec_id: String,
    pub result_id: String,
    pub outcome: AgentOutcome,
    pub agent_result: AgentResult,
    pub recorded_at: u64,
}

impl IngestedAgentResult {
    pub fn new(
        run_id: impl Into<String>,
        spec_id: impl Into<String>,
        result_id: impl Into<String>,
        outcome: AgentOutcome,
        agent_result: AgentResult,
    ) -> Result<Self, String> {
        Self::new_at(run_id, spec_id, result_id, outcome, agent_result, now())
    }

    pub fn new_at(
        run_id: impl Into<String>,
        spec_id: impl Into<String>,
        result_id: impl Into<String>,
        outcome: AgentOutcome,
        agent_result: AgentResult,
        recorded_at: u64,
    ) -> Result<Self, String> {
        let result = Self {
            run_id: run_id.into(),
            spec_id: spec_id.into(),
            result_id: result_id.into(),
            outcome,
            agent_result,
            recorded_at,
        };
        validate_result(&result)?;
        Ok(result)
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":{AGENT_RESULT_SCHEMA},\"run_id\":\"{}\",\"spec_id\":\"{}\",\"result_id\":\"{}\",\"outcome\":{},\"agent_result\":{},\"recorded_at\":{}}}",
            escape(&self.run_id),
            escape(&self.spec_id),
            escape(&self.result_id),
            self.outcome.to_json(),
            self.agent_result.to_json(),
            self.recorded_at,
        )
    }

    pub fn from_json(document: &str) -> Result<Self, String> {
        let mut object = JsonParser::new(document)
            .parse()?
            .into_object("ingested agent result")?;
        require_keys(
            &object,
            &[
                "schema",
                "run_id",
                "spec_id",
                "result_id",
                "outcome",
                "agent_result",
                "recorded_at",
            ],
            "ingested agent result",
        )?;
        let schema = take(&mut object, "schema", "ingested agent result")?.into_number("schema")?;
        if schema != AGENT_RESULT_SCHEMA {
            return Err(format!(
                "unsupported ingested agent result schema: {schema}"
            ));
        }
        let result = Self {
            run_id: take(&mut object, "run_id", "ingested agent result")?.into_string("run_id")?,
            spec_id: take(&mut object, "spec_id", "ingested agent result")?
                .into_string("spec_id")?,
            result_id: take(&mut object, "result_id", "ingested agent result")?
                .into_string("result_id")?,
            outcome: AgentOutcome::from_json(take(
                &mut object,
                "outcome",
                "ingested agent result",
            )?)?,
            agent_result: AgentResult::from_json_value(take(
                &mut object,
                "agent_result",
                "ingested agent result",
            )?)?,
            recorded_at: take(&mut object, "recorded_at", "ingested agent result")?
                .into_number("recorded_at")?,
        };
        validate_result(&result)?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_result(self)
    }

    pub(crate) fn persist_locked(&self, root: impl AsRef<Path>) -> Result<Self, String> {
        self.validate()?;
        let binding = ResultBinding::new(&self.run_id, &self.spec_id, &self.result_id)?;
        let paths = ResultPaths::new(root.as_ref(), &binding);
        if let Some(existing) = load_with_recovery(&paths, &binding)? {
            if existing.same_identity(self) {
                return Ok(existing);
            }
            return Err(format!(
                "agent result id {} already exists with different content",
                self.result_id
            ));
        }
        write_new(&paths, &self.to_json())?;
        Ok(self.clone())
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.run_id == other.run_id
            && self.spec_id == other.spec_id
            && self.result_id == other.result_id
            && self.outcome == other.outcome
            && self.agent_result == other.agent_result
    }
}

struct ResultPaths {
    root: PathBuf,
    autospec_directory: PathBuf,
    runs_directory: PathBuf,
    run_directory: PathBuf,
    agent_results_directory: PathBuf,
    spec_directory: PathBuf,
    primary: PathBuf,
    temporary: PathBuf,
}

impl ResultPaths {
    fn new(root: &Path, binding: &ResultBinding<'_>) -> Self {
        let root = root.to_path_buf();
        let autospec_directory = root.join(".autospec");
        let runs_directory = autospec_directory.join("runs");
        let run_directory = runs_directory.join(binding.run_id);
        let agent_results_directory = run_directory.join("agent-results");
        let spec_directory = agent_results_directory.join(binding.spec_id);
        Self {
            primary: spec_directory.join(format!("{}.json", binding.result_id)),
            temporary: spec_directory.join(format!("{}.json.tmp", binding.result_id)),
            root,
            autospec_directory,
            runs_directory,
            run_directory,
            agent_results_directory,
            spec_directory,
        }
    }
}

struct ResultBinding<'a> {
    run_id: &'a str,
    spec_id: &'a str,
    result_id: &'a str,
}

impl<'a> ResultBinding<'a> {
    fn new(run_id: &'a str, spec_id: &'a str, result_id: &'a str) -> Result<Self, String> {
        if !is_valid_run_id(run_id) {
            return Err(format!("invalid agent result run id: {run_id}"));
        }
        if !is_valid_spec_id(spec_id) {
            return Err(format!("invalid agent result spec id: {spec_id}"));
        }
        if !is_valid_run_id(result_id) {
            return Err(format!("invalid agent result id: {result_id}"));
        }
        Ok(Self {
            run_id,
            spec_id,
            result_id,
        })
    }
}

enum ResultFile {
    Missing,
    Valid(IngestedAgentResult),
    Invalid(String),
    Operational(String),
}

fn load_with_recovery(
    paths: &ResultPaths,
    expected: &ResultBinding<'_>,
) -> Result<Option<IngestedAgentResult>, String> {
    let primary = match load_result(&paths.primary) {
        ResultFile::Valid(result) => match bind_result(result, expected) {
            Ok(result) => return Ok(Some(result)),
            Err(error) => ResultFile::Invalid(error),
        },
        file => file,
    };
    match primary {
        primary @ (ResultFile::Missing | ResultFile::Invalid(_)) => {
            match load_result(&paths.temporary) {
                ResultFile::Valid(result) => {
                    let result = bind_result(result, expected)?;
                    promote_recovery(paths)?;
                    Ok(Some(result))
                }
                ResultFile::Missing => match primary {
                    ResultFile::Missing => Ok(None),
                    ResultFile::Invalid(error) => Err(format!(
                        "invalid agent result {}: {error}",
                        paths.primary.display()
                    )),
                    ResultFile::Valid(_) | ResultFile::Operational(_) => unreachable!(),
                },
                ResultFile::Invalid(error) => match primary {
                    ResultFile::Missing => {
                        discard_incomplete_recovery(paths, &error)?;
                        Ok(None)
                    }
                    ResultFile::Invalid(_) => Err(format!(
                        "invalid agent result recovery file {}: {error}",
                        paths.temporary.display()
                    )),
                    ResultFile::Valid(_) | ResultFile::Operational(_) => unreachable!(),
                },
                ResultFile::Operational(error) => Err(error),
            }
        }
        ResultFile::Valid(_) => unreachable!("valid agent results return before recovery"),
        ResultFile::Operational(error) => Err(error),
    }
}

fn load_result(path: &Path) -> ResultFile {
    match fs::read_to_string(path) {
        Ok(document) => IngestedAgentResult::from_json(&document)
            .map(ResultFile::Valid)
            .unwrap_or_else(ResultFile::Invalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ResultFile::Missing,
        Err(error) => ResultFile::Operational(format!(
            "failed to read agent result {}: {error}",
            path.display()
        )),
    }
}

fn bind_result(
    result: IngestedAgentResult,
    expected: &ResultBinding<'_>,
) -> Result<IngestedAgentResult, String> {
    if result.run_id != expected.run_id
        || result.spec_id != expected.spec_id
        || result.result_id != expected.result_id
    {
        return Err(format!(
            "agent result binding does not match path: {}/{}/{}",
            expected.run_id, expected.spec_id, expected.result_id
        ));
    }
    Ok(result)
}

fn write_new(paths: &ResultPaths, document: &str) -> Result<(), String> {
    fs::create_dir_all(&paths.spec_directory).map_err(|error| {
        format!(
            "failed to create agent result directory {}: {error}",
            paths.spec_directory.display()
        )
    })?;
    sync_directory_chain(paths)?;
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&paths.temporary)
        .map_err(|error| {
            format!(
                "failed to claim temporary agent result {}: {error}",
                paths.temporary.display()
            )
        })?;
    temporary.write_all(document.as_bytes()).map_err(|error| {
        format!(
            "failed to write temporary agent result {}: {error}",
            paths.temporary.display()
        )
    })?;
    temporary.sync_all().map_err(|error| {
        format!(
            "failed to synchronize temporary agent result {}: {error}",
            paths.temporary.display()
        )
    })?;
    drop(temporary);
    fs::hard_link(&paths.temporary, &paths.primary).map_err(|error| {
        format!(
            "failed to publish agent result {}: {error}",
            paths.primary.display()
        )
    })?;
    sync_directory_chain(paths)?;
    fs::remove_file(&paths.temporary).map_err(|error| {
        format!(
            "failed to finalize agent result {}: {error}",
            paths.temporary.display()
        )
    })?;
    sync_directory_chain(paths)
}

fn promote_recovery(paths: &ResultPaths) -> Result<(), String> {
    fs::rename(&paths.temporary, &paths.primary).map_err(|error| {
        format!(
            "failed to promote agent result recovery file {}: {error}",
            paths.temporary.display()
        )
    })?;
    sync_directory_chain(paths)
}

fn discard_incomplete_recovery(paths: &ResultPaths, _reason: &str) -> Result<(), String> {
    fs::remove_file(&paths.temporary).map_err(|error| {
        format!(
            "failed to discard incomplete agent result recovery file {}: {error}",
            paths.temporary.display()
        )
    })?;
    sync_directory_chain(paths)
}

#[cfg(unix)]
fn sync_directory_chain(paths: &ResultPaths) -> Result<(), String> {
    for directory in [
        &paths.spec_directory,
        &paths.agent_results_directory,
        &paths.run_directory,
        &paths.runs_directory,
        &paths.autospec_directory,
        &paths.root,
    ] {
        File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("failed to synchronize {}: {error}", directory.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory_chain(_paths: &ResultPaths) -> Result<(), String> {
    Err(
        "durable agent-result writes require directory synchronization support on this platform"
            .to_string(),
    )
}

fn validate_result(result: &IngestedAgentResult) -> Result<(), String> {
    ResultBinding::new(&result.run_id, &result.spec_id, &result.result_id)?;
    match &result.outcome {
        AgentOutcome::Passed | AgentOutcome::Failed { .. }
            if result.agent_result.validation.trim().is_empty() =>
        {
            Err("passed or failed agent results require a validation summary".to_string())
        }
        AgentOutcome::Blocked
            if !result
                .agent_result
                .blockers
                .iter()
                .any(|blocker| !blocker.trim().is_empty()) =>
        {
            Err("blocked agent results require at least one blocker".to_string())
        }
        _ => Ok(()),
    }
}

fn require_null(value: JsonValue, context: &str) -> Result<(), String> {
    match value {
        JsonValue::Null => Ok(()),
        _ => Err(format!("{context} must be null")),
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
