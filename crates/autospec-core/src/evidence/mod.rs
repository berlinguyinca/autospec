use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::state::json::{JsonParser, JsonValue};
use crate::state::{SpecLifecycle, SpecRunState};

const BUNDLE_SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceCommand {
    pub command: String,
    pub exit_code: i32,
    pub stdout_path: String,
    pub stderr_path: String,
    pub captured_at: u64,
}

impl EvidenceCommand {
    pub fn new(
        command: impl Into<String>,
        exit_code: i32,
        stdout_path: impl Into<String>,
        stderr_path: impl Into<String>,
        captured_at: u64,
    ) -> Self {
        Self {
            command: command.into(),
            exit_code,
            stdout_path: stdout_path.into(),
            stderr_path: stderr_path.into(),
            captured_at,
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
                    "{{\"command\":\"{}\",\"exit_code\":{},\"stdout_path\":\"{}\",\"stderr_path\":\"{}\",\"captured_at\":{}}}",
                    escape_json(&command.command),
                    command.exit_code,
                    escape_json(&command.stdout_path),
                    escape_json(&command.stderr_path),
                    command.captured_at
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"schema\":{BUNDLE_SCHEMA_VERSION},\"run_id\":\"{}\",\"commands\":[{}],\"artifacts\":{}}}",
            escape_json(&self.run_id),
            commands,
            json_array(&self.artifacts)
        )
    }

    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), String> {
        validate_bundle(self)?;
        let paths = EvidencePaths::new(root.as_ref(), &self.run_id)?;
        write_bundle(&paths, &self.to_json())
    }

    pub fn load_named(root: impl AsRef<Path>, run_id: &str) -> Result<Option<Self>, String> {
        let paths = EvidencePaths::new(root.as_ref(), run_id)?;
        load_with_recovery(&paths, run_id)
    }
}

enum BundleFile {
    Missing,
    Valid(EvidenceBundle),
    Invalid(String),
    Operational(String),
}

struct EvidencePaths {
    root: PathBuf,
    autospec_directory: PathBuf,
    evidence_directory: PathBuf,
    directory: PathBuf,
    primary: PathBuf,
    temporary: PathBuf,
}

impl EvidencePaths {
    fn new(root: &Path, run_id: &str) -> Result<Self, String> {
        if !valid_run_id(run_id) {
            return Err(format!("invalid evidence run id: {run_id}"));
        }
        let root = root.to_path_buf();
        let autospec_directory = root.join(".autospec");
        let evidence_directory = autospec_directory.join("evidence");
        let directory = evidence_directory.join(run_id);
        Ok(Self {
            primary: directory.join("bundle.json"),
            temporary: directory.join("bundle.json.tmp"),
            root,
            autospec_directory,
            evidence_directory,
            directory,
        })
    }
}

