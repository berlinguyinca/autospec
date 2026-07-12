use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::state::json::{JsonParser, JsonValue};
use crate::state::{SpecLifecycle, SpecRunState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceCommand {
    pub command: String,
    pub exit_code: i32,
    pub stdout_path: String,
    pub stderr_path: String,
}

impl EvidenceCommand {
    pub fn new(
        command: impl Into<String>,
        exit_code: i32,
        stdout_path: impl Into<String>,
        stderr_path: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            exit_code,
            stdout_path: stdout_path.into(),
            stderr_path: stderr_path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBundle {
    pub run_id: String,
    pub commands: Vec<EvidenceCommand>,
    pub artifacts: Vec<String>,
}

impl EvidenceBundle {
    pub fn new(
        run_id: impl Into<String>,
        commands: Vec<EvidenceCommand>,
        artifacts: Vec<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            commands,
            artifacts,
        }
    }

    pub fn to_json(&self) -> String {
        let commands = self
            .commands
            .iter()
            .map(|command| {
                format!(
                    "{{\"command\":\"{}\",\"exit_code\":{},\"stdout_path\":\"{}\",\"stderr_path\":\"{}\"}}",
                    escape_json(&command.command),
                    command.exit_code,
                    escape_json(&command.stdout_path),
                    escape_json(&command.stderr_path)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"run_id\":\"{}\",\"commands\":[{}],\"artifacts\":{}}}",
            escape_json(&self.run_id),
            commands,
            json_array(&self.artifacts)
        )
    }

    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), String> {
        validate_bundle(self)?;
        let directory = evidence_directory(root.as_ref(), &self.run_id)?;
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let temporary = directory.join("bundle.json.tmp");
        let primary = directory.join("bundle.json");
        fs::write(&temporary, self.to_json()).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &primary)
            .or_else(|error| {
                if primary.exists() {
                    fs::remove_file(&primary)?;
                    fs::rename(&temporary, &primary)
                } else {
                    Err(error)
                }
            })
            .map_err(|error| error.to_string())
    }

    pub fn load_named(root: impl AsRef<Path>, run_id: &str) -> Result<Option<Self>, String> {
        let directory = evidence_directory(root.as_ref(), run_id)?;
        let primary = directory.join("bundle.json");
        let temporary = directory.join("bundle.json.tmp");
        match load_bundle(&primary) {
            BundleFile::Valid(bundle) => bind_bundle(bundle, run_id).map(Some),
            BundleFile::Missing | BundleFile::Invalid(_) => match load_bundle(&temporary) {
                BundleFile::Valid(bundle) => {
                    let bundle = bind_bundle(bundle, run_id)?;
                    fs::rename(&temporary, &primary)
                        .or_else(|error| {
                            if primary.exists() {
                                fs::remove_file(&primary)?;
                                fs::rename(&temporary, &primary)
                            } else {
                                Err(error)
                            }
                        })
                        .map_err(|error| error.to_string())?;
                    Ok(Some(bundle))
                }
                BundleFile::Missing if !primary.exists() => Ok(None),
                BundleFile::Missing => {
                    Err("invalid evidence bundle without recovery file".to_string())
                }
                BundleFile::Invalid(error) => Err(error),
            },
        }
    }
}

enum BundleFile {
    Missing,
    Valid(EvidenceBundle),
    Invalid(String),
}

fn evidence_directory(root: &Path, run_id: &str) -> Result<std::path::PathBuf, String> {
    if !valid_run_id(run_id) {
        return Err(format!("invalid evidence run id: {run_id}"));
    }
    Ok(root.join(".autospec").join("evidence").join(run_id))
}

fn load_bundle(path: &Path) -> BundleFile {
    match fs::read_to_string(path) {
        Ok(value) => parse_bundle(&value)
            .map(BundleFile::Valid)
            .unwrap_or_else(BundleFile::Invalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BundleFile::Missing,
        Err(error) => BundleFile::Invalid(error.to_string()),
    }
}

fn bind_bundle(bundle: EvidenceBundle, run_id: &str) -> Result<EvidenceBundle, String> {
    if bundle.run_id != run_id {
        return Err(format!(
            "evidence bundle run id does not match path: {run_id}"
        ));
    }
    Ok(bundle)
}

fn parse_bundle(document: &str) -> Result<EvidenceBundle, String> {
    let mut object = JsonParser::new(document)
        .parse()?
        .into_object("evidence bundle")?;
    require_keys(
        &object,
        &["run_id", "commands", "artifacts"],
        "evidence bundle",
    )?;
    let run_id = take(&mut object, "run_id", "evidence bundle")?.into_string("run_id")?;
    let commands = take(&mut object, "commands", "evidence bundle")?
        .into_array("commands")?
        .into_iter()
        .map(parse_command)
        .collect::<Result<Vec<_>, _>>()?;
    let artifacts = take(&mut object, "artifacts", "evidence bundle")?
        .into_array("artifacts")?
        .into_iter()
        .map(|value| value.into_string("artifact"))
        .collect::<Result<Vec<_>, _>>()?;
    let bundle = EvidenceBundle {
        run_id,
        commands,
        artifacts,
    };
    validate_bundle(&bundle)?;
    Ok(bundle)
}

fn parse_command(value: JsonValue) -> Result<EvidenceCommand, String> {
    let mut object = value.into_object("evidence command")?;
    require_keys(
        &object,
        &["command", "exit_code", "stdout_path", "stderr_path"],
        "evidence command",
    )?;
    let command = take(&mut object, "command", "evidence command")?.into_string("command")?;
    let exit_code = i32::try_from(
        take(&mut object, "exit_code", "evidence command")?.into_number("exit_code")?,
    )
    .map_err(|_| "exit code exceeds i32".to_string())?;
    let stdout_path =
        take(&mut object, "stdout_path", "evidence command")?.into_string("stdout_path")?;
    let stderr_path =
        take(&mut object, "stderr_path", "evidence command")?.into_string("stderr_path")?;
    Ok(EvidenceCommand::new(
        command,
        exit_code,
        stdout_path,
        stderr_path,
    ))
}

fn validate_bundle(bundle: &EvidenceBundle) -> Result<(), String> {
    if !valid_run_id(&bundle.run_id) {
        return Err(format!("invalid evidence run id: {}", bundle.run_id));
    }
    let prefix = format!(".autospec/evidence/{}/", bundle.run_id);
    let mut artifacts = BTreeMap::new();
    for artifact in &bundle.artifacts {
        if !valid_evidence_path(artifact, &prefix) || artifacts.insert(artifact, ()).is_some() {
            return Err(format!(
                "invalid or duplicate evidence artifact: {artifact}"
            ));
        }
    }
    for command in &bundle.commands {
        if !valid_evidence_path(&command.stdout_path, &prefix)
            || !valid_evidence_path(&command.stderr_path, &prefix)
        {
            return Err("evidence output path escapes bundle directory".to_string());
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
fn valid_evidence_path(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseReport {
    pub version: String,
    pub passed: usize,
    pub failed: usize,
    pub blocked: usize,
    pub deferred: usize,
    pub superseded: usize,
}

impl ReleaseReport {
    pub fn from_states(
        version: impl Into<String>,
        states: &[SpecLifecycle],
    ) -> Result<Self, String> {
        let mut report = Self {
            version: version.into(),
            passed: 0,
            failed: 0,
            blocked: 0,
            deferred: 0,
            superseded: 0,
        };

        for state in states {
            match state.state {
                SpecRunState::Passed => report.passed += 1,
                SpecRunState::Failed => report.failed += 1,
                SpecRunState::Blocked => report.blocked += 1,
                SpecRunState::Deferred => report.deferred += 1,
                SpecRunState::Superseded => report.superseded += 1,
                SpecRunState::Planned | SpecRunState::Ready | SpecRunState::Running => {
                    return Err(format!(
                        "{} has unknown or unfinished state {}",
                        state.spec_id,
                        state.state.as_str()
                    ));
                }
            }
        }

        Ok(report)
    }

    pub fn to_markdown(&self) -> String {
        format!(
            "# AutoSpec Release Report {}\n\npassed: {}\nfailed: {}\nblocked: {}\ndeferred: {}\nsuperseded: {}\n",
            self.version, self.passed, self.failed, self.blocked, self.deferred, self.superseded
        )
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"version\":\"{}\",\"passed\":{},\"failed\":{},\"blocked\":{},\"deferred\":{},\"superseded\":{}}}",
            escape_json(&self.version),
            self.passed,
            self.failed,
            self.blocked,
            self.deferred,
            self.superseded
        )
    }
}

fn json_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("\"{}\"", escape_json(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn escape_json(value: &str) -> String {
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