fn load_bundle(path: &Path) -> BundleFile {
    match fs::read_to_string(path) {
        Ok(value) => parse_bundle(&value)
            .map(BundleFile::Valid)
            .unwrap_or_else(BundleFile::Invalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BundleFile::Missing,
        Err(error) => BundleFile::Operational(format!(
            "failed to read evidence bundle {}: {error}",
            path.display()
        )),
    }
}

fn load_with_recovery(
    paths: &EvidencePaths,
    run_id: &str,
) -> Result<Option<EvidenceBundle>, String> {
    let primary = match load_bundle(&paths.primary) {
        BundleFile::Valid(bundle) => match bind_bundle(bundle, run_id) {
            Ok(bundle) => return Ok(Some(bundle)),
            Err(error) => BundleFile::Invalid(error),
        },
        file => file,
    };

    match primary {
        primary @ (BundleFile::Missing | BundleFile::Invalid(_)) => {
            match load_bundle(&paths.temporary) {
                BundleFile::Valid(bundle) => {
                    let bundle = bind_bundle(bundle, run_id)?;
                    promote(paths)?;
                    Ok(Some(bundle))
                }
                BundleFile::Missing => match primary {
                    BundleFile::Missing => Ok(None),
                    BundleFile::Invalid(error) => Err(format!(
                        "invalid evidence bundle {}: {error}",
                        paths.primary.display()
                    )),
                    BundleFile::Valid(_) | BundleFile::Operational(_) => unreachable!(),
                },
                BundleFile::Invalid(error) => Err(format!(
                    "invalid evidence recovery bundle {}: {error}",
                    paths.temporary.display()
                )),
                BundleFile::Operational(error) => Err(error),
            }
        }
        BundleFile::Valid(_) => unreachable!("valid primary bundles return before recovery"),
        BundleFile::Operational(error) => Err(error),
    }
}

fn write_bundle(paths: &EvidencePaths, document: &str) -> Result<(), String> {
    fs::create_dir_all(&paths.directory).map_err(|error| {
        format!(
            "failed to create evidence directory {}: {error}",
            paths.directory.display()
        )
    })?;
    sync_directory_chain(paths)?;
    let mut temporary = File::create(&paths.temporary).map_err(|error| {
        format!(
            "failed to create temporary evidence bundle {}: {error}",
            paths.temporary.display()
        )
    })?;
    temporary.write_all(document.as_bytes()).map_err(|error| {
        format!(
            "failed to write temporary evidence bundle {}: {error}",
            paths.temporary.display()
        )
    })?;
    temporary.sync_all().map_err(|error| {
        format!(
            "failed to synchronize temporary evidence bundle {}: {error}",
            paths.temporary.display()
        )
    })?;
    sync_directory_chain(paths)?;
    drop(temporary);
    promote(paths)
}

fn promote(paths: &EvidencePaths) -> Result<(), String> {
    fs::rename(&paths.temporary, &paths.primary)
        .or_else(|first_error| {
            if paths.primary.exists() {
                fs::remove_file(&paths.primary)?;
                fs::rename(&paths.temporary, &paths.primary)
            } else {
                Err(first_error)
            }
        })
        .map_err(|error| {
            format!(
                "failed to promote temporary evidence bundle {} to {}: {error}",
                paths.temporary.display(),
                paths.primary.display()
            )
        })?;
    sync_directory_chain(paths)
}

#[cfg(unix)]
fn sync_directory_chain(paths: &EvidencePaths) -> Result<(), String> {
    for directory in [
        &paths.directory,
        &paths.evidence_directory,
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
fn sync_directory_chain(_paths: &EvidencePaths) -> Result<(), String> {
    Err(
        "durable evidence bundle writes require directory synchronization support on this platform"
            .to_string(),
    )
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
        &["schema", "run_id", "commands", "artifacts"],
        "evidence bundle",
    )?;
    let schema = take(&mut object, "schema", "evidence bundle")?.into_number("schema")?;
    if schema != BUNDLE_SCHEMA_VERSION {
        return Err(format!("unsupported evidence bundle schema: {schema}"));
    }
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
        &[
            "command",
            "exit_code",
            "stdout_path",
            "stderr_path",
            "captured_at",
        ],
        "evidence command",
    )?;
    let command = take(&mut object, "command", "evidence command")?.into_string("command")?;
    let exit_code = i32::try_from(
        take(&mut object, "exit_code", "evidence command")?.into_signed_number("exit_code")?,
    )
    .map_err(|_| "exit code exceeds i32".to_string())?;
    let stdout_path =
        take(&mut object, "stdout_path", "evidence command")?.into_string("stdout_path")?;
    let stderr_path =
        take(&mut object, "stderr_path", "evidence command")?.into_string("stderr_path")?;
    let captured_at =
        take(&mut object, "captured_at", "evidence command")?.into_number("captured_at")?;
    Ok(EvidenceCommand::new(
        command,
        exit_code,
        stdout_path,
        stderr_path,
        captured_at,
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
        && !value.contains('\\')
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
